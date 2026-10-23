//! Lag compensation (#268's step 3, D-010): the authority keeps each tick's hit shapes for a
//! window, so a shot is tested against the world as its shooter saw it, not as it stands when the
//! shot arrives (server-side rewind, Bernier 2001; `docs/research/netcode.md` §4 and §9).
//!
//! A client draws the other players some time behind the authority (its snapshots' interpolation
//! and the link's delay). A shot carries the moment its shooter was drawing them, a tick and its
//! fraction ([`View`]); the authority takes the shapes of the two ticks about it from its
//! [`History`], interpolates them, and tests the shot against those ([`History::ray`]). The
//! window clamps how far back a shot may reach, so a client claiming an old view gains nothing past
//! it.
//!
//! The view is the client's own, never a delay the authority sets per client from the timing of
//! its packets (the reading of US 10,940,388 B1 in the research's §9). Everything here is plain
//! `f64` arithmetic and square roots in a fixed order: the same history and shot give the same
//! answer on every machine (D-016).

use std::collections::VecDeque;

use glam::DVec3;

/// What an entity is hit as: a capsule, its axis from `a` to `b`, the world's metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    /// One end of its axis.
    pub a: DVec3,
    /// The other.
    pub b: DVec3,
    /// Its radius.
    pub radius: f64,
}

impl Capsule {
    /// An upright capsule standing on `feet`, `height` tall and `radius` round: a person's.
    pub fn standing(feet: DVec3, height: f64, radius: f64) -> Self {
        Self {
            a: feet + DVec3::Y * radius,
            b: feet + DVec3::Y * (height - radius).max(radius),
            radius,
        }
    }

    /// Between `self` and `other`, `t` from 0 to 1 of the way.
    pub fn lerp(&self, other: &Capsule, t: f64) -> Capsule {
        Capsule {
            a: self.a.lerp(other.a, t),
            b: self.b.lerp(other.b, t),
            radius: self.radius + (other.radius - self.radius) * t,
        }
    }

    /// How far along the ray from `origin` in the unit direction `dir` it first meets the
    /// capsule, within `reach`; none when it misses. A ray starting inside meets it at 0.
    pub fn ray(&self, origin: DVec3, dir: DVec3, reach: f64) -> Option<f64> {
        // The nearest of the ray's meetings with the side and with the two end spheres.
        let mut best: Option<f64> = None;
        let mut keep = |t: f64| {
            if (0.0..=reach).contains(&t) && best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        };
        let r2 = self.radius * self.radius;
        for centre in [self.a, self.b] {
            let m = origin - centre;
            let b = m.dot(dir);
            let c = m.length_squared() - r2;
            if c <= 0.0 {
                keep(0.0);
                continue;
            }
            let disc = b * b - c;
            if disc >= 0.0 && b <= 0.0 {
                keep(-b - disc.sqrt());
            }
        }
        // The side: a cylinder between the ends.
        let axis = self.b - self.a;
        let len2 = axis.length_squared();
        if len2 > 1e-12 {
            let m = origin - self.a;
            let (md, nd, dd) = (m.dot(axis), dir.dot(axis), len2);
            let a = dd - nd * nd;
            let k = m.length_squared() - r2;
            let c = dd * k - md * md;
            // Starting inside the side.
            if c <= 0.0 && (0.0..=dd).contains(&md) {
                keep(0.0);
            }
            if a.abs() > 1e-12 {
                let b = dd * m.dot(dir) - nd * md;
                let disc = b * b - a * c;
                if disc >= 0.0 {
                    let t = (-b - disc.sqrt()) / a;
                    let along = md + t * nd;
                    if along >= 0.0 && along <= dd {
                        keep(t);
                    }
                }
            }
        }
        best
    }
}

/// When a shot's shooter saw the others: a tick of the authority's clock and the share of the
/// way to the next (a client draws between two snapshots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The tick.
    pub tick: u64,
    /// The share of the way to the next, 0 to 1.
    pub fraction: f64,
}

/// What a rewound shot met: the entity, how far along the ray, and the view it was tested in
/// (clamped to the window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The entity's id.
    pub id: u32,
    /// Metres along the ray.
    pub distance: f64,
    /// The view the shot was tested in.
    pub view: View,
}

/// Each recent tick's hit shapes, by entity id, the oldest dropped past the window.
#[derive(Clone, Debug)]
pub struct History {
    window: usize,
    ticks: VecDeque<(u64, Vec<(u32, Capsule)>)>,
}

impl History {
    /// A history `window` ticks long (12 at 60 Hz: 200 ms, about what shooters allow).
    pub fn new(window: usize) -> Self {
        Self {
            window: window.max(1),
            ticks: VecDeque::new(),
        }
    }

    /// The shapes as they stand at `tick` (after its step): kept in the order given, the oldest
    /// tick dropped past the window. Ticks must come in order.
    pub fn record(&mut self, tick: u64, shapes: impl IntoIterator<Item = (u32, Capsule)>) {
        debug_assert!(self.ticks.back().is_none_or(|(t, _)| *t < tick));
        if self.ticks.len() == self.window {
            self.ticks.pop_front();
        }
        self.ticks.push_back((tick, shapes.into_iter().collect()));
    }

    /// The oldest and newest ticks kept.
    pub fn span(&self) -> Option<(u64, u64)> {
        Some((self.ticks.front()?.0, self.ticks.back()?.0))
    }

