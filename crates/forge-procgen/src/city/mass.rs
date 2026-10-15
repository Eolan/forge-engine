//! Mass models: each lot's building as a footprint and a height, the first stage of D-039's
//! grammar ("a mass model fitted to its lot"). A lot's building fills the strip between the
//! building line (the block's outline moved in by half the width of each street it fronts) and the courtyard (a
//! building's depth further in), measured from the street it faces. Its floors come from its
//! district's range, capped by its street's profile: the tallest on avenues, lower on shared
//! streets, the lowest on pedestrian lanes (heights that follow street widths,
//! `city-generation.md` §15).

use forge_core::SplitMix64;
use glam::DVec2;

use super::bible::StyleBible;
use super::blocks::{BlockUse, edge_half_width};
use super::geom::{self, PointGrid};
use super::{CityPlan, DistrictKind, Profile};

/// What a roof carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Roof {
    /// Planted.
    Green,
    /// Solar panels.
    Solar,
    /// A terrace people use.
    Terrace,
    /// Pitched and tiled, hipped from every side: the old towns' terracotta.
    Tiled,
}

/// A building's mass.
#[derive(Clone, Debug, PartialEq)]
pub struct Mass {
    /// Its lot's index.
    pub lot: u32,
    /// Its footprint, metres on the plan (positive area).
    pub footprint: Vec<DVec2>,
    /// Its floors.
    pub floors: u8,
    /// Each storey's floor to floor, ground first, metres: by its district's era
    /// ([`super::structure::Era::storeys`]).
    pub storeys: Vec<f64>,
    /// Its height to the roof, metres: its storeys' sum.
    pub height: f64,
    /// What its roof carries.
    pub roof: Roof,
    /// The width of the street it fronts, metres.
    pub street_width: f64,
    /// Whether it looks across its street onto a square, a park or a quay.
    pub open: bool,
    /// Whether its ground floor is shops, cafés and services rather than homes
    /// (`active_chance`).
    pub active: bool,
    /// How many of its upper storeys, from the first floor up, are workplaces rather than homes
    /// (`office_chance`): offices, practices and services. All of them make an office building,
    /// whose ground floor is its reception where it has no shops.
    pub offices: u8,
}

impl Mass {
    /// Whether its ground floor is shops, cafés and services: where commerce gathers, on the
    /// main streets, by the tram stops, the squares, the sights and the water, in parades rather
    /// than everywhere (`research/ground-floors.md`; the owner, 2026-10-15: not every ground
    /// floor on a pavement is a shop or a café).
    pub fn lively(&self) -> bool {
        self.active
    }

    /// Whether every upper storey is a workplace: an office building.
    pub fn office_building(&self) -> bool {
        self.storeys.len() >= 2 && usize::from(self.offices) + 1 >= self.storeys.len()
    }

    /// Whether storey `k` (0 the ground) is a workplace: one of its [`Mass::offices`], or an
    /// office building's ground storey without shops.
    pub fn works(&self, k: usize) -> bool {
        (k >= 1 && k <= usize::from(self.offices))
            || (k == 0 && !self.active && self.office_building())
    }

    /// Its ground storey's floor to floor, metres.
    pub fn ground(&self) -> f64 {
        self.storeys.first().copied().unwrap_or(self.height)
    }

    /// Its upper storeys' mean floor to floor, metres (its ground storey's if it has none).
    pub fn upper(&self) -> f64 {
        if self.storeys.len() < 2 {
            self.ground()
        } else {
            (self.height - self.ground()) / (self.storeys.len() - 1) as f64
        }
    }
}

