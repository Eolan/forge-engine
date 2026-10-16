//! The buildings' flats and shops (D-060, step 3): each storey's floor cut into units round its
//! stair cores, as French housing does it ("2 à 4 logements par palier"):
//! - each core serves the floor halfway to the next one along the building, its territory;
//! - behind the core a lobby 1.40 m deep continues the landing, where the doors open;
//! - on each side of the core a flat runs through from the street to the court, two aspects;
//! - behind the lobby, where the building is deep enough, a smaller flat looks onto the court;
//! - a side too big for one flat (over [`MAX_THROUGH`]) has a corridor continuing the lobby, with
//!   single-aspect flats either side of it, sized by the French mix of new homes;
//! - a narrow house's compact core (the lift behind the hall) serves one flat a storey.
//!
//! A lively ground storey holds shops instead, entered from the street, and the storeys its mass
//! keeps for work hold offices ([`Mass::works`]): the same cut, to let. A unit with no window
//! on its storey is a store room, not a home. A small wing without a core is reached through
//! its building's other mass.

use glam::DVec2;

use super::cores::Core;
use super::geom;
use super::mass::Mass;
use super::structure::{OpeningKind, Structure, WallRole};

/// The lobby behind a core, where the flats' doors open: as deep as a common corridor (1.20 m)
/// and a little more to turn, metres.
pub const LOBBY: f64 = 1.4;
/// The smallest home, m² net (a studio; the décence decree's floor is 9 m²).
pub const MIN_FLAT: f64 = 25.0;
/// The largest flat running through from street to court before its side gets a corridor, m²
/// net (a T5 is about 100 m²).
pub const MAX_THROUGH: f64 = 110.0;
/// A flat's narrowest width along a corridor, metres.
pub const MIN_WIDTH: f64 = 3.6;
/// The new homes' mix in France (2013, the research note): T1 to T5's shares and their net
/// areas, m².
pub const MIX: [(f64, f64); 5] = [
    (0.10, 30.0),
    (0.25, 46.0),
    (0.38, 66.0),
    (0.22, 86.0),
    (0.05, 100.0),
];

/// What a unit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// A home with this many main rooms (a living room and bedrooms): T1 to T5.
    Home(u8),
    /// A shop, a café or a bar on a lively ground storey, entered from the street.
    Shop,
    /// Offices, a practice or a service upstairs, or an office building's reception: on a
    /// storey its mass keeps for work ([`Mass::works`]).
    Office,
    /// A room without a window: a store room, a bike room.
    Store,
}

/// A unit of a storey.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// Its storey.
    pub storey: usize,
    /// The core whose landing it opens on, an index into the structure's cores (`None`: a shop
    /// entered from the street, a one-storey building, or a wing reached through its building's
    /// other mass).
    pub core: Option<usize>,
    /// Its floor, pieces on the plan inside the walls.
    pub pieces: Vec<Vec<DVec2>>,
    /// Its net floor area, less the partitions, m².
    pub net: f64,
    /// Where its door is: on the lobby or the corridor of its core's landing.
    pub door: DVec2,
    /// What it is.
    pub use_: Use,
    /// Its windows, shopfronts and doors to the outside on its storey.
    pub windows: usize,
    /// Whether they look onto both the street and the court (a through flat).
    pub through: bool,
}

/// A storey's lobbies and corridors: where the flats' doors open, m² on the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    /// Its storey.
    pub storey: usize,
    /// Its core.
    pub core: usize,
    /// Its pieces.
    pub pieces: Vec<Vec<DVec2>>,
}

/// A frame on the plan: `u` along a wall from its `a`, `n` inwards from its inner face.
#[derive(Clone, Copy, Debug)]
struct Frame {
    origin: DVec2,
    t: DVec2,
    n: DVec2,
}

impl Frame {
    fn of(s: &Structure, c: &Core) -> Self {
        let w = &s.walls[c.wall];
        let t = (w.b - w.a).normalize_or_zero();
        let n = DVec2::new(-t.y, t.x);
        Self {
            origin: w.a + n * w.thickness,
            t,
            n,
        }
    }

    fn at(&self, u: f64, d: f64) -> DVec2 {
        self.origin + self.t * u + self.n * d
    }

    fn u(&self, p: DVec2) -> f64 {
        (p - self.origin).dot(self.t)
    }

