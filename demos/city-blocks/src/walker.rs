//! A first-person walker in a scene of convex pieces (#290): the shared character
//! ([`crate::character::Player`], Jolt's `CharacterVirtual`) in a world of its own that holds
//! the pieces within reach of its feet, kept as it goes, on a ground that may have holes.
//!
//! - **The pieces** ([`Pieces`]) are anything it walks on or against: walls, floors, stair
//!   steps, partitions, guards, furniture, each a convex piece given by its corners. Floors are
//!   pieces like the rest, so a scene has as many storeys as it builds, below the ground too.
//! - **They are kept by their outline:** each piece's footprint goes into every cell it
//!   overlaps, and those whose footprint lies within [`WalkSettings::reach`] of the feet become
//!   fixed bodies, dropped past 1.3 times it. A long wall is kept while any part of it is near.
//! - **The ground** ([`Ground`]) is a slab, whole or with rectangular holes where flights go
//!   below it, kept whole from the start.
//!
//! The city plan's streets and houses walk it (D-059's step 4), and scenes outside the
//! repository can too.

use std::collections::HashMap;

use anyhow::Result;
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Shape, World, WorldDesc};
use forge_sim::TICK;
use glam::{DVec3, Vec2, Vec3};

use crate::character::Player;

/// Its eyes over its feet, metres.
pub const EYES: f32 = 1.65;
/// Ticks run at most in a frame; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;
/// How far the ground runs on round its area, metres.
const GROUND_EXTENT: f32 = 20_000.0;
/// A piece over more cells than this is kept from the start rather than by its cells.
const MOST_CELLS: i64 = 400;

/// How a walker keeps its pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkSettings {
    /// The cells the pieces are found by, metres a side.
    pub cell: f32,
    /// How far from the feet a piece's footprint is kept, metres; dropped past 1.3 times it.
    pub reach: f32,
    /// The pieces' bodies' convex radius, metres (Jolt's rounding of their edges).
    pub convex_radius: f32,
}

impl Default for WalkSettings {
    /// The city plan's: cells of 50 m, pieces within 80 m, edges rounded by 5 cm.
    fn default() -> Self {
        Self {
            cell: 50.0,
            reach: 80.0,
            convex_radius: 0.05,
        }
    }
}

/// The pieces to walk on and against, found by the cells their footprints overlap.
pub struct Pieces {
    pieces: Vec<Vec<Vec3>>,
    /// Each piece's footprint: its least and greatest x and z.
    bounds: Vec<[Vec2; 2]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The pieces over more than [`MOST_CELLS`] cells, kept from the start.
    always: Vec<u32>,
    cell: f32,
}

impl Pieces {
    /// The pieces, each a convex piece given by its corners in the world's metres, found by cells
    /// of `settings.cell`.
    pub fn new(pieces: Vec<Vec<Vec3>>, settings: &WalkSettings) -> Self {
        let cell = settings.cell;
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut always = Vec::new();
        let mut bounds = Vec::with_capacity(pieces.len());
        for (i, piece) in pieces.iter().enumerate() {
            let b = footprint(piece);
            let (lo, hi) = (cell_of(b[0], cell), cell_of(b[1], cell));
            let count = i64::from(hi.0 - lo.0 + 1) * i64::from(hi.1 - lo.1 + 1);
            if count > MOST_CELLS {
                always.push(i as u32);
            } else {
                for x in lo.0..=hi.0 {
                    for z in lo.1..=hi.1 {
                        cells.entry((x, z)).or_default().push(i as u32);
                    }
                }
            }
            bounds.push(b);
        }
        Self {
            pieces,
            bounds,
            cells,
            always,
            cell,
        }
    }

