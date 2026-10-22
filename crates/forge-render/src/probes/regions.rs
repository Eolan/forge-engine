//! The probes' region volumes (D-068) as sets a scene adds and removes as it runs (#338 step 3):
//! a streamed cell's rooms come and go with it.
//!
//! - **Sets.** A set's volumes stand together in one table, recesses and doorways first, then the
//!   rooms, then the houses' bodies (the lookup takes the first that holds a point), with a grid
//!   of [`REGION_CELL`] cells over its own box listing them.
//! - **The coarse grid.** A hashed table of [`TOP_CELL`] cells lists the sets whose box reaches
//!   each; the lookup (`region_at` in `probes.slang`) walks a cell's sets in the order they were
//!   added, then each set's own grid. Sets are self-contained: a cell's volumes do not overlap
//!   another's.
//! - **The frames.** The volumes and the sets' cells and indices sit in tables at a capacity, a
//!   new set's in ranges no frame reads until it joins; each frame slot has its own copy of the
//!   sets' table, the coarse grid and the doorways' splits, rewritten when it lags the edits. A
//!   set removed leaves the slots' tables at once and gives its ranges back
//!   `FRAMES_IN_FLIGHT + 1` frames later.
//! - **The probes about a set** added or removed are woken (`gi/probe wake regions`): young
//!   again, they take their region anew, where [`super::Probes::set_regions`] starts every
//!   probe over.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, GpuError, MemoryCategory, MemoryLocation, Result,
    vk,
};
use glam::{Vec2, Vec3};

use super::{RegionKind, RegionVolume};
use crate::meshlet::dynamic::Ranges;

/// The side of the cells of a set's grid, metres.
pub(super) const REGION_CELL: f32 = 2.0;
/// The side of the coarse grid's cells, metres.
pub(super) const TOP_CELL: f32 = 64.0;

/// Mirrors `RegionVolume` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuRegionVolume {
    pub(super) corners: [[f32; 2]; 4],
    pub(super) floor: f32,
    pub(super) ceiling: f32,
    pub(super) region: u32,
    pub(super) other: u32,
    pub(super) kind: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuRegionVolume>() == 56);

/// Mirrors `RegionSet` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct GpuRegionSet {
    origin: [f32; 3],
    cells: u32,
    dims: [u32; 2],
    pad: [u32; 2],
}

/// Mirrors `RegionTop` in `probes.slang`: a coarse cell, the first of its sets in the list and
/// their count (0: an empty entry).
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct GpuTopEntry {
    key: [i32; 2],
    first: u32,
    count: u32,
}

/// What a scene reserves for its region sets ([`super::Probes::reserve_regions`], #338).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionCapacity {
    /// Volumes over all sets at once.
    pub volumes: u32,
    /// Sets at once.
    pub sets: u32,
    /// Cells of the sets' grids over all sets ([`REGION_CELL`] apart over each set's box).
    pub cells: u32,
    /// Entries of the sets' grids: a volume once in each cell it reaches.
    pub indices: u32,
}

impl RegionCapacity {
    /// Room for `volumes` volumes in `sets` sets, each set's box up to 512 m square, each volume
    /// in eight of its grid's cells on average.
    pub fn new(volumes: u32, sets: u32) -> Self {
        Self {
            volumes,
            sets,
            cells: sets.saturating_mul(65_536),
            indices: volumes.saturating_mul(8),
        }
    }
}

/// A set of region volumes in the probes ([`super::Probes::add_regions`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RegionSetId(pub(super) u32);

/// A set as the store keeps it.
struct Set {
    /// Its ranges, (first, count): volumes, its grid's cells, their indices.
    volumes: (u32, u32),
    cells: (u32, u32),
    indices: (u32, u32),
    /// Its origin in the scene frame: its grid's corner, at its lowest floor. Its volumes and
    /// its grid are relative to it, so that a shift of the scene's origin moves this alone.
    origin: Vec3,
    dims: [u32; 2],
    /// Its box in the scene frame.
    lo: Vec3,
    hi: Vec3,
    /// Per volume as the host gave them, its place in the volumes' table.
    place: Vec<u32>,
}