    /// The part of `poly` with `u` in `u0..u1` and `n` in `d0..d1`.
    fn slice(&self, poly: &[DVec2], (u0, u1): (f64, f64), (d0, d1): (f64, f64)) -> Vec<DVec2> {
        // Keeps the side of the line through `o` where `(p − o) · w ≥ 0`.
        let keep = |poly: &[DVec2], o: DVec2, w: DVec2| {
            if poly.len() < 3 {
                return Vec::new();
            }
            geom::clip_half_plane(poly, o, DVec2::new(w.y, -w.x))
        };
        let p = keep(poly, self.at(u0, 0.0), self.t);
        let p = keep(&p, self.at(u1, 0.0), -self.t);
        let p = keep(&p, self.at(0.0, d0), self.n);
        let p = keep(&p, self.at(0.0, d1), -self.n);
        if p.len() >= 3 && geom::signed_area(&p).abs() > 0.05 {
            p
        } else {
            Vec::new()
        }
    }
}

/// Far enough to reach past any floor, metres.
const FAR: f64 = 1.0e4;

/// The units and landings of a mass's storeys. `floor` is its floor inside the walls;
/// `partitions` the share the partitions take; `seed` draws the flats' sizes.
pub fn units(
    m: &Mass,
    s: &Structure,
    floor: &[DVec2],
    partitions: f64,
    seed: u64,
) -> (Vec<Unit>, Vec<Landing>) {
    let mut units = Vec::new();
    let mut landings = Vec::new();
    let area = |pieces: &[Vec<DVec2>]| -> f64 {
        pieces
            .iter()
            .map(|p| geom::signed_area(p).abs())
            .sum::<f64>()
            * (1.0 - partitions)
    };
    for k in 0..s.storeys.len() {
        let shops = k == 0 && m.lively();
        let work = !shops && m.works(k);
        let use_ = if shops {
            Use::Shop
        } else if work {
            Use::Office
        } else {
            Use::Home(0)
        };
        if s.cores.is_empty() {
            // One unit a storey: a house, a shop, or a wing reached through its other mass.
            let pieces = vec![floor.to_vec()];
            let door = s
                .openings
                .iter()
                .find(|o| o.storey == 0 && o.kind == OpeningKind::Door)
                .map_or_else(|| geom::centroid(floor), |o| point(s, o.wall, o.along));
            units.push(Unit {
                storey: k,
                core: None,
                net: area(&pieces),
                pieces,
                door,
                use_,
                windows: 0,
                through: false,
            });
            continue;
        }
        // Each core's territory: the floor nearer it than any other core (cut by the bisectors
        // between their middles), so cores on a corner's two walls share it too.
        let middles: Vec<DVec2> = s
            .cores
            .iter()
            .map(|c| (c.outline[0] + c.outline[1] + c.outline[2] + c.outline[3]) * 0.25)
            .collect();
        for ci in 0..s.cores.len() {
            let c = &s.cores[ci];
            let mut territory = floor.to_vec();
            for (cj, &other) in middles.iter().enumerate() {
                if cj == ci || territory.len() < 3 {
                    continue;
                }
                let towards = middles[ci] - other;
                let w = DVec2::new(towards.y, -towards.x);
                territory = geom::clip_half_plane(&territory, (middles[ci] + other) * 0.5, w);
            }
            if territory.len() < 3 {
                continue;
            }
            let floor = territory.as_slice();
            // In the core's own frame: its wall's direction, its depth from that wall's face.
            let local = Frame::of(s, c);
            let (lo, hi) = (-FAR, FAR);
            let (u0, u1) = c.span;
            let behind = c.deep;
            let compact = c.house || c.hall[0].distance(c.hall[3]) < c.deep - 0.1;
            let hall = (c.hall[0] + c.hall[1] + c.hall[2] + c.hall[3]) * 0.25;
            let sides = [
                local.slice(floor, (lo, u0), (-FAR, FAR)),
                local.slice(floor, (u1, hi), (-FAR, FAR)),
            ];
            let back = local.slice(floor, (u0, u1), (behind, FAR));
            let mut made: Vec<Unit> = Vec::new();
            let flat = |pieces: Vec<Vec<DVec2>>, door: DVec2, made: &mut Vec<Unit>| {
                let pieces: Vec<Vec<DVec2>> = pieces.into_iter().filter(|p| p.len() >= 3).collect();
                if pieces.is_empty() {
                    return;
                }
                made.push(Unit {
                    storey: k,
                    core: Some(ci),
                    net: area(&pieces),
                    pieces,
                    door,
                    use_,
                    windows: 0,
                    through: false,
                });
            };
            let total = area(&sides) + area(std::slice::from_ref(&back));
            if compact || total < 2.0 * MIN_FLAT + 10.0 {
                // One flat round the core: its door on the hall.
                let mut pieces = sides.to_vec();
                pieces.push(back);
                flat(pieces, hall, &mut made);
            } else {
                // The lobby behind the core, and the flat behind the lobby where there is room.
                let lobby = local.slice(floor, (u0, u1), (behind, behind + LOBBY));
                let rear = local.slice(floor, (u0, u1), (behind + LOBBY, FAR));
                let rear_ok = area(std::slice::from_ref(&rear)) >= MIN_FLAT;
                let mid = (u0 + u1) * 0.5;
                if rear_ok {
                    flat(vec![rear], local.at(mid, behind + LOBBY), &mut made);
                }
                let mut corridors = vec![lobby];
                for (side, piece) in sides.iter().enumerate() {
                    // The side's share behind the core when no flat stands there.
                    let extra = if rear_ok {
                        Vec::new()
                    } else if side == 0 {
                        local.slice(floor, (u0, mid), (behind + LOBBY, FAR))
                    } else {
                        local.slice(floor, (mid, u1), (behind + LOBBY, FAR))
                    };
                    let edge = if side == 0 { u0 } else { u1 };
                    let door = local.at(edge, behind + LOBBY * 0.5);
                    let size = area(&[piece.clone(), extra.clone()]);
                    if size <= MAX_THROUGH {
                        flat(vec![piece.clone(), extra], door, &mut made);
                        continue;
                    }
                    // Too big for one: a corridor on from the lobby, flats either side of it,
                    // and the share behind the core one more (or a neighbour's, if small).
                    flat(vec![extra], door, &mut made);
                    let (a, b) = if side == 0 { (lo, u0) } else { (u1, hi) };
                    corridors.push(local.slice(floor, (a, b), (behind, behind + LOBBY)));
                    for band in [(-FAR, behind), (behind + LOBBY, FAR)] {
                        let strip = local.slice(floor, (a, b), band);
                        if strip.is_empty() {
                            continue;
                        }
                        let (s0, s1) = strip
                            .iter()
                            .map(|&p| local.u(p))
                            .fold((f64::MAX, f64::MIN), |(x, y), u| (x.min(u), y.max(u)));
                        let deep = area(std::slice::from_ref(&strip)) / (s1 - s0).max(0.1);
                        // From the core outwards, flats drawn from the mix.
                        let mut cuts = Vec::new();
                        let mut at = if side == 0 { s1 } else { s0 };
                        let mut n = 0_u64;
                        loop {
                            let left = if side == 0 { at - s0 } else { s1 - at };
                            let key = seed
                                ^ ((k as u64) << 40)
                                ^ ((ci as u64) << 32)
                                ^ ((side as u64) << 24)
                                ^ n;
                            let want = (draw(key) / deep.max(1.0)).max(MIN_WIDTH);
                            n += 1;
                            if left - want < MIN_WIDTH {
                                cuts.push((at, if side == 0 { s0 } else { s1 }));
                                break;
                            }
                            let next = if side == 0 { at - want } else { at + want };
                            cuts.push((at, next));
                            at = next;
                        }
                        let d = if band.0 < 0.0 { behind } else { behind + LOBBY };
                        for (x, y) in cuts {
                            let (x, y) = (x.min(y), x.max(y));
                            let piece = local.slice(floor, (x, y), band);
                            flat(vec![piece], local.at((x + y) * 0.5, d), &mut made);
                        }
                    }
                }
                landings.push(Landing {
                    storey: k,
                    core: ci,
                    pieces: corridors.into_iter().filter(|p| p.len() >= 3).collect(),
                });
            }
            // A flat too small to live in joins its nearest neighbour.
            merge_small(&mut made);
            units.extend(made);
        }
    }
    // Windows, aspects and uses.
    for u in &mut units {
        let (mut street, mut court) = (false, false);
        for o in s.openings.iter().filter(|o| o.storey == u.storey) {
            if o.kind == OpeningKind::Passage {
                continue;
            }
            let w = &s.walls[o.wall];
            let t = (w.b - w.a).normalize_or_zero();
            let inside = w.a + t * o.along + DVec2::new(-t.y, t.x) * (w.thickness + 0.3);
            if u.pieces.iter().any(|p| geom::contains(p, inside)) {
                u.windows += 1;
                street |= w.role == WallRole::Street;
                court |= w.role == WallRole::Court;
            }
        }
        u.through = street && court;
        if u.use_ == Use::Home(0) {
            u.use_ = if u.windows == 0 && u.core.is_some() {
                Use::Store
            } else {
                Use::Home(rooms(u.net))
            };
        }
    }
    (units, landings)
}

