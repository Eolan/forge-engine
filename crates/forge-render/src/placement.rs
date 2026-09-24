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

/// Fills `scene`'s slots `first..first + layout.total` (reserved with the counts of
/// [`mesh_counts`]) on the GPU, then reads them back for the report. Initialisation only.
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
}