    /// How many pieces.
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// The pieces whose footprints lie within `r` of `p` (the world's x and z), each once, in
    /// order, the ones kept from the start included.
    pub fn near(&self, p: Vec2, r: f32) -> Vec<u32> {
        let (lo, hi) = (
            cell_of(p - Vec2::splat(r), self.cell),
            cell_of(p + Vec2::splat(r), self.cell),
        );
        let mut out: Vec<u32> = self.always.clone();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                if let Some(list) = self.cells.get(&(x, z)) {
                    out.extend(list.iter().copied().filter(|&i| self.distance(i, p) <= r));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The distance from `p` to piece `i`'s footprint, metres (0 inside it).
    fn distance(&self, i: u32, p: Vec2) -> f32 {
        let [lo, hi] = self.bounds[i as usize];
        (lo - p).max(p - hi).max(Vec2::ZERO).length()
    }
}

/// A piece's footprint: its least and greatest x and z.
fn footprint(piece: &[Vec3]) -> [Vec2; 2] {
    piece.iter().fold(
        [Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)],
        |[lo, hi], p| {
            let q = Vec2::new(p.x, p.z);
            [lo.min(q), hi.max(q)]
        },
    )
}

/// The cell of a point.
fn cell_of(p: Vec2, cell: f32) -> (i32, i32) {
    ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32)
}

/// The ground under the pieces: a slab, whole or with holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Ground {
    /// Its top, metres.
    pub top: f32,
    /// Its thickness, metres. Over a cellar its underside is the cellar's ceiling: a slab thicker
    /// than the gap to the ceiling below stops a capsule's head at the foot of the flight.
    pub thickness: f32,
    /// The rectangle its holes are cut in, its least and greatest x and z; outside it the ground
    /// runs on whole. None: whole everywhere.
    pub area: Option<[Vec2; 2]>,
    /// The holes, each its least and greatest x and z, within `area`: where flights go below it.
    pub holes: Vec<[Vec2; 2]>,
}

impl Ground {
    /// A whole slab, its top at `top`, `thickness` thick.
    pub fn slab(top: f32, thickness: f32) -> Self {
        Self {
            top,
            thickness,
            area: None,
            holes: Vec::new(),
        }
    }

    /// Its rectangles, each its least and greatest x and z: the area less its holes (bands
    /// between the holes' edges, cut where the holes are), then four round the area out to
    /// 20 km; one rectangle 40 km a side without an area.
    pub fn rectangles(&self) -> Vec<[Vec2; 2]> {
        let far = GROUND_EXTENT;
        let Some([lo, hi]) = self.area else {
            return vec![[Vec2::splat(-far), Vec2::splat(far)]];
        };
        let holes: Vec<[Vec2; 2]> = self
            .holes
            .iter()
            .map(|h| [h[0].max(lo), h[1].min(hi)])
            .filter(|h| h[0].x < h[1].x && h[0].y < h[1].y)
            .collect();
        // Bands along z between every hole's edges.
        let mut zs: Vec<f32> = vec![lo.y, hi.y];
        for h in &holes {
            zs.push(h[0].y);
            zs.push(h[1].y);
        }
        zs.sort_by(f32::total_cmp);
        zs.dedup();
        let mut out = Vec::new();
        for band in zs.windows(2) {
            let (z0, z1) = (band[0], band[1]);
            if z1 <= z0 {
                continue;
            }
            // The holes across this band, by x, merged.
            let mut cut: Vec<(f32, f32)> = holes
                .iter()
                .filter(|h| h[0].y < z1 && h[1].y > z0)
                .map(|h| (h[0].x, h[1].x))
                .collect();
            cut.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut x = lo.x;
            for (a, b) in cut {
                if a > x {
                    out.push([Vec2::new(x, z0), Vec2::new(a, z1)]);
                }
                x = x.max(b);
            }
            if x < hi.x {
                out.push([Vec2::new(x, z0), Vec2::new(hi.x, z1)]);
            }
        }
        // Round the area: the sides along z full length, the ends between them.
        out.push([Vec2::new(-far, -far), Vec2::new(lo.x, far)]);
        out.push([Vec2::new(hi.x, -far), Vec2::new(far, far)]);
        out.push([Vec2::new(lo.x, -far), Vec2::new(hi.x, lo.y)]);
        out.push([Vec2::new(lo.x, hi.y), Vec2::new(hi.x, far)]);
        out
    }
}

/// The walker, its world and the pieces it keeps.
pub struct Walker {
    world: World,
    player: Player,
    character: CharacterId,
    kept: HashMap<u32, BodyId>,
    settings: WalkSettings,
    /// Seconds not yet ticked.
    owed: f64,
}

