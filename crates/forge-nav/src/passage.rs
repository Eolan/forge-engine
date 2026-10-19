//! Passages (#316, D-076): the places a wall piece can be crossed other than by walking through
//! it, as data made with the pieces (a window's sill, a low garden wall, a gap), never detected
//! from the geometry. A passage carries its span, its heights and its state; its kind fixes the
//! crossing ([`CrossingKind`]), whatever the height of the body that crosses it (D-076, kept clear
//! of Techland's US 8,979,652 B1 and US 9,238,175 B2). The navigation takes a passage as a
//! [`Link`] for the archetypes that may cross it; a player crosses the one its movement meets
//! ([`Passages::met_by`]), automatically (the owner, 2026-10-19), never one picked from those near
//! by a distance score.

use glam::{Vec2, Vec3};

use crate::Link;

/// What a passage crosses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PassageKind {
    /// A doorway (the field walks an open one; the passage carries its state).
    Door,
    /// A window.
    Window,
    /// A wall or fence a person vaults (D-076: up to about 1.1 m).
    LowFence,
    /// A wall or fence a climber climbs (up to about 2.2 m).
    TallFence,
    /// A gap a person jumps (up to about 1.5 m across).
    Gap,
}

/// How a passage is crossed: fixed by its kind (D-076).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CrossingKind {
    /// Walked through (a door).
    Walk,
    /// Climbed through (a window).
    ClimbThrough,
    /// Vaulted (a low fence).
    Vault,
    /// Climbed over (a tall fence).
    Climb,
    /// Jumped (a gap).
    Jump,
}

impl PassageKind {
    /// The crossing this kind takes.
    pub fn crossing(self) -> CrossingKind {
        match self {
            PassageKind::Door => CrossingKind::Walk,
            PassageKind::Window => CrossingKind::ClimbThrough,
            PassageKind::LowFence => CrossingKind::Vault,
            PassageKind::TallFence => CrossingKind::Climb,
            PassageKind::Gap => CrossingKind::Jump,
        }
    }
}

/// A passage's state, switched at run time (a window opened, smashed or barred).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PassageState {
    /// Open: crossed as its kind says.
    Open,
    /// Shut: crossed only by who opens or breaks it.
    Shut,
    /// Broken (a smashed window): open, the glass left in its frame the game's to judge.
    Broken,
    /// Barred: never crossed.
    Barred,
}

/// A passage (#316): where a wall piece is crossed from its side A to its side B.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Passage {
    /// What it crosses.
    pub kind: PassageKind,
    /// Its span along the wall's middle line: its two ends, on side A's floor, metres.
    pub ends: [Vec3; 2],
    /// From side A to side B: horizontal, unit, across the span.
    pub across: Vec3,
    /// The height a body passes over, above side A's floor: a window's sill, a fence's top, 0 for
    /// a doorway or a gap.
    pub sill: f32,
    /// A window's or a doorway's head above side A's floor; `None` for a fence or a gap.
    pub head: Option<f32>,
    /// The wall's thickness (a gap's width across), metres.
    pub depth: f32,
    /// Side B's floor less side A's, metres.
    pub drop: f32,
    /// Whether it is crossed from B to A too.
    pub both_ways: bool,
    /// Its state now.
    pub state: PassageState,
    /// The game's own number for the piece it crosses.
    pub piece: u32,
}

impl Passage {
    /// Its clear width along its span, metres.
    pub fn width(&self) -> f32 {
        horizontal(self.ends[1] - self.ends[0]).length()
    }

    /// Its middle on the wall's middle line, on side A's floor.
    pub fn middle(&self) -> Vec3 {
        (self.ends[0] + self.ends[1]) * 0.5
    }

    /// The crossing its kind takes.
    pub fn crossing(&self) -> CrossingKind {
        self.kind.crossing()
    }

    /// The link that crosses it for the navigation ([`crate::Field::path`]): from side A's floor
    /// `reach` metres out from the wall's face to side B's, at `cost` seconds, carrying the
    /// game's `kind`; open unless barred (a shut one is the archetype's to judge: pass
    /// `enabled` as it may open or break it).
    pub fn link(&self, cost: f32, kind: u16, reach: f32, enabled: bool) -> Link {
        let out = self.across * (self.depth * 0.5 + reach);
        let m = self.middle();
        Link {
            a: m - out,
            b: m + out + Vec3::Y * self.drop,
            cost,
            kind,
            both_ways: self.both_ways,
            enabled: enabled && self.state != PassageState::Barred,
        }
    }

