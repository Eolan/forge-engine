//! Rooms lit by probe grids of their own (D-078, #353, `research/probe-levels-indoors.md` §2.1).
//!
//! A room volume of the region sets (D-068) near the camera gets a small grid fixed in the world:
//! a box over its outline, its outer probes [`INSET`] in from the walls, the floor and the
//! ceiling. Its surfaces, and the probes' rays that meet them, take their light from that grid
//! alone (`room_lookup` in `probes.slang`): no masks, no cascade loop, no step where a cascade
//! ends, since the grid does not move with the camera.
//!
//! - **The pool.** The grids' probes take slots of a pool after the cascades' rows in the atlases
//!   ([`super::ProbeParams::rooms`]); the rays, state and blend passes update them with the
//!   cascades' probes, every frame each (no turns, so nothing ranks them).
//! - **Coming and going.** A room within [`RoomGridParams::radius`] of the camera gets its grid,
//!   the nearest first; one past the radius and [`HYSTERESIS`] more fades out and gives its slots
//!   back. A full pool lets its farthest grid go for a room [`EVICT_MARGIN`] nearer: in a street
//!   of houses the pool, not the radius, sets how far the grids reach. A new grid takes its first [`YOUNG_FRAMES`] updates, then
//!   fades in over [`FADE_FRAMES`]: until then, the cascades light the room as before.
//! - **The frames.** Each frame slot has its own tables (the grids, the pool's slots and the
//!   regions' hash), rewritten each frame: a few kilobytes.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, MemoryCategory, MemoryLocation, Result, vk,
};
use glam::{Vec2, Vec3};

use crate::meshlet::dynamic::Ranges;

/// Metres between a grid's outer probes and its room's walls, floor and ceiling, so no probe
/// stands in a wall or a window's recess: a grid's probes are never moved (`probe_state_main`),
/// and one inside a solid is inactive.
pub(super) const INSET: f32 = 0.35;
/// Metres past the radius a grid stays before it fades out: a camera on the boundary does not
/// make and drop it again and again.
pub(super) const HYSTERESIS: f32 = 10.0;
/// Metres farther than a room without one a grid must stand before a full pool lets it go for
/// that room: a camera between two rooms does not trade their grids back and forth.
pub(super) const EVICT_MARGIN: f32 = 5.0;
/// A new grid's updates before it fades in: the young probes' (`PROBE_SETTLE_UPDATES`).
pub(super) const YOUNG_FRAMES: u32 = 8;
/// Frames a grid takes to fade in and out: about half a second.
pub(super) const FADE_FRAMES: u32 = 30;
/// Grids at once, at most.
pub(super) const MAX_GRIDS: usize = 256;
/// `ROOM_NONE` in `probes.slang`: no grid.
pub(super) const ROOM_NONE: u32 = u32::MAX;

/// The room grids' layout (D-078).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomGridParams {
    /// Probe slots of the pool: the grids at once hold at most this many probes.
    pub probes: u32,
    /// Metres from the camera to a room's box within which it gets its grid.
    pub radius: f32,
    /// Metres between a grid's probes across the floor, at most.
    pub spacing: f32,
    /// Metres between its layers up, at most.
    pub height_spacing: f32,
    /// Rays a grid's probe traces a frame, the first 32 with fixed directions (64 up to
    /// [`super::ProbeParams::rays`], which a larger count takes): the grids' cost is mostly their
    /// rays, every probe every frame. Measured in the village's room view (serial, 2026-10-23):
    /// 128 rays 4.80 ms, 96 4.71, 64 4.67; no more flicker with the camera still (0.05 % of the
    /// pixels between frames 10 apart at each), but a room's light settles 2.7 (96) and 3.9 (64)
    /// sRGB levels away from 128's on average: the rotations repeat every 8 frames, so the
    /// fewer the rays the fewer directions a probe ever sees. `None`: the cascades' count.
    pub rays: Option<u32>,
}

impl Default for RoomGridParams {
    /// 4,096 probes, rooms within 20 m, probes at most 1 m apart: three layers in a room 2.7 m
    /// high. Measured against probes twice as dense (the cascades 0.5 m apart, grids 0.75 m) in the
    /// village's room and hall, 1 m grids came within 2.5 sRGB levels, 1.5 m ones up to 7 too
    /// bright (the walls' light drawn from farther in), and the 1 m cascade up to 8.5 too dark. Two
    /// layers 2 m apart left a bright V over a table: the lower layer stood under it, and its
    /// visibility test left the wall above to the upper one.
    fn default() -> Self {
        Self {
            probes: 4096,
            radius: 20.0,
            spacing: 1.0,
            height_spacing: 1.0,
            rays: None,
        }
    }
}

