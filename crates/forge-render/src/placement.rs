//! GPU placement of the city-blocks instances (issue #35; `place_main` in
//! `shaders/meshlet.slang`). A compute pass writes the scene's instance table once, before the
//! first frame, a thread per slot from the seed and the slot's index alone: buildings on the
//! block lots of a street grid, lamp posts along both sides of every street, a fountain and
//! four columns on some crossings, and rocks over the hills around the city, each set on the
//! ground the terrain's samples give.
//!
//! The CPU only lays out the categories ([`CityLayout::counts`]) and mirrors the mesh choice
//! ([`mesh_counts`], the same `pcg4d` as the shader) so the scene knows how many instances
//! show each mesh. After the pass the table is read back once: its checksum goes to the log
//! (the same seed gives the same table), and the meshes it holds are checked against the
//! mirror.

use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{ComputePipelineDesc, Device, Result, ShaderCompiler, ShaderStage, vk};
use glam::Vec3;

use crate::meshlet::{MeshId, MeshletScene};

/// The city's layout and the number of instances to place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CityLayout {
    /// Seed of every random choice.
    pub seed: u32,
    /// Half the city square's side, metres (the terrain's flat part).
    pub city_half: f32,
    /// Block pitch: a block and a street, metres.
    pub block: f32,
    /// Street width, metres.
    pub street: f32,
    /// Metres between lamp posts.
    pub lamp_step: f32,
    /// Crossings `k` with `k % plaza_every == plaza_every / 2` along both axes hold a plaza.
    pub plaza_every: u32,
    /// Instances to place in all; what the city does not take goes to the hills as rocks.
    pub total: u32,
    /// Where the city's centre sits in the world, metres (issue #93): the layout is made around
    /// its own centre and the instances are written this far from the world's origin.
    pub origin: Vec3,
}

/// Slots per category, in the order the shader lays them out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CategoryCounts {
    /// Four per block.
    pub buildings: u32,
    /// Along both sides of every street.
    pub lamps: u32,
    /// Five per plaza: a fountain and four columns.
    pub plaza_slots: u32,
    /// The rest.
    pub rocks: u32,
}

impl CityLayout {
    /// The city-blocks layout: a 2.4 km city of 100 m blocks (20 m streets), a lamp post
    /// every 25 m, a plaza on every fourth crossing, `total` instances in all.
    pub fn city(total: u32) -> Self {
        Self {
            seed: 3535,
            city_half: 1200.0,
            block: 100.0,
            street: 20.0,
            lamp_step: 25.0,
            plaza_every: 4,
            total,
            origin: Vec3::ZERO,
        }
    }

    /// Blocks per side of the city.
    pub fn blocks(&self) -> u32 {
        (2.0 * self.city_half / self.block) as u32
    }

    /// Plazas per side (crossings `0..=blocks` whose index fits the plaza rule).
    fn plazas_per_side(&self) -> u32 {
        let e = self.plaza_every.max(1);
        (self.blocks() + e - e / 2) / e
    }

    /// Lamp posts per street line and side.
    fn lamps_per_line(&self) -> u32 {
        (2.0 * self.city_half / self.lamp_step) as u32
    }

    /// Slots per category.
    pub fn counts(&self) -> CategoryCounts {
        let b = self.blocks();
        let buildings = 4 * b * b;
        let lamps = 2 * (b + 1) * 2 * self.lamps_per_line();
        let plaza_slots = 5 * self.plazas_per_side().pow(2);
        CategoryCounts {
            buildings,
            lamps,
            plaza_slots,
            rocks: self.total.saturating_sub(buildings + lamps + plaza_slots),
        }
    }
}

/// Which meshes the categories use.
#[derive(Clone, Debug)]
pub struct CityMeshes {
    /// Buildings, chosen per lot (at most 16).
    pub buildings: Vec<MeshId>,
    /// Rocks and rubble, chosen per slot (at most 8).
    pub rocks: Vec<MeshId>,
    /// The lamp post.
    pub lamp: MeshId,
    /// The fountain.
    pub fountain: MeshId,
    /// The column.
    pub column: MeshId,
}

/// The shader's choice of a building for `slot`: `pcg4d(slot, seed, 1, 0).x`, mirrored.
fn building_choice(layout: &CityLayout, slot: u32, count: u32) -> usize {
    (forge_core::hash::pcg4d([slot, layout.seed, 1, 0])[0] % count) as usize
}

/// The shader's choice of a rock for `slot` (its index among all placed slots).
fn rock_choice(layout: &CityLayout, slot: u32, count: u32) -> usize {
    (forge_core::hash::pcg4d([slot, layout.seed, 6, 0])[0] % count) as usize
}