/// What a frame reads of the regions ([`RegionStore::frame`]): the addresses its field names.
pub(super) struct RegionFrame {
    pub volumes: u64,
    pub cells: u64,
    pub indices: u64,
    pub sets: u64,
    pub top: u64,
    pub top_list: u64,
    pub splits: u64,
    pub top_mask: u32,
    pub count: u32,
}

/// The region sets of a scene, on the GPU (see the module notes).
pub(super) struct RegionStore {
    capacity: RegionCapacity,
    volumes: Buffer,
    cells: Buffer,
    indices: Buffer,
    volume_ranges: Ranges,
    cell_ranges: Ranges,
    index_ranges: Ranges,
    sets: Vec<Option<Set>>,
    /// Per volume slot, its kind (`GpuRegionVolume::kind`), whether [`ordered`] turned it round
    /// ([`turned`]) and its doorway's split.
    kinds: Vec<u32>,
    turned: Vec<bool>,
    splits: Vec<f32>,
    /// The splits' edits, (generation, first, count), and the generation each slot's copy holds.
    split_log: Vec<(u64, u32, u32)>,
    split_generation: u64,
    split_written: [u64; FRAMES_IN_FLIGHT],
    slot_splits: Vec<Buffer>,
    /// Per slot: the sets' table, the coarse grid's entries, then its list of sets; rewritten
    /// when `tables_generation` moves.
    slot_tables: Vec<Buffer>,
    tables: Vec<u8>,
    tables_generation: u64,
    tables_written: [u64; FRAMES_IN_FLIGHT],
    top_entries: u32,
    top_list: u32,
    top_mask: u32,
    live_volumes: u32,
    /// Ranges a removed set gives back from frame `.0` on.
    leaving: Vec<(u64, Set)>,
    frame: u64,
    /// Boxes whose probes the next frame wakes.
    wake: Vec<(Vec3, Vec3)>,
}

/// Whether `v` is given clockwise in x and z: [`ordered`] turns it round, and a doorway turned
/// round swaps its sides.
pub(super) fn turned(v: &RegionVolume) -> bool {
    let c = v.corners;
    let area: f32 = (0..4)
        .map(|i| {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    area < 0.0
}

/// A shut doorway's split (its leaf's share of the way across from its `region` side) as the
/// lookup measures it: from the side [`ordered`] made its `region`, the other one when it was
/// turned round. Open (0) stays open.
pub(super) fn split_as_stored(split: f32, turned: bool) -> f32 {
    if turned && split > 0.0 {
        (1.0 - split).max(1e-3)
    } else {
        split
    }
}

/// The volumes in the order the lookup tries them, recesses and doorways first (they overlap
/// their rooms' edges), the houses' bodies last (round their rooms), each anticlockwise in x and z
/// (a doorway turned round swaps its sides).
pub(super) fn ordered(volumes: &[RegionVolume]) -> Vec<GpuRegionVolume> {
    let mut out: Vec<GpuRegionVolume> = volumes
        .iter()
        .map(|v| {
            let c = v.corners;
            let (corners, region, other) = if turned(v) {
                ([c[3], c[2], c[1], c[0]], v.other, v.region)
            } else {
                (c, v.region, v.other)
            };
            GpuRegionVolume {
                corners: corners.map(|p| p.to_array()),
                floor: v.floor,
                ceiling: v.ceiling,
                region,
                other,
                kind: match v.kind {
                    RegionKind::Room => 0,
                    RegionKind::Recess => 1,
                    RegionKind::Doorway => 2,
                    RegionKind::Solid => 3,
                },
                pad: 0,
            }
        })
        .collect();
    // Recesses and doorways, then the rooms, then the houses' bodies.
    out.sort_by_key(|v| match v.kind {
        1 | 2 => 0,
        0 => 1,
        _ => 2,
    });
    out
}

/// Where each of `volumes` stands in [`ordered`]'s order (the sort is stable, by kind alone).
pub(super) fn places(volumes: &[RegionVolume]) -> Vec<u32> {
    let rank = |k: RegionKind| match k {
        RegionKind::Recess | RegionKind::Doorway => 0,
        RegionKind::Room => 1,
        RegionKind::Solid => 2,
    };
    let mut order: Vec<usize> = (0..volumes.len()).collect();
    order.sort_by_key(|&i| rank(volumes[i].kind));
    let mut place = vec![0u32; volumes.len()];
    for (at, &i) in order.iter().enumerate() {
        place[i] = at as u32;
    }
    place
}

/// The grid over `volumes` ([`REGION_CELL`] cells): its corner, its size in cells, and per cell
/// the first of its volumes' indices and their count, then the indices.
pub(super) fn region_grid(
    volumes: &[GpuRegionVolume],
) -> ([f32; 2], [u32; 2], Vec<[u32; 2]>, Vec<u32>) {
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for v in volumes {
        for c in v.corners {
            lo = lo.min(Vec2::from(c));
            hi = hi.max(Vec2::from(c));
        }
    }
    let origin = lo - Vec2::splat(REGION_CELL);
    let dims = ((hi - origin) / REGION_CELL).ceil().as_uvec2() + glam::UVec2::ONE;
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); (dims.x * dims.y) as usize];
    for (i, v) in volumes.iter().enumerate() {
        let (mut a, mut b) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for c in v.corners {
            a = a.min(Vec2::from(c));
            b = b.max(Vec2::from(c));
        }
        let (c0, c1) = (
            ((a - origin) / REGION_CELL).floor().as_uvec2(),
            ((b - origin) / REGION_CELL)
                .floor()
                .as_uvec2()
                .min(dims - 1),
        );
        for z in c0.y..=c1.y {
            for x in c0.x..=c1.x {
                lists[(z * dims.x + x) as usize].push(i as u32);
            }
        }
    }
    let mut cells = Vec::with_capacity(lists.len());
    let mut indices = Vec::new();
    for list in lists {
        cells.push([indices.len() as u32, list.len() as u32]);
        indices.extend(list);
    }
    (origin.to_array(), dims.to_array(), cells, indices)
}

