//! Sweeping a navigator's shape through what was built (#276). The owner, 2026-10-17: "when
//! testing geometry for people if it's navigable and such use the 'capsule' hit box ..., not just
//! rays ... Basically we should test with the shape the navigation places are supposed to be
//! navigated by"; then: "Key or points can be a first fast pass, but we need more practical tests
//! as well". The point samples stay the fast first pass; this is the practical test.
//!
//! A walker is an upright [`Capsule`]: a disc of its radius from the height it steps over to its
//! head. The solids it is swept against are prisms, rings extruded between two heights, as the
//! drawn insides are made: partitions, a cupboard, a guard, a floor round its stairwell. A
//! [`sweep`] moves the capsule along a path over the ground under it, in steps shorter than its
//! radius so the samples overlap, and reports where it meets a solid.

use glam::DVec2;

use super::geom;
use super::walk;

/// An upright capsule: a walker's shape for navigation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    /// Its radius, metres: half a walker's width.
    pub radius: f64,
    /// The height over the ground under it where it starts, metres: what a walker steps over or
    /// onto (a step, a kerb, a door's sill) is the walk's to climb, not an obstacle.
    pub foot: f64,
    /// The height of its top over the ground, metres.
    pub top: f64,
}

/// A walker: shoulders 0.6 m across (as [`walk::RADIUS`]), stepping over 35 cm (a stair's
/// riser and some), 1.95 m tall with the bob of a stride.
pub const WALKER: Capsule = Capsule {
    radius: walk::RADIUS,
    foot: 0.35,
    top: 1.95,
};

/// A solid: a ring (anticlockwise or not) extruded from `y0` to `y1`, metres.
#[derive(Clone, Copy, Debug)]
pub struct Prism<'a> {
    /// Its outline seen from above.
    pub ring: &'a [DVec2],
    /// Its underside.
    pub y0: f64,
    /// Its top.
    pub y1: f64,
}

/// Whether the disc of `radius` round `centre` meets `ring`: the centre inside it, or one of its
/// edges nearer than the radius.
pub fn disc_meets_ring(centre: DVec2, radius: f64, ring: &[DVec2]) -> bool {
    if ring.len() < 3 {
        return false;
    }
    if geom::contains(ring, centre) {
        return true;
    }
    (0..ring.len()).any(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        geom::distance_to_segment(centre, a, b) < radius
    })
}

/// The first of `solids` the capsule standing at `at` on ground at height `ground` meets.
pub fn meets(capsule: &Capsule, at: DVec2, ground: f64, solids: &[Prism<'_>]) -> Option<usize> {
    let (from, to) = (ground + capsule.foot, ground + capsule.top);
    solids
        .iter()
        .position(|s| s.y0 < to && s.y1 > from && disc_meets_ring(at, capsule.radius, s.ring))
}

/// Where the capsule met a solid along a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// How far along the path, metres.
    pub along: f64,
    /// Where the capsule stood, and the ground's height there.
    pub at: DVec2,
    /// The ground's height there.
    pub ground: f64,
    /// The solid met, its index.
    pub solid: usize,
}

/// Sweeps `capsule` along the polyline `path`, standing on `ground(along, at)` (the height of
/// the ground under it, metres), in steps of at most `spacing` metres (keep it under the radius),
/// against `solids`: every place it meets one, the first solid met at each.
pub fn sweep(
    capsule: &Capsule,
    path: &[DVec2],
    spacing: f64,
    ground: impl Fn(f64, DVec2) -> f64,
    solids: &[Prism<'_>],
) -> Vec<Hit> {
    let mut hits = Vec::new();
    let spacing = spacing.max(1e-3);
    let mut along = 0.0;
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = a.distance(b);
        // The segment's samples, its start included, its end left to the next.
        let steps = (length / spacing).ceil().max(1.0) as usize;
        for i in 0..steps {
            let t = i as f64 / steps as f64;
            let at = a.lerp(b, t);
            let d = along + length * t;
            let g = ground(d, at);
            if let Some(solid) = meets(capsule, at, g, solids) {
                hits.push(Hit {
                    along: d,
                    at,
                    ground: g,
                    solid,
                });
            }
        }
        along += length;
    }
    if let Some(&end) = path.last() {
        let g = ground(along, end);
        if let Some(solid) = meets(capsule, end, g, solids) {
            hits.push(Hit {
                along,
                at: end,
                ground: g,
                solid,
            });
        }
    }
    hits
}

/// A vehicle's body as an upright box: a tram's section, a taxi.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cuboid {
    /// Half its length along its heading, metres.
    pub half_length: f64,
    /// Half its width, metres.
    pub half_width: f64,
    /// Its underside over the ground, metres.
    pub bottom: f64,
    /// Its roof over the ground, metres.
    pub top: f64,
}