/// Main rooms a person in a French flat: 1.5 (INSEE, *Les conditions de logement fin 2013*,
/// Insee Première 1546), with 2.9 rooms a flat, so about 1.9 people a flat.
pub const ROOMS_A_PERSON: f64 = 1.5;
/// The share of homes standing empty: 5 %, the design value of `research/city-generation.md`'s
/// table per 1 000 residents.
pub const VACANT: f64 = 0.05;
/// Net office floor a job, m²: the British Council for Offices' 15 m² an employee
/// (`research/city-generation.md`).
pub const OFFICE_AREA: f64 = 15.0;
/// Net shop floor a job, m²: the population's default (30 m² gross).
pub const SHOP_AREA: f64 = 25.0;

/// The people living in unit `k` of `s`: a home's main rooms by [`ROOMS_A_PERSON`], at least one,
/// the fraction drawn; nobody in [`VACANT`] of the homes, nor in any other unit.
pub fn occupants(s: &Structure, k: usize) -> u8 {
    let Use::Home(rooms) = s.units[k].use_ else {
        return 0;
    };
    let key = (u64::from(s.mass) << 20) ^ k as u64;
    let unit = |salt: u64| (mix64(key ^ salt) >> 11) as f64 / (1u64 << 53) as f64;
    if unit(0x07AC_44E7) < VACANT {
        return 0;
    }
    let people = (f64::from(rooms) / ROOMS_A_PERSON).max(1.0);
    (people.floor() + f64::from(u8::from(unit(0x009E_091E) < people.fract()))) as u8
}

