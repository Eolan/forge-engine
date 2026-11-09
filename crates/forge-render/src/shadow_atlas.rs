//! The lamps' shadow atlas (D-108, step 2): shadow from lamps without rays.
//!
//! One map of 4096 × 4096 texels shared by the lamps that show most, each given slots by halving,
//! as large as its reach stands on the screen. A cone takes a slot, anything else a cube's six
//! faces. A slot is drawn once, kept, and drawn again only when what stands in the lamp's reach
//! has changed. As with the rings the static casters have a layer of their own, and the movers are
//! drawn over a copy of it.
//!
//! This file is the part that needs no GPU: which lamps, which slots, what to draw this frame.
//! [`crate::MeshletRenderer`] records the passes.

use std::collections::{BTreeSet, HashMap};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::lamps::{GpuLamp, LAMP_CAPACITY, LAMP_SHADOWLESS};

/// Texels along a side of the atlas.
pub const ATLAS_TEXELS: u32 = 4096;
/// The smallest slot's side, texels.
pub const SMALLEST_SLOT: u32 = 128;
/// The largest slot's side, texels.
pub const LARGEST_SLOT: u32 = 1024;
/// The most lamps shadowed at once, of the [`LAMP_CAPACITY`] a frame holds: raster modes shadow
/// only a fraction of their lights (D-108).
pub const MAX_SHADOWED_LAMPS: usize = 48;
/// The most faces the atlas's table holds: those shadowed, and some still fading out.
pub const MAX_FACES: usize = (MAX_SHADOWED_LAMPS + 16) * 6;
/// Frames a lamp's slots outlive its last being wanted, so that one flickering at the edge of the
/// list is not drawn again.
const KEPT_FRAMES: u64 = 240;
/// Frames a lamp's shadow takes to come or go, so that one joining or leaving the list of those
/// shadowed does not jump.
const FADE_FRAMES: f32 = 16.0;
/// The widest cone a single slot takes, its half-angle, radians.
const WIDEST_CONE: f32 = 65.0 * std::f32::consts::PI / 180.0;
/// Texels a slot of side `size` is drawn past its face's edge: as far as the shading's taps reach
/// (`atlas_look` in `meshlet.slang`).
fn guard_texels(size: u32) -> f32 {
    (size / 32).max(8) as f32
}

/// A cube's faces: where each looks, and which way is up.
const FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// Mirrors `AtlasFace` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuAtlasFace {
    /// From a point relative to the lamp to this face's clip space.
    pub from_lamp: [f32; 16],
    /// Where the slot lies in the atlas, as fractions: x, y, width, height. A width of 0: no face.
    pub rect: [f32; 4],
}

/// Mirrors `AtlasLamp` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuAtlasLamp {
    /// Its first face in the table.
    pub first_face: u32,
    /// 0: it has none, its light unshadowed; 1: a cone; 6: a cube.
    pub kind: u32,
    /// Its near plane, metres.
    pub near: f32,
    /// Its projection's scale, 1 / the tangent of half its field of view.
    pub scale: f32,
    /// How much of its shadow shows, 0 to 1 (it comes and goes over [`FADE_FRAMES`]).
    pub strength: f32,
}

/// Mirrors the head of `LampAtlasSet` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuAtlasHead {
    /// Sampled index of the composed layer.
    pub image: u32,
    /// Texels along its side.
    pub texels: f32,
    /// Keeps what follows on 16 bytes.
    pub pad: [u32; 2],
}

/// Bytes of a frame's table: the head, a record per lamp, a record per face.
pub const ATLAS_SET_BYTES: u64 = (std::mem::size_of::<GpuAtlasHead>()
    + LAMP_CAPACITY * std::mem::size_of::<GpuAtlasLamp>()
    + MAX_FACES * std::mem::size_of::<GpuAtlasFace>()) as u64;

/// How a lamp is shadowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LampShape {
    /// Not at all: it asked for none.
    None,
    /// A cone, as wide as this half-angle, radians.
    Cone(f32),
    /// Every way, but for the face behind a flat piece.
    Cube(Option<usize>),
}

/// How `lamp` is shadowed (`lamp_flat`, `lamp_tube` in `meshlet.slang`).
pub fn lamp_shape(lamp: &GpuLamp) -> LampShape {
    if lamp.flags[0] & LAMP_SHADOWLESS != 0 {
        return LampShape::None;
    }
    let kind = lamp.beam_edge[2];
    // A tube or a wedge: from its middle, every way.
    if kind < -1.5 {
        return LampShape::Cube(None);
    }
    let axis = Vec3::new(lamp.beam[0], lamp.beam[1], lamp.beam[2]);
    if kind > 0.0 {
        // A flat piece lights its own side: nothing is behind it. The face left out is one with
        // every direction it holds behind the piece, its corners too.
        let n = axis.normalize_or_zero();
        let behind = FACES.iter().position(|(dir, up)| {
            dir.dot(n) + dir.cross(*up).dot(n).abs() + up.dot(n).abs() <= 0.0
        });
        return LampShape::Cube(behind);
    }
    if kind < 0.0 || axis.length_squared() < 1e-6 {
        return LampShape::Cube(None);
    }
    let half = lamp.beam_edge[0].clamp(-1.0, 1.0).acos();
    if half <= WIDEST_CONE {
        LampShape::Cone(half)
    } else {
        LampShape::Cube(None)
    }
}

