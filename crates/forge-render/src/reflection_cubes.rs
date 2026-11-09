//! Cubes for what the screen cannot show (D-108, step 3): what a reflected ray meets when it misses.
//!
//! A cube is a volume of the scene (a room, say) and a point inside it the scene is drawn from, six
//! faces, into one tile of an octahedral atlas whose mips are the roughness. A point inside a
//! volume reads its cube, the ray taken to the volume's walls and back to the cube's point; a point
//! inside none reads the sky.
//!
//! This file is the part that needs no GPU: which cubes have tiles, which faces to draw this frame.
//! [`crate::MeshletRenderer`] records the passes.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Texels along a side of a tile, at the atlas's level 0.
pub const CUBE_TILE_TEXELS: u32 = 256;
/// Tiles along a side of the atlas.
pub const CUBE_TILES: u32 = 8;
/// Texels along a side of the atlas.
pub const CUBE_ATLAS_TEXELS: u32 = CUBE_TILES * CUBE_TILE_TEXELS;
/// The atlas's levels: level 0 a mirror, the last the roughest that reads it.
pub const CUBE_LEVELS: u32 = 5;
/// The most cubes the atlas holds.
pub const CUBE_CAPACITY: usize = (CUBE_TILES * CUBE_TILES) as usize;
/// The most cubes the shading looks through (`CubeSet` in `meshlet.slang`).
pub const CUBE_LIST: usize = 32;
/// Texels of a face as it is drawn.
pub const CUBE_FACE_TEXELS: u32 = 256;
/// How far from a cube's point a face begins, metres.
const CUBE_NEAR: f32 = 0.1;
/// How near the camera a cube must be to be drawn, metres: past this, what it holds is not wanted.
const CUBE_REACH: f32 = 96.0;
/// The least distance a face sees, metres, whatever the volume's size.
const LEAST_FACE_REACH: f32 = 64.0;
/// Frames between the starts of a cube's rounds of drawing, however often what it holds changes.
const ROUND_APART: u32 = 240;
/// The sun's turn past which a cube is drawn again, as the cosine of the angle.
const SUN_MOVED_COSINE: f32 = 0.9962; // 5 degrees

/// A cube's six faces: where each looks, and which way is up.
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// A volume a cube stands for, in the scene frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeVolume {
    /// The point its faces are drawn from.
    pub center: Vec3,
    /// Its lowest corner: the walls a reflected ray is taken to.
    pub min: Vec3,
    /// Its highest corner.
    pub max: Vec3,
}

impl CubeVolume {
    /// How far `point` lies outside it, 0 inside.
    fn distance(&self, point: Vec3) -> f32 {
        ((self.min - point).max(point - self.max))
            .max(Vec3::ZERO)
            .length()
    }
}

/// Mirrors `Cube` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCube {
    /// The point its faces were drawn from.
    pub center: [f32; 4],
    /// Its lowest corner.
    pub min: [f32; 4],
    /// Its highest corner.
    pub max: [f32; 4],
    /// Its tile in the atlas's level 0, in tiles: x, y, then unused.
    pub tile: [u32; 4],
}

/// Mirrors `CubeSet` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCubeSet {
    /// The atlas's sampled index.
    pub image: u32,
    /// Cubes that follow.
    pub count: u32,
    /// Keeps what follows on 16 bytes.
    pub pad: [u32; 2],
    /// The nearest cubes that are whole.
    pub cubes: [GpuCube; CUBE_LIST],
}

/// One face to draw this frame.
#[derive(Clone, Copy, Debug)]
pub struct CubeDraw {
    /// Which cube of those given.
    pub cube: usize,
    /// Which of its six faces.
    pub face: u32,
    /// Its tile, in tiles.
    pub tile: [u32; 2],
    /// The point it is drawn from, in the scene frame.
    pub center: Vec3,
    /// From a point relative to `center` to the face's clip space.
    pub view_proj: Mat4,
    /// Its view alone.
    pub view: Mat4,
    /// Where it looks.
    pub forward: Vec3,
    /// Its near plane, metres.
    pub near: f32,
    /// Its projection's scale.
    pub scale: f32,
    /// How far it sees, metres: well past its volume's walls.
    pub reach: f32,
}