    /// `view` clamped into what the history holds.
    pub fn clamp(&self, view: View) -> Option<View> {
        let (lo, hi) = self.span()?;
        Some(if view.tick < lo {
            View {
                tick: lo,
                fraction: 0.0,
            }
        } else if view.tick >= hi {
            View {
                tick: hi,
                fraction: 0.0,
            }
        } else {
            View {
                tick: view.tick,
                fraction: view.fraction.clamp(0.0, 1.0),
            }
        })
    }

    /// The shapes as the shooter saw them at `view` (clamped): each entity present at both
    /// ticks about it interpolated, one present at the first only as it stood then.
    pub fn at(&self, view: View) -> Vec<(u32, Capsule)> {
        let Some(view) = self.clamp(view) else {
            return Vec::new();
        };
        let (lo, _) = self.span().expect("a history");
        let i = (view.tick - lo) as usize;
        let (_, now) = &self.ticks[i];
        let next = self.ticks.get(i + 1).map(|(_, s)| s);
        now.iter()
            .map(|&(id, shape)| {
                let then = next
                    .filter(|_| view.fraction > 0.0)
                    .and_then(|s| s.iter().find(|(o, _)| *o == id));
                (
                    id,
                    then.map_or(shape, |(_, t)| shape.lerp(t, view.fraction)),
                )
            })
            .collect()
    }

    /// A shot from `origin` along the unit `dir`, within `reach`, tested against the shapes at
    /// `view` (clamped), `shooter` left out: the nearest it meets, ties to the lower id.
    pub fn ray(
        &self,
        view: View,
        origin: DVec3,
        dir: DVec3,
        reach: f64,
        shooter: Option<u32>,
    ) -> Option<Hit> {
        let clamped = self.clamp(view)?;
        self.at(clamped)
            .into_iter()
            .filter(|&(id, _)| Some(id) != shooter)
            .filter_map(|(id, c)| c.ray(origin, dir, reach).map(|d| (id, d)))
            .min_by(|x, y| x.1.total_cmp(&y.1).then(x.0.cmp(&y.0)))
            .map(|(id, distance)| Hit {
                id,
                distance,
                view: clamped,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TICK;

    #[test]
    fn a_ray_meets_a_capsule_on_its_side_and_its_ends() {
        let c = Capsule::standing(DVec3::ZERO, 1.8, 0.3);
        // Side on, at chest height, from 10 m.
        let d = c.ray(DVec3::new(-10.0, 1.2, 0.0), DVec3::X, 100.0).unwrap();
        assert!((d - 9.7).abs() < 1e-9, "{d}");
        // From above, down onto its top sphere.
        let d = c
            .ray(DVec3::new(0.0, 5.0, 0.0), DVec3::NEG_Y, 100.0)
            .unwrap();
        assert!((d - 3.2).abs() < 1e-9, "{d}");
        // Past it, and beyond the reach.
        assert!(
            c.ray(DVec3::new(-10.0, 1.2, 0.5), DVec3::X, 100.0)
                .is_none()
        );
        assert!(c.ray(DVec3::new(-10.0, 1.2, 0.0), DVec3::X, 5.0).is_none());
        // From inside.
        assert_eq!(c.ray(DVec3::new(0.0, 1.0, 0.0), DVec3::X, 1.0), Some(0.0));
    }

    /// A target running across at 6 m/s; the shooter draws it 6 ticks (100 ms) behind the
    /// authority and aims at it there. Tested where the target stands now the shot misses; at the
    /// shooter's view it hits. A view older than the window is clamped to its oldest tick.
    #[test]
    fn a_rewound_shot_meets_the_target_where_its_shooter_saw_it() {
        let speed = 6.0;
        let at = |tick: u64| DVec3::new(-3.0 + speed * f64::from(TICK) * tick as f64, 0.0, 20.0);
        let mut h = History::new(12);
        for tick in 0..30u64 {
            h.record(
                tick,
                [
                    (1, Capsule::standing(at(tick), 1.8, 0.3)),
                    (7, Capsule::standing(DVec3::ZERO, 1.8, 0.3)),
                ],
            );
        }
        assert_eq!(h.span(), Some((18, 29)));
        // The shooter (7) at the origin saw the target at tick 23 and a half.
        let view = View {
            tick: 23,
            fraction: 0.5,
        };
        let seen = at(23).lerp(at(24), 0.5);
        let aim = (seen + DVec3::Y * 1.2 - DVec3::Y * 1.6).normalize();
        let eye = DVec3::Y * 1.6;
        let hit = h
            .ray(view, eye, aim, 100.0, Some(7))
            .expect("a hit at the shooter's view");
        assert_eq!(hit.id, 1);
        assert_eq!(hit.view, view);
        // Where it stands now (tick 29), 0.63 m further on: a miss.
        let now = View {
            tick: 29,
            fraction: 0.0,
        };
        assert_eq!(h.ray(now, eye, aim, 100.0, Some(7)), None);
        // A view from before the window is tested at its oldest tick instead.
        let old = h.clamp(View {
            tick: 3,
            fraction: 0.9,
        });
        assert_eq!(
            old,
            Some(View {
                tick: 18,
                fraction: 0.0
            })
        );
        // The shooter never hits itself.
        assert_eq!(
            h.ray(view, DVec3::new(0.0, 1.0, -5.0), DVec3::Z, 10.0, Some(7)),
            None
        );
    }
}