/// Who a structure holds, and its floor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Occupancy {
    /// Its occupied homes, a household each.
    pub households: u32,
    /// The people living in them ([`occupants`]).
    pub residents: u32,
    /// Its jobs: its offices' floor by [`OFFICE_AREA`], its shops' by [`SHOP_AREA`].
    pub jobs: u32,
    /// Its net floor area, m² ([`Structure::net`]).
    pub net: f64,
}

/// Who `s` holds: the population counted by home (D-060 step 3).
pub fn occupancy(s: &Structure) -> Occupancy {
    let mut o = Occupancy {
        net: s.net,
        ..Occupancy::default()
    };
    let (mut offices, mut shops) = (0.0, 0.0);
    for (k, u) in s.units.iter().enumerate() {
        match u.use_ {
            Use::Home(_) => {
                let people = occupants(s, k);
                o.households += u32::from(people > 0);
                o.residents += u32::from(people);
            }
            Use::Office => offices += u.net,
            Use::Shop => shops += u.net,
            Use::Store => {}
        }
    }
    o.jobs = (offices / OFFICE_AREA + shops / SHOP_AREA) as u32;
    o
}

/// A home's main rooms by its net area (the research note's sizes: a studio 37 m², a T2 42–48,
/// a T3 65–67, a T4 86, a T5 100).
pub fn rooms(net: f64) -> u8 {
    match net {
        x if x < 40.0 => 1,
        x if x < 56.0 => 2,
        x if x < 76.0 => 3,
        x if x < 93.0 => 4,
        _ => 5,
    }
}