/// The masses of every lot on a street.
pub fn masses(plan: &CityPlan, bible: &StyleBible, seed: u64) -> Vec<Mass> {
    let green_roofs = bible.green_roofs;
    // The streets' points, to find the profile a lot faces.
    let mut streets = PointGrid::new(plan.size, 50.0);
    for (i, e) in plan.roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 10.0) {
            streets.insert(p, i as u32);
        }
    }
    // The squares, parks and quays, by their centres: who looks onto them.
    let mut open_blocks = PointGrid::new(plan.size, 100.0);
    for (i, b) in plan.blocks.iter().enumerate() {
        if matches!(b.use_, BlockUse::Square | BlockUse::Park | BlockUse::Quay) {
            open_blocks.insert(b.centre, i as u32);
        }
    }
    let gaps = open_gaps(plan, seed);
    // Where shops gather (`research/ground-floors.md` §4): the centre (its district's anchor),
    // the tram stops, the sights.
    let hub = plan
        .districts
        .iter()
        .find(|d| d.kind == DistrictKind::Centre)
        .or_else(|| {
            plan.districts
                .iter()
                .find(|d| d.kind == DistrictKind::OldTown)
        })
        .map_or(DVec2::splat(plan.size * 0.5), |d| d.anchor);
    let mut stops = PointGrid::new(plan.size, 100.0);
    for l in &plan.tram_lines {
        for &s in &l.stops {
            stops.insert(s, 0);
        }
    }
    // The interchanges, where offices gather (`research/city-generation.md` §14: offices in a
    // share of upper floors near the tram hubs): a stop with another line's within 150 m.
    let mut interchanges = PointGrid::new(plan.size, 100.0);
    for (i, l) in plan.tram_lines.iter().enumerate() {
        for &s in &l.stops {
            let shared = plan
                .tram_lines
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.stops.iter().any(|&q| q.distance(s) < 150.0));
            if shared {
                interchanges.insert(s, 0);
            }
        }
    }
    let sights: Vec<DVec2> = plan.landmarks.iter().map(|l| l.pos).collect();
    let mut out = Vec::new();
    for (index, lot) in plan.lots.iter().enumerate() {
        if !lot.frontage || gaps.contains(&index) {
            continue;
        }
        let block = &plan.blocks[lot.block as usize];
        // A taxi depot's block holds its deck, no flats; a tram depot's yard none on its lots.
        if block.use_ != BlockUse::Built
            || plan.depots.iter().any(|d| d.block == lot.block)
            || plan
                .tram_depots
                .iter()
                .any(|d| d.block == lot.block && d.covers(&lot.polygon))
        {
            continue;
        }
        let district = &plan.districts[block.district];
        let (lo, hi) = district.floors;
        if hi == 0 {
            continue;
        }
        let outline = &block.buildable;
        let Some((a, t)) = frontage(&lot.polygon, outline) else {
            continue;
        };
        // The lot inside the building line. Lots are cut from the block's buildable outline,
        // already on the building lines; where that outline fell back to the streets'
        // centrelines, each edge the lot touches cuts it by what is left of half its street.
        let mut inside = lot.polygon.clone();
        let n = outline.len();
        let m = block.polygon.len();
        // The edges the lot fronts for 3 m or more, with what is left of their streets' half
        // widths, longest first.
        let mut fronts: Vec<(DVec2, DVec2, f64, f64)> = Vec::new();
        for i in 0..n {
            if inside.len() < 3 {
                break;
            }
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            let edge = (q - p).normalize_or_zero();
            let touches = lot
                .polygon
                .iter()
                .any(|&v| geom::distance_to_segment(v, p, q) < 0.5);
            if edge == DVec2::ZERO || !touches {
                continue;
            }
            let inward = DVec2::new(-edge.y, edge.x);
            let mid = (p + q) * 0.5;
            let moved = (0..m)
                .map(|j| {
                    geom::distance_to_segment(mid, block.polygon[j], block.polygon[(j + 1) % m])
                })
                .fold(f64::MAX, f64::min);
            let half = (edge_half_width(&streets, &plan.roads, p, q) - moved).max(0.0);
            inside = geom::clip_half_plane(&inside, p + inward * half, edge);
            let shared = shared_length(&lot.polygon, p, q);
            if shared >= 3.0 {
                fronts.push((p, q, half, shared));
            }
        }
        fronts.sort_by(|x, y| y.3.total_cmp(&x.3));
        // Strips of a building's depth along each street the lot fronts, the longest frontage
        // first: a lot between two streets is built on both, a corner lot wraps its corner, and
        // the courtyard (inset by the widest street round the block) stays clear. Subtracting
        // the courtyard's convex hull instead ate the buildings along concave and curved sides.
        let pieces = match district.building_depth() {
            Some(depth) => {
                // The strips, the first `extra` metres deeper, with their fronts' indices.
                let strips = |extra: f64| {
                    let mut rest = inside.clone();
                    let mut strips = Vec::new();
                    for (k, &(p, q, half, _)) in fronts.iter().enumerate() {
                        if rest.len() < 3 {
                            break;
                        }
                        let edge = (q - p).normalize_or_zero();
                        let deep = if k == 0 { depth + extra } else { depth };
                        let line = p + DVec2::new(-edge.y, edge.x) * (half + deep);
                        let near = geom::clip_half_plane(&rest, line, -edge);
                        if near.len() >= 3 {
                            strips.push((k, near));
                        }
                        rest = geom::clip_half_plane(&rest, line, edge);
                    }
                    strips
                };
                // A corner lot a little deeper than the first strip leaves a sliver along its
                // second street: the first strip takes it in instead.
                let first = strips(0.0);
                let sliver = first
                    .iter()
                    .filter(|(k, s)| {
                        *k > 0 && geom::signed_area(s).abs() >= 20.0 && thickness(s) < MIN_THICKNESS
                    })
                    .map(|(_, s)| thickness(s))
                    .fold(0.0, f64::max);
                let chosen = if sliver > 0.0 {
                    strips(sliver + 0.5)
                } else {
                    first
                };
                chosen.into_iter().map(|(_, s)| s).collect()
            }
            None => vec![inside],
        };
        let pieces: Vec<Vec<DVec2>> = pieces
            .into_iter()
            .filter(|p| p.len() >= 3 && geom::signed_area(p).abs() >= 20.0)
            // Slivers a strip leaves in the courtyard where the street bends.
            .filter(|p| {
                block
                    .courtyard
                    .as_ref()
                    .is_none_or(|c| !geom::contains(c, geom::centroid(p)))
            })
            .map(|p| {
                if geom::signed_area(&p) < 0.0 {
                    p.into_iter().rev().collect()
                } else {
                    p
                }
            })
            .collect();
        // No wedge to a point: acute corners cut square (the owner saw an impossible building at a
        // block's sharp corner).
        // After the thin strips join their neighbours: a joined piece's hull can bring a sharp
        // corner back.
        let pieces: Vec<Vec<DVec2>> = absorb_thin(
            pieces
                .into_iter()
                .filter(|p| p.len() >= 3 && geom::signed_area(p) >= 20.0)
                .collect(),
        )
        .into_iter()
        // Cleaned first: near-repeated points made false corners that used up its cuts.
        .map(|p| blunt(&geom::clean(&p, 0.01)))
        // A cut wedge can leave a sliver no room fits in: none.
        .filter(|p| p.len() >= 3 && geom::signed_area(p) >= 20.0 && thickness(p) >= MIN_THICKNESS)
        // No repeated points or straight runs: ear clipping stalled on them (a roof lost half
        // its triangles).
        .map(|p| geom::clean(&p, 0.01))
        .filter(|p| p.len() >= 3)
        .collect();
        if pieces.is_empty() {
            continue;
        }
        // The profile of the street the lot faces.
        let front = a + t * 1.0;
        let street = streets.nearest(front, 60.0).map(|(_, e)| e);
        let profile = street.map_or(Profile::Shared, |e| plan.roads.edges[e as usize].profile);
        let mut rng = SplitMix64::new(seed ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let span = u64::from(hi.saturating_sub(lo)) + 1;
        let drawn = lo + rng.below(span) as u8;
        // Heights follow street widths: the bible's ratio caps the floors.
        let street_width = profile.width();
        // Across the street from the middle of the lot's front: a square, a park or a quay?
        let open = {
            let c = geom::centroid(&lot.polygon);
            let on = a + t * (c - a).dot(t);
            let inward = DVec2::new(-t.y, t.x);
            let outward = if geom::contains(&block.polygon, on + inward * 2.0) {
                -inward
            } else {
                inward
            };
            let across = on + outward * (street_width + 8.0);
            // The square, park or quay across the street, if any.
            let mut facing = None;
            open_blocks.for_each_within(across, 1500.0, |_, b| {
                let block = &plan.blocks[b as usize];
                if facing.is_none() && geom::contains(&block.polygon, across) {
                    facing = Some(block.use_);
                }
            });
            facing
        };
        // Its ground floor: shops, cafés and services, or homes (`research/ground-floors.md`:
        // shops gather on the main streets, by the stops, the squares, the sights and the water;
        // they were on every avenue and lane, the owner: not every ground floor is a shop).
        let active = {
            let c = geom::centroid(&lot.polygon);
            let chance = active_chance(Frontage {
                profile,
                district: district.kind,
                facing: open,
                from_hub: c.distance(hub),
                from_heart: c.distance(district.anchor),
                stop: [100.0, 250.0].map(|r| stops.any_within(c, r, |_| false)),
                sight: sights.iter().any(|s| s.distance(c) < 200.0),
            });
            // Drawn by stretches of about 120 m of a street, then a little by the lot: the shops
            // of a street gather in parades rather than one here, one there.
            let stretch = street.map_or(0, |e| {
                let start = plan.roads.edges[e as usize].points[0];
                (u64::from(e) << 16) ^ (front.distance(start) / 120.0) as u64
            });
            let draw = 0.7 * unit(seed ^ 0xAC71 ^ stretch) + 0.3 * unit(seed ^ index as u64);
            draw < chance
        };
        let open = open.is_some();
        // Its storeys by its district's era (D-060): shops' or homes' ground storey, then the
        // uppers; the bible's ratio caps how many fit, and the height is their sum.
        let era = super::structure::Era::of(district.kind);
        let shops = active;
        let cap = bible.max_floors(street_width, era, shops);
        let floors = drawn.min(cap).max(1);
        let storeys = era.storeys(floors, shops);
        let height = storeys.iter().sum();
        // Its workplaces upstairs (D-060 step 3): offices over the shops on the main streets,
        // whole office buildings by the interchanges. Drawn by block, then a little by the lot,
        // so offices gather; from R+2 up.
        let offices = {
            let c = geom::centroid(&lot.polygon);
            let first = office_chance(Frontage {
                profile,
                district: district.kind,
                facing: None,
                from_hub: c.distance(hub),
                from_heart: c.distance(district.anchor),
                stop: [100.0, 250.0].map(|r| stops.any_within(c, r, |_| false)),
                sight: false,
            });
            let hub_near = [150.0, 400.0].map(|r| interchanges.any_within(c, r, |_| false));
            let whole = first * WHOLE_OFFICE
                + if hub_near[0] {
                    0.35
                } else if hub_near[1] {
                    0.15
                } else {
                    0.0
                };
            // A block's draw and the lot's mixed, made even again so a chance is what it says.
            let draw = even_mix(
                unit(seed ^ 0x0FF1CE ^ u64::from(lot.block)),
                unit(seed ^ 0xB0A2D ^ index as u64),
            );
            let upper = floors.saturating_sub(1);
            if floors < 3 || draw >= (whole + first).min(0.9) {
                0
            } else if draw < whole {
                upper
            } else {
                // The first floor, and the second over five storeys.
                upper.min(1 + u8::from(floors > 5))
            }
        };
        let r = rng.next_f64();
        let tiled = rng.next_f64() < district.tiled_share();
        let roof = if tiled {
            Roof::Tiled
        } else if r < green_roofs {
            Roof::Green
        } else if r < green_roofs + (1.0 - green_roofs) * 0.6 {
            Roof::Solar
        } else {
            Roof::Terrace
        };
        for footprint in pieces {
            out.push(Mass {
                lot: index as u32,
                footprint,
                floors,
                storeys: storeys.clone(),
                height,
                roof,
                street_width,
                open,
                active,
                offices,
            });
        }
    }
    out
}