/// The coarse cell `(x, z)`'s first slot in a table of `mask + 1` entries (`top_hash` in
/// `probes.slang`).
fn top_hash(key: [i32; 2], mask: u32) -> u32 {
    let h = (key[0] as u32).wrapping_mul(0x8DA6_B343) ^ (key[1] as u32).wrapping_mul(0xD816_3841);
    (h ^ (h >> 15)) & mask
}

impl RegionStore {
    /// A store with room for `capacity`, its tables made on `device`.
    pub(super) fn new(device: &Arc<Device>, capacity: RegionCapacity) -> Result<Self> {
        let gpu = |bytes: u64, name: &str| -> Result<Buffer> {
            device.create_buffer(BufferDesc {
                size: bytes.max(4),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name,
            })
        };
        let volumes = gpu(
            u64::from(capacity.volumes) * std::mem::size_of::<GpuRegionVolume>() as u64,
            "probe region volumes",
        )?;
        let cells = gpu(u64::from(capacity.cells) * 8, "probe region cells")?;
        let indices = gpu(u64::from(capacity.indices) * 4, "probe region indices")?;
        // The coarse grid: a list entry per coarse cell a set reaches, a table twice as large.
        let top_list = capacity.cells / 256 + capacity.sets.saturating_mul(16) + 16;
        let top_entries = (2 * top_list).next_power_of_two();
        let table_bytes = u64::from(capacity.sets) * std::mem::size_of::<GpuRegionSet>() as u64
            + u64::from(top_entries) * std::mem::size_of::<GpuTopEntry>() as u64
            + u64::from(top_list) * 4;
        let host = |bytes: u64, name: String| -> Result<Buffer> {
            device.create_buffer(BufferDesc {
                size: bytes.max(4),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Frame,
                name: &name,
            })
        };
        let slot_tables = (0..FRAMES_IN_FLIGHT)
            .map(|i| host(table_bytes, format!("probe region sets {i}")))
            .collect::<Result<Vec<_>>>()?;
        let slot_splits = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let buffer = host(
                    u64::from(capacity.volumes) * 4,
                    format!("probe region splits {i}"),
                )?;
                buffer.write(0, &vec![0.0_f32; capacity.volumes as usize]);
                Ok(buffer)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut store = Self {
            capacity,
            volumes,
            cells,
            indices,
            volume_ranges: Ranges::new(capacity.volumes, 0),
            cell_ranges: Ranges::new(capacity.cells, 0),
            index_ranges: Ranges::new(capacity.indices, 0),
            sets: Vec::new(),
            kinds: vec![0; capacity.volumes as usize],
            turned: vec![false; capacity.volumes as usize],
            splits: vec![0.0; capacity.volumes as usize],
            split_log: Vec::new(),
            split_generation: 0,
            split_written: [0; FRAMES_IN_FLIGHT],
            slot_splits,
            slot_tables,
            tables: Vec::new(),
            tables_generation: 1,
            tables_written: [0; FRAMES_IN_FLIGHT],
            top_entries,
            top_list,
            top_mask: top_entries - 1,
            live_volumes: 0,
            leaving: Vec::new(),
            frame: 0,
            wake: Vec::new(),
        };
        store.rebuild_tables()?;
        Ok(store)
    }