    /// Where along a movement from `from` to `to` it meets the passage (0 at `from`, 1 at `to`):
    /// in plan, the movement crosses its span's line within its ends, heading from A to B (or
    /// back, when it leads back), the mover on the floor of the side it leaves (within `step`
    /// metres). `None` when it does not meet it.
    pub fn met_by(&self, from: Vec3, to: Vec3, step: f32) -> Option<f32> {
        let (p, d) = (plan(from), plan(to) - plan(from));
        let (a, e) = (plan(self.ends[0]), plan(self.ends[1]) - plan(self.ends[0]));
        let heading = d.dot(plan(self.across));
        let forward = heading > 0.0;
        if heading == 0.0 || (!forward && !self.both_ways) {
            return None;
        }
        // p + d t = a + e u, solved by the cross products (D-016: no transcendental function).
        let denominator = d.perp_dot(e);
        if denominator == 0.0 {
            return None;
        }
        let w = a - p;
        let t = w.perp_dot(e) / denominator;
        let u = w.perp_dot(d) / denominator;
        if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
            return None;
        }
        let floor = if forward {
            self.ends[0].y
        } else {
            self.ends[0].y + self.drop
        };
        ((from.y - floor).abs() <= step).then_some(t)
    }
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn plan(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// The passages of a region, found by the cells of [`PASSAGE_CELL`] metres their spans cross.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passages {
    list: Vec<Passage>,
    /// Per cell (x, z), the passages whose span's bounds overlap it, in order.
    cells: std::collections::BTreeMap<(i32, i32), Vec<u32>>,
}

/// The side of a [`Passages`] cell, metres.
pub const PASSAGE_CELL: f32 = 8.0;

fn cell_of(p: Vec2) -> (i32, i32) {
    (
        (p.x / PASSAGE_CELL).floor() as i32,
        (p.y / PASSAGE_CELL).floor() as i32,
    )
}

impl Passages {
    /// The passages `list`, indexed.
    pub fn new(list: Vec<Passage>) -> Self {
        let mut cells: std::collections::BTreeMap<(i32, i32), Vec<u32>> = Default::default();
        for (k, p) in list.iter().enumerate() {
            let (a, b) = (plan(p.ends[0]), plan(p.ends[1]));
            let (lo, hi) = (cell_of(a.min(b)), cell_of(a.max(b)));
            for x in lo.0..=hi.0 {
                for z in lo.1..=hi.1 {
                    cells.entry((x, z)).or_default().push(k as u32);
                }
            }
        }
        Self { list, cells }
    }

    /// All of them, in order.
    pub fn all(&self) -> &[Passage] {
        &self.list
    }

    /// Passage `k`.
    pub fn get(&self, k: usize) -> &Passage {
        &self.list[k]
    }

    /// Switches passage `k`'s state (a window opened, smashed or barred).
    pub fn set_state(&mut self, k: usize, state: PassageState) {
        self.list[k].state = state;
    }

    /// The passages whose span's bounds come within `r` metres of `p` in plan, each once, in
    /// order.
    pub fn near(&self, p: Vec3, r: f32) -> Vec<usize> {
        let q = plan(p);
        let (lo, hi) = (cell_of(q - Vec2::splat(r)), cell_of(q + Vec2::splat(r)));
        let mut out: Vec<usize> = Vec::new();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                if let Some(list) = self.cells.get(&(x, z)) {
                    out.extend(list.iter().map(|&k| k as usize));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|&k| {
            let s = &self.list[k];
            let (a, b) = (plan(s.ends[0]), plan(s.ends[1]));
            let (lo, hi) = (a.min(b) - Vec2::splat(r), a.max(b) + Vec2::splat(r));
            q.x >= lo.x && q.y >= lo.y && q.x <= hi.x && q.y <= hi.y
        });
        out
    }