/// What a lot's street front is, for the chance its ground floor is active.
struct Frontage {
    /// The street it faces.
    profile: Profile,
    /// Its district's kind.
    district: DistrictKind,
    /// The open block across the street: a square, a park or a quay.
    facing: Option<BlockUse>,
    /// How far it lies from the city's centre, metres.
    from_hub: f64,
    /// How far it lies from its district's heart (its anchor), metres.
    from_heart: f64,
    /// Whether a tram stop lies within 100 m, and within 250 m.
    stop: [bool; 2],
    /// Whether a sight lies within 200 m.
    sight: bool,
}

/// The chance that a ground floor on `f` is shops, cafés and services (`research/ground-floors.md`
/// §4.2–4.3, all estimates there): by the street (a central high street 0.9, a tram avenue
/// 0.65, a taxi road 0.35, an old town's lane 0.55, another pedestrian street 0.1), or what it
/// faces (a square 0.9 in the centre, 0.6 elsewhere; the water 0.8 and 0.4; a park 0.35); then
/// less far from the centre (×0.8 at 3 km, ×0.6 past 5 km), more by a tram stop (+0.15 within
/// 100 m, +0.07 within 250 m) and a sight (+0.15), at most 0.95.
fn active_chance(f: Frontage) -> f64 {
    let central = matches!(f.district, DistrictKind::Centre) || f.from_hub < 1000.0;
    let old = matches!(f.district, DistrictKind::Centre | DistrictKind::OldTown);
    let by_street = match f.profile {
        Profile::TramAvenue if central => 0.9,
        Profile::TramAvenue => 0.65,
        Profile::Avenue if central => 0.75,
        Profile::Avenue => 0.35,
        Profile::Pedestrian if old => 0.55,
        Profile::Pedestrian => 0.1,
        Profile::Shared if old => 0.35,
        Profile::Shared => 0.12,
        Profile::Path => 0.0,
    };
    let by_facing = match f.facing {
        Some(BlockUse::Square) if central => 0.9,
        Some(BlockUse::Square) => 0.6,
        Some(BlockUse::Quay) if central => 0.8,
        Some(BlockUse::Quay) => 0.4,
        Some(BlockUse::Park) => 0.35,
        _ => 0.0,
    };
    let mut chance = f64::max(by_street, by_facing);
    // Round a district's heart, its own shopping streets: the neighbourhood's parades.
    if f.from_heart < 400.0 && f.profile != Profile::Path {
        chance = chance.max(0.5);
    }
    if f.district == DistrictKind::Industrial {
        chance *= 0.4;
    }
    let far = ((f.from_hub - 1000.0) / 2000.0).clamp(0.0, 1.0) * 0.2
        + ((f.from_hub - 3000.0) / 2000.0).clamp(0.0, 1.0) * 0.2;
    chance *= 1.0 - far;
    if f.stop[0] {
        chance += 0.15;
    } else if f.stop[1] {
        chance += 0.07;
    }
    if f.sight {
        chance += 0.15;
    }
    chance.clamp(0.0, 0.95)
}

