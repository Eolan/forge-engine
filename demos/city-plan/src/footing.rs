//! What a foot lands on outdoors (#318, D-007): the plan's ground section under each metre,
//! painted from the tiles' ground, and the sound of each section's row. Indoors the rooms' floors
//! decide ([`crate::ground_under`]).

use forge_audio::Surface;
use glam::{Vec2, Vec3};

/// The grid's cell, metres: a lawn's edge heard within half a metre.
pub(crate) const CELL: f32 = 1.0;

/// The most cells a side: plans larger than 2 km (the city) keep the streets' stone.
const MOST_SIDE: usize = 2048;

/// The ground's sections under the plan, a cell each (`NONE` where no ground was laid).
pub(crate) struct Footing {
    corner: Vec2,
    side: usize,
    cells: Vec<u8>,
    /// The cells a house stands on ([`Self::paint_built`]): the ground under it is laid too.
    built: Vec<bool>,
}

/// A cell no ground covers.
const NONE: u8 = u8::MAX;

/// Metres between the samples of [`Footing::open_share`].
const OPEN_STEP: f32 = 4.0;

impl Footing {
    /// An empty grid over the square of side `size` metres from `corner` (its least x and z);
    /// `None` when it would be too large.
    pub(crate) fn new(corner: Vec2, size: f32) -> Option<Self> {
        let side = (size / CELL).ceil() as usize;
        (side <= MOST_SIDE).then(|| Self {
            corner,
            side,
            cells: vec![NONE; side * side],
            built: vec![false; side * side],
        })
    }

    /// Marks the cells whose middle lies in the triangle `p` (seen from above) as `section`.
    pub(crate) fn paint(&mut self, p: [Vec3; 3], section: u8) {
        let q = p.map(|v| Vec2::new(v.x, v.z));
        for i in self.cells_in(q) {
            self.cells[i] = section;
        }
    }

    /// Marks the cells whose middle lies in the quad `corners` (x and z, either way round) as
    /// built on: a house's ground floor (#357).
    pub(crate) fn paint_built(&mut self, corners: [Vec2; 4]) {
        for tri in [[0, 1, 2], [0, 2, 3]] {
            for i in self.cells_in(tri.map(|k| corners[k])) {
                self.built[i] = true;
            }
        }
    }

    /// The distance from `at` (seen from above) to the nearest house painted by
    /// [`Self::paint_built`], to [`OPEN_STEP`] metres: `None` when none stands within `reach`.
    pub(crate) fn nearest_built(&self, at: Vec3, reach: f32) -> Option<f32> {
        let n = (reach / OPEN_STEP).ceil() as i32;
        let mut nearest: Option<f32> = None;
        for j in -n..=n {
            for i in -n..=n {
                let d = Vec2::new(i as f32, j as f32) * OPEN_STEP;
                let r = d.length();
                if r > reach || nearest.is_some_and(|m| r >= m) {
                    continue;
                }
                let c = ((Vec2::new(at.x, at.z) + d - self.corner) / CELL).floor();
                if c.x < 0.0 || c.y < 0.0 {
                    continue;
                }
                let (x, y) = (c.x as usize, c.y as usize);
                if x < self.side && y < self.side && self.built[y * self.side + x] {
                    nearest = Some(r);
                }
            }
        }
        nearest
    }

    /// The cells whose middle lies in the triangle `p` (x and z), by their index.
    fn cells_in(&self, p: [Vec2; 3]) -> Vec<usize> {
        let q = p.map(|v| (v - self.corner) / CELL);
        let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
        if area.abs() < 1e-9 {
            return Vec::new();
        }
        let lo = q[0].min(q[1]).min(q[2]).floor().max(Vec2::ZERO);
        let hi = q[0].max(q[1]).max(q[2]).ceil();
        let last = (self.side - 1) as f32;
        let mut cells = Vec::new();
        for y in lo.y as usize..=(hi.y.min(last) as usize) {
            for x in lo.x as usize..=(hi.x.min(last) as usize) {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                // Inside when on the same side of the three edges as the triangle's area.
                let w = [
                    (q[1] - q[0]).perp_dot(c - q[0]),
                    (q[2] - q[1]).perp_dot(c - q[1]),
                    (q[0] - q[2]).perp_dot(c - q[2]),
                ];
                if w.iter().all(|&w| w * area >= 0.0) {
                    cells.push(y * self.side + x);
                }
            }
        }
        cells
    }

    /// How many cells each section covers, by section (the last entry: none laid).
    pub(crate) fn counts(&self) -> Vec<usize> {
        let mut counts = vec![0; 256];
        for &c in &self.cells {
            counts[usize::from(c)] += 1;
        }
        let laid = counts[..usize::from(NONE)]
            .iter()
            .rposition(|&n| n > 0)
            .map_or(0, |k| k + 1);
        let none = counts[usize::from(NONE)];
        counts.truncate(laid);
        counts.push(none);
        counts
    }

    /// The section under `at`, if ground was laid there.
    pub(crate) fn section_at(&self, at: Vec3) -> Option<u8> {
        let c = ((Vec2::new(at.x, at.z) - self.corner) / CELL).floor();
        if c.x < 0.0 || c.y < 0.0 {
            return None;
        }
        let (x, y) = (c.x as usize, c.y as usize);
        if x >= self.side || y >= self.side {
            return None;
        }
        Some(self.cells[y * self.side + x]).filter(|&s| s != NONE)
    }