    /// Room for exactly `volumes` as one set: what [`super::Probes::set_regions`] makes.
    pub(super) fn fitting(volumes: &[RegionVolume]) -> RegionCapacity {
        let (_, _, cells, indices) = region_grid(&ordered(volumes));
        RegionCapacity {
            volumes: volumes.len() as u32,
            sets: 1,
            cells: cells.len() as u32,
            indices: indices.len() as u32,
        }
    }

    /// Adds `volumes` as a set: its volumes, its grid, its place in the coarse grid; its probes
    /// woken at the next frame (unless `wake` is false: a scene's first set, whose probes start
    /// over).
    pub(super) fn add(
        &mut self,
        device: &Arc<Device>,
        volumes: &[RegionVolume],
        wake: bool,
    ) -> Result<RegionSetId> {
        let mut table = ordered(volumes);
        let (corner, dims, cells, indices) = region_grid(&table);
        // Its volumes relative to its origin (their grid's corner, their lowest floor): the
        // lookup takes the point relative to it too, a shift of the scene's origin this alone.
        let floor = table.iter().map(|v| v.floor).fold(f32::MAX, f32::min);
        let origin = Vec3::new(corner[0], floor, corner[1]);
        let absolute = table.clone();
        for v in &mut table {
            v.corners = v.corners.map(|[x, z]| [x - origin.x, z - origin.z]);
            v.floor -= origin.y;
            v.ceiling -= origin.y;
        }
        let full = |what: &str| {
            GpuError::Unsupported(format!("the probes' regions are out of room for {what}"))
        };
        let n = table.len() as u32;
        let first_volume = self.volume_ranges.take(n).ok_or_else(|| full("volumes"))?;
        let first_cell = match self.cell_ranges.take(cells.len() as u32) {
            Some(c) => c,
            None => {
                self.volume_ranges.give(first_volume, n);
                return Err(full("cells"));
            }
        };
        let first_index = match self.index_ranges.take(indices.len() as u32) {
            Some(i) => i,
            None => {
                self.volume_ranges.give(first_volume, n);
                self.cell_ranges.give(first_cell, cells.len() as u32);
                return Err(full("indices"));
            }
        };
        // Its tables at their ranges' places: a cell's first entry and an entry's volume made
        // absolute.
        let cells: Vec<[u32; 2]> = cells
            .iter()
            .map(|&[first, count]| [first + first_index, count])
            .collect();
        let indices: Vec<u32> = indices.iter().map(|&i| i + first_volume).collect();
        let volume_size = std::mem::size_of::<GpuRegionVolume>() as u64;
        device.write_buffers_staged(&[
            (
                &self.volumes,
                u64::from(first_volume) * volume_size,
                bytemuck::cast_slice(&table),
            ),
            (
                &self.cells,
                u64::from(first_cell) * 8,
                bytemuck::cast_slice(&cells),
            ),
            (
                &self.indices,
                u64::from(first_index) * 4,
                bytemuck::cast_slice(&indices),
            ),
        ])?;
        for (k, v) in table.iter().enumerate() {
            self.kinds[first_volume as usize + k] = v.kind;
            self.splits[first_volume as usize + k] = 0.0;
        }
        self.split_generation += 1;
        self.split_log
            .push((self.split_generation, first_volume, n));
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &absolute {
            for c in v.corners {
                lo = lo.min(Vec3::new(c[0], v.floor, c[1]));
                hi = hi.max(Vec3::new(c[0], v.ceiling, c[1]));
            }
        }
        let place: Vec<u32> = places(volumes).iter().map(|&p| p + first_volume).collect();
        for (v, &at) in volumes.iter().zip(&place) {
            self.turned[at as usize] = turned(v);
        }
        let set = Set {
            volumes: (first_volume, n),
            cells: (first_cell, cells.len() as u32),
            indices: (first_index, indices.len() as u32),
            origin,
            dims,
            lo,
            hi,
            place,
        };
        if wake {
            self.wake.push((set.lo, set.hi));
        }
        let id = match self.sets.iter().position(Option::is_none) {
            Some(free) => free,
            None => {
                if self.sets.len() >= self.capacity.sets as usize {
                    self.give_back(&set);
                    return Err(full("sets"));
                }
                self.sets.push(None);
                self.sets.len() - 1
            }
        };
        self.sets[id] = Some(set);
        self.live_volumes += n;
        if let Err(e) = self.rebuild_tables() {
            let set = self.sets[id].take().expect("the set just added");
            self.live_volumes -= n;
            self.give_back(&set);
            return Err(e);
        }
        Ok(RegionSetId(id as u32))
    }