/// How many placed instances show each mesh (the CPU mirror of the shader's choices), for
/// [`crate::MeshletSceneBuilder::reserve_instances`].
pub fn mesh_counts(layout: &CityLayout, meshes: &CityMeshes) -> Vec<(MeshId, u32)> {
    let counts = layout.counts();
    let mut buildings = vec![0_u32; meshes.buildings.len()];
    for slot in 0..counts.buildings {
        buildings[building_choice(layout, slot, meshes.buildings.len() as u32)] += 1;
    }
    let mut rocks = vec![0_u32; meshes.rocks.len()];
    let first_rock = counts.buildings + counts.lamps + counts.plaza_slots;
    for slot in first_rock..first_rock + counts.rocks {
        rocks[rock_choice(layout, slot, meshes.rocks.len() as u32)] += 1;
    }
    let plazas = counts.plaza_slots / 5;
    let mut out: Vec<(MeshId, u32)> = meshes.buildings.iter().copied().zip(buildings).collect();
    out.extend(meshes.rocks.iter().copied().zip(rocks));
    out.push((meshes.lamp, counts.lamps));
    out.push((meshes.fountain, plazas));
    out.push((meshes.column, 4 * plazas));
    out
}

/// Mirrors `Placement` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuPlacement {
    instances: u64,
    meshes: u64,
    heights: u64,
    first: u32,
    count: u32,
    samples: u32,
    seed: u32,
    spacing: f32,
    half_size: f32,
    city_half: f32,
    block: f32,
    street: f32,
    lamp_step: f32,
    blocks: u32,
    plaza_every: u32,
    buildings: u32,
    lamps: u32,
    plaza_slots: u32,
    rocks: u32,
    building_mesh_count: u32,
    rock_mesh_count: u32,
    lamp_mesh: u32,
    fountain_mesh: u32,
    column_mesh: u32,
    pad: u32,
    building_meshes: [u32; 16],
    rock_meshes: [u32; 8],
    /// The scene's offset from the world's origin (issue #93).
    origin: [f32; 3],
    pad2: f32,
}

/// The ground the instances stand on: `samples × samples` heights, `spacing` metres apart,
/// rows along +z, centred on the origin (the terrain mesh's vertices in cooking order).
pub struct Ground<'a> {
    /// Heights, metres.
    pub heights: &'a [f32],
    /// Samples per side.
    pub samples: u32,
    /// Metres between samples.
    pub spacing: f32,
}

/// The ground's layers (issue #42): the ids of [`ground_layers`], in the order of the rows that
/// follow the ground's layered material row.
pub mod layer {
    /// Grass, on the lots and the gentle hills.
    pub const GRASS: u8 = 0;
    /// Asphalt, down the middle of the streets.
    pub const ASPHALT: u8 = 1;
    /// Sidewalks, the edges of the streets.
    pub const SIDEWALK: u8 = 2;
    /// Paving, on the plazas.
    pub const PAVING: u8 = 3;
    /// Rock, where the hills are steep.
    pub const ROCK: u8 = 4;
    /// How many layers there are.
    pub const COUNT: u8 = 5;
}

/// Metres of sidewalk along each side of a street.
const SIDEWALK_WIDTH: f32 = 3.5;
/// Radius of a plaza's paving around its crossing, metres.
const PLAZA_RADIUS: f32 = 14.0;
/// Rise over run above which the hills are rock.
const ROCK_SLOPE: f32 = 0.45;

impl Ground<'_> {
    /// Half the side of the square the ground covers, metres.
    pub fn half_size(&self) -> f32 {
        (self.samples - 1) as f32 * self.spacing * 0.5
    }

    /// The height at (x, z), bilinearly (the placement shader's `ground`).
    fn height(&self, x: f32, z: f32) -> f32 {
        let n = self.samples as usize;
        let max = (self.samples - 1) as f32 - 1e-3;
        let gx = ((x + self.half_size()) / self.spacing).clamp(0.0, max);
        let gz = ((z + self.half_size()) / self.spacing).clamp(0.0, max);
        let (i, j) = (gx as usize, gz as usize);
        let (fx, fz) = (gx - i as f32, gz - j as f32);
        let h = |i: usize, j: usize| self.heights[j * n + i];
        let top = h(i, j) + (h(i + 1, j) - h(i, j)) * fx;
        let bottom = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * fx;
        top + (bottom - top) * fz
    }

    /// Rise over run at (x, z).
    fn slope(&self, x: f32, z: f32) -> f32 {
        let d = self.spacing;
        let gx = (self.height(x + d, z) - self.height(x - d, z)) / (2.0 * d);
        let gz = (self.height(x, z + d) - self.height(x, z - d)) / (2.0 * d);
        (gx * gx + gz * gz).sqrt()
    }
}