/// A frame's work on the cubes.
#[derive(Clone, Debug, Default)]
pub struct CubePlan {
    /// The faces to draw.
    pub draws: Vec<CubeDraw>,
    /// Tiles whose six faces are new: their mips are to be made after the draws.
    pub finished: Vec<[u32; 2]>,
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    tile: [u32; 2],
    /// The faces to draw again, a bit each.
    waiting: u8,
    /// The faces of this round already drawn.
    fresh: u8,
    /// Whether all six have been drawn at least once.
    whole: bool,
    /// The sun it was last drawn under.
    sun: Vec3,
    /// It is to be drawn again when this round is over and enough frames have gone.
    again: bool,
    /// Frames since its last round began.
    since_round: u32,
}

/// The cubes' state.
#[derive(Clone, Debug, Default)]
pub struct CubeSet {
    volumes: Vec<CubeVolume>,
    slots: Vec<Option<Slot>>,
    free: Vec<[u32; 2]>,
    /// Nearest first, the frame's order.
    near: Vec<usize>,
}

impl CubeSet {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cubes the scene has, which forgets what was drawn.
    pub fn set_volumes(&mut self, volumes: Vec<CubeVolume>) {
        self.slots = vec![None; volumes.len()];
        self.volumes = volumes;
        self.free = (0..CUBE_TILES)
            .rev()
            .flat_map(|y| (0..CUBE_TILES).rev().map(move |x| [x, y]))
            .collect();
        self.near.clear();
    }

    /// How many cubes the scene gave.
    pub fn len(&self) -> usize {
        self.volumes.len()
    }

    /// Whether the scene gave none.
    pub fn is_empty(&self) -> bool {
        self.volumes.is_empty()
    }

