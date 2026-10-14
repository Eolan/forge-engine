//! Taxi stands (D-061): where the city's automated taxis wait when no one needs them and pick
//! people up. The owner's model: taxis booked from a smart pad or communicator, which come to the
//! person; idle ones park at set places; at busy times they circulate round the busy places.
//!
//! A stand is a lay-by along a taxi road (an avenue without trams, so no tram platform meets it),
//! cut into the pavement's verge at the carriageway's level, with a lit totem behind it. Stands go
//! first by the tram stops, the squares and the landmarks (the nearest place on a taxi road within
//! 150 m), then wherever a built block is left without one within [`REACH`].

use glam::DVec2;

use super::blocks::{Block, BlockUse};
use super::geom;
use super::graph::RoadGraph;
use super::transit::TramLine;
use super::{LAND, Profile, Site};

/// A lay-by's length along the kerb, metres: two taxis.
pub const LENGTH: f64 = 14.0;
/// How deep a lay-by cuts into the verge, metres.
pub const DEPTH: f64 = 2.5;
/// The farthest a built block's middle lies from a stand, metres (a few minutes' walk).
pub const REACH: f64 = 300.0;
/// The least distance between two stands, metres.
const SPACING: f64 = 200.0;

/// A taxi stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stand {
    /// The road it lies along.
    pub edge: u32,
    /// The middle of its kerb line, where the lay-by meets the carriageway.
    pub kerb: DVec2,
    /// Along the road, unit.
    pub along: DVec2,
    /// From the carriageway into the lay-by, unit.
    pub out: DVec2,
}

impl Stand {
    /// The lay-by's outline at the carriageway's level: from a little inside the carriageway (it
    /// joins it) to [`DEPTH`] out, [`LENGTH`] long, its ends splayed so a taxi pulls in and out.
    pub fn outline(&self) -> Vec<DVec2> {
        let (a, n) = (self.along * (LENGTH * 0.5), self.out);
        let splay = self.along * 3.0;
        let (near, far) = (self.kerb - n * 0.3, self.kerb + n * DEPTH);
        vec![near - a - splay, near + a + splay, far + a, far - a]
    }

    /// Where its totem stands: behind the lay-by's middle, on the pavement.
    pub fn totem(&self) -> DVec2 {
        self.kerb + self.out * (DEPTH + 0.6)
    }
}