/// A room volume as the grids read it: its outline and its height in the scene frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RoomOutline {
    pub region: u32,
    pub corners: [Vec2; 4],
    pub floor: f32,
    pub ceiling: f32,
}

impl RoomOutline {
    /// Its box's distance from `p`, metres (0 inside).
    fn distance(&self, p: Vec3) -> f32 {
        let lo = self
            .corners
            .iter()
            .fold(Vec2::splat(f32::MAX), |a, &c| a.min(c));
        let hi = self
            .corners
            .iter()
            .fold(Vec2::splat(f32::MIN), |a, &c| a.max(c));
        let lo = Vec3::new(lo.x, self.floor, lo.y);
        let hi = Vec3::new(hi.x, self.ceiling, hi.y);
        (lo - p).max(p - hi).max(Vec3::ZERO).length()
    }
}

/// A grid's place and shape: probe (i, j, k) stands at `origin + i·step.x·axis + j·step.y·up +
/// k·step.z·across`, with `across` the axis turned a quarter (`(−axis.y, axis.x)` in x and z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GridShape {
    pub origin: Vec3,
    pub axis: Vec2,
    pub step: Vec3,
    pub counts: [u32; 3],
}

impl GridShape {
    /// Its probes.
    pub fn len(&self) -> u32 {
        self.counts.iter().product()
    }

    /// Probe (i, j, k)'s place.
    #[cfg(test)]
    pub fn position(&self, i: u32, j: u32, k: u32) -> Vec3 {
        let axis = Vec3::new(self.axis.x, 0.0, self.axis.y);
        let across = Vec3::new(-self.axis.y, 0.0, self.axis.x);
        self.origin
            + axis * (i as f32 * self.step.x)
            + Vec3::Y * (j as f32 * self.step.y)
            + across * (k as f32 * self.step.z)
    }
}

/// The grid over `room`: its box's axis along the outline's edge that gives the smallest box (a
/// rectangle's own), at least two probes along each axis, at most `spacing` apart across the
/// floor and `height_spacing` up, the outer ones [`INSET`] in (less in a room under four insets).
pub(super) fn grid_shape(room: &RoomOutline, spacing: f32, height_spacing: f32) -> GridShape {
    let mut best: Option<(f32, Vec2, [f32; 4])> = None;
    for k in 0..4 {
        let edge = room.corners[(k + 1) % 4] - room.corners[k];
        if edge.length_squared() < 1e-8 {
            continue;
        }
        let axis = edge.normalize();
        let across = Vec2::new(-axis.y, axis.x);
        let (mut u0, mut u1, mut w0, mut w1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for c in room.corners {
            let (u, w) = (c.dot(axis), c.dot(across));
            (u0, u1, w0, w1) = (u0.min(u), u1.max(u), w0.min(w), w1.max(w));
        }
        let area = (u1 - u0) * (w1 - w0);
        if best.is_none_or(|b| area < b.0 - 1e-4) {
            best = Some((area, axis, [u0, u1, w0, w1]));
        }
    }
    let (_, axis, [u0, u1, w0, w1]) = best.unwrap_or((0.0, Vec2::X, [0.0; 4]));
    let across = Vec2::new(-axis.y, axis.x);
    // Along one axis: its inset, its probes and their step.
    let layout = |length: f32, most: f32| {
        let inset = INSET.min(length * 0.25);
        let span = (length - 2.0 * inset).max(0.0);
        let n = (1 + (span / most).ceil() as u32).max(2);
        (inset, n, span / (n - 1) as f32)
    };
    let (iu, nu, su) = layout(u1 - u0, spacing);
    let (iy, ny, sy) = layout(room.ceiling - room.floor, height_spacing);
    let (iw, nw, sw) = layout(w1 - w0, spacing);
    let ground = axis * (u0 + iu) + across * (w0 + iw);
    GridShape {
        origin: Vec3::new(ground.x, room.floor + iy, ground.y),
        axis,
        step: Vec3::new(su, sy, sw),
        counts: [nu, ny, nw],
    }
}

/// Mirrors `RoomGrid` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuRoomGrid {
    origin: [f32; 3],
    first: u32,
    axis: [f32; 2],
    spacing: f32,
    region: u32,
    step: [f32; 3],
    fade: f32,
    counts: [u32; 3],
    next: u32,
    fresh: u32,
    pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<GpuRoomGrid>() == 80);