/// The ground's layer of each of `texels × texels` cells over the whole terrain, rows along
/// +z (the layer map of the terrain's layered material). The city's streets are asphalt
/// with sidewalks along both sides, its plazas paving, its lots grass. Beyond the city the
/// hills are grass, or rock where they rise more than [`ROCK_SLOPE`].
pub fn ground_layers(layout: &CityLayout, ground: &Ground<'_>, texels: u32) -> Vec<u8> {
    let half = ground.half_size();
    let cell = 2.0 * half / texels as f32;
    let layer_at = |x: f32, z: f32| ground_layer(layout, ground, x, z);
    let mut layers = vec![0_u8; (texels * texels) as usize];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per = (texels as usize).div_ceil(threads);
    std::thread::scope(|s| {
        for (chunk, rows) in layers.chunks_mut(rows_per * texels as usize).enumerate() {
            s.spawn(move || {
                for (r, row) in rows.chunks_mut(texels as usize).enumerate() {
                    let z = -half + ((chunk * rows_per + r) as f32 + 0.5) * cell;
                    for (i, texel) in row.iter_mut().enumerate() {
                        *texel = layer_at(-half + (i as f32 + 0.5) * cell, z);
                    }
                }
            });
        }
    });
    layers
}

/// The ground's layer at (x, z) (see [`ground_layers`]).
pub fn ground_layer(layout: &CityLayout, ground: &Ground<'_>, x: f32, z: f32) -> u8 {
    let blocks = layout.blocks() as f32;
    let street_half = layout.street * 0.5;
    let city_edge = layout.city_half + street_half;
    let e = layout.plaza_every.max(1);
    let plaza = |k: f32| (k as u32) % e == e / 2;
    if x.abs() <= city_edge && z.abs() <= city_edge {
        // The nearest street line along each axis, and how far from it.
        let line = |p: f32| {
            ((p + layout.city_half) / layout.block)
                .round()
                .clamp(0.0, blocks)
        };
        let (kx, kz) = (line(x), line(z));
        let dx = (x - (-layout.city_half + kx * layout.block)).abs();
        let dz = (z - (-layout.city_half + kz * layout.block)).abs();
        if plaza(kx) && plaza(kz) && dx.hypot(dz) < PLAZA_RADIUS {
            return layer::PAVING;
        }
        let d = dx.min(dz);
        if d <= street_half - SIDEWALK_WIDTH {
            return layer::ASPHALT;
        }
        if d <= street_half {
            return layer::SIDEWALK;
        }
        return layer::GRASS;
    }
    if ground.slope(x, z) > ROCK_SLOPE {
        layer::ROCK
    } else {
        layer::GRASS
    }
}

/// What a placement did.
#[derive(Clone, Copy, Debug)]
pub struct PlacementReport {
    /// Instances placed.
    pub placed: u32,
    /// Milliseconds of the pass: uploads of its inputs, the dispatch and the wait.
    pub ms: f64,
    /// FNV-1a of the placed range of the table, as read back.
    pub checksum: u64,
    /// The table's meshes, counted from the read-back, match [`mesh_counts`].
    pub matches_mirror: bool,
}

/// FNV-1a over `bytes`.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Spreads the low 16 bits of `v` to the even bits of the result.
fn spread_bits(v: u32) -> u32 {
    let mut x = v & 0xFFFF;
    x = (x | (x << 8)) & 0x00FF_00FF;
    x = (x | (x << 4)) & 0x0F0F_0F0F;
    x = (x | (x << 2)) & 0x3333_3333;
    (x | (x << 1)) & 0x5555_5555
}

/// The instance records of `table` (`record` bytes each, the centre at bytes 64..76) reordered
/// along a Morton curve of their centres' x and z over the square of half-side `half` around
/// `origin`, ties kept in table order, so that runs of consecutive records are compact patches.
fn morton_sorted(table: &[u8], record: usize, half: f32, origin: Vec3) -> Vec<u8> {
    let records: Vec<&[u8]> = table.chunks_exact(record).collect();
    let coordinate = |bytes: &[u8]| f32::from_le_bytes(bytes.try_into().expect("4 bytes"));
    let cell = |v: f32| (((v + half) / (2.0 * half)).clamp(0.0, 1.0) * 65535.0) as u32;
    let mut keys: Vec<(u32, u32)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let x = coordinate(&r[64..68]) - origin.x;
            let z = coordinate(&r[72..76]) - origin.z;
            (spread_bits(cell(x)) | (spread_bits(cell(z)) << 1), i as u32)
        })
        .collect();
    keys.sort_unstable();
    keys.iter()
        .flat_map(|&(_, i)| records[i as usize].iter().copied())
        .collect()
}