    /// Removes set `id`: it leaves the frames' tables at the next frame, its probes are woken,
    /// and its ranges are given back once no frame reads them. Nothing for an id not in.
    pub(super) fn remove(&mut self, id: RegionSetId) -> Result<()> {
        let Some(set) = self.sets.get_mut(id.0 as usize).and_then(Option::take) else {
            return Ok(());
        };
        self.live_volumes -= set.volumes.1;
        self.wake.push((set.lo, set.hi));
        self.leaving
            .push((self.frame + FRAMES_IN_FLIGHT as u64 + 1, set));
        self.rebuild_tables()
    }

    /// Shuts or opens doorway `volume` of set `id` (its place in what the set was given): its
    /// split ([`super::Probes::set_doorway`]). Nothing for another kind or an id not in.
    pub(super) fn set_doorway(&mut self, id: RegionSetId, volume: usize, split: f32) {
        let Some(Some(set)) = self.sets.get(id.0 as usize) else {
            return;
        };
        let Some(&at) = set.place.get(volume) else {
            return;
        };
        if self.kinds[at as usize] != 2 {
            return;
        }
        self.splits[at as usize] = split_as_stored(split, self.turned[at as usize]);
        self.split_generation += 1;
        self.split_log.push((self.split_generation, at, 1));
    }

    fn give_back(&mut self, set: &Set) {
        self.volume_ranges.give(set.volumes.0, set.volumes.1);
        self.cell_ranges.give(set.cells.0, set.cells.1);
        self.index_ranges.give(set.indices.0, set.indices.1);
    }