    /// Every cube is to be drawn again.
    pub fn invalidate(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            slot.waiting = 0x3F;
        }
    }

    /// This frame's work. `camera` and `changed` (spheres where what stays has changed) in the
    /// scene frame, `sun` the direction to it; `budget` the faces it may draw.
    pub fn plan(
        &mut self,
        camera: Vec3,
        sun: Vec3,
        changed: &[(Vec3, f32)],
        budget: usize,
    ) -> CubePlan {
        // The nearest have tiles: those further give theirs up.
        let mut order: Vec<usize> = (0..self.volumes.len()).collect();
        let volumes = &self.volumes;
        order.sort_by(|a, b| {
            let (da, db) = (volumes[*a].distance(camera), volumes[*b].distance(camera));
            da.total_cmp(&db).then(a.cmp(b))
        });
        for &i in order.iter().skip(CUBE_CAPACITY) {
            if let Some(slot) = self.slots[i].take() {
                self.free.push(slot.tile);
            }
        }
        for &i in order.iter().take(CUBE_CAPACITY) {
            if self.slots[i].is_none() {
                let Some(tile) = self.free.pop() else {
                    continue;
                };
                self.slots[i] = Some(Slot {
                    tile,
                    waiting: 0x3F,
                    fresh: 0,
                    whole: false,
                    sun,
                    again: false,
                    since_round: 0,
                });
            }
        }
        for (slot, volume) in self.slots.iter_mut().zip(&self.volumes) {
            let Some(slot) = slot else {
                continue;
            };
            let touched = changed.iter().any(|(c, r)| volume.distance(*c) <= *r);
            slot.since_round = slot.since_round.saturating_add(1);
            if touched || slot.sun.dot(sun) < SUN_MOVED_COSINE {
                slot.again = true;
                slot.sun = sun;
            }
            // Whatever keeps changing, a round that has begun is finished.
            if slot.again && slot.waiting == 0 && slot.since_round >= ROUND_APART {
                slot.again = false;
                slot.since_round = 0;
                slot.waiting = 0x3F;
            }
        }
        self.near = order;
        let mut plan = CubePlan::default();
        let mut left = budget;
        for &i in &self.near {
            let volume = self.volumes[i];
            if volume.distance(camera) > CUBE_REACH {
                // The rest are further still.
                break;
            }
            let Some(slot) = self.slots[i].as_mut() else {
                continue;
            };
            for face in 0..6u32 {
                let bit = 1u8 << face;
                if slot.waiting & bit == 0 {
                    continue;
                }
                if left == 0 {
                    break;
                }
                left -= 1;
                slot.waiting &= !bit;
                slot.fresh |= bit;
                let (forward, up) = FACES[face as usize];
                let view = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, forward, up);
                let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(
                    std::f32::consts::FRAC_PI_2,
                    1.0,
                    CUBE_NEAR,
                );
                plan.draws.push(CubeDraw {
                    cube: i,
                    face,
                    tile: slot.tile,
                    center: volume.center,
                    view_proj: projection * view,
                    view,
                    forward,
                    near: CUBE_NEAR,
                    scale: projection.x_axis.x,
                    reach: (2.0 * (volume.max - volume.min).length()).max(LEAST_FACE_REACH),
                });
            }
            if slot.fresh == 0x3F {
                slot.fresh = 0;
                slot.whole = true;
                plan.finished.push(slot.tile);
            }
            if left == 0 {
                break;
            }
        }
        plan
    }

    /// The table the shading reads: the nearest cubes that are whole.
    pub fn table(&self, image: u32) -> GpuCubeSet {
        let mut set = GpuCubeSet {
            image,
            count: 0,
            pad: [0; 2],
            cubes: [GpuCube::zeroed(); CUBE_LIST],
        };
        let whole = self
            .near
            .iter()
            .filter_map(|&i| Some((i, self.slots[i]?)))
            .filter(|(_, slot)| slot.whole);
        for (i, slot) in whole.take(CUBE_LIST) {
            let v = &self.volumes[i];
            set.cubes[set.count as usize] = GpuCube {
                center: v.center.extend(0.0).to_array(),
                min: v.min.extend(0.0).to_array(),
                max: v.max.extend(0.0).to_array(),
                tile: [slot.tile[0], slot.tile[1], 0, 0],
            };
            set.count += 1;
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(x: f32) -> CubeVolume {
        CubeVolume {
            center: Vec3::new(x, 1.5, 0.0),
            min: Vec3::new(x - 3.0, 0.0, -3.0),
            max: Vec3::new(x + 3.0, 3.0, 3.0),
        }
    }

    /// Frames pass, with nothing drawn: up to the last before a waiting round may begin.
    fn settle(c: &mut CubeSet, sun: Vec3) {
        for _ in 0..ROUND_APART {
            assert!(c.plan(Vec3::ZERO, sun, &[], 0).draws.is_empty());
        }
    }

    #[test]
    fn six_faces_make_a_cube_whole() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        let plan = c.plan(Vec3::ZERO, Vec3::Y, &[], 4);
        assert_eq!(plan.draws.len(), 4);
        assert!(plan.finished.is_empty());
        assert_eq!(c.table(7).count, 0, "not read until whole");
        let plan = c.plan(Vec3::ZERO, Vec3::Y, &[], 4);
        assert_eq!(plan.draws.len(), 2);
        assert_eq!(plan.finished.len(), 1);
        let set = c.table(7);
        assert_eq!((set.image, set.count), (7, 1));
    }

    #[test]
    fn a_whole_cube_is_kept() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        assert!(c.plan(Vec3::ZERO, Vec3::Y, &[], 6).draws.is_empty());
    }

    #[test]
    fn the_nearest_go_first() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(100.0), room(0.0)]);
        let plan = c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        assert!(plan.draws.iter().all(|d| d.cube == 1));
    }

    #[test]
    fn what_changes_inside_draws_it_again() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        let far = [(Vec3::new(50.0, 0.0, 0.0), 1.0)];
        assert!(c.plan(Vec3::ZERO, Vec3::Y, &far, 6).draws.is_empty());
        let near = [(Vec3::new(1.0, 1.0, 0.0), 0.5)];
        assert!(
            c.plan(Vec3::ZERO, Vec3::Y, &near, 6).draws.is_empty(),
            "not at once"
        );
        settle(&mut c, Vec3::Y);
        assert_eq!(c.plan(Vec3::ZERO, Vec3::Y, &[], 6).draws.len(), 6);
    }

    #[test]
    fn what_keeps_changing_does_not_stop_a_cube_from_being_whole() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        let near = [(Vec3::new(1.0, 1.0, 0.0), 0.5)];
        for _ in 0..6 {
            c.plan(Vec3::ZERO, Vec3::Y, &near, 1);
        }
        assert_eq!(
            c.table(0).count,
            1,
            "six faces, one a frame, whatever changed"
        );
    }

    #[test]
    fn a_small_turn_of_the_sun_is_let_be_and_a_large_one_is_not() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        let little = Vec3::new(0.05, 1.0, 0.0).normalize();
        assert!(c.plan(Vec3::ZERO, little, &[], 6).draws.is_empty());
        let turned = Vec3::new(0.3, 1.0, 0.0).normalize();
        c.plan(Vec3::ZERO, turned, &[], 6);
        settle(&mut c, turned);
        assert_eq!(c.plan(Vec3::ZERO, turned, &[], 6).draws.len(), 6);
    }

    #[test]
    fn a_redrawn_cube_stays_readable() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        c.invalidate();
        c.plan(Vec3::ZERO, Vec3::Y, &[], 2);
        assert_eq!(
            c.table(0).count,
            1,
            "what it held stays until the new one is whole"
        );
    }

    #[test]
    fn more_cubes_than_tiles_keep_the_nearest() {
        let mut c = CubeSet::new();
        c.set_volumes(
            (0..CUBE_CAPACITY + 10)
                .map(|i| room(i as f32 * 10.0))
                .collect(),
        );
        c.plan(Vec3::ZERO, Vec3::Y, &[], 6 * (CUBE_CAPACITY + 10));
        assert_eq!(
            c.slots.iter().flatten().count(),
            CUBE_CAPACITY,
            "every tile in use"
        );
        assert!(c.slots[CUBE_CAPACITY + 9].is_none());
        // Further along, the far ones take the tiles of those left behind.
        let far = Vec3::new((CUBE_CAPACITY + 9) as f32 * 10.0, 0.0, 0.0);
        c.plan(far, Vec3::Y, &[], 6 * (CUBE_CAPACITY + 10));
        assert!(c.slots[CUBE_CAPACITY + 9].is_some());
        assert!(c.slots[0].is_none());
    }

    #[test]
    fn a_cube_past_the_reach_waits() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(500.0)]);
        assert!(c.plan(Vec3::ZERO, Vec3::Y, &[], 6).draws.is_empty());
        let near = Vec3::new(480.0, 0.0, 0.0);
        assert_eq!(c.plan(near, Vec3::Y, &[], 6).draws.len(), 6);
    }

    #[test]
    fn a_face_sees_past_its_walls() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![CubeVolume {
            center: Vec3::ZERO,
            min: Vec3::splat(-100.0),
            max: Vec3::splat(100.0),
        }]);
        let plan = c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        assert!(plan.draws[0].reach > 300.0);
    }

    #[test]
    fn a_face_looks_where_its_table_says() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(0.0)]);
        let plan = c.plan(Vec3::ZERO, Vec3::Y, &[], 6);
        let plus_x = plan.draws.iter().find(|d| d.face == 0).unwrap();
        let clip = plus_x.view_proj * glam::Vec4::new(5.0, 0.0, 0.0, 1.0);
        assert!((clip.w - 5.0).abs() < 1e-3, "{clip:?}");
        assert!(clip.x.abs() < 1e-3 && clip.y.abs() < 1e-3);
        assert!(clip.z > 0.0, "reversed-Z: near over distance");
    }

    #[test]
    fn the_shading_list_is_nearest_first() {
        let mut c = CubeSet::new();
        c.set_volumes(vec![room(100.0), room(0.0)]);
        c.plan(Vec3::ZERO, Vec3::Y, &[], 12);
        assert_eq!(c.table(0).cubes[0].center[0], 0.0);
    }
}
