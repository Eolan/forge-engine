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
}

/// A cell no ground covers.
const NONE: u8 = u8::MAX;

impl Footing {
    /// An empty grid over the square of side `size` metres from `corner` (its least x and z);
    /// `None` when it would be too large.
    pub(crate) fn new(corner: Vec2, size: f32) -> Option<Self> {
        let side = (size / CELL).ceil() as usize;
        (side <= MOST_SIDE).then(|| Self {
            corner,
            side,
            cells: vec![NONE; side * side],
        })
    }

    /// Marks the cells whose middle lies in the triangle `p` (seen from above) as `section`.
    pub(crate) fn paint(&mut self, p: [Vec3; 3], section: u8) {
        let q = p.map(|v| (Vec2::new(v.x, v.z) - self.corner) / CELL);
        let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
        if area.abs() < 1e-9 {
            return;
        }
        let lo = q[0].min(q[1]).min(q[2]).floor().max(Vec2::ZERO);
        let hi = q[0].max(q[1]).max(q[2]).ceil();
        let last = (self.side - 1) as f32;
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
                    self.cells[y * self.side + x] = section;
                }
            }
        }
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
}