    /// The passage a movement from `from` to `to` meets first, and where along it: the one a
    /// player crosses (D-076: automatically, on walking into it). Barred ones are not met; ties
    /// go to the lower index.
    pub fn met_by(&self, from: Vec3, to: Vec3, step: f32) -> Option<(usize, f32)> {
        let reach = (plan(to) - plan(from)).length();
        let mid = (from + to) * 0.5;
        self.near(mid, reach * 0.5 + 0.01)
            .into_iter()
            .filter(|&k| self.list[k].state != PassageState::Barred)
            .filter_map(|k| self.list[k].met_by(from, to, step).map(|t| (k, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
    }

    /// The links of the passages an archetype crosses, for a path's search: `rule` gives, for
    /// each passage, its link's cost (seconds) and the game's kind, or `None` where the archetype
    /// does not cross it. A barred passage is left out.
    pub fn links(&self, reach: f32, rule: impl Fn(&Passage) -> Option<(f32, u16)>) -> Vec<Link> {
        self.list
            .iter()
            .filter(|p| p.state != PassageState::Barred)
            .filter_map(|p| rule(p).map(|(cost, kind)| p.link(cost, kind, reach, true)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window in a wall along x at z = 5, 1 m wide from x = 2 to 3, its sill at 0.9 m, side A
    /// the low z.
    fn window() -> Passage {
        Passage {
            kind: PassageKind::Window,
            ends: [Vec3::new(2.0, 0.0, 5.0), Vec3::new(3.0, 0.0, 5.0)],
            across: Vec3::Z,
            sill: 0.9,
            head: Some(2.1),
            depth: 0.3,
            drop: 0.0,
            both_ways: true,
            state: PassageState::Open,
            piece: 7,
        }
    }

    #[test]
    fn a_movement_meets_the_passage_it_crosses_and_no_other() {
        let w = window();
        assert_eq!(w.width(), 1.0);
        assert_eq!(w.crossing(), CrossingKind::ClimbThrough);
        // Straight across its middle, half way along the movement.
        let t = w.met_by(Vec3::new(2.5, 0.0, 4.5), Vec3::new(2.5, 0.0, 5.5), 0.4);
        assert_eq!(t, Some(0.5));
        // Beside it, along it, short of it, or on another storey: not met.
        assert!(
            w.met_by(Vec3::new(3.5, 0.0, 4.5), Vec3::new(3.5, 0.0, 5.5), 0.4)
                .is_none()
        );
        assert!(
            w.met_by(Vec3::new(2.0, 0.0, 4.8), Vec3::new(3.0, 0.0, 4.8), 0.4)
                .is_none()
        );
        assert!(
            w.met_by(Vec3::new(2.5, 0.0, 4.0), Vec3::new(2.5, 0.0, 4.9), 0.4)
                .is_none()
        );
        assert!(
            w.met_by(Vec3::new(2.5, 3.0, 4.5), Vec3::new(2.5, 3.0, 5.5), 0.4)
                .is_none()
        );
        // Back from B to A only where it leads back.
        let back = (Vec3::new(2.5, 0.0, 5.5), Vec3::new(2.5, 0.0, 4.5));
        assert!(w.met_by(back.0, back.1, 0.4).is_some());
        let one_way = Passage {
            both_ways: false,
            ..window()
        };
        assert!(one_way.met_by(back.0, back.1, 0.4).is_none());
    }

    #[test]
    fn the_first_passage_met_is_crossed_and_a_barred_one_never() {
        let near = window();
        let far = Passage {
            ends: [Vec3::new(2.0, 0.0, 7.0), Vec3::new(3.0, 0.0, 7.0)],
            kind: PassageKind::LowFence,
            sill: 1.0,
            head: None,
            ..window()
        };
        let mut all = Passages::new(vec![far, near]);
        let (from, to) = (Vec3::new(2.5, 0.0, 4.0), Vec3::new(2.5, 0.0, 8.0));
        assert_eq!(all.met_by(from, to, 0.4).map(|m| m.0), Some(1));
        assert_eq!(all.near(Vec3::new(2.5, 0.0, 7.2), 0.5), vec![0]);
        all.set_state(1, PassageState::Barred);
        assert_eq!(all.met_by(from, to, 0.4).map(|m| m.0), Some(0));
        // The links an archetype crosses: a climber the window and the fence, a wheelchair none.
        all.set_state(1, PassageState::Open);
        let climber = all.links(0.5, |p| Some((1.5, p.kind as u16)));
        assert_eq!(climber.len(), 2);
        let wheelchair = all.links(0.5, |p| (p.kind == PassageKind::Door).then_some((0.0, 0)));
        assert!(wheelchair.is_empty());
        // A window's link stands half its depth and the reach out from its middle, either side.
        let link = all.get(1).link(1.5, 1, 0.5, true);
        assert!((link.a.z - 4.35).abs() < 1e-5 && (link.b.z - 5.65).abs() < 1e-5);
    }
}