/// The stands along `graph`'s taxi roads, clear of the junctions (`avoid`: points no stand may
/// come within 35 m of, where the corners and crossings are) and of the water.
pub fn stands(
    graph: &RoadGraph,
    site: &Site,
    trams: &[TramLine],
    blocks: &[Block],
    avoid: &[DVec2],
) -> Vec<Stand> {
    // The places a stand may take: every 10 m along each taxi road, 35 m from its ends, either
    // side, on land and 30 m from a crossing.
    let mut places: Vec<Stand> = Vec::new();
    for (k, e) in graph.edges.iter().enumerate() {
        if e.profile != Profile::Avenue {
            continue;
        }
        let length = e.length();
        let kerb = e.profile.carriageway() * 0.5;
        let mut s = 35.0;
        while s < length - 35.0 {
            let c = geom::point_at(&e.points, s);
            let t = geom::tangent_at(&e.points, s);
            let n = DVec2::new(-t.y, t.x);
            // Straight here: a lay-by on a curve would not line up with the kerb.
            let (t0, t1) = (
                geom::tangent_at(&e.points, s - LENGTH),
                geom::tangent_at(&e.points, s + LENGTH),
            );
            let clear = avoid.iter().all(|p| p.distance(c) > 35.0);
            if t0.dot(t1) > 0.995 && clear {
                for side in [1.0, -1.0] {
                    let out = n * side;
                    let far = c + out * (kerb + DEPTH + 1.0);
                    if site.water_at(c) == LAND && site.water_at(far) == LAND {
                        places.push(Stand {
                            edge: k as u32,
                            kerb: c + out * kerb,
                            along: t,
                            out,
                        });
                    }
                }
            }
            s += 10.0;
        }
    }
    let mut chosen: Vec<Stand> = Vec::new();
    let take = |chosen: &mut Vec<Stand>, at: DVec2, within: f64| {
        let best = places
            .iter()
            .filter(|p| chosen.iter().all(|c| c.kerb.distance(p.kerb) >= SPACING))
            .map(|p| (p.kerb.distance(at), *p))
            .filter(|&(d, _)| d <= within)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, p)) = best {
            chosen.push(p);
        }
    };
    // First by the tram stops, the squares and the landmarks' squares: where people change to a
    // taxi or come out in numbers.
    let mut targets: Vec<DVec2> = trams.iter().flat_map(|l| l.stops.iter().copied()).collect();
    targets.extend(
        blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Square)
            .map(|b| b.centre),
    );
    for t in targets {
        take(&mut chosen, t, 150.0);
    }
    // Then every built block left without one within reach, the farthest first.
    let built: Vec<DVec2> = blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Built)
        .map(|b| b.centre)
        .collect();
    let mut tried: Vec<DVec2> = Vec::new();
    loop {
        let far = built
            .iter()
            .copied()
            .filter(|b| !tried.contains(b))
            .filter(|b| chosen.iter().all(|c| c.kerb.distance(*b) > REACH))
            .filter(|b| places.iter().any(|p| p.kerb.distance(*b) <= REACH))
            .max_by(|a, b| {
                let gap = |p: DVec2| {
                    chosen
                        .iter()
                        .map(|c| c.kerb.distance(p))
                        .fold(f64::MAX, f64::min)
                };
                gap(*a).total_cmp(&gap(*b))
            });
        let Some(far) = far else { break };
        let before = chosen.len();
        take(&mut chosen, far, REACH);
        if chosen.len() == before {
            // No place left near it: the next one.
            tried.push(far);
        }
    }
    chosen
}

/// A depot's width along its avenue, metres (`forge_geom::depot::WIDTH`).
pub const DEPOT_WIDTH: f64 = 48.0;
/// How deep a depot reaches into its block from the building line, metres
/// (`forge_geom::depot::DEPTH`).
pub const DEPOT_DEPTH: f64 = 32.0;
/// The cabs a depot holds: 15 bays a row, four rows to each level of its split-level deck's two
/// halves, three levels, less the three before its way in (`forge_geom::depot::bays`).
pub const DEPOT_CAPACITY: usize = 177;
/// Where a depot's way out opens, along its front from its middle, metres
/// (`forge_geom::depot::EXIT_X`, across [`Depot::across`]).
pub const DEPOT_EXIT: f64 = -21.3;
/// Half a depot's driveways' width, metres: each one lane, the way in at the front's middle and
/// the way out at its corner.
pub const DRIVE_HALF: f64 = 1.75;
/// Stands a depot serves: the cabs no stand holds wait at the nearest.
const STANDS_PER_DEPOT: usize = 25;
/// The least distance between two depots, metres.
const DEPOT_SPACING: f64 = 1000.0;
/// How much nearer a site in an industrial district counts, metres: depots belong there
/// (D-061), but not at the far end of the city from the stands they serve.
const INDUSTRIAL: f64 = 800.0;

/// A taxi depot (D-061): a split-level parking deck where the cabs no stand holds wait and
/// charge, on a taxi road, its driveways crossing the pavement at the walking level (the cabs
/// climb the verge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Depot {
    /// The block it stands in.
    pub block: u32,
    /// The road it opens onto.
    pub edge: u32,
    /// Its way in's driveway's middle where it meets the carriageway.
    pub kerb: DVec2,
    /// Along the road, unit.
    pub along: DVec2,
    /// From the road into the depot, unit.
    pub out: DVec2,
    /// From the kerb to the building line, metres: the verge and the footway.
    pub front: f64,
}

impl Depot {
    /// The cabs it holds.
    pub fn capacity(&self) -> usize {
        DEPOT_CAPACITY
    }