/// Slots by halving, from whole [`LARGEST_SLOT`] squares.
#[derive(Clone, Debug)]
struct Slots {
    /// Free slots per size, the largest first.
    free: [BTreeSet<(u32, u32)>; 4],
}

fn level_of(size: u32) -> usize {
    (LARGEST_SLOT / size).trailing_zeros() as usize
}

impl Slots {
    fn new() -> Self {
        let mut free: [BTreeSet<(u32, u32)>; 4] = Default::default();
        for y in (0..ATLAS_TEXELS).step_by(LARGEST_SLOT as usize) {
            for x in (0..ATLAS_TEXELS).step_by(LARGEST_SLOT as usize) {
                free[0].insert((x, y));
            }
        }
        Self { free }
    }

    fn take(&mut self, size: u32) -> Option<(u32, u32)> {
        let level = level_of(size);
        if let Some(&slot) = self.free[level].iter().next() {
            self.free[level].remove(&slot);
            return Some(slot);
        }
        if level == 0 {
            return None;
        }
        // Halve one of the next size up: its other three quarters stay free.
        let (x, y) = self.take(size * 2)?;
        self.free[level].insert((x + size, y));
        self.free[level].insert((x, y + size));
        self.free[level].insert((x + size, y + size));
        Some((x, y))
    }

    fn give(&mut self, (x, y): (u32, u32), size: u32) {
        let level = level_of(size);
        if level > 0 {
            let parent = size * 2;
            let (px, py) = (x - x % parent, y - y % parent);
            let quarters = [
                (px, py),
                (px + size, py),
                (px, py + size),
                (px + size, py + size),
            ];
            let others_free = quarters
                .iter()
                .filter(|q| **q != (x, y))
                .all(|q| self.free[level].contains(q));
            if others_free {
                for q in &quarters {
                    self.free[level].remove(q);
                }
                self.give((px, py), parent);
                return;
            }
        }
        self.free[level].insert((x, y));
    }

    #[cfg(test)]
    fn free_area(&self) -> u64 {
        self.free
            .iter()
            .enumerate()
            .map(|(level, set)| {
                let size = u64::from(LARGEST_SLOT >> level);
                set.len() as u64 * size * size
            })
            .sum()
    }
}

/// One face's slot and what it waits for.
#[derive(Clone, Copy, Debug)]
struct Face {
    x: u32,
    y: u32,
    /// The casters that stay are to be drawn.
    wants_static: bool,
    /// The composed layer is to be laid again.
    wants_compose: bool,
    /// The composed layer has been laid at all.
    composed: bool,
}

#[derive(Clone, Debug)]
struct Entry {
    lamp: GpuLamp,
    size: u32,
    shape: LampShape,
    faces: [Option<Face>; 6],
    last_wanted: u64,
    importance: f32,
    /// How much of its shadow shows.
    strength: f32,
    /// The reach its faces were drawn for.
    drawn_reach: f32,
}

impl Entry {
    fn position(&self) -> Vec3 {
        Vec3::new(
            self.lamp.position_radius[0],
            self.lamp.position_radius[1],
            self.lamp.position_radius[2],
        )
    }

    fn reach(&self) -> f32 {
        self.drawn_reach
    }

    fn ready(&self) -> bool {
        self.faces.iter().flatten().all(|f| f.composed) && self.faces.iter().flatten().count() > 0
    }

    fn near(&self) -> f32 {
        self.lamp.position_radius[3].max(0.05)
    }

    /// The clip space of face `index` (a cone has only the first).
    fn view_and_projection(&self, index: usize) -> (Mat4, Mat4, Vec3) {
        let guard = 1.0 + 2.0 * guard_texels(self.size) / self.size as f32;
        let (forward, half_tan) = match self.shape {
            LampShape::Cone(half) => (
                Vec3::new(self.lamp.beam[0], self.lamp.beam[1], self.lamp.beam[2]).normalize(),
                half.tan() * guard,
            ),
            _ => (FACES[index].0, guard),
        };
        let up = match self.shape {
            LampShape::Cone(_) if forward.y.abs() > 0.99 => Vec3::Z,
            LampShape::Cone(_) => Vec3::Y,
            _ => FACES[index].1,
        };
        let view = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, forward, up);
        let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(
            2.0 * half_tan.atan(),
            1.0,
            self.near(),
        );
        (view, projection, forward)
    }

    /// Face `index`, in its slot at (`x`, `y`), to draw.
    fn draw(&self, index: usize, x: u32, y: u32) -> AtlasDraw {
        let (view, projection, forward) = self.view_and_projection(index);
        let mut lamp = self.lamp;
        lamp.colour_reach[3] = self.drawn_reach;
        AtlasDraw {
            lamp,
            slot: [x, y, self.size],
            view_proj: projection * view,
            view,
            forward,
            near: self.near(),
            scale: projection.x_axis.x,
        }
    }
}

/// A lamp as the frame gives it: its stable name, and its record.
#[derive(Clone, Copy, Debug)]
pub struct AtlasLampIn {
    /// Its place in the scene's list of lamps, which stays from frame to frame.
    pub id: u32,
    /// Its record.
    pub lamp: GpuLamp,
}

/// What the atlas needs of the camera.
#[derive(Clone, Copy, Debug)]
pub struct AtlasView {
    /// Where it is, in the scene's frame.
    pub camera: Vec3,
    /// Its frustum's planes, relative to it.
    pub planes: [[f32; 4]; 6],
    /// Pixels per unit of size over distance.
    pub pixels: f32,
}