/// Whether `body`, its middle at `centre` heading along `heading` (unit) on ground at height
/// `ground`, meets the upright disc of `radius` round `at` from `y0` to `y1` (a trunk, a
/// crown, a post).
#[allow(clippy::too_many_arguments)]
pub fn cuboid_meets_disc(
    body: &Cuboid,
    centre: DVec2,
    heading: DVec2,
    ground: f64,
    at: DVec2,
    radius: f64,
    y0: f64,
    y1: f64,
) -> bool {
    if y0 >= ground + body.top || y1 <= ground + body.bottom {
        return false;
    }
    let d = at - centre;
    let across = DVec2::new(-heading.y, heading.x);
    let outside = DVec2::new(
        d.dot(heading).abs() - body.half_length,
        d.dot(across).abs() - body.half_width,
    )
    .max(DVec2::ZERO);
    outside.length() < radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_meets_a_disc_by_its_sides_and_its_corners() {
        let body = Cuboid {
            half_length: 3.0,
            half_width: 1.0,
            bottom: 0.2,
            top: 3.5,
        };
        let meets = |at: DVec2, r: f64, y0: f64, y1: f64| {
            cuboid_meets_disc(&body, DVec2::ZERO, DVec2::X, 0.0, at, r, y0, y1)
        };
        // Beside its long side, 0.3 m off: a post of 0.25 misses, one of 0.35 meets.
        assert!(!meets(DVec2::new(0.0, 1.3), 0.25, 0.0, 5.0));
        assert!(meets(DVec2::new(0.0, 1.3), 0.35, 0.0, 5.0));
        // Past a corner, 0.3 m off each way: 0.42 m away.
        assert!(!meets(DVec2::new(3.3, 1.3), 0.4, 0.0, 5.0));
        assert!(meets(DVec2::new(3.3, 1.3), 0.45, 0.0, 5.0));
        // A crown over the roof, a kerb under the skirt.
        assert!(!meets(DVec2::ZERO, 2.0, 3.6, 8.0));
        assert!(!meets(DVec2::ZERO, 2.0, 0.0, 0.15));
        // Turned: the same post across a body heading north-east.
        let ne = DVec2::new(1.0, 1.0).normalize();
        assert!(cuboid_meets_disc(
            &body,
            DVec2::ZERO,
            ne,
            0.0,
            ne * 2.5,
            0.1,
            0.0,
            5.0
        ));
        assert!(!cuboid_meets_disc(
            &body,
            DVec2::ZERO,
            ne,
            0.0,
            DVec2::new(-ne.y, ne.x) * 1.3,
            0.2,
            0.0,
            5.0
        ));
    }

    fn square(x: f64, z: f64, half: f64) -> Vec<DVec2> {
        vec![
            DVec2::new(x - half, z - half),
            DVec2::new(x + half, z - half),
            DVec2::new(x + half, z + half),
            DVec2::new(x - half, z + half),
        ]
    }

    #[test]
    fn a_disc_meets_a_ring_by_its_edge_or_from_inside() {
        let ring = square(0.0, 0.0, 1.0);
        assert!(disc_meets_ring(DVec2::ZERO, 0.1, &ring));
        assert!(disc_meets_ring(DVec2::new(1.25, 0.0), 0.3, &ring));
        assert!(!disc_meets_ring(DVec2::new(1.35, 0.0), 0.3, &ring));
        // Past a corner: the corner's distance, not the box's.
        assert!(!disc_meets_ring(DVec2::new(1.25, 1.25), 0.3, &ring));
    }

    #[test]
    fn a_walker_passes_under_a_head_and_over_a_sill_but_not_a_low_beam() {
        let post = square(2.0, 0.3, 0.1);
        // A path along x, the post's near face 0.2 m off it: the walker's 0.3 m reach meets it.
        let path = [DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0)];
        let flat = |_: f64, _: DVec2| 0.0;
        let tall = [Prism {
            ring: &post,
            y0: 0.0,
            y1: 3.0,
        }];
        let hits = sweep(&WALKER, &path, 0.05, flat, &tall);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| (h.at.x - 2.0).abs() < 0.45), "{hits:?}");
        // A sill 20 cm high: stepped over. A head 2.1 m up: walked under. A beam at 1.5–1.8 m:
        // met.
        let beam = square(2.0, 0.0, 0.2);
        for (y0, y1, met) in [(0.0, 0.2, false), (2.1, 2.4, false), (1.5, 1.8, true)] {
            let solid = [Prism {
                ring: &beam,
                y0,
                y1,
            }];
            assert_eq!(
                !sweep(&WALKER, &path, 0.05, flat, &solid).is_empty(),
                met,
                "{y0}..{y1}"
            );
        }
        // On ground that rises with the path (a ramp), the same beam is met earlier.
        let solid = [Prism {
            ring: &beam,
            y0: 2.1,
            y1: 2.4,
        }];
        let ramp = |along: f64, _: DVec2| along * 0.25;
        assert!(!sweep(&WALKER, &path, 0.05, ramp, &solid).is_empty());
    }

    #[test]
    fn the_samples_overlap_and_reach_the_path_s_end() {
        let end = square(3.0, 0.0, 0.05);
        let path = [DVec2::new(0.0, 0.0), DVec2::new(2.8, 0.0)];
        let solid = [Prism {
            ring: &end,
            y0: 0.0,
            y1: 3.0,
        }];
        // The end's sample at 2.8 m: 0.15 m from the post's face, within the reach.
        let hits = sweep(&WALKER, &path, 0.25, |_, _| 0.0, &solid);
        assert_eq!(hits.last().map(|h| h.along), Some(2.8));
    }
}