impl Walker {
    /// A walker standing at `feet` (the world's metres) among `pieces`, on `ground`.
    pub fn new(
        pieces: &Pieces,
        ground: &Ground,
        feet: Vec3,
        settings: WalkSettings,
    ) -> Result<Self> {
        let mut world = World::new(&WorldDesc::default());
        let half_thick = 0.5 * ground.thickness;
        for [lo, hi] in ground.rectangles() {
            let half = 0.5 * (hi - lo);
            let middle = 0.5 * (hi + lo);
            let slab = Shape::cuboid(Vec3::new(half.x, half_thick, half.y), 0.05, 1000.0)?;
            world.add_body(&BodyDesc::fixed(
                &slab,
                DVec3::new(
                    f64::from(middle.x),
                    f64::from(ground.top) - f64::from(half_thick),
                    f64::from(middle.y),
                ),
            ))?;
        }
        let character = world.add_character(&CharacterDesc {
            position: feet.as_dvec3(),
            ..CharacterDesc::default()
        });
        let mut walker = Self {
            world,
            player: Player {
                character: Some(character),
                ..Player::default()
            },
            character,
            kept: HashMap::new(),
            settings,
            owed: 0.0,
        };
        walker.keep(pieces)?;
        tracing::info!(
            at = %format_args!("{:.1},{:.1},{:.1}", feet.x, feet.y, feet.z),
            pieces = walker.kept.len(),
            "the walker (#290)"
        );
        Ok(walker)
    }

    /// Its feet, the world's metres.
    pub fn feet(&self) -> Vec3 {
        self.world.character(self.character).position.as_vec3()
    }

    /// How many pieces it holds as bodies.
    pub fn kept(&self) -> usize {
        self.kept.len()
    }

    /// Walks `walk` (m/s along the ground, the world's x and z) for the ticks `dt` seconds owe,
    /// at most four, then keeps the pieces round its feet.
    pub fn advance(&mut self, pieces: &Pieces, walk: [f32; 2], dt: f64) {
        self.player.walk = walk;
        self.owed += dt;
        let tick = f64::from(TICK);
        let due = (self.owed / tick).floor();
        self.owed -= due * tick;
        for _ in 0..(due as u32).min(CATCH_UP) {
            self.player.tick(&mut self.world, None, 0, TICK, |_| None);
        }
        if let Err(e) = self.keep(pieces) {
            tracing::warn!("the walker's pieces: {e}");
        }
    }

    /// The pieces within reach as bodies, those past 1.3 times it dropped.
    fn keep(&mut self, pieces: &Pieces) -> Result<()> {
        let feet = self.feet();
        let p = Vec2::new(feet.x, feet.z);
        let reach = self.settings.reach;
        let mut changed = false;
        for i in pieces.near(p, reach) {
            if self.kept.contains_key(&i) {
                continue;
            }
            let points = &pieces.pieces[i as usize];
            let Ok(shape) = Shape::convex_hull(points, self.settings.convex_radius, 1000.0) else {
                continue;
            };
            let body = self.world.add_body(&BodyDesc::fixed(&shape, DVec3::ZERO))?;
            self.kept.insert(i, body);
            changed = true;
        }
        let always = &pieces.always;
        let mut far: Vec<u32> = self
            .kept
            .keys()
            .copied()
            .filter(|&i| !always.contains(&i) && pieces.distance(i, p) > reach * 1.3)
            .collect();
        far.sort_unstable();
        for i in far {
            if let Some(body) = self.kept.remove(&i) {
                self.world.remove_body(body);
                changed = true;
            }
        }
        if changed {
            self.world.optimize_broad_phase();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box's eight corners.
    fn cuboid(lo: Vec3, hi: Vec3) -> Vec<Vec3> {
        (0..8)
            .map(|k| {
                Vec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                )
            })
            .collect()
    }

    #[test]
    fn a_long_wall_is_near_while_any_of_it_is() {
        // A wall 200 m long along x from 0: its middle 100 m away from a point by its end.
        let wall = cuboid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(200.0, 3.0, 0.2));
        let post = cuboid(Vec3::new(500.0, 0.0, 0.0), Vec3::new(500.2, 1.0, 0.2));
        let pieces = Pieces::new(vec![wall, post], &WalkSettings::default());
        assert_eq!(pieces.near(Vec2::new(-10.0, 5.0), 20.0), vec![0]);
        assert_eq!(pieces.near(Vec2::new(250.0, 0.0), 40.0), Vec::<u32>::new());
        assert_eq!(pieces.near(Vec2::new(250.0, 0.0), 300.0), vec![0, 1]);
        // A piece over more cells than MOST_CELLS is always near.
        let huge = cuboid(
            Vec3::new(-5000.0, 0.0, -5000.0),
            Vec3::new(5000.0, 0.1, 5000.0),
        );
        let pieces = Pieces::new(vec![huge], &WalkSettings::default());
        assert_eq!(pieces.near(Vec2::new(9000.0, 9000.0), 1.0), vec![0]);
    }

