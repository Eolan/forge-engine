//! Crossings (#316, D-076): a body carried over a passage, from where it stands on the side it
//! leaves to the floor of the other, while a clip plays.
//!
//! A crossing has three targets in the passage's frame: the **entry** (the feet where the
//! crossing starts), the **top** (over the passage's middle line, the sill and the spec's lift
//! under the feet) and the **exit** (past the far face by the body's radius, on the far floor).
//! The feet reach the top and then the exit:
//! - **Without a clip,** a curve stands in. Before the top the feet rise early and go ahead late;
//!   after it they go ahead early and come down late, so they clear both faces of the wall. The
//!   top's time is set so that the speed ahead is the same on both sides of it.
//! - **With a clip,** its root path ([`RootPath`]) is warped onto the targets: each part of it is
//!   scaled along each axis of the crossing's frame, so that it meets the top at its window's end
//!   and the exit at its own. This is Witkin and Popović's motion warping (1995); the live patents
//!   on traversal claim how ledges are found and chosen, not the warp
//!   (`docs/research/navigation-traversal.md`, §4).
//!
//! The body is carried kinematically: what it would meet on the way is not tested, and the
//! controller checks that it fits at the exit. Only `+ − × ÷` (D-016).

use glam::Vec3;

use crate::passage::{CrossingKind, Passage};

/// How far past the far face, beyond the body's radius, the exit is, metres.
const EXIT_CLEAR: f32 = 0.05;
/// An axis along which a clip's root moves less than this in a part is not scaled, metres: the
/// part's error along it is spread over its time instead.
const SCALED_AXIS: f32 = 0.05;

/// How a crossing of some kind goes, as data: one per kind and archetype.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossingSpec {
    /// Seconds it lasts.
    pub seconds: f32,
    /// How far the feet clear the sill at the top, metres (for a jump, the rise over the floor).
    pub lift: f32,
}

impl CrossingSpec {
    /// A person's crossing of each kind, to start with (D-076; the clips will set them).
    pub fn person(kind: CrossingKind) -> Self {
        let (seconds, lift) = match kind {
            CrossingKind::Walk => (0.8, 0.0),
            CrossingKind::ClimbThrough => (1.4, 0.05),
            CrossingKind::Vault => (0.8, 0.15),
            CrossingKind::Climb => (1.8, 0.05),
            CrossingKind::Jump => (0.7, 0.4),
        };
        Self { seconds, lift }
    }
}

/// A clip's root path over a crossing: where its root is at evenly spaced times from the clip's
/// start (the first point, zero) to its end, in the character's frame at the start (`x` right,
/// `y` up, `−z` ahead), as `forge_anim`'s root motion gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct RootPath {
    /// The root's places, two at least, the first at zero.
    pub points: Vec<Vec3>,
    /// The part of the clip, 0 to 1, at which its root is over the passage's middle: the end of
    /// its first warp window.
    pub top_at: f32,
}

impl RootPath {
    /// Its place at `s` (0 to 1 over the clip), between the points.
    fn at(&self, s: f32) -> Vec3 {
        let last = self.points.len() - 1;
        let x = s.clamp(0.0, 1.0) * last as f32;
        let k = (x as usize).min(last.saturating_sub(1));
        let f = x - k as f32;
        self.points[k] + (self.points[(k + 1).min(last)] - self.points[k]) * f
    }
}

/// A crossing under way: its targets and how the feet go between them.
#[derive(Clone, Debug, PartialEq)]
pub struct CrossingMotion {
    /// The crossing.
    pub kind: CrossingKind,
    /// The feet where it starts, metres.
    pub entry: Vec3,
    /// The feet over the passage's middle line.
    pub top: Vec3,
    /// The feet on the far floor, past the far face.
    pub exit: Vec3,
    /// Ahead: horizontal and unit, from the side it leaves to the other.
    pub ahead: Vec3,
    /// Seconds it lasts.
    pub seconds: f32,
    /// The part of it, 0 to 1, at which the feet are at the top.
    pub top_at: f32,
    /// The clip's root path when the crossing has one.
    pub root: Option<RootPath>,
}

