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