/// One face to draw this frame.
#[derive(Clone, Debug)]
pub struct AtlasDraw {
    /// Its lamp's record.
    pub lamp: GpuLamp,
    /// The slot, texels: x, y, side.
    pub slot: [u32; 3],
    /// From a point relative to the lamp to the slot's clip space.
    pub view_proj: Mat4,
    /// Its view alone.
    pub view: Mat4,
    /// Where it looks.
    pub forward: Vec3,
    /// Its near plane.
    pub near: f32,
    /// Its projection's scale.
    pub scale: f32,
}

/// What a frame draws of the atlas.
#[derive(Clone, Debug, Default)]
pub struct AtlasPlan {
    /// Faces whose casters that stay are drawn, most important lamps first.
    pub statics: Vec<AtlasDraw>,
    /// Slots laid again from the static layer: x, y, side.
    pub copies: Vec<[u32; 3]>,
    /// Faces the movers are drawn over.
    pub movers: Vec<AtlasDraw>,
}

/// The atlas's state.
#[derive(Clone, Debug)]
pub struct LampAtlas {
    slots: Slots,
    entries: HashMap<u32, Entry>,
    frame: u64,
}

impl Default for LampAtlas {
    fn default() -> Self {
        Self::new()
    }
}

fn sphere_in(planes: &[[f32; 4]; 6], center: Vec3, radius: f32) -> bool {
    planes
        .iter()
        .all(|p| Vec3::new(p[0], p[1], p[2]).dot(center) + p[3] >= -radius)
}