impl CrossingMotion {
    /// The crossing of `passage` by a body of `radius` metres whose feet are at `feet`, as `spec`
    /// says: from the side its feet are on to the other, over the span's point nearest them (kept
    /// a radius inside its ends). `None` when it leads back over a one-way passage.
    pub fn over(passage: &Passage, feet: Vec3, radius: f32, spec: CrossingSpec) -> Option<Self> {
        let across = Vec3::new(passage.across.x, 0.0, passage.across.z).normalize_or_zero();
        let (a, b) = (passage.ends[0], passage.ends[1]);
        let forward = (feet - passage.middle()).dot(across) <= 0.0;
        if !forward && !passage.both_ways {
            return None;
        }
        let ahead = if forward { across } else { -across };
        // The span's point nearest the feet, a radius inside its ends (its middle when narrower).
        let span = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
        let width = span.length();
        let along = if width > 2.0 * radius {
            let u = (feet - a).dot(span / width);
            u.clamp(radius, width - radius)
        } else {
            0.5 * width
        };
        let line = if width > 0.0 {
            a + span * (along / width)
        } else {
            a
        };
        let top = Vec3::new(line.x, a.y + passage.sill + spec.lift, line.z);
        let floor = if forward { a.y + passage.drop } else { a.y };
        let out = ahead * (0.5 * passage.depth + radius + EXIT_CLEAR);
        let exit = Vec3::new(line.x + out.x, floor, line.z + out.z);
        // The top's time: the same speed ahead on both sides of it (ease-in, then ease-out).
        let before = (top - feet).dot(ahead).max(0.0);
        let after = (exit - top).dot(ahead).max(0.0);
        let top_at = if before + after > 0.0 {
            before / (before + after)
        } else {
            0.5
        };
        Some(Self {
            kind: passage.crossing(),
            entry: feet,
            top,
            exit,
            ahead,
            seconds: spec.seconds,
            top_at,
            root: None,
        })
    }

    /// This crossing landing at `at` instead of past the far face: where an agent's path goes
    /// on from the link (`Follow::link_end`), a place on its field. The top's time is set again;
    /// a clip's ([`Self::with_root`]) comes after.
    pub fn landing(mut self, at: Vec3) -> Self {
        self.exit = at;
        let before = (self.top - self.entry).dot(self.ahead).max(0.0);
        let after = (self.exit - self.top).dot(self.ahead).max(0.0);
        self.top_at = if before + after > 0.0 {
            before / (before + after)
        } else {
            0.5
        };
        self
    }

    /// This crossing played with a clip: its root path warped onto the targets, the top met at
    /// the path's `top_at`, over the clip's own seconds.
    pub fn with_root(mut self, root: RootPath, seconds: f32) -> Self {
        self.top_at = root.top_at;
        self.seconds = seconds;
        self.root = Some(root);
        self
    }

    /// The feet `t` seconds into it (held at the exit after its end).
    pub fn at(&self, t: f32) -> Vec3 {
        let s = if self.seconds > 0.0 {
            (t / self.seconds).clamp(0.0, 1.0)
        } else {
            1.0
        };
        match &self.root {
            Some(root) => self.warped(root, s),
            None => self.curve(s),
        }
    }

    /// Whether it is over `t` seconds in.
    pub fn done(&self, t: f32) -> bool {
        t >= self.seconds
    }

    /// The curve without a clip, at `s` of the crossing.
    fn curve(&self, s: f32) -> Vec3 {
        let up = Vec3::Y;
        if s < self.top_at {
            // Up early, ahead late.
            let u = s / self.top_at;
            let (rise, go) = (1.0 - (1.0 - u) * (1.0 - u), u * u);
            self.part(self.entry, self.top, up, go, rise)
        } else {
            // Ahead early, down late.
            let u = if self.top_at < 1.0 {
                (s - self.top_at) / (1.0 - self.top_at)
            } else {
                1.0
            };
            let (go, fall) = (1.0 - (1.0 - u) * (1.0 - u), u * u);
            self.part(self.top, self.exit, up, go, fall)
        }
    }

    /// From `from` to `to`: `go` of the way in plan, `climb` of it in height.
    fn part(&self, from: Vec3, to: Vec3, up: Vec3, go: f32, climb: f32) -> Vec3 {
        let d = to - from;
        let vertical = up * d.dot(up);
        from + (d - vertical) * go + vertical * climb
    }

    /// The clip's root path at `s` of the crossing, warped onto the targets.
    fn warped(&self, root: &RootPath, s: f32) -> Vec3 {
        let (s0, s1, from, to) = if s < root.top_at {
            (0.0, root.top_at, self.entry, self.top)
        } else {
            (root.top_at, 1.0, self.top, self.exit)
        };
        let u = if s1 > s0 { (s - s0) / (s1 - s0) } else { 1.0 };
        // The crossing's frame: ahead, up, right; the clip's: −z, y, x.
        let right = self.ahead.cross(Vec3::Y);
        let frame = |p: Vec3| Vec3::new(-p.z, p.y, p.x);
        let (c0, c1, c) = (frame(root.at(s0)), frame(root.at(s1)), frame(root.at(s)));
        let d = to - from;
        let target = Vec3::new(d.dot(self.ahead), d.y, d.dot(right));
        let (moved, now) = (c1 - c0, c - c0);
        let mut out = [0.0; 3];
        for k in 0..3 {
            out[k] = if moved[k].abs() > SCALED_AXIS {
                now[k] * (target[k] / moved[k])
            } else {
                now[k] + (target[k] - moved[k]) * u
            };
        }
        from + self.ahead * out[0] + Vec3::Y * out[1] + right * out[2]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passage::{PassageKind, PassageState};

    /// A garden wall 1 m high and 0.2 m thick along x, from x = −2 to 2, crossed towards +z.
    fn wall(kind: PassageKind, sill: f32, drop: f32) -> Passage {
        Passage {
            kind,
            ends: [Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)],
            across: Vec3::Z,
            sill,
            head: None,
            depth: 0.2,
            drop,
            both_ways: true,
            state: PassageState::Open,
            piece: 7,
        }
    }