/// A grid as the host keeps it.
#[derive(Clone, Copy, Debug)]
struct Grid {
    /// Its room: its set's serial and its place among the set's rooms.
    key: (u64, u32),
    region: u32,
    shape: GridShape,
    /// Its first slot in the pool.
    first: u32,
    /// Frames since it was made (0 on its first).
    age: u32,
    /// It fades out: its room went past the radius and the hysteresis, or the pool needs its
    /// slots for a nearer room.
    leaving: bool,
    /// The pool let it go for a nearer room: it leaves even back within the radius.
    evicted: bool,
    /// Its share of its room's light: 0 while young, rising by 1 / [`FADE_FRAMES`] a frame, falling
    /// so again as it leaves, from wherever it stands.
    fade: f32,
}

impl Grid {
    /// Its fade one frame on.
    fn step_fade(&mut self) {
        let target = if self.leaving || self.age < YOUNG_FRAMES {
            0.0
        } else {
            1.0
        };
        let step = 1.0 / FADE_FRAMES as f32;
        self.fade = (self.fade + (target - self.fade).clamp(-step, step)).clamp(0.0, 1.0);
    }
}

/// What a frame reads of the room grids: the addresses its field names.
pub(super) struct RoomFrame {
    pub slots: u64,
    pub grids: u64,
    pub table: u64,
    pub table_mask: u32,
    pub count: u32,
}

/// The hash's entries for `grids` grids: a power of two at least twice as many, 16 at least.
fn table_size(grids: usize) -> usize {
    (2 * grids).next_power_of_two().max(16)
}

/// The entry of `region` in a table of `mask + 1` (`room_hash` in `probes.slang`).
pub(super) fn room_hash(region: u32, mask: u32) -> u32 {
    // A full avalanche (the "lowbias32" mix): the rooms' numbers run in sequence.
    let mut h = region;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h & mask
}

/// The grids' bookkeeping, frame by frame: which rooms have one, where in the pool, how far
/// faded in (the GPU's tables are [`RoomGrids`]').
struct RoomBook {
    params: RoomGridParams,
    pool: Ranges,
    grids: Vec<Grid>,
    /// Slot ranges given back from frame `.0` on: frames in flight may still read them.
    freed: Vec<(u64, u32, u32)>,
    frame: u64,
    /// Per slot of the pool, its grid's place in `grids`, or [`ROOM_NONE`].
    slots: Vec<u32>,
    /// Rooms past the pool, reported once.
    full_reported: bool,
}

impl RoomBook {
    fn new(params: RoomGridParams) -> Self {
        Self {
            params,
            pool: Ranges::new(params.probes, 0),
            grids: Vec::new(),
            freed: Vec::new(),
            frame: 0,
            slots: vec![ROOM_NONE; params.probes as usize],
            full_reported: false,
        }
    }

    /// Drops `g`: its slots come back once no frame in flight reads them.
    fn drop_grid(&mut self, g: &Grid) {
        self.freed.push((
            self.frame + FRAMES_IN_FLIGHT as u64 + 1,
            g.first,
            g.shape.len(),
        ));
        for s in &mut self.slots[g.first as usize..(g.first + g.shape.len()) as usize] {
            *s = ROOM_NONE;
        }
    }

    fn clear(&mut self) {
        for g in std::mem::take(&mut self.grids) {
            self.drop_grid(&g);
        }
    }

    fn restart(&mut self) {
        for g in &mut self.grids {
            g.age = 0;
            g.fade = 0.0;
        }
    }