/// The share of a lot's [`office_chance`] that makes it a whole office building as well, on top
/// of the interchanges' (+0.35 within 150 m, +0.15 within 400 m).
const WHOLE_OFFICE: f64 = 0.35;

/// `0.6 a + 0.4 b` for two even draws, made even again (its distribution function): the block's
/// share keeps neighbours alike, and a chance stays the chance it says. The sum alone made a
/// 15 % chance come out at 5 %.
fn even_mix(a: f64, b: f64) -> f64 {
    let s = 0.6 * a + 0.4 * b;
    if s < 0.4 {
        s * s / 0.48
    } else if s < 0.6 {
        (s - 0.2) / 0.6
    } else {
        1.0 - (1.0 - s) * (1.0 - s) / 0.48
    }
}

/// The chance that a lot's building has workplaces on its first floors: offices, practices and
/// services over the shops or the homes' hall (D-060 step 3; `research/city-generation.md` §14:
/// uses by floor and frontage, offices in a share of upper floors near the tram hubs; Euronantes
/// and Part-Dieu by their stations). All estimates: by the street (a central tram avenue 0.6, a
/// tram avenue 0.4, a central avenue 0.5, an avenue 0.3, a central lane or shared street 0.3,
/// elsewhere 0.15), the converted halls' quarter more (×1.3, the Île de Nantes), the old town and
/// the eco-quarters less (×0.6), the villages hardly (×0.2); less far from the centre, more by a
/// tram stop (+0.1 within 100 m).
fn office_chance(f: Frontage) -> f64 {
    let central = matches!(f.district, DistrictKind::Centre) || f.from_hub < 1000.0;
    let by_street = match f.profile {
        Profile::TramAvenue if central => 0.6,
        Profile::TramAvenue => 0.4,
        Profile::Avenue if central => 0.5,
        Profile::Avenue => 0.3,
        Profile::Pedestrian | Profile::Shared if central => 0.3,
        Profile::Pedestrian | Profile::Shared => 0.15,
        Profile::Path => 0.0,
    };
    let by_district = match f.district {
        DistrictKind::Industrial => 1.3,
        DistrictKind::OldTown | DistrictKind::Eco => 0.6,
        DistrictKind::Village => 0.2,
        _ => 1.0,
    };
    let far = ((f.from_hub - 1500.0) / 3000.0).clamp(0.0, 0.5);
    let mut chance = by_street * by_district * (1.0 - far);
    if f.stop[0] {
        chance += 0.1;
    }
    chance.clamp(0.0, 0.9)
}