    #[test]
    fn the_ground_with_holes_covers_its_area_but_the_holes_once() {
        let ground = Ground {
            top: 0.0,
            thickness: 0.15,
            area: Some([Vec2::new(-10.0, -10.0), Vec2::new(10.0, 10.0)]),
            holes: vec![
                [Vec2::new(2.0, -1.0), Vec2::new(5.0, 1.0)],
                [Vec2::new(-6.0, 0.0), Vec2::new(-3.0, 4.0)],
                // Overlapping the first, and one reaching out of the area.
                [Vec2::new(4.0, 0.5), Vec2::new(6.0, 2.0)],
                [Vec2::new(8.0, 8.0), Vec2::new(12.0, 12.0)],
            ],
        };
        let rects = ground.rectangles();
        let inside =
            |r: &[Vec2; 2], p: Vec2| p.x > r[0].x && p.x < r[1].x && p.y > r[0].y && p.y < r[1].y;
        let in_hole = |p: Vec2| ground.holes.iter().any(|h| inside(h, p));
        // Every point of a grid over and round the area: in exactly one rectangle, or in a hole.
        for i in 0..120 {
            for j in 0..120 {
                let p = Vec2::new(-14.95 + i as f32 * 0.25, -14.95 + j as f32 * 0.25);
                let n = rects.iter().filter(|r| inside(r, p)).count();
                let hole = in_hole(p) && p.x.abs() < 10.0 && p.y.abs() < 10.0;
                assert_eq!(n, usize::from(!hole), "{p}");
            }
        }
        // Whole: one slab.
        assert_eq!(Ground::slab(0.16, 2.0).rectangles().len(), 1);
    }

    #[test]
    fn a_walker_goes_down_a_flight_through_a_hole_in_a_thin_ground() {
        // A ground 15 cm thick with a hole over a flight of ten steps down to a cellar 2.5 m
        // under it, the cellar's floor beyond the flight's foot.
        let ground = Ground {
            top: 0.0,
            thickness: 0.15,
            area: Some([Vec2::new(-10.0, -10.0), Vec2::new(20.0, 10.0)]),
            holes: vec![[Vec2::new(2.0, -1.0), Vec2::new(5.0, 1.0)]],
        };
        let mut pieces = Vec::new();
        for k in 0..10 {
            let top = -0.25 * (k + 1) as f32;
            let x = 2.0 + 0.3 * k as f32;
            pieces.push(cuboid(
                Vec3::new(x, top - 0.2, -1.0),
                Vec3::new(x + 0.3, top, 1.0),
            ));
        }
        pieces.push(cuboid(
            Vec3::new(5.0, -2.7, -3.0),
            Vec3::new(12.0, -2.5, 3.0),
        ));
        let settings = WalkSettings::default();
        let pieces = Pieces::new(pieces, &settings);
        let mut walker =
            Walker::new(&pieces, &ground, Vec3::new(0.0, 0.01, 0.0), settings).unwrap();
        for _ in 0..240 {
            walker.advance(&pieces, [2.0, 0.0], f64::from(TICK));
        }
        let feet = walker.feet();
        assert!(feet.x > 6.0 && (feet.y + 2.5).abs() < 0.05, "{feet}");
        // A ground 1 m thick would have stopped its head at the flight's foot: the capsule could
        // not pass under the slab into the cellar.
        let thick = Ground {
            thickness: 1.0,
            ..ground
        };
        let mut walker = Walker::new(&pieces, &thick, Vec3::new(0.0, 0.01, 0.0), settings).unwrap();
        for _ in 0..240 {
            walker.advance(&pieces, [2.0, 0.0], f64::from(TICK));
        }
        assert!(walker.feet().x < 5.5, "{}", walker.feet());
    }
}