/// Fills `scene`'s slots `first..first + layout.total` (reserved with the counts of
/// [`mesh_counts`]) on the GPU and reads them back for the report (its checksum is the table
/// as placed), then writes them again in Morton order of their centres (issue #38). The city
/// stands `layout.origin` from the world's origin (issue #93): the positions carry the offset,
/// the Morton order is taken without it. Initialisation only.
pub fn place(
    device: &Arc<Device>,
    shaders: &ShaderCompiler,
    scene: &MeshletScene,
    first: u32,
    layout: &CityLayout,
    meshes: &CityMeshes,
    ground: &Ground<'_>,
) -> Result<PlacementReport> {
    assert!(meshes.buildings.len() <= 16 && meshes.rocks.len() <= 8);
    let start = Instant::now();
    let module = device.create_shader_module(
        &shaders.compile("meshlet.slang", "place_main", ShaderStage::Compute)?,
        "placement",
    )?;
    let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
        shader: (module, "place_main"),
        push_constant_bytes: 8,
        name: "placement",
    });
    device.destroy_shader_module(module);
    let pipeline = pipeline?;
    let heights = device.create_buffer_with_data(
        ground.heights,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        forge_gpu::MemoryCategory::Transfer,
        "placement heights",
    )?;
    let counts = layout.counts();
    let mut building_meshes = [0_u32; 16];
    for (slot, mesh) in building_meshes.iter_mut().zip(&meshes.buildings) {
        *slot = mesh.index();
    }
    let mut rock_meshes = [0_u32; 8];
    for (slot, mesh) in rock_meshes.iter_mut().zip(&meshes.rocks) {
        *slot = mesh.index();
    }
    let params = GpuPlacement {
        instances: scene.instance_buffer().address(),
        meshes: scene.mesh_buffer().address(),
        heights: heights.address(),
        first,
        count: layout.total,
        samples: ground.samples,
        seed: layout.seed,
        spacing: ground.spacing,
        half_size: (ground.samples - 1) as f32 * ground.spacing * 0.5,
        city_half: layout.city_half,
        block: layout.block,
        street: layout.street,
        lamp_step: layout.lamp_step,
        blocks: layout.blocks(),
        plaza_every: layout.plaza_every,
        buildings: counts.buildings,
        lamps: counts.lamps,
        plaza_slots: counts.plaza_slots,
        rocks: counts.rocks,
        building_mesh_count: meshes.buildings.len() as u32,
        rock_mesh_count: meshes.rocks.len() as u32,
        lamp_mesh: meshes.lamp.index(),
        fountain_mesh: meshes.fountain.index(),
        column_mesh: meshes.column.index(),
        pad: 0,
        building_meshes,
        rock_meshes,
        origin: layout.origin.to_array(),
        pad2: 0.0,
    };
    let params = device.create_buffer_with_data(
        &[params],
        vk::BufferUsageFlags::STORAGE_BUFFER,
        forge_gpu::MemoryCategory::Transfer,
        "placement parameters",
    )?;
    let address = params.address();
    device.execute_compute_once(|commands| {
        commands.bind_pipeline(&pipeline);
        commands.push_constants(&pipeline, &address);
        commands.dispatch(layout.total.div_ceil(64), 1, 1);
    })?;
    let ms = start.elapsed().as_secs_f64() * 1e3;

    // The table as the GPU wrote it: its checksum, and its meshes against the mirror.
    const INSTANCE_BYTES: u64 = 96;
    let bytes = device.read_back(
        scene.instance_buffer(),
        u64::from(first) * INSTANCE_BYTES,
        u64::from(layout.total) * INSTANCE_BYTES,
    )?;
    let checksum = fnv1a64(&bytes);
    let mut seen = std::collections::HashMap::<u32, u32>::new();
    for instance in bytes.as_chunks::<{ INSTANCE_BYTES as usize }>().0 {
        // `mesh` follows the model (64 bytes), the centre (12) and the radius (4).
        let mesh = u32::from_le_bytes(instance[80..84].try_into().expect("4 bytes"));
        *seen.entry(mesh).or_default() += 1;
    }
    let mut expected = std::collections::HashMap::<u32, u32>::new();
    for (mesh, count) in mesh_counts(layout, meshes) {
        *expected.entry(mesh.index()).or_default() += count;
    }
    expected.retain(|_, count| *count > 0);

    // The table in Morton order of the centres (issue #38): the instance culls read it in cells
    // of 64 consecutive slots, which the rocks' random slots would spread over every hill.
    let half = (ground.samples - 1) as f32 * ground.spacing * 0.5;
    let sorted = morton_sorted(&bytes, INSTANCE_BYTES as usize, half, layout.origin);
    device.write_buffer_staged(
        scene.instance_buffer(),
        u64::from(first) * INSTANCE_BYTES,
        &sorted,
    )?;
    Ok(PlacementReport {
        placed: layout.total,
        ms,
        checksum,
        matches_mirror: seen == expected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ground_layers_follow_the_street_grid_and_the_slopes() {
        let layout = CityLayout::city(1_000);
        // A flat 4 km ground with a steep ramp along its east edge.
        let samples = 401_u32;
        let spacing = 10.0;
        let heights: Vec<f32> = (0..samples * samples)
            .map(|i| {
                let x = (i % samples) as f32 * spacing - 2000.0;
                if x > 1700.0 { (x - 1700.0) * 0.8 } else { 0.0 }
            })
            .collect();
        let ground = Ground {
            heights: &heights,
            samples,
            spacing,
        };
        let at = |x: f32, z: f32| ground_layer(&layout, &ground, x, z);
        // The street line at x = -1200 + 100 k: asphalt in the middle, sidewalk at its edges.
        assert_eq!(at(-1100.0, 30.0), layer::ASPHALT);
        assert_eq!(at(-1100.0 + 8.0, 30.0), layer::SIDEWALK);
        // Inside a block: a lot.
        assert_eq!(at(-1050.0, 50.0), layer::GRASS);
        // Crossing k = (2, 2) holds a plaza (every fourth crossing, the second of four).
        assert_eq!(at(-1000.0 + 5.0, -1000.0 + 5.0), layer::PAVING);
        assert_eq!(at(-1100.0 + 5.0, -1100.0 + 5.0), layer::ASPHALT);
        // Beyond the city: grass on the flat, rock on the ramp.
        assert_eq!(at(1500.0, 0.0), layer::GRASS);
        assert_eq!(at(1850.0, 0.0), layer::ROCK);
        // The map is the same rule, a texel at a time.
        let map = ground_layers(&layout, &ground, 400);
        assert_eq!(map.len(), 400 * 400);
        assert!(map.iter().all(|&l| l < layer::COUNT));
        assert!(map.contains(&layer::ROCK) && map.contains(&layer::ASPHALT));
    }

    #[test]
    fn the_city_takes_what_it_holds_and_the_hills_the_rest() {
        let layout = CityLayout::city(1_000_000);
        let c = layout.counts();
        assert_eq!(layout.blocks(), 24);
        assert_eq!(c.buildings, 4 * 24 * 24);
        assert_eq!(c.lamps, 2 * 25 * 2 * 96);
        // Crossings 2, 6, 10, 14, 18 and 22 on each axis.
        assert_eq!(c.plaza_slots, 5 * 6 * 6);
        assert_eq!(c.buildings + c.lamps + c.plaza_slots + c.rocks, 1_000_000);
    }

    #[test]
    fn the_morton_order_keeps_every_record_and_packs_neighbours_together() {
        // 256 records on a 16 × 16 grid of 10 m, in a scrambled order, their index as the id.
        const RECORD: usize = 96;
        let mut table = Vec::new();
        for k in 0..256_u32 {
            let i = (k * 97) % 256;
            let (x, z) = ((i % 16) as f32 * 10.0 - 75.0, (i / 16) as f32 * 10.0 - 75.0);
            let mut record = [0_u8; RECORD];
            record[64..68].copy_from_slice(&x.to_le_bytes());
            record[72..76].copy_from_slice(&z.to_le_bytes());
            record[84..88].copy_from_slice(&i.to_le_bytes());
            table.extend_from_slice(&record);
        }
        let sorted = morton_sorted(&table, RECORD, 80.0, Vec3::ZERO);
        let field = |r: &[u8], at: usize| f32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let records = sorted.as_chunks::<RECORD>().0;
        let mut ids: Vec<u32> = records
            .iter()
            .map(|r| u32::from_le_bytes(r[84..88].try_into().unwrap()))
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..256).collect::<Vec<_>>());
        // Along the curve, every run of 16 records is a 4 × 4 block of the grid.
        for run in records.chunks(16) {
            for at in [64, 72] {
                let values = run.iter().map(|r| field(r, at));
                let (lo, hi) = values.fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v), h.max(v)));
                assert_eq!(hi - lo, 30.0);
            }
        }
    }
}