fn luminance(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl LampAtlas {
    /// An empty atlas.
    pub fn new() -> Self {
        Self {
            slots: Slots::new(),
            entries: HashMap::new(),
            frame: 0,
        }
    }

    /// Forgets everything: every lamp is drawn again.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Every face is drawn again, but what is shown stays until it is: what stood in their way
    /// may not have fitted in the lists they were drawn with.
    pub fn redraw_all(&mut self) {
        for entry in self.entries.values_mut() {
            for face in entry.faces.iter_mut().flatten() {
                face.wants_static = true;
            }
        }
    }

    /// Cluster pages have come in, and a face drawn from coarse ones would keep them: a lamp at a
    /// time, in turn, is drawn again. Called before [`Self::plan`].
    pub fn refresh_turn(&mut self) {
        if !self.frame.is_multiple_of(4) {
            return;
        }
        let mut ids: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_wanted == self.frame)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        let Some(&id) = ids.get((self.frame / 4) as usize % ids.len().max(1)) else {
            return;
        };
        let entry = self.entries.get_mut(&id).expect("a lamp just listed");
        for face in entry.faces.iter_mut().flatten() {
            face.wants_static = true;
        }
    }

    fn release(&mut self, id: u32) {
        if let Some(entry) = self.entries.remove(&id) {
            for face in entry.faces.iter().flatten() {
                self.slots.give((face.x, face.y), entry.size);
            }
        }
    }

    /// Slots for `count` faces of `size`, the lamps wanted longest ago giving theirs up.
    fn place(&mut self, count: usize, size: u32) -> Option<Vec<(u32, u32)>> {
        let mut got = Vec::with_capacity(count);
        while got.len() < count {
            match self.slots.take(size) {
                Some(slot) => got.push(slot),
                None => {
                    let oldest = self
                        .entries
                        .iter()
                        .filter(|(_, e)| e.last_wanted < self.frame)
                        .min_by_key(|(id, e)| (e.last_wanted, **id))
                        .map(|(id, _)| *id);
                    match oldest {
                        Some(id) => self.release(id),
                        None => {
                            for slot in got {
                                self.slots.give(slot, size);
                            }
                            return None;
                        }
                    }
                }
            }
        }
        Some(got)
    }

    /// This frame's work. `changed` and `moved` are spheres in the scene's frame where the casters
    /// that stay, and the movers, have changed; `budget` the faces of the first kind a frame may
    /// draw, `mover_budget` of the second.
    pub fn plan(
        &mut self,
        lamps: &[AtlasLampIn],
        view: AtlasView,
        changed: &[(Vec3, f32)],
        moved: &[(Vec3, f32)],
        budget: usize,
        mover_budget: usize,
    ) -> AtlasPlan {
        self.frame += 1;
        for entry in self.entries.values_mut() {
            let (position, reach) = (entry.position(), entry.reach());
            let touches =
                |list: &[(Vec3, f32)]| list.iter().any(|(c, r)| c.distance(position) < reach + r);
            let casters = touches(changed);
            let movers = touches(moved);
            for face in entry.faces.iter_mut().flatten() {
                face.wants_static |= casters;
                face.wants_compose |= movers;
            }
        }
        // Those whose light can reach the view, those that show most first.
        let mut wanted: Vec<(f32, u32, GpuLamp, LampShape, f32)> = lamps
            .iter()
            .filter_map(|l| {
                let shape = lamp_shape(&l.lamp);
                if shape == LampShape::None {
                    return None;
                }
                let p = &l.lamp.position_radius;
                let offset = Vec3::new(p[0], p[1], p[2]) - view.camera;
                let reach = l.lamp.colour_reach[3];
                if !sphere_in(&view.planes, offset, reach) {
                    return None;
                }
                let dist = offset.length();
                let near = dist.max(reach * 0.25).max(0.5);
                let importance = luminance(l.lamp.colour_reach) * reach / (near * near);
                Some((importance, l.id, l.lamp, shape, reach / dist.max(1.0)))
            })
            .collect();
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0));
        wanted.truncate(MAX_SHADOWED_LAMPS);

        // As large as each stands on the screen, then smaller, the least important first, until
        // they fit.
        let faces_of = |shape: LampShape| match shape {
            LampShape::Cone(_) => 1_u64,
            LampShape::Cube(None) => 6,
            _ => 5,
        };
        let mut sizes: Vec<u32> = wanted
            .iter()
            .map(|&(_, _, _, shape, across)| {
                let largest = if matches!(shape, LampShape::Cone(_)) {
                    LARGEST_SLOT
                } else {
                    LARGEST_SLOT / 2
                };
                ((across * view.pixels).ceil() as u32)
                    .max(1)
                    .next_power_of_two()
                    .clamp(SMALLEST_SLOT, largest)
            })
            .collect();
        let room = u64::from(ATLAS_TEXELS) * u64::from(ATLAS_TEXELS) * 9 / 10;
        let area = |sizes: &[u32]| -> u64 {
            sizes
                .iter()
                .zip(&wanted)
                .map(|(s, w)| faces_of(w.3) * u64::from(*s) * u64::from(*s))
                .sum()
        };
        'fit: while area(&sizes) > room {
            let mut shrunk = false;
            for k in (0..sizes.len()).rev() {
                if sizes[k] > SMALLEST_SLOT {
                    sizes[k] /= 2;
                    shrunk = true;
                    if area(&sizes) <= room {
                        break 'fit;
                    }
                }
            }
            if !shrunk {
                break;
            }
        }

        // A lamp in the list is not given up to make room, and one that moved, or now reaches past
        // what was drawn for it, waits for its faces like any other. A reach that follows a
        // flickering brightness settles at its largest.
        for &(importance, id, lamp, shape, _) in &wanted {
            let Some(entry) = self.entries.get_mut(&id).filter(|e| e.shape == shape) else {
                continue;
            };
            entry.last_wanted = self.frame;
            entry.importance = importance;
            if bytemuck::bytes_of(&entry.lamp) != bytemuck::bytes_of(&lamp) {
                let moved_light = entry.lamp.position_radius != lamp.position_radius
                    || entry.lamp.beam != lamp.beam
                    || entry.lamp.beam_edge != lamp.beam_edge;
                let reach = lamp.colour_reach[3];
                let grown = reach > entry.drawn_reach * 1.1;
                entry.lamp = lamp;
                if moved_light || grown {
                    entry.drawn_reach = reach;
                    for face in entry.faces.iter_mut().flatten() {
                        face.wants_static = true;
                    }
                }
            }
        }
        // The faces this frame's budget has to draw before a resized lamp's: those of the lamps
        // still without their shadow. Redrawing one that has it can wait a frame.
        let mut pending: usize = wanted
            .iter()
            .map(
                |w| match self.entries.get(&w.1).filter(|e| e.shape == w.3) {
                    Some(e) if e.ready() => 0,
                    Some(e) => e.faces.iter().flatten().filter(|f| f.wants_static).count(),
                    None => faces_of(w.3) as usize,
                },
            )
            .sum();

        let mut chosen: Vec<u32> = Vec::with_capacity(wanted.len());
        for ((importance, id, lamp, shape, _), want) in wanted.into_iter().zip(sizes) {
            let held = self
                .entries
                .get(&id)
                .filter(|e| e.shape == shape)
                .map(|e| want <= e.size * 2 && e.size < want * 2);
            // Kept until it is well off, so a lamp at a threshold is not drawn again and again; and
            // when it is, until the new slot can be drawn this very frame, or the lamp would go a
            // frame without its shadow.
            let new_faces = faces_of(shape) as usize;
            if held == Some(true) || (held == Some(false) && pending + new_faces > budget) {
                chosen.push(id);
                continue;
            }
            let indices: Vec<usize> = match shape {
                LampShape::Cone(_) => vec![0],
                LampShape::Cube(skip) => (0..6).filter(|i| Some(*i) != skip).collect(),
                LampShape::None => unreachable!("left out above"),
            };
            let strength = if held.is_some() {
                self.entries.get(&id).map_or(0.0, |e| e.strength)
            } else {
                0.0
            };
            // A smaller slot fits in the room of its own larger one. A larger one has to be found
            // first: without room, the lamp keeps what it has.
            let shrinking = held.is_some() && want < self.entries[&id].size;
            if shrinking {
                self.release(id);
            }
            let Some(places) = self.place(indices.len(), want) else {
                if held.is_some() && !shrinking {
                    chosen.push(id);
                } else {
                    self.release(id);
                }
                continue;
            };
            if held.is_some() {
                pending += new_faces;
            }
            if !shrinking {
                self.release(id);
            }
            let mut faces = [None; 6];
            for (&index, &(x, y)) in indices.iter().zip(&places) {
                faces[index] = Some(Face {
                    x,
                    y,
                    wants_static: true,
                    wants_compose: true,
                    composed: false,
                });
            }
            self.entries.insert(
                id,
                Entry {
                    lamp,
                    size: want,
                    shape,
                    faces,
                    last_wanted: self.frame,
                    importance,
                    strength,
                    drawn_reach: lamp.colour_reach[3],
                },
            );
            chosen.push(id);
        }
        let stale: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_wanted + KEPT_FRAMES < self.frame)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.release(id);
        }

        let mut plan = AtlasPlan::default();
        chosen.sort_by(|a, b| {
            let (ea, eb) = (&self.entries[a], &self.entries[b]);
            eb.importance.total_cmp(&ea.importance).then(a.cmp(b))
        });
        let (mut statics_left, mut movers_left) = (budget, mover_budget);
        // The casters that stay. The lamps still without their shadow first, those that show most
        // at the head; then the others' turns go round, so that one that keeps moving does not
        // hold up the rest.
        let turn = self.frame as usize % chosen.len().max(1);
        let waiting = chosen.iter().filter(|id| !self.entries[*id].ready());
        let others = chosen
            .iter()
            .cycle()
            .skip(turn)
            .take(chosen.len())
            .filter(|id| self.entries[*id].ready());
        let order: Vec<u32> = waiting.chain(others).copied().collect();
        for id in &order {
            let entry = self.entries.get_mut(id).expect("a chosen lamp's entry");
            let snapshot = entry.clone();
            for (index, slot) in entry.faces.iter_mut().enumerate() {
                let Some(face) = slot.as_mut().filter(|f| f.wants_static) else {
                    continue;
                };
                if statics_left == 0 {
                    break;
                }
                statics_left -= 1;
                plan.statics.push(snapshot.draw(index, face.x, face.y));
                face.wants_static = false;
                face.wants_compose = true;
            }
        }
        // The movers' turns go round too: the most important lamps first would keep the rest from
        // ever showing a mover that moves every frame.
        for id in chosen.iter().cycle().skip(turn).take(chosen.len()) {
            let entry = self.entries.get_mut(id).expect("a chosen lamp's entry");
            let snapshot = entry.clone();
            for (index, slot) in entry.faces.iter_mut().enumerate() {
                let Some(face) = slot.as_mut().filter(|f| !f.wants_static && f.wants_compose)
                else {
                    continue;
                };
                // A face never laid goes in at once: a lamp is not read until they all are.
                if face.composed {
                    if movers_left == 0 {
                        continue;
                    }
                    movers_left -= 1;
                }
                plan.copies.push([face.x, face.y, snapshot.size]);
                plan.movers.push(snapshot.draw(index, face.x, face.y));
                face.wants_compose = false;
                face.composed = true;
            }
        }
        // A shadow comes once all of it is there, and goes with the lamp from the list.
        let step = 1.0 / FADE_FRAMES;
        for entry in self.entries.values_mut() {
            let on = entry.last_wanted == self.frame && entry.ready();
            entry.strength = (entry.strength + if on { step } else { -step }).clamp(0.0, 1.0);
        }
        plan
    }

    /// The tables a frame's shading reads: per lamp of `lamps` (the frame's order), and the faces.
    pub fn tables(&self, lamps: &[AtlasLampIn]) -> (Vec<GpuAtlasLamp>, Vec<GpuAtlasFace>) {
        let none = GpuAtlasLamp {
            first_face: 0,
            kind: 0,
            near: 0.0,
            scale: 1.0,
            strength: 0.0,
        };
        let empty = GpuAtlasFace {
            from_lamp: [0.0; 16],
            rect: [0.0; 4],
        };
        let mut records = vec![none; lamps.len()];
        let mut faces = Vec::new();
        // Those in the list first: one fading out does not take the room of one that stays.
        for in_list in [true, false] {
            for (record, input) in records.iter_mut().zip(lamps) {
                let Some(entry) = self.entries.get(&input.id).filter(|e| {
                    e.ready() && e.strength > 0.0 && (e.last_wanted == self.frame) == in_list
                }) else {
                    continue;
                };
                let count = if matches!(entry.shape, LampShape::Cone(_)) {
                    1
                } else {
                    6
                };
                if faces.len() + count > MAX_FACES {
                    continue;
                }
                let first_face = faces.len() as u32;
                let mut scale = 1.0;
                for index in 0..count {
                    faces.push(match &entry.faces[index] {
                        Some(face) => {
                            let (view, projection, _) = entry.view_and_projection(index);
                            scale = projection.x_axis.x;
                            let side = 1.0 / ATLAS_TEXELS as f32;
                            GpuAtlasFace {
                                from_lamp: (projection * view).to_cols_array(),
                                rect: [
                                    face.x as f32 * side,
                                    face.y as f32 * side,
                                    entry.size as f32 * side,
                                    entry.size as f32 * side,
                                ],
                            }
                        }
                        None => empty,
                    });
                }
                *record = GpuAtlasLamp {
                    first_face,
                    kind: count as u32,
                    near: entry.near(),
                    scale,
                    strength: entry.strength,
                };
            }
        }
        (records, faces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(position: Vec3, candela: f32, reach: f32) -> GpuLamp {
        GpuLamp {
            position_radius: [position.x, position.y, position.z, 0.05],
            colour_reach: [candela, candela, candela, reach],
            regions: [u32::MAX; crate::lamps::LAMP_REGIONS],
            beam: [0.0; 4],
            beam_edge: [0.0; 4],
            flags: [0; 4],
        }
    }

    fn cone(position: Vec3) -> GpuLamp {
        let mut l = lamp(position, 100.0, 20.0);
        l.beam = [0.0, -1.0, 0.0, 0.9];
        l.beam_edge = [0.7, 0.0, 0.0, 0.0];
        l
    }

    fn view() -> AtlasView {
        AtlasView {
            camera: Vec3::ZERO,
            planes: [[0.0, 0.0, 0.0, 1.0]; 6],
            pixels: 1000.0,
        }
    }

    fn input(lamps: &[GpuLamp]) -> Vec<AtlasLampIn> {
        lamps
            .iter()
            .enumerate()
            .map(|(i, l)| AtlasLampIn {
                id: i as u32,
                lamp: *l,
            })
            .collect()
    }

    #[test]
    fn slots_split_and_merge_back() {
        let mut s = Slots::new();
        let whole = s.free_area();
        assert_eq!(whole, u64::from(ATLAS_TEXELS) * u64::from(ATLAS_TEXELS));
        let a = s.take(128).unwrap();
        assert_eq!(s.free_area(), whole - 128 * 128);
        let b = s.take(512).unwrap();
        s.give(a, 128);
        s.give(b, 512);
        assert_eq!(s.free_area(), whole);
        assert_eq!(s.free[0].len(), 16, "merged back whole");
    }

    #[test]
    fn slots_never_overlap() {
        let mut s = Slots::new();
        let mut taken = Vec::new();
        for size in [1024, 128, 256, 128, 512, 128, 256, 1024, 512] {
            let (x, y) = s.take(size).unwrap();
            taken.push((x, y, size));
        }
        for (i, a) in taken.iter().enumerate() {
            for b in &taken[i + 1..] {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.2 <= b.1 || b.1 + b.2 <= a.1;
                assert!(apart, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn shapes() {
        assert_eq!(
            lamp_shape(&lamp(Vec3::ZERO, 1.0, 1.0)),
            LampShape::Cube(None)
        );
        assert!(matches!(lamp_shape(&cone(Vec3::ZERO)), LampShape::Cone(_)));
        let mut tube = lamp(Vec3::ZERO, 1.0, 1.0);
        tube.beam_edge[2] = -2.0;
        assert_eq!(lamp_shape(&tube), LampShape::Cube(None), "from its middle");
        let mut off = lamp(Vec3::ZERO, 1.0, 1.0);
        off.flags[0] = LAMP_SHADOWLESS;
        assert_eq!(lamp_shape(&off), LampShape::None);
        let mut flat = lamp(Vec3::ZERO, 1.0, 1.0);
        flat.beam = [0.0, -1.0, 0.0, 0.0];
        flat.beam_edge = [0.0, 0.0, 0.5, 0.0];
        assert_eq!(lamp_shape(&flat), LampShape::Cube(Some(2)), "the face up");
        let mut oblique = flat;
        let n = Vec3::new(0.72, 0.6, 0.35).normalize();
        oblique.beam = [n.x, n.y, n.z, 0.0];
        assert_eq!(
            lamp_shape(&oblique),
            LampShape::Cube(None),
            "each face still holds directions it lights"
        );
    }

    #[test]
    fn a_lamp_is_drawn_once_and_kept() {
        let mut a = LampAtlas::new();
        let lamps = input(&[lamp(Vec3::new(0.0, 3.0, -5.0), 100.0, 20.0)]);
        let plan = a.plan(&lamps, view(), &[], &[], 8, 8);
        assert_eq!(plan.statics.len(), 6);
        assert_eq!(plan.copies.len(), 6);
        let plan = a.plan(&lamps, view(), &[], &[], 8, 8);
        assert!(plan.statics.is_empty() && plan.copies.is_empty());
    }

    #[test]
    fn a_lamp_not_finished_is_not_read() {
        let mut a = LampAtlas::new();
        let lamps = input(&[lamp(Vec3::new(0.0, 3.0, -5.0), 100.0, 20.0)]);
        a.plan(&lamps, view(), &[], &[], 3, 8);
        assert_eq!(a.tables(&lamps).0[0].kind, 0, "three faces of six");
        a.plan(&lamps, view(), &[], &[], 3, 8);
        let (records, faces) = a.tables(&lamps);
        assert_eq!(records[0].kind, 6);
        assert_eq!(faces.len(), 6);
    }

    #[test]
    fn what_changed_in_its_reach_draws_a_lamp_again() {
        let mut a = LampAtlas::new();
        let lamps = input(&[cone(Vec3::new(0.0, 3.0, -5.0))]);
        a.plan(&lamps, view(), &[], &[], 8, 8);
        let far = [(Vec3::new(500.0, 0.0, 0.0), 1.0)];
        assert!(a.plan(&lamps, view(), &far, &[], 8, 8).statics.is_empty());
        let near = [(Vec3::new(1.0, 0.0, -5.0), 1.0)];
        assert_eq!(a.plan(&lamps, view(), &near, &[], 8, 8).statics.len(), 1);
    }

    #[test]
    fn movers_lay_the_slot_again_without_drawing_the_static_layer() {
        let mut a = LampAtlas::new();
        let lamps = input(&[cone(Vec3::new(0.0, 3.0, -5.0))]);
        a.plan(&lamps, view(), &[], &[], 8, 8);
        let near = [(Vec3::new(1.0, 0.0, -5.0), 1.0)];
        let plan = a.plan(&lamps, view(), &[], &near, 8, 8);
        assert!(plan.statics.is_empty());
        assert_eq!((plan.copies.len(), plan.movers.len()), (1, 1));
    }

    #[test]
    fn a_lamp_that_moved_is_drawn_again() {
        let mut a = LampAtlas::new();
        let mut l = cone(Vec3::new(0.0, 3.0, -5.0));
        a.plan(&input(&[l]), view(), &[], &[], 8, 8);
        l.position_radius[0] += 0.5;
        assert_eq!(
            a.plan(&input(&[l]), view(), &[], &[], 8, 8).statics.len(),
            1
        );
    }

    #[test]
    fn those_that_show_most_are_kept() {
        let mut a = LampAtlas::new();
        let mut lamps: Vec<GpuLamp> = (0..200)
            .map(|i| cone(Vec3::new(i as f32, 3.0, -5.0)))
            .collect();
        lamps[150] = cone(Vec3::new(0.5, 3.0, -1.0));
        lamps[150].colour_reach = [5000.0, 5000.0, 5000.0, 20.0];
        let list = input(&lamps);
        a.plan(&list, view(), &[], &[], 512, 512);
        let (records, _) = a.tables(&list);
        assert_eq!(
            records.iter().filter(|r| r.kind != 0).count(),
            MAX_SHADOWED_LAMPS
        );
        assert_ne!(records[150].kind, 0);
    }

    #[test]
    fn a_full_atlas_lets_the_oldest_go() {
        let mut a = LampAtlas::new();
        // Cubes that fill it at their largest.
        let first: Vec<GpuLamp> = (0..12)
            .map(|i| lamp(Vec3::new(i as f32 * 0.1, 3.0, -2.0), 1e4, 500.0))
            .collect();
        a.plan(&input(&first), view(), &[], &[], 512, 512);
        let filled = a.entries.len();
        assert!(filled > 0);
        for _ in 0..3 {
            let others: Vec<AtlasLampIn> = (0..12)
                .map(|i| AtlasLampIn {
                    id: 100 + i,
                    lamp: lamp(Vec3::new(i as f32 * 0.1, 3.0, 2.0), 1e4, 500.0),
                })
                .collect();
            a.plan(&others, view(), &[], &[], 512, 512);
        }
        assert!(a.entries.keys().any(|id| *id >= 100), "the new ones got in");
    }

    #[test]
    fn statics_go_to_the_lamps_that_show_most() {
        let mut a = LampAtlas::new();
        let mut bright = cone(Vec3::new(0.0, 3.0, -5.0));
        bright.colour_reach = [5000.0, 5000.0, 5000.0, 20.0];
        let dim = cone(Vec3::new(4.0, 3.0, -5.0));
        let list = input(&[dim, bright]);
        for _ in 0..8 {
            // Each frame anew, whatever the movers' turn.
            a.clear();
            let plan = a.plan(&list, view(), &[], &[], 1, 0);
            assert_eq!(plan.statics.len(), 1);
            assert_eq!(plan.statics[0].lamp.colour_reach[0], 5000.0);
        }
    }

    #[test]
    fn a_shadow_comes_and_goes_by_degrees() {
        let mut a = LampAtlas::new();
        let lamps = input(&[cone(Vec3::new(0.0, 3.0, -5.0))]);
        a.plan(&lamps, view(), &[], &[], 8, 8);
        let first = a.tables(&lamps).0[0].strength;
        assert!(first > 0.0 && first < 0.2, "{first}");
        for _ in 0..FADE_FRAMES as usize {
            a.plan(&lamps, view(), &[], &[], 8, 8);
        }
        assert_eq!(a.tables(&lamps).0[0].strength, 1.0);
        // Out of the list: still read while it goes, then not at all.
        a.plan(&[], view(), &[], &[], 8, 8);
        let going = a.tables(&lamps).0[0];
        assert_ne!(going.kind, 0);
        assert!(going.strength < 1.0);
        for _ in 0..FADE_FRAMES as usize {
            a.plan(&[], view(), &[], &[], 8, 8);
        }
        assert_eq!(a.tables(&lamps).0[0].kind, 0);
    }

    #[test]
    fn a_lamp_changing_size_keeps_its_shadow() {
        let mut a = LampAtlas::new();
        let lamps = input(&[lamp(Vec3::new(0.0, 3.0, -5.0), 100.0, 20.0)]);
        for _ in 0..4 {
            a.plan(&lamps, view(), &[], &[], 8, 8);
        }
        let before = a.tables(&lamps).0[0].strength;
        let size = a.entries[&0].size;
        // Far smaller on the screen, with no budget for the new slot: the old one stays.
        let mut small = view();
        small.pixels = 1.0;
        a.plan(&lamps, small, &[], &[], 0, 0);
        assert_eq!(a.entries[&0].size, size);
        assert_ne!(a.tables(&lamps).0[0].kind, 0);
        // With a budget, the new one is drawn in that frame, and read at once.
        let plan = a.plan(&lamps, small, &[], &[], 8, 8);
        assert_eq!(plan.statics.len(), 6);
        assert!(a.entries[&0].size < size);
        let record = a.tables(&lamps).0[0];
        assert_ne!(record.kind, 0, "no frame without it");
        assert!(record.strength > before, "{} {before}", record.strength);
    }

    #[test]
    fn one_that_keeps_moving_does_not_hold_the_rest() {
        let mut a = LampAtlas::new();
        let mut restless = lamp(Vec3::new(0.0, 3.0, -2.0), 1e5, 20.0);
        let others: Vec<GpuLamp> = (0..4)
            .map(|i| cone(Vec3::new(i as f32 * 3.0 + 4.0, 3.0, -5.0)))
            .collect();
        let mut list = vec![restless];
        list.extend(&others);
        let mut settled = false;
        for frame in 0..64 {
            restless.position_radius[0] = frame as f32 * 0.1;
            list[0] = restless;
            a.plan(&input(&list), view(), &[], &[], 6, 8);
            // Something near one of the others changes, over and over.
            let near = [(Vec3::new(4.0, 3.0, -5.0), 0.1)];
            a.plan(&input(&list), view(), &near, &[], 6, 8);
            settled = a.tables(&input(&list)).0[1..].iter().all(|r| r.kind != 0);
        }
        assert!(settled, "the lamps behind it got their faces");
    }

    #[test]
    fn a_reach_that_grows_draws_again_and_a_flicker_settles() {
        let mut a = LampAtlas::new();
        let mut l = cone(Vec3::new(0.0, 3.0, -5.0));
        a.plan(&input(&[l]), view(), &[], &[], 8, 8);
        l.colour_reach[3] = 40.0;
        let plan = a.plan(&input(&[l]), view(), &[], &[], 8, 8);
        assert_eq!(plan.statics.len(), 1);
        assert_eq!(plan.statics[0].lamp.colour_reach[3], 40.0);
        l.colour_reach[3] = 25.0;
        assert!(
            a.plan(&input(&[l]), view(), &[], &[], 8, 8)
                .statics
                .is_empty()
        );
        l.colour_reach[3] = 40.0;
        assert!(
            a.plan(&input(&[l]), view(), &[], &[], 8, 8)
                .statics
                .is_empty()
        );
    }

    #[test]
    fn a_newcomer_does_not_cost_a_resized_lamp_its_shadow() {
        let mut a = LampAtlas::new();
        let old = lamp(Vec3::new(0.0, 3.0, -5.0), 100.0, 20.0);
        for _ in 0..4 {
            a.plan(&input(&[old]), view(), &[], &[], 8, 8);
        }
        let size = a.entries[&0].size;
        let mut small = view();
        small.pixels = 1.0;
        let mut newcomer = lamp(Vec3::new(1.0, 3.0, -2.0), 1e5, 20.0);
        newcomer.position_radius[0] = 1.0;
        let both = input(&[old, newcomer]);
        a.plan(&both, small, &[], &[], 6, 8);
        assert_eq!(a.entries[&0].size, size, "no budget for the two: it waits");
        assert_ne!(a.tables(&both).0[0].kind, 0, "and keeps what it has");
    }

    #[test]
    fn a_lamp_in_the_list_is_not_given_up_however_full() {
        let mut a = LampAtlas::new();
        // Sixteen large cones, about as many as fit.
        let mut lamps: Vec<GpuLamp> = (0..16)
            .map(|i| cone(Vec3::new(i as f32 * 0.1, 3.0, -2.0)))
            .collect();
        for l in &mut lamps {
            l.colour_reach[3] = 500.0;
        }
        let list = input(&lamps);
        a.plan(&list, view(), &[], &[], 64, 64);
        a.plan(&list, view(), &[], &[], 64, 64);
        // Nothing to give up, and every slot wanted: none of them is lost.
        let before = a.entries.len();
        a.plan(&list, view(), &[], &[], 64, 64);
        assert_eq!(a.entries.len(), before);
        assert!(a.tables(&list).0.iter().filter(|r| r.kind != 0).count() == before);
    }

    #[test]
    fn redrawing_keeps_what_is_shown() {
        let mut a = LampAtlas::new();
        let lamps = input(&[cone(Vec3::new(0.0, 3.0, -5.0))]);
        a.plan(&lamps, view(), &[], &[], 8, 8);
        a.redraw_all();
        assert_ne!(a.tables(&lamps).0[0].kind, 0);
        assert_eq!(a.plan(&lamps, view(), &[], &[], 8, 8).statics.len(), 1);
    }

    #[test]
    fn pages_that_come_in_redraw_a_lamp_at_a_time() {
        let mut a = LampAtlas::new();
        let lamps = input(&[
            cone(Vec3::new(0.0, 3.0, -5.0)),
            cone(Vec3::new(2.0, 3.0, -5.0)),
        ]);
        for _ in 0..8 {
            a.plan(&lamps, view(), &[], &[], 8, 8);
        }
        let mut drawn = 0;
        for _ in 0..16 {
            a.refresh_turn();
            drawn += a.plan(&lamps, view(), &[], &[], 8, 8).statics.len();
        }
        assert_eq!(drawn, 4, "one lamp every fourth frame");
    }

    #[test]
    fn a_lamp_out_of_view_is_left_out() {
        let mut a = LampAtlas::new();
        let mut v = view();
        v.planes[0] = [0.0, 0.0, -1.0, 0.0]; // only what lies in front of the camera, along -z
        let lamps = input(&[lamp(Vec3::new(0.0, 0.0, 100.0), 100.0, 5.0)]);
        assert!(a.plan(&lamps, v, &[], &[], 8, 8).statics.is_empty());
    }

    #[test]
    fn a_point_lands_inside_its_face() {
        let mut a = LampAtlas::new();
        let l = lamp(Vec3::new(1.0, 2.0, 3.0), 100.0, 20.0);
        let list = input(&[l]);
        for _ in 0..2 {
            a.plan(&list, view(), &[], &[], 64, 64);
        }
        let (records, faces) = a.tables(&list);
        let face = faces[records[0].first_face as usize]; // +x
        let m = Mat4::from_cols_array(&face.from_lamp);
        let clip = m * glam::Vec4::new(4.0, 0.5, -0.5, 1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0, "{ndc:?}");
        assert!(ndc.z > 0.0 && ndc.z < 1.0, "reversed-Z: near over distance");
        assert!(
            (clip.w - 4.0).abs() < 1e-3,
            "w is the distance along the face"
        );
    }
}