    /// The sets' table and the coarse grid as the sets stand, for the slots to copy.
    fn rebuild_tables(&mut self) -> Result<()> {
        let mut records = vec![GpuRegionSet::default(); self.capacity.sets as usize];
        let mut lists: std::collections::BTreeMap<[i32; 2], Vec<u32>> = Default::default();
        for (id, set) in self.sets.iter().enumerate() {
            let Some(set) = set else {
                continue;
            };
            records[id] = GpuRegionSet {
                origin: set.origin.to_array(),
                cells: set.cells.0,
                dims: set.dims,
                pad: [0; 2],
            };
            let lo = (Vec2::new(set.lo.x, set.lo.z) / TOP_CELL)
                .floor()
                .as_ivec2();
            let hi = (Vec2::new(set.hi.x, set.hi.z) / TOP_CELL)
                .floor()
                .as_ivec2();
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    lists.entry([x, z]).or_default().push(id as u32);
                }
            }
        }
        let listed: u32 = lists.values().map(|l| l.len() as u32).sum();
        if listed > self.top_list || lists.len() as u32 * 2 > self.top_entries {
            return Err(GpuError::Unsupported(format!(
                "the probes' coarse region grid is out of room ({} cells, {listed} entries; room \
                 for {} and {})",
                lists.len(),
                self.top_entries / 2,
                self.top_list
            )));
        }
        let mut entries = vec![GpuTopEntry::default(); self.top_entries as usize];
        let mut list = Vec::with_capacity(listed as usize);
        for (key, sets) in lists {
            let mut at = top_hash(key, self.top_mask);
            while entries[at as usize].count != 0 {
                at = (at + 1) & self.top_mask;
            }
            entries[at as usize] = GpuTopEntry {
                key,
                first: list.len() as u32,
                count: sets.len() as u32,
            };
            list.extend(sets);
        }
        list.resize(self.top_list as usize, 0);
        self.tables.clear();
        self.tables
            .extend_from_slice(bytemuck::cast_slice(&records));
        self.tables
            .extend_from_slice(bytemuck::cast_slice(&entries));
        self.tables.extend_from_slice(bytemuck::cast_slice(&list));
        self.tables_generation += 1;
        Ok(())
    }

    /// Frame slot `slot`'s copies brought up to the edits, the ranges no frame reads any more
    /// given back, and the addresses the frame's field names.
    pub(super) fn frame(&mut self, slot: usize) -> RegionFrame {
        self.frame += 1;
        let frame = self.frame;
        let mut k = 0;
        while k < self.leaving.len() {
            if self.leaving[k].0 <= frame {
                let (_, set) = self.leaving.swap_remove(k);
                self.give_back(&set);
            } else {
                k += 1;
            }
        }
        if self.tables_written[slot] != self.tables_generation {
            self.slot_tables[slot].write(0, &self.tables);
            self.tables_written[slot] = self.tables_generation;
        }
        if self.split_written[slot] != self.split_generation {
            let since = self.split_written[slot];
            for &(generation, first, count) in &self.split_log {
                if generation > since {
                    let range = first as usize..(first + count) as usize;
                    self.slot_splits[slot].write(u64::from(first) * 4, &self.splits[range]);
                }
            }
            self.split_written[slot] = self.split_generation;
            let oldest = self.split_written.iter().copied().min().unwrap_or(0);
            self.split_log
                .retain(|&(generation, ..)| generation > oldest);
        }
        let base = self.slot_tables[slot].address();
        let sets_bytes = u64::from(self.capacity.sets) * std::mem::size_of::<GpuRegionSet>() as u64;
        let top_bytes = u64::from(self.top_entries) * std::mem::size_of::<GpuTopEntry>() as u64;
        RegionFrame {
            volumes: self.volumes.address(),
            cells: self.cells.address(),
            indices: self.indices.address(),
            sets: base,
            top: base + sets_bytes,
            top_list: base + sets_bytes + top_bytes,
            splits: self.slot_splits[slot].address(),
            top_mask: self.top_mask,
            count: self.live_volumes,
        }
    }

    /// The boxes whose probes this frame wakes, taken.
    pub(super) fn take_wake(&mut self) -> Vec<(Vec3, Vec3)> {
        std::mem::take(&mut self.wake)
    }

    /// Bytes it holds on the GPU and in the frames' copies.
    pub(super) fn bytes(&self) -> u64 {
        self.volumes.size()
            + self.cells.size()
            + self.indices.size()
            + self.slot_tables.iter().map(Buffer::size).sum::<u64>()
            + self.slot_splits.iter().map(Buffer::size).sum::<u64>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coarse_hash_spreads_neighbouring_cells() {
        let mask = 255;
        let mut seen = std::collections::HashSet::new();
        for x in -4..4 {
            for z in -4..4 {
                seen.insert(top_hash([x, z], mask));
            }
        }
        assert!(seen.len() > 48, "{} slots for 64 cells", seen.len());
    }
}