/// A flat's net area drawn from [`MIX`] by `key`, m².
fn draw(key: u64) -> f64 {
    let x = (mix64(key) >> 11) as f64 / (1u64 << 53) as f64;
    let mut acc = 0.0;
    for &(share, area) in &MIX {
        acc += share;
        if x < acc {
            return area;
        }
    }
    MIX[MIX.len() - 1].1
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Joins each unit under [`MIN_FLAT`] to the nearest other, its pieces added, the bigger's door
/// kept.
fn merge_small(units: &mut Vec<Unit>) {
    loop {
        let Some(i) = units
            .iter()
            .position(|u| u.net < MIN_FLAT)
            .filter(|_| units.len() > 1)
        else {
            return;
        };
        let c = geom::centroid(&units[i].pieces[0]);
        let j = (0..units.len())
            .filter(|&j| j != i)
            .min_by(|&a, &b| {
                let d = |k: usize| geom::centroid(&units[k].pieces[0]).distance(c);
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0);
        let small = units.remove(i);
        let j = if j > i { j - 1 } else { j };
        units[j].net += small.net;
        units[j].pieces.extend(small.pieces);
    }
}

/// The point `along` a wall of `s`, on its outer face.
fn point(s: &Structure, wall: usize, along: f64) -> DVec2 {
    let w = &s.walls[wall];
    w.a + (w.b - w.a).normalize_or_zero() * along
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{example, mass, plan, structure};

    /// How far `p` lies from the polygon: nothing inside, else to its nearest edge.
    fn distance(p: DVec2, poly: &[DVec2]) -> f64 {
        if geom::contains(poly, p) {
            return 0.0;
        }
        (0..poly.len())
            .map(|i| geom::distance_to_segment(p, poly[i], poly[(i + 1) % poly.len()]))
            .fold(f64::MAX, f64::min)
    }

    #[test]
    fn every_floor_is_cut_into_homes_that_add_up_and_open_on_a_landing() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = mass::masses(&city, &crate::city::bible::StyleBible::default(), 7);
        let all = structure::structures(&city, &masses);
        let (mut rooms, mut shops, mut stores, mut through, mut homes) = ([0usize; 6], 0, 0, 0, 0);
        let mut offices = 0;
        let (mut off, mut checked, mut far) = (0, 0, 0);
        for s in &all {
            let p = s.era.table().partitions;
            let units: f64 = s.units.iter().map(|u| u.net).sum();
            let landings: f64 = s
                .landings
                .iter()
                .flat_map(|l| &l.pieces)
                .map(|q| geom::signed_area(q).abs())
                .sum::<f64>()
                * (1.0 - p);
            if s.net > 1.0 && !s.units.is_empty() {
                checked += 1;
                off += usize::from((units + landings - s.net).abs() > 0.05 * s.net);
            }
            for u in &s.units {
                match u.use_ {
                    Use::Home(r) => {
                        rooms[usize::from(r)] += 1;
                        homes += 1;
                        through += usize::from(u.through);
                    }
                    Use::Shop => shops += 1,
                    Use::Office => offices += 1,
                    Use::Store => stores += 1,
                }
                // A flat's door on its core's hall, lobby or corridor.
                if let (Some(c), Use::Home(_) | Use::Store) = (u.core, u.use_) {
                    let core = &s.cores[c];
                    let near = std::iter::once(core.hall.to_vec())
                        .chain(
                            s.landings
                                .iter()
                                .filter(|l| l.core == c && l.storey == u.storey)
                                .flat_map(|l| l.pieces.clone()),
                        )
                        .any(|q| distance(u.door, &q) <= 0.6);
                    far += usize::from(!near);
                }
            }
        }
        println!(
            "{homes} homes (T1–T5: {:?}), {through} through, {shops} shops, {offices} offices, \
             {stores} store rooms; {off} of {checked} masses whose units and landings miss their \
             net area by over 5 %; {far} doors off their landing",
            &rooms[1..]
        );
        assert!(homes > 50_000, "{homes} homes");
        assert!(off * 50 <= checked, "{off} of {checked} masses off");
        assert!(far * 100 <= homes, "{far} doors off their landing");
        assert!(stores * 20 <= homes, "{stores} store rooms");
    }

    #[test]
    fn offices_gather_upstairs_by_the_interchanges_and_the_homes_count_the_people() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = mass::masses(&city, &crate::city::bible::StyleBible::default(), 7);
        let all = structure::structures(&city, &masses);
        // The interchanges: a stop with another line's within 150 m.
        let lines = &city.tram_lines;
        let hubs: Vec<DVec2> = lines
            .iter()
            .enumerate()
            .flat_map(|(i, l)| {
                l.stops.iter().copied().filter(move |&s| {
                    lines
                        .iter()
                        .enumerate()
                        .any(|(j, o)| j != i && o.stops.iter().any(|&q| q.distance(s) < 150.0))
                })
            })
            .collect();
        // The upper floors' net area, offices and all, near an interchange and elsewhere.
        let (mut near, mut far) = ([0.0_f64; 2], [0.0_f64; 2]);
        let (mut buildings, mut whole, mut receptions) = (0, 0, 0);
        let mut total = Occupancy::default();
        for (m, s) in masses.iter().zip(&all) {
            let c = geom::centroid(&m.footprint);
            let by_hub = hubs.iter().any(|h| h.distance(c) < 400.0);
            for u in s.units.iter().filter(|u| u.storey > 0) {
                let side = if by_hub { &mut near } else { &mut far };
                side[0] += u.net;
                side[1] += if u.use_ == Use::Office { u.net } else { 0.0 };
            }
            buildings += usize::from(m.offices > 0);
            whole += usize::from(m.office_building());
            receptions += usize::from(
                s.units
                    .iter()
                    .any(|u| u.storey == 0 && u.use_ == Use::Office),
            );
            let o = occupancy(s);
            total.households += o.households;
            total.residents += o.residents;
            total.jobs += o.jobs;
        }
        let share = |a: [f64; 2]| a[1] / a[0].max(1.0);
        let all_share = (near[1] + far[1]) / (near[0] + far[0]);
        let per_home = f64::from(total.residents) / f64::from(total.households);
        println!(
            "upper floors as workplaces: {:.1} % ({:.1} % within 400 m of {} interchanges, \
             {:.1} % elsewhere); {buildings} buildings with offices, {whole} office buildings, \
             {receptions} receptions; {} households, {} residents ({per_home:.2} a home), {} jobs",
            all_share * 100.0,
            share(near) * 100.0,
            hubs.len(),
            share(far) * 100.0,
            total.households,
            total.residents,
            total.jobs
        );
        assert!((0.08..0.3).contains(&all_share), "{all_share}");
        assert!(share(near) > 1.5 * share(far));
        assert!(whole > 0 && receptions > 0);
        // INSEE: 2.9 rooms a flat at 1.5 a person, about 1.9 people a home.
        assert!((1.7..2.1).contains(&per_home), "{per_home}");
    }

    #[test]
    fn people_reach_every_home_from_the_street_by_stairs_and_lift() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = mass::masses(&city, &crate::city::bible::StyleBible::default(), 7);
        let all = structure::structures(&city, &masses);
        // The routine: from the street on foot to the core's door, in, up the stairs (and, from
        // R+3, the lift) to the landing of the home's storey, to its door.
        let (shut, _) = structure::unreached(&city, &masses, &all);
        let shut: std::collections::HashSet<(u32, usize)> = shut.into_iter().collect();
        let (mut homes, mut by_lift, mut through_wing, mut stranded) = (0, 0, 0, 0);
        for s in &all {
            for u in s.units.iter().filter(|u| matches!(u.use_, Use::Home(_))) {
                homes += 1;
                match u.core {
                    Some(c) => {
                        let core = &s.cores[c];
                        let door_ok = !shut.contains(&(s.mass, core.door));
                        // Every storey's flight climbs it, none over 17 cm.
                        let stairs_ok = u.storey < core.risers.len();
                        let lift_ok = s.storeys.len() < crate::city::structure::LIFT_FROM
                            || core.lift.is_some();
                        stranded += usize::from(!(door_ok && stairs_ok && lift_ok));
                        by_lift += usize::from(u.storey > 0 && core.lift.is_some());
                    }
                    // A one-storey house from its street door; a wing through its other mass.
                    None => through_wing += usize::from(s.storeys.len() >= 2),
                }
            }
        }
        println!(
            "{homes} homes: {by_lift} above the ground with a lift, {through_wing} in wings \
             reached through their building's other mass, {stranded} stranded"
        );
        assert_eq!(stranded, 0);
        assert!(through_wing * 20 <= homes, "{through_wing} homes in wings");
    }
}