    /// Its deck's `x` along its front, unit: the deck's frame (`forge_geom::depot`) turned so
    /// its `+z` faces the road.
    pub fn across(&self) -> DVec2 {
        DVec2::new(-self.out.y, self.out.x)
    }

    /// The middle of its front, on the building line.
    pub fn entrance(&self) -> DVec2 {
        self.kerb + self.out * self.front
    }

    /// Its way out's driveway's middle where it meets the carriageway.
    pub fn exit_kerb(&self) -> DVec2 {
        self.kerb + self.across() * DEPOT_EXIT
    }

    /// Its outline: [`DEPOT_WIDTH`] along the road, [`DEPOT_DEPTH`] back from the building line.
    pub fn footprint(&self) -> Vec<DVec2> {
        let (a, e) = (self.along * (DEPOT_WIDTH * 0.5), self.entrance());
        let b = e + self.out * DEPOT_DEPTH;
        vec![e - a, e + a, b + a, b - a]
    }

    /// Its driveways' ramps up the verge from the carriageway to the walking level, `verge`
    /// metres long, the way in's and the way out's: the footway they then cross stays level for
    /// the people walking along it.
    pub fn ramps(&self, verge: f64) -> [super::ground::Ramp; 2] {
        [self.kerb, self.exit_kerb()].map(|kerb| {
            super::ground::Ramp::driveway(kerb, self.along, self.out, DRIVE_HALF, verge)
        })
    }

    /// Whether `p` lies on its deck, `margin` metres round it.
    pub fn on_deck(&self, p: DVec2, margin: f64) -> bool {
        let d = p - self.entrance();
        let (s, t) = (d.dot(self.along), d.dot(self.out));
        s.abs() <= DEPOT_WIDTH * 0.5 + margin && (-margin..=DEPOT_DEPTH + margin).contains(&t)
    }

    /// Whether `p` lies on one of its driveways, from the kerb to the building line, `margin`
    /// metres round it.
    pub fn on_driveway(&self, p: DVec2, margin: f64) -> bool {
        [self.kerb, self.exit_kerb()].iter().any(|&kerb| {
            let d = p - kerb;
            let (s, t) = (d.dot(self.along), d.dot(self.out));
            s.abs() <= DRIVE_HALF + margin && (-margin..=self.front + margin).contains(&t)
        })
    }
}