/// A number in 0..1 from `key`.
fn unit(key: u64) -> f64 {
    let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Under this run (metres) an open-block district opens its corners too.
const OPEN_CORNERS: f64 = 60.0;

/// Open blocks (`DistrictSpec::gaps`): the lots left as gardens between the buildings, so light,
/// views and people reach the courtyards (the owner found the long rows of big buildings too
/// close, 2026-10-11). Each block's street lots in order round its outline: a lot between its
/// corners (one street front) becomes a gap at the district's share, and wherever the front
/// has run its district's [`super::DistrictSpec::run_max`] since the last gap; never two side by
/// side. Where that run is under [`OPEN_CORNERS`], a corner lot may be a gap too: separate
/// buildings round a shared green (the owner: open and organic, not oppressive).
fn open_gaps(plan: &CityPlan, seed: u64) -> std::collections::HashSet<usize> {
    let mut by_block: std::collections::HashMap<u32, Vec<usize>> = Default::default();
    for (i, lot) in plan.lots.iter().enumerate() {
        if lot.frontage {
            by_block.entry(lot.block).or_default().push(i);
        }
    }
    let mut gaps = std::collections::HashSet::new();
    let mut blocks: Vec<_> = by_block.into_iter().collect();
    blocks.sort_by_key(|(b, _)| *b);
    for (b, lots) in blocks {
        let block = &plan.blocks[b as usize];
        let district = &plan.districts[block.district];
        let (share, run_max) = (district.gap_share(), district.run_max());
        let corners = run_max < OPEN_CORNERS;
        if share <= 0.0 || block.use_ != BlockUse::Built {
            continue;
        }
        let outline = &block.buildable;
        let n = outline.len();
        // Where each edge starts along the outline.
        let mut start = Vec::with_capacity(n);
        let mut s = 0.0;
        for i in 0..n {
            start.push(s);
            s += outline[i].distance(outline[(i + 1) % n]);
        }
        // Each lot: where its front lies along the outline, its length, its street edges.
        let mut fronts: Vec<(f64, f64, usize, usize)> = lots
            .iter()
            .filter_map(|&l| {
                let poly = &plan.lots[l].polygon;
                let shared: Vec<(usize, f64)> = (0..n)
                    .map(|i| (i, shared_length(poly, outline[i], outline[(i + 1) % n])))
                    .filter(|&(_, len)| len >= 3.0)
                    .collect();
                let &(edge, length) = shared.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
                let (a, q) = (outline[edge], outline[(edge + 1) % n]);
                let t = (q - a).normalize_or_zero();
                let along = start[edge] + (geom::centroid(poly) - a).dot(t);
                Some((along, length, shared.len(), l))
            })
            .collect();
        fronts.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut run = 0.0;
        let mut last = false;
        for (_, length, edges, l) in fronts {
            let draw =
                SplitMix64::new(seed ^ (l as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)).next_f64();
            if (edges == 1 || corners) && !last && (draw < share || run >= run_max) {
                gaps.insert(l);
                run = 0.0;
                last = true;
            } else {
                run += length;
                last = false;
            }
        }
    }
    gaps
}

/// The thinnest a building stands, metres.
const MIN_THICKNESS: f64 = 4.0;

/// A piece's thickness: the lesser of its extents along and across its longest edge.
fn thickness(piece: &[DVec2]) -> f64 {
    let (_, extent, _) = geom::oriented_extent(piece);
    extent.x.min(extent.y)
}

/// The sharpest corner a building keeps, radians: 55°.
const SHARPEST: f64 = 55.0 * std::f64::consts::PI / 180.0;
/// How wide an acute corner's cut is, metres: a room's width.
const BLUNT: f64 = 6.0;

/// A footprint (positive area) with each corner sharper than [`SHARPEST`] cut square to its
/// bisector where the wedge is [`BLUNT`] wide: a chamfered corner, not a wedge to a point.
fn blunt(polygon: &[DVec2]) -> Vec<DVec2> {
    // Each sharp corner replaced by two points equally far along its two edges, the cut square
    // to its bisector; only the corner moves, so a concave outline stays sound (a clip across
    // the whole outline could leave a bridge).
    let mut out = polygon.to_vec();
    for _ in 0..8 {
        let n = out.len();
        if n < 3 {
            break;
        }
        let acute = (0..n).find_map(|i| {
            let (prev, v, next) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
            let (a, b) = (
                (prev - v).normalize_or_zero(),
                (next - v).normalize_or_zero(),
            );
            let convex = geom::cross(v - prev, next - v) > 0.0;
            let angle = a.angle_to(b).abs();
            let room = 0.45 * v.distance(prev).min(v.distance(next));
            (convex && angle > 1e-3 && angle < SHARPEST).then_some((i, v, a, b, angle, room))
        });
        let Some((i, v, a, b, angle, room)) = acute else {
            break;
        };
        let s = (BLUNT * 0.5 / (angle * 0.5).sin()).min(room);
        out.splice(i..=i, [v + a * s, v + b * s]);
    }
    out
}

/// A lot's strips with each thinner than [`MIN_THICKNESS`] (what is left along a second street
/// past the first strip, at a corner) joined to another of them where the two make a convex
/// piece (their hull adds under 3 %), or dropped.
fn absorb_thin(mut pieces: Vec<Vec<DVec2>>) -> Vec<Vec<DVec2>> {
    let mut i = 0;
    while i < pieces.len() {
        if thickness(&pieces[i]) >= MIN_THICKNESS {
            i += 1;
            continue;
        }
        let thin = pieces.remove(i);
        let area = geom::signed_area(&thin);
        let hull = |other: &[DVec2]| {
            let points: Vec<DVec2> = other.iter().chain(&thin).copied().collect();
            let hull = geom::convex_hull(&points);
            (geom::signed_area(&hull) <= (geom::signed_area(other) + area) * 1.03).then_some(hull)
        };
        if let Some((j, joined)) = (0..pieces.len()).find_map(|j| hull(&pieces[j]).map(|h| (j, h)))
        {
            pieces[j] = joined;
        }
    }
    pieces
}

/// How much of a lot's outline lies along the block edge from `a` to `b`, metres.
fn shared_length(lot: &[DVec2], a: DVec2, b: DVec2) -> f64 {
    let m = lot.len();
    let mut shared = 0.0;
    for j in 0..m {
        let (p, q) = (lot[j], lot[(j + 1) % m]);
        if geom::distance_to_segment(p, a, b) < 0.5 && geom::distance_to_segment(q, a, b) < 0.5 {
            shared += p.distance(q);
        }
    }
    shared
}

/// The block edge a lot fronts on: its start and unit direction, the edge sharing the longest
/// stretch with the lot's outline.
fn frontage(lot: &[DVec2], block: &[DVec2]) -> Option<(DVec2, DVec2)> {
    let n = block.len();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..n {
        let (a, b) = (block[i], block[(i + 1) % n]);
        let shared = shared_length(lot, a, b);
        if shared > 1.0 && best.is_none_or(|(s, _)| shared > s) {
            best = Some((shared, i));
        }
    }
    let (_, i) = best?;
    let (a, b) = (block[i], block[(i + 1) % n]);
    Some((a, (b - a).normalize_or_zero()))
}

#[cfg(test)]
mod tests {
    use super::super::{example, plan};
    use super::*;
    use crate::city::bible::StyleBible;

    #[test]
    fn open_blocks_leave_gaps_between_their_corners() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let mut params = example::params(size);
        let closed = plan(&site, &params);
        for d in &mut params.districts {
            d.gaps = Some(0.1);
        }
        let open = plan(&site, &params);
        let gaps = open_gaps(&open, 9);
        assert!(!gaps.is_empty());
        // Gaps fall between a block's corners: a lot with one street front.
        for &l in &gaps {
            let lot = &open.lots[l];
            let outline = &open.blocks[lot.block as usize].buildable;
            let n = outline.len();
            let fronts = (0..n)
                .filter(|&i| shared_length(&lot.polygon, outline[i], outline[(i + 1) % n]) >= 3.0)
                .count();
            assert_eq!(fronts, 1);
        }
        let style = StyleBible::default();
        let (with, without) = (
            masses(&open, &style, 9).len(),
            masses(&closed, &style, 9).len(),
        );
        assert!(with < without, "{with} masses with gaps, {without} without");
        assert_eq!(closed.lots.len(), open.lots.len());
    }

    #[test]
    fn the_buildings_round_the_squares_and_quays_open_onto_them() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let open: Vec<_> = masses.iter().filter(|m| m.open).collect();
        let share = open.len() as f64 / masses.len() as f64;
        assert!((0.02..0.4).contains(&share), "{share:.3} open");
        // Each within its street's width and a little of a square, a park or a quay.
        let near = |m: &Mass| {
            let c = geom::centroid(&m.footprint);
            city.blocks.iter().any(|b| {
                matches!(b.use_, BlockUse::Square | BlockUse::Park | BlockUse::Quay)
                    && (0..b.polygon.len()).any(|i| {
                        let (p, q) = (b.polygon[i], b.polygon[(i + 1) % b.polygon.len()]);
                        geom::distance_to_segment(c, p, q) < m.street_width + 40.0
                    })
            })
        };
        assert!(open.iter().all(|m| near(m)));
        // Most of them shops and cafés, more than along the plain streets.
        let active = |list: &[&Mass]| {
            list.iter().filter(|m| m.active).count() as f64 / list.len().max(1) as f64
        };
        let plain: Vec<_> = masses.iter().filter(|m| !m.open).collect();
        assert!(
            active(&open) > 0.3,
            "{:.2} of the open fronts active",
            active(&open)
        );
        assert!(
            active(&open) > active(&plain) + 0.1,
            "open fronts {:.2}, plain {:.2}",
            active(&open),
            active(&plain)
        );
    }

    #[test]
    fn shops_gather_where_people_pass_and_leave_most_ground_floors_homes() {
        // `research/ground-floors.md`: shops on the main streets, by the stops and the squares;
        // a city's ground floors are not all shops (the owner).
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let share = |profile: Option<Profile>| {
            let list: Vec<&Mass> = masses
                .iter()
                .filter(|m| !m.open && profile.is_none_or(|p| p.width() == m.street_width))
                .collect();
            let active = list.iter().filter(|m| m.active).count();
            (active as f64 / list.len().max(1) as f64, list.len())
        };
        let (all, _) = share(None);
        let (tram, n_tram) = share(Some(Profile::TramAvenue));
        let (shared, n_shared) = share(Some(Profile::Shared));
        println!(
            "active ground floors: {:.0} % of all, {:.0} % on the tram avenues ({n_tram}), {:.0} % on the shared streets ({n_shared})",
            all * 100.0,
            tram * 100.0,
            shared * 100.0
        );
        assert!(
            (0.1..0.6).contains(&all),
            "{all:.2} of the ground floors active"
        );
        assert!(
            tram > shared + 0.3,
            "tram avenues {tram:.2}, shared streets {shared:.2}"
        );
        // In parades: an active building's street neighbour is more often active than chance.
        let mut grid = PointGrid::new(size, 50.0);
        for (k, m) in masses.iter().enumerate() {
            grid.insert(geom::centroid(&m.footprint), k as u32);
        }
        let (mut pairs, mut both) = (0, 0);
        for m in masses.iter().filter(|m| m.active) {
            let c = geom::centroid(&m.footprint);
            grid.for_each_within(c, 25.0, |_, k| {
                let o = &masses[k as usize];
                if o.lot != m.lot && o.street_width == m.street_width {
                    pairs += 1;
                    both += usize::from(o.active);
                }
            });
        }
        let together = both as f64 / pairs.max(1) as f64;
        println!(
            "an active building's neighbour active: {:.0} %",
            together * 100.0
        );
        assert!(
            together > all + 0.15,
            "{together:.2} against {all:.2}: not in parades"
        );
    }

    #[test]
    fn masses_stand_between_the_building_line_and_the_courtyard() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        assert!(
            masses.len() > city.lots.len() / 2,
            "{} of {}",
            masses.len(),
            city.lots.len()
        );
        for m in masses.iter().take(2000) {
            let lot = &city.lots[m.lot as usize];
            let block = &city.blocks[lot.block as usize];
            assert!((m.height - m.storeys.iter().sum::<f64>()).abs() < 1e-9 && m.height >= 3.0);
            assert!(geom::signed_area(&m.footprint) > 0.0);
            let c = geom::centroid(&m.footprint);
            assert!(
                geom::contains(&block.polygon, c),
                "a mass outside its block"
            );
            if let Some(courtyard) = &block.courtyard {
                assert!(!geom::contains(courtyard, c), "a mass in its courtyard");
            }
        }
        // No sliver of a building: what is left past a corner strip joins it.
        let thinnest = masses
            .iter()
            .map(|m| thickness(&m.footprint))
            .fold(f64::MAX, f64::min);
        assert!(thinnest >= MIN_THICKNESS, "a mass {thinnest:.1} m thick");
        let green = masses.iter().filter(|m| m.roof == Roof::Green).count();
        assert!(green * 3 > masses.len(), "about half the roofs are green");
    }

    #[test]
    fn no_building_comes_to_a_point() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let mut sharp = 0;
        for m in &masses {
            let n = m.footprint.len();
            for i in 0..n {
                let (prev, v, next) = (
                    m.footprint[(i + n - 1) % n],
                    m.footprint[i],
                    m.footprint[(i + 1) % n],
                );
                let (a, b) = (
                    (prev - v).normalize_or_zero(),
                    (next - v).normalize_or_zero(),
                );
                if geom::cross(v - prev, next - v) > 0.0 && a.angle_to(b).abs() < SHARPEST - 1e-6 {
                    sharp += 1;
                }
            }
        }
        assert_eq!(sharp, 0, "{sharp} corners sharper than 55°");
    }

    #[test]
    fn every_footprint_triangulates_whole() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = masses(&city, &StyleBible::default(), 9);
        let mut bad = Vec::new();
        for (i, m) in masses.iter().enumerate() {
            let area = geom::signed_area(&m.footprint).abs();
            let covered: f64 = geom::triangulate(&m.footprint)
                .iter()
                .map(|t| geom::signed_area(&t.map(|k| m.footprint[k])).abs())
                .sum();
            if (covered - area).abs() > 0.01 * area {
                bad.push((i, area, covered));
            }
        }
        assert!(
            bad.is_empty(),
            "{} footprints not covered, e.g. {:?}",
            bad.len(),
            &bad[..bad.len().min(5)]
        );
    }
}