    /// The crossing's feet sampled at 60 Hz, the end included.
    fn samples(m: &CrossingMotion) -> Vec<Vec3> {
        let n = (m.seconds * 60.0) as usize;
        (0..=n)
            .map(|k| m.at(k as f32 / n as f32 * m.seconds))
            .collect()
    }

    #[test]
    fn a_vault_clears_both_faces_and_lands_past_the_far_one() {
        let fence = wall(PassageKind::LowFence, 1.0, 0.0);
        let spec = CrossingSpec::person(CrossingKind::Vault);
        let feet = Vec3::new(0.7, 0.0, -0.45);
        let m = CrossingMotion::over(&fence, feet, 0.3, spec).unwrap();
        assert_eq!(m.kind, CrossingKind::Vault);
        assert_eq!(m.at(0.0), feet);
        assert!((m.at(m.seconds) - Vec3::new(0.7, 0.0, 0.45)).length() < 1e-5);
        let path = samples(&m);
        for (k, p) in path.iter().enumerate() {
            // Over the wall's thickness, the feet are over its top.
            if p.z.abs() <= 0.1 {
                assert!(p.y > 1.0, "{k}: {p}");
            }
            // Never past the top, never back.
            assert!(p.y <= 1.15 + 1e-5, "{k}: {p}");
            if k > 0 {
                let step = *p - path[k - 1];
                assert!(step.z >= -1e-6 && step.length() < 0.12, "{k}: {step}");
            }
        }
    }

    #[test]
    fn a_crossing_from_the_far_side_heads_back_and_down_a_drop() {
        // Side B's floor 0.5 m under side A's: from B, the feet climb 1.55 m and land at 0.
        let fence = wall(PassageKind::TallFence, 1.0, -0.5);
        let spec = CrossingSpec::person(CrossingKind::Climb);
        let feet = Vec3::new(-1.9, -0.5, 0.6);
        let m = CrossingMotion::over(&fence, feet, 0.3, spec).unwrap();
        assert_eq!(m.ahead, Vec3::NEG_Z);
        // Kept a radius inside the span's end.
        assert!(
            (m.top - Vec3::new(-1.7, 1.05, 0.0)).length() < 1e-5,
            "{}",
            m.top
        );
        assert!(
            (m.exit - Vec3::new(-1.7, 0.0, -0.45)).length() < 1e-5,
            "{}",
            m.exit
        );
        // A one-way passage is not crossed back.
        let one_way = Passage {
            both_ways: false,
            ..fence
        };
        assert!(CrossingMotion::over(&one_way, feet, 0.3, spec).is_none());
        // Landing where a path goes on: there at the end, the top still over the middle.
        let landed = CrossingMotion::over(&fence, feet, 0.3, spec)
            .unwrap()
            .landing(Vec3::new(-1.7, 0.0, -0.8));
        assert_eq!(landed.at(landed.seconds), Vec3::new(-1.7, 0.0, -0.8));
        assert!((landed.at(landed.top_at * landed.seconds) - landed.top).length() < 1e-5);
    }

    #[test]
    fn a_clips_root_path_is_warped_onto_the_passage() {
        // A vault clip made for a 0.9 m wall, its root up 0.9 m half way and 1.2 m ahead at the
        // end (−z ahead in its frame), drifting 3 cm to the right.
        let points: Vec<Vec3> = (0..=20)
            .map(|k| {
                let s = k as f32 / 20.0;
                let up = 0.9 * (1.0 - (2.0 * s - 1.0) * (2.0 * s - 1.0));
                Vec3::new(0.03 * s, up, -1.2 * s)
            })
            .collect();
        let root = RootPath {
            points,
            top_at: 0.5,
        };
        let fence = wall(PassageKind::LowFence, 1.0, 0.0);
        let spec = CrossingSpec::person(CrossingKind::Vault);
        let m = CrossingMotion::over(&fence, Vec3::new(0.0, 0.0, -0.6), 0.3, spec)
            .unwrap()
            .with_root(root, 0.9);
        assert_eq!(m.seconds, 0.9);
        // The top at its window's end, the exit at the clip's.
        assert!((m.at(0.45) - m.top).length() < 1e-5, "{}", m.at(0.45));
        assert!((m.at(0.9) - m.exit).length() < 1e-5, "{}", m.at(0.9));
        assert!((m.at(0.0) - m.entry).length() < 1e-6);
        // Its shape kept: the rise scaled to the wall, still a smooth arch, always ahead.
        let path = samples(&m);
        for w in path.windows(2) {
            assert!(w[1].z >= w[0].z - 1e-6, "{} {}", w[0], w[1]);
        }
        let quarter = m.at(0.225);
        assert!((quarter.y - 0.75 * 1.15).abs() < 0.02, "{quarter}");
        // The drift under 5 cm is spread over each part's time, back on the line at its end.
        assert!(quarter.x.abs() < 0.03, "{quarter}");
    }
}