/// The depots for `stands`: one for each [`STANDS_PER_DEPOT`] of them, spread over them (the
/// stands' clusters), each on a taxi road in a built block it fits behind the building line, in
/// an industrial district where one is near (`industrial`, by block); [`DEPOT_SPACING`] apart,
/// clear of the stands, the junctions and the landmarks' places (`avoid`).
pub fn depots(
    graph: &RoadGraph,
    site: &Site,
    blocks: &[Block],
    industrial: &[bool],
    stands: &[Stand],
    junctions: &[DVec2],
    avoid: &[DVec2],
) -> Vec<Depot> {
    if stands.is_empty() {
        return Vec::new();
    }
    // Where depots may go: every 10 m along each taxi road, either side, where the deck fits
    // in the built block behind the building line.
    let mut sites: Vec<Depot> = Vec::new();
    for (k, e) in graph.edges.iter().enumerate() {
        if e.profile != Profile::Avenue {
            continue;
        }
        let length = e.length();
        let kerb = e.profile.carriageway() * 0.5;
        let front = e.profile.width() * 0.5 - kerb;
        let mut s = 40.0;
        while s < length - 40.0 {
            let c = geom::point_at(&e.points, s);
            let t = geom::tangent_at(&e.points, s);
            let straight = geom::tangent_at(&e.points, s - DEPOT_WIDTH * 0.5)
                .dot(geom::tangent_at(&e.points, s + DEPOT_WIDTH * 0.5))
                > 0.995;
            let clear = junctions.iter().all(|j| j.distance(c) > 45.0)
                && stands.iter().all(|st| st.kerb.distance(c) > 40.0)
                && avoid.iter().all(|a| a.distance(c) > 150.0);
            if straight && clear {
                for side in [1.0, -1.0] {
                    let out = DVec2::new(-t.y, t.x) * side;
                    let depot = Depot {
                        block: u32::MAX,
                        edge: k as u32,
                        kerb: c + out * kerb,
                        along: t,
                        out,
                        front,
                    };
                    if let Some(b) = fits(blocks, site, &depot) {
                        sites.push(Depot { block: b, ..depot });
                    }
                }
            }
            s += 10.0;
        }
    }
    // The stands' clusters: centres spread farthest first, then a few rounds of Lloyd's.
    let n = stands.len().div_ceil(STANDS_PER_DEPOT).max(1);
    let points: Vec<DVec2> = stands.iter().map(|s| s.kerb).collect();
    let mean = points.iter().copied().sum::<DVec2>() / points.len() as f64;
    let mut centres = vec![
        *points
            .iter()
            .min_by(|a, b| a.distance(mean).total_cmp(&b.distance(mean)))
            .expect("a stand"),
    ];
    while centres.len() < n {
        let gap = |p: DVec2| {
            centres
                .iter()
                .map(|c| c.distance(p))
                .fold(f64::MAX, f64::min)
        };
        let far = points
            .iter()
            .copied()
            .max_by(|a, b| gap(*a).total_cmp(&gap(*b)))
            .expect("a stand");
        centres.push(far);
    }
    let mut members = vec![0_usize; n];
    for _ in 0..8 {
        let mut sum = vec![DVec2::ZERO; n];
        members.fill(0);
        for p in &points {
            let c = (0..n)
                .min_by(|&a, &b| centres[a].distance(*p).total_cmp(&centres[b].distance(*p)))
                .expect("a centre");
            sum[c] += *p;
            members[c] += 1;
        }
        for c in 0..n {
            if members[c] > 0 {
                centres[c] = sum[c] / members[c] as f64;
            }
        }
    }
    // The largest clusters first, each its nearest site (an industrial one counted nearer).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| members[b].cmp(&members[a]).then(a.cmp(&b)));
    let mut chosen: Vec<Depot> = Vec::new();
    for c in order {
        let best = sites
            .iter()
            .filter(|s| {
                chosen
                    .iter()
                    .all(|d| d.block != s.block && d.kerb.distance(s.kerb) >= DEPOT_SPACING)
            })
            .map(|s| {
                let bonus = if industrial.get(s.block as usize).copied().unwrap_or(false) {
                    INDUSTRIAL
                } else {
                    0.0
                };
                (s.kerb.distance(centres[c]) - bonus, *s)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, s)) = best {
            chosen.push(s);
        }
    }
    chosen
}

/// The built block `depot`'s deck fits in, behind its building line: every corner inside the
/// block, the back and the sides 13 m from its other edges (the streets round it: half a wide
/// one and a pavement), on land.
fn fits(blocks: &[Block], site: &Site, depot: &Depot) -> Option<u32> {
    let deck = depot.footprint();
    let middle = geom::centroid(&deck);
    let (b, block) = blocks
        .iter()
        .enumerate()
        .find(|(_, b)| b.use_ == BlockUse::Built && geom::contains(&b.polygon, middle))?;
    let poly = &block.polygon;
    let m = poly.len();
    // The block's edge along the road: the nearest to the driveway.
    let front = (0..m).min_by(|&i, &j| {
        let d = |k: usize| geom::distance_to_segment(depot.kerb, poly[k], poly[(k + 1) % m]);
        d(i).total_cmp(&d(j))
    })?;
    let ok = deck.iter().all(|&p| {
        geom::contains(poly, p)
            && site.water_at(p) == LAND
            && (0..m).all(|k| {
                let d = geom::distance_to_segment(p, poly[k], poly[(k + 1) % m]);
                d >= if k == front { depot.front + 0.5 } else { 13.0 }
            })
    });
    ok.then_some(b as u32)
}