    /// The frame's grids for the camera at `camera` among `rooms`: the slots given back, the
    /// grids gone or leaving, the near rooms' new ones.
    fn advance<'a>(
        &mut self,
        camera: Vec3,
        rooms: impl Iterator<Item = ((u64, u32), &'a RoomOutline)>,
    ) {
        self.frame += 1;
        let frame = self.frame;
        let mut k = 0;
        while k < self.freed.len() {
            if self.freed[k].0 <= frame {
                let (_, first, len) = self.freed.swap_remove(k);
                self.pool.give(first, len);
            } else {
                k += 1;
            }
        }
        // The rooms near enough, nearest first; every room's distance.
        let radius = self.params.radius;
        let mut near: Vec<(f32, (u64, u32), &RoomOutline)> = Vec::new();
        let mut present = std::collections::HashMap::new();
        for (key, room) in rooms {
            let d = room.distance(camera);
            present.insert(key, d);
            if d <= radius {
                near.push((d, key, room));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Those whose room went (its set removed) leave at once, those past the radius and the
        // hysteresis fade out (back within the radius, in again from where they stood), and those
        // faded out give their slots back.
        for mut g in std::mem::take(&mut self.grids) {
            let gone = match present.get(&g.key) {
                None => true,
                Some(&d) => {
                    if d > radius + HYSTERESIS {
                        g.leaving = true;
                    } else if d <= radius && !g.evicted {
                        g.leaving = false;
                    }
                    g.leaving && g.fade <= 0.0
                }
            };
            if gone {
                self.drop_grid(&g);
            } else {
                self.grids.push(g);
            }
        }
        // The near rooms without a grid get one while the pool has room.
        for (wanting, key, room) in near {
            if self.grids.iter().any(|g| g.key == key) {
                continue;
            }
            if self.grids.len() >= MAX_GRIDS {
                break;
            }
            let shape = grid_shape(room, self.params.spacing, self.params.height_spacing);
            let Some(first) = self.pool.take(shape.len()) else {
                // The pool full: the farthest grid, if [`EVICT_MARGIN`] farther than this room (one
                // past the radius kept by the hysteresis first of all), fades out, and its slots
                // come back for the nearer room a few frames later: in a street of houses the pool,
                // not the radius, sets how far the grids reach. One at a time: none while another
                // fades out or its slots wait.
                let waiting = !self.freed.is_empty() || self.grids.iter().any(|g| g.leaving);
                let farthest = self
                    .grids
                    .iter_mut()
                    .filter(|g| !waiting && !g.leaving)
                    .filter_map(|g| present.get(&g.key).map(|&d| (d, g)))
                    .filter(|(d, _)| *d > wanting + EVICT_MARGIN)
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                match farthest {
                    Some((_, g)) => {
                        g.leaving = true;
                        g.evicted = true;
                    }
                    None if !waiting && !self.full_reported => {
                        self.full_reported = true;
                        tracing::warn!(
                            pool = self.params.probes,
                            grids = self.grids.len(),
                            "the room grids' pool is full: the farther rooms keep the cascades (D-078)"
                        );
                    }
                    None => {}
                }
                break;
            };
            self.grids.push(Grid {
                key,
                region: room.region,
                shape,
                first,
                age: 0,
                leaving: false,
                evicted: false,
                fade: 0.0,
            });
        }
    }

    /// The GPU's tables for the grids as they stand: their records (each chained to the next of
    /// its region, its first probe after `first_probe`), the regions' hash and its mask; the
    /// pool's slots are brought up to them.
    fn tables(&mut self, first_probe: u32) -> (Vec<GpuRoomGrid>, Vec<[u32; 2]>, u32) {
        let size = table_size(self.grids.len());
        let mask = size as u32 - 1;
        let mut table = vec![[0u32, ROOM_NONE]; size];
        let mut records = Vec::with_capacity(self.grids.len());
        for (i, g) in self.grids.iter().enumerate() {
            let mut at = room_hash(g.region, mask);
            let mut next = ROOM_NONE;
            loop {
                let entry = &mut table[at as usize];
                if entry[1] == ROOM_NONE {
                    *entry = [g.region, i as u32];
                    break;
                }
                if entry[0] == g.region {
                    next = entry[1];
                    entry[1] = i as u32;
                    break;
                }
                at = (at + 1) & mask;
            }
            for s in &mut self.slots[g.first as usize..(g.first + g.shape.len()) as usize] {
                *s = i as u32;
            }
            records.push(GpuRoomGrid {
                origin: g.shape.origin.to_array(),
                first: first_probe + g.first,
                axis: g.shape.axis.to_array(),
                spacing: g.shape.step.x.max(g.shape.step.z),
                region: g.region,
                step: g.shape.step.to_array(),
                fade: g.fade,
                counts: g.shape.counts,
                next,
                fresh: u32::from(g.age == 0),
                pad: [0; 3],
            });
        }
        (records, table, mask)
    }

    /// Each grid one update older for the next frame (a new one, or one started over, is fresh on
    /// this one alone), its fade a step on.
    fn finish(&mut self) {
        for g in &mut self.grids {
            g.age = g.age.saturating_add(1);
            g.step_fade();
        }
    }
}

/// The room grids of a scene (see the module notes): their bookkeeping and, per frame slot, the
/// tables the GPU reads.
pub(super) struct RoomGrids {
    book: RoomBook,
    /// The pool's first probe among every probe.
    first_probe: u32,
    /// Per frame slot: the pool's slots (a grid each, or [`ROOM_NONE`]), the grids, the regions'
    /// hash.
    slot_tables: Vec<Buffer>,
}

impl RoomGrids {
    /// The grids for a pool whose first probe is `first_probe`.
    pub(super) fn new(
        device: &Arc<Device>,
        params: RoomGridParams,
        first_probe: u32,
    ) -> Result<Self> {
        let bytes = u64::from(params.probes) * 4
            + (MAX_GRIDS * std::mem::size_of::<GpuRoomGrid>()) as u64
            + (table_size(MAX_GRIDS) * 8) as u64;
        let slot_tables = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: bytes,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("probe room grids {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            book: RoomBook::new(params),
            first_probe,
            slot_tables,
        })
    }

    /// Drops every grid at once (the regions changed whole): their rooms' next grids start young.
    pub(super) fn clear(&mut self) {
        self.book.clear();
    }

    /// Starts every grid over (`Probes::reset`): each is young again, its probes new.
    pub(super) fn restart(&mut self) {
        self.book.restart();
    }

    /// Bytes of the frames' tables.
    pub(super) fn bytes(&self) -> u64 {
        self.slot_tables.iter().map(Buffer::size).sum()
    }

    /// The grids live this frame (for logs and tests): their regions, probes and fades.
    pub(super) fn live(&self) -> Vec<(u32, u32, f32)> {
        self.book
            .grids
            .iter()
            .map(|g| (g.region, g.shape.len(), g.fade))
            .collect()
    }

    /// The frame's grids for the camera at `camera` among `rooms` (each with its set's serial and
    /// its place among the set's rooms), written to frame slot `slot`.
    pub(super) fn frame<'a>(
        &mut self,
        slot: usize,
        camera: Vec3,
        rooms: impl Iterator<Item = ((u64, u32), &'a RoomOutline)>,
    ) -> RoomFrame {
        self.book.advance(camera, rooms);
        let (records, table, mask) = self.book.tables(self.first_probe);
        let buffer = &self.slot_tables[slot];
        let grids_at = u64::from(self.book.params.probes) * 4;
        let table_at = grids_at + (MAX_GRIDS * std::mem::size_of::<GpuRoomGrid>()) as u64;
        buffer.write(0, &self.book.slots);
        buffer.write(grids_at, &records);
        buffer.write(table_at, &table);
        self.book.finish();
        RoomFrame {
            slots: buffer.address(),
            grids: buffer.address() + grids_at,
            table: buffer.address() + table_at,
            table_mask: mask,
            count: records.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(corners: [[f32; 2]; 4], floor: f32, ceiling: f32) -> RoomOutline {
        RoomOutline {
            region: 1,
            corners: corners.map(Vec2::from),
            floor,
            ceiling,
        }
    }

    #[test]
    fn a_rooms_grid_stands_inset_in_its_box() {
        // 4 × 6 m, 2.7 m high: 4 probes across x (3.3 m at most 1.5 apart), 5 across z, 3 layers.
        let r = room([[0.0, 0.0], [4.0, 0.0], [4.0, 6.0], [0.0, 6.0]], 0.0, 2.7);
        let g = grid_shape(&r, 1.5, 1.0);
        assert_eq!(g.counts, [4, 3, 5]);
        let first = g.position(0, 0, 0);
        let last = g.position(3, 2, 4);
        assert!(
            (first - Vec3::new(INSET, INSET, INSET)).length() < 1e-4,
            "{first}"
        );
        assert!(
            (last - Vec3::new(4.0 - INSET, 2.7 - INSET, 6.0 - INSET)).length() < 1e-4,
            "{last}"
        );
    }

    #[test]
    fn a_turned_room_keeps_its_own_axes() {
        // The same room turned 30°: the same counts, every probe inside it.
        let (s, c) = 30f32.to_radians().sin_cos();
        let turn = |x: f32, z: f32| [c * x - s * z + 10.0, s * x + c * z - 5.0];
        let r = room(
            [
                turn(0.0, 0.0),
                turn(4.0, 0.0),
                turn(4.0, 6.0),
                turn(0.0, 6.0),
            ],
            1.0,
            3.7,
        );
        let g = grid_shape(&r, 1.5, 2.0);
        let mut counts = g.counts;
        counts.sort_unstable();
        assert_eq!(counts, [2, 4, 5]);
        let within = |v: f32, lo: f32, hi: f32| (lo - 1e-3..=hi + 1e-3).contains(&v);
        for i in 0..g.counts[0] {
            for j in 0..g.counts[1] {
                for k in 0..g.counts[2] {
                    // Back in the room's own frame: every probe INSET in from its walls.
                    let p = g.position(i, j, k);
                    let (dx, dz) = (p.x - 10.0, p.z + 5.0);
                    let (x, z) = (c * dx + s * dz, -s * dx + c * dz);
                    assert!(within(x, INSET, 4.0 - INSET), "{x}");
                    assert!(within(z, INSET, 6.0 - INSET), "{z}");
                    assert!(within(p.y, 1.0 + INSET, 3.7 - INSET), "{}", p.y);
                }
            }
        }
    }

    #[test]
    fn a_closet_keeps_two_probes_each_way() {
        let r = room([[0.0, 0.0], [0.8, 0.0], [0.8, 1.0], [0.0, 1.0]], 0.0, 2.5);
        let g = grid_shape(&r, 1.5, 2.0);
        assert_eq!(g.counts, [2, 2, 2]);
        assert!(g.position(1, 0, 1).x < 0.8 && g.position(0, 0, 0).x > 0.0);
    }

    #[test]
    fn the_hash_finds_every_region() {
        let mask = 63;
        let mut seen = [0u32; 64];
        for region in 1..=32u32 {
            seen[room_hash(region, mask) as usize] += 1;
        }
        assert!(seen.iter().all(|&n| n <= 3), "{seen:?}");
    }

    /// A 4 × 4 m room at `x` along the street, 2.7 m high: 5 × 3 × 5 = 75 probes at 1 m.
    fn street_room(x: f32, region: u32) -> RoomOutline {
        RoomOutline {
            region,
            ..room(
                [[x, 0.0], [x + 4.0, 0.0], [x + 4.0, 4.0], [x, 4.0]],
                0.0,
                2.7,
            )
        }
    }

    /// One frame of `book` for the camera at `camera` among `rooms` (set 0's): its tables made
    /// as the GPU's are, then its grids aged.
    fn step(book: &mut RoomBook, camera: Vec3, rooms: &[RoomOutline]) {
        book.advance(
            camera,
            rooms.iter().enumerate().map(|(k, r)| ((0, k as u32), r)),
        );
        book.tables(0);
        book.finish();
    }

    fn params(probes: u32) -> RoomGridParams {
        RoomGridParams {
            probes,
            radius: 20.0,
            spacing: 1.0,
            height_spacing: 1.0,
            rays: None,
        }
    }

    #[test]
    fn a_grid_fades_in_after_its_young_updates() {
        let rooms = [street_room(0.0, 1)];
        let mut book = RoomBook::new(params(256));
        let mut fades = Vec::new();
        for _ in 0..(YOUNG_FRAMES + FADE_FRAMES + 2) {
            step(&mut book, Vec3::ZERO, &rooms);
            fades.push(book.grids[0].fade);
        }
        assert_eq!(book.grids[0].shape.len(), 75);
        // Nothing while its probes are young, then a steady rise to all of its room's light.
        assert!(
            fades[..YOUNG_FRAMES as usize - 1].iter().all(|&f| f == 0.0),
            "{fades:?}"
        );
        assert!(fades.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*fades.last().unwrap(), 1.0);
    }

    #[test]
    fn a_room_left_behind_fades_out_and_gives_its_slots_back() {
        let rooms = [street_room(0.0, 1)];
        let mut book = RoomBook::new(params(75));
        for _ in 0..50 {
            step(&mut book, Vec3::ZERO, &rooms);
        }
        // Past the radius but within the hysteresis: kept.
        let band = Vec3::new(4.0 + 25.0, 0.0, 2.0);
        for _ in 0..50 {
            step(&mut book, band, &rooms);
        }
        assert_eq!(book.grids[0].fade, 1.0);
        // Past both: it fades out, then goes; its slots come back once no frame reads them.
        let far = Vec3::new(4.0 + 31.0, 0.0, 2.0);
        step(&mut book, far, &rooms);
        assert!(book.grids[0].leaving && book.grids[0].fade < 1.0);
        for _ in 0..FADE_FRAMES {
            step(&mut book, far, &rooms);
        }
        assert!(book.grids.is_empty());
        assert!(book.slots.iter().all(|&s| s == ROOM_NONE));
        assert!(book.pool.take(75).is_none());
        for _ in 0..=FRAMES_IN_FLIGHT {
            step(&mut book, far, &rooms);
        }
        assert_eq!(book.pool.take(75), Some(0));
    }

    #[test]
    fn a_full_pool_lets_the_farthest_grid_past_the_radius_go() {
        // Room 1 at the camera, room 2 25 m off (in the hysteresis once the camera moves), room 3
        // out of reach; the pool holds two grids.
        let rooms = [
            street_room(0.0, 1),
            street_room(16.0, 2),
            street_room(60.0, 3),
        ];
        let mut book = RoomBook::new(params(150));
        for _ in 0..50 {
            step(&mut book, Vec3::new(2.0, 1.0, 2.0), &rooms);
        }
        let regions = |b: &RoomBook| b.grids.iter().map(|g| g.region).collect::<Vec<_>>();
        assert_eq!(regions(&book), [1, 2]);
        // The camera walks on to room 3: room 1 falls 25 m behind (kept), room 3 wants a grid.
        let on = Vec3::new(45.0, 1.0, 2.0);
        for _ in 0..(FADE_FRAMES + FRAMES_IN_FLIGHT as u32 + 4) {
            step(&mut book, on, &rooms);
        }
        assert_eq!(regions(&book), [2, 3]);
    }

    #[test]
    fn a_full_pool_trades_its_farthest_grid_for_a_nearer_room() {
        // A at the camera, B 12 m off, C 14 m off the other way; the pool holds two grids.
        let rooms = [
            street_room(0.0, 1),
            street_room(14.0, 2),
            street_room(-16.0, 3),
        ];
        let mut book = RoomBook::new(params(150));
        let regions = |b: &RoomBook| b.grids.iter().map(|g| g.region).collect::<Vec<_>>();
        for _ in 0..50 {
            step(&mut book, Vec3::new(2.0, 1.0, 2.0), &rooms);
        }
        assert_eq!(regions(&book), [1, 2]);
        // 6 m on towards C: A 4 m off, C 8 m, B 18 m, all within the radius. B is more than
        // EVICT_MARGIN farther than C: it goes, and C takes its slots.
        let on = Vec3::new(-4.0, 1.0, 2.0);
        for _ in 0..(FADE_FRAMES + FRAMES_IN_FLIGHT as u32 + 4) {
            step(&mut book, on, &rooms);
        }
        assert_eq!(regions(&book), [1, 3]);
        // Back halfway: none is EVICT_MARGIN farther than B, so nothing trades again.
        let back = Vec3::new(2.0, 1.0, 2.0);
        for _ in 0..100 {
            step(&mut book, back, &rooms);
        }
        assert_eq!(regions(&book), [1, 3]);
    }

    #[test]
    fn a_set_removed_takes_its_grids_at_once() {
        // Two sets of a room each (serials 0 and 1); then set 0 removed.
        let (a, b) = (street_room(0.0, 1), street_room(6.0, 2));
        let mut book = RoomBook::new(params(256));
        for _ in 0..50 {
            book.advance(Vec3::ZERO, [((0, 0), &a), ((1, 0), &b)].into_iter());
            book.tables(0);
            book.finish();
        }
        book.advance(Vec3::ZERO, [((1, 0), &b)].into_iter());
        assert_eq!(book.grids.len(), 1);
        assert_eq!(book.grids[0].region, 2);
        assert_eq!(book.grids[0].fade, 1.0);
    }
}