    /// The share of the ground within `reach` of `at` (seen from above) that is open, 0 to 1:
    /// sampled every [`OPEN_STEP`] metres, a sample is open on grass, gravel or water and past
    /// the plan, built under a house ([`Self::paint_built`]) and where the ground is stone, boards
    /// or metal or none was laid. The ambience's cicadas sing only in the open (#357).
    pub(crate) fn open_share(&self, at: Vec3, reach: f32, sounds: &[Option<Surface>]) -> f32 {
        let n = (reach / OPEN_STEP).floor() as i32;
        let (mut open, mut all) = (0_u32, 0_u32);
        for j in -n..=n {
            for i in -n..=n {
                let d = Vec2::new(i as f32, j as f32) * OPEN_STEP;
                if d.length() > reach {
                    continue;
                }
                all += 1;
                let p = at + Vec3::new(d.x, 0.0, d.y);
                let c = ((Vec2::new(p.x, p.z) - self.corner) / CELL).floor();
                let inside = c.x >= 0.0
                    && c.y >= 0.0
                    && (c.x as usize) < self.side
                    && (c.y as usize) < self.side;
                let house = inside && self.built[c.y as usize * self.side + c.x as usize];
                let surface = self
                    .section_at(p)
                    .map(|s| sounds.get(usize::from(s)).copied().flatten());
                let is_open = !house
                    && match surface {
                        Some(Some(Surface::Grass | Surface::Gravel | Surface::Water)) => true,
                        Some(_) => false,
                        None => !inside,
                    };
                open += u32::from(is_open);
            }
        }
        if all == 0 {
            1.0
        } else {
            open as f32 / all as f32
        }
    }

    /// What a foot at `at` lands on: the section's sound from `sounds` (by section), `beyond`
    /// where no ground was laid.
    pub(crate) fn surface_at(
        &self,
        at: Vec3,
        sounds: &[Option<Surface>],
        beyond: Surface,
    ) -> Surface {
        self.section_at(at)
            .and_then(|s| sounds.get(usize::from(s)).copied().flatten())
            .unwrap_or(beyond)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lawn triangle over a paved square: the cells whose middles it covers are the lawn's,
    /// the rest the paving's, and nothing past the grid.
    #[test]
    fn a_foot_lands_on_the_section_painted_under_it() {
        let mut f = Footing::new(Vec2::new(-10.0, -10.0), 20.0).expect("a small grid");
        let square = [
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, -10.0),
            Vec3::new(10.0, 0.0, 10.0),
        ];
        f.paint(square, 3);
        f.paint([square[0], square[2], Vec3::new(-10.0, 0.0, 10.0)], 3);
        f.paint(
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(6.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 6.0),
            ],
            1,
        );
        assert_eq!(f.section_at(Vec3::new(1.2, 5.0, 1.7)), Some(1));
        assert_eq!(f.section_at(Vec3::new(5.5, 0.0, 5.5)), Some(3));
        assert_eq!(f.section_at(Vec3::new(-4.0, 0.0, 8.0)), Some(3));
        assert_eq!(f.section_at(Vec3::new(10.5, 0.0, 0.0)), None);
        let sounds = [None, Some(Surface::Grass), None, Some(Surface::Stone)];
        assert_eq!(
            f.surface_at(Vec3::new(1.2, 0.0, 1.7), &sounds, Surface::Wood),
            Surface::Grass
        );
        assert_eq!(
            f.surface_at(Vec3::new(-20.0, 0.0, 0.0), &sounds, Surface::Wood),
            Surface::Wood
        );
        assert!(Footing::new(Vec2::ZERO, 5000.0).is_none());
    }

    /// The open share (#357): a meadow is open, a paved square and a house (no ground laid) are
    /// not, and their edge is in between; past the plan is open country.
    #[test]
    fn the_open_share_counts_grass_and_not_paving_or_houses() {
        let mut f = Footing::new(Vec2::new(-100.0, -100.0), 200.0).expect("a grid");
        let quad = |f: &mut Footing, x0: f32, x1: f32, section: u8| {
            let [a, b, c, d] = [
                Vec3::new(x0, 0.0, -100.0),
                Vec3::new(x1, 0.0, -100.0),
                Vec3::new(x1, 0.0, 100.0),
                Vec3::new(x0, 0.0, 100.0),
            ];
            f.paint([a, b, c], section);
            f.paint([a, c, d], section);
        };
        // A meadow west of x = 0, paving from 0 to 40, a house (nothing laid) east of 40, and a
        // house on the meadow's ground round (-60, 60).
        quad(&mut f, -100.0, 0.0, 1);
        quad(&mut f, 0.0, 40.0, 3);
        f.paint_built([
            Vec2::new(-80.0, 40.0),
            Vec2::new(-40.0, 40.0),
            Vec2::new(-40.0, 80.0),
            Vec2::new(-80.0, 80.0),
        ]);
        let sounds = [None, Some(Surface::Grass), None, Some(Surface::Stone)];
        let open = |x: f32| f.open_share(Vec3::new(x, 0.0, 0.0), 20.0, &sounds);
        assert_eq!(open(-50.0), 1.0);
        assert_eq!(open(20.0), 0.0);
        assert_eq!(open(70.0), 0.0);
        let edge = open(0.0);
        assert!(edge > 0.3 && edge < 0.7, "{edge}");
        // Beyond the plan: the open country round it.
        assert_eq!(open(-150.0), 1.0);
        // On the house laid on the meadow: built.
        let on = f.open_share(Vec3::new(-60.0, 0.0, 60.0), 12.0, &sounds);
        assert_eq!(on, 0.0);
        // Its nearest edge from the meadow's middle, 40 m south of it, to a sample's step; none
        // within a shorter reach.
        let near = f.nearest_built(Vec3::new(-60.0, 0.0, 0.0), 70.0);
        assert!(
            near.is_some_and(|d| (d - 40.0).abs() <= OPEN_STEP),
            "{near:?}"
        );
        assert_eq!(f.nearest_built(Vec3::new(-60.0, 0.0, 0.0), 30.0), None);
    }
}
