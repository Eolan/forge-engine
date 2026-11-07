//! The textures' feedback (#385's first step, `FORGE_TEXTURE_FEEDBACK=1`): which level of each
//! texture the frame's surfaces want, to measure what a world needs resident before choosing how
//! to stream its textures.
//!
//! After the resolve, a compute pass (`texture_need_main` in `meshlet.slang`) looks at every 8th
//! pixel each way, a different one of each 8 x 8 block each frame. It rebuilds each pixel's
//! surface as the resolve does, and for each texture its material samples it records the finest
//! level an anisotropic sample there takes, the least over the frame, by the texture's bindless
//! index. A layered material's layers count at their own tiling, its masks and microblends too.
//!
//! The frame slot's copy is read when the slot comes round again and summed into:
//! - **resident:** the textures' bytes, every level;
//! - **needed over the run:** each texture from the finest level it was ever wanted at;
//! - **the working set:** each texture from the finest level wanted within the last two seconds,
//!   its mean and its largest over the frames.
//!
//! In each, a texture's levels of 128 texels and less count as always resident. The totals are
//! logged every 1,200 frames and when the renderer is dropped.
//!
//! **What it doesn't see:** decals, glass, holograms, particles, impostors and the rays' hits,
//! which sample their own levels, and a layered material's presence maps (small, always wanted).

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    SAMPLED_IMAGE_CAPACITY, ShaderCompiler, ShaderStage, vk,
};

use crate::material::TextureSet;

/// Must match `TextureNeedPush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextureNeedPush {
    frame: u64,
    need: u64,
    vis_image: u32,
    width: u32,
    height: u32,
    offset: u32,
}

/// No level wanted.
const NONE: u32 = u32::MAX;
/// Levels a texture holds at most.
const LEVELS: usize = 16;
/// Seconds a level wanted counts in the working set.
const WINDOW: f32 = 2.0;
/// A level this many texels across or less is always resident.
const TAIL: u32 = 128;
/// Frames between the logged totals.
const LOG_EVERY: u64 = 1200;

/// The feedback pass, its buffers and the run's totals.
pub(crate) struct TextureFeedback {
    pipeline: Pipeline,
    /// Per bindless index, the finest level the frame wanted (device-local, atomics).
    need: GraphBuffer,
    /// Its copy per frame slot, for the host.
    readback: Vec<GraphBuffer>,
    totals: RefCell<Totals>,
}

/// What the run has wanted so far.
#[derive(Default)]
struct Totals {
    start: Option<Instant>,
    /// The scene's textures it has the sizes of (their count), and per bindless index their levels'
    /// bytes, the finest first, and their first level of the tail.
    known: usize,
    bytes: Vec<Vec<u64>>,
    tail: Vec<usize>,
    /// Per bindless index, the finest level the run wanted (`NONE`: never).
    finest: Vec<u32>,
    /// Per bindless index and level, when it was last wanted (seconds; negative: never).
    last: Vec<[f32; LEVELS]>,
    frames: u64,
    working_sum: f64,
    working_max: u64,
}

impl TextureFeedback {
    /// The feedback with `FORGE_TEXTURE_FEEDBACK=1`, its entry compiled from `shading` (the
    /// renderer's shader file); else none, and nothing compiled.
    pub(crate) fn from_env(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        shading: &'static str,
    ) -> Result<Option<Self>> {
        if !std::env::var("FORGE_TEXTURE_FEEDBACK").is_ok_and(|v| v == "1") {
            return Ok(None);
        }
        let entry = "texture_need_main";
        let module = device.create_shader_module(
            &shaders.compile(shading, entry, ShaderStage::Compute)?,
            "texture feedback",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, entry),
            push_constant_bytes: std::mem::size_of::<TextureNeedPush>() as u32,
            name: "texture feedback",
        });
        device.destroy_shader_module(module);
        let pipeline = pipeline?;
        let bytes = u64::from(SAMPLED_IMAGE_CAPACITY) * 4;
        let need = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: bytes,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "texture needs",
        })?);
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let b = device.create_buffer(BufferDesc {
                    size: bytes,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("texture needs readback {i}"),
                })?;
                b.write(0, &vec![NONE; SAMPLED_IMAGE_CAPACITY as usize]);
                Ok(GraphBuffer::new(b))
            })
            .collect::<Result<Vec<_>>>()?;
        tracing::info!(
            "texture feedback on (#385): every 8th pixel each way, totals logged every {LOG_EVERY} frames and at the end"
        );
        Ok(Some(Self {
            pipeline,
            need,
            readback,
            totals: RefCell::default(),
        }))
    }

    /// The frame's passes, after the resolve: the needs cleared, recorded from the visibility
    /// buffer `visibility` (the frame block at `frame`), and copied into the slot's readback.
    pub(crate) fn record<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        visibility: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let need_buffer: &'f GraphBuffer = &self.need;
        let readback_buffer: &'f GraphBuffer = &self.readback[slot.index];
        let need = graph.import_buffer(need_buffer);
        let readback = graph.import_buffer(readback_buffer);
        let bytes = need_buffer.size();
        graph
            .pass("textures/need clear")
            .buffer(need, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(need_buffer, 0, bytes, NONE);
                Ok(())
            });
        let pipeline = &self.pipeline;
        let offset = (slot.frame_number % 64) as u32;
        graph
            .pass("textures/need")
            .image(visibility, ImageAccess::Sampled(compute))
            .buffer(need, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &TextureNeedPush {
                        frame,
                        need: need_buffer.address(),
                        vis_image: resources.sampled(visibility).0,
                        width: extent.width,
                        height: extent.height,
                        offset,
                    },
                );
                // A thread a pixel of each 8 x 8 block, 8 x 8 threads a group.
                commands.dispatch(extent.width.div_ceil(64), extent.height.div_ceil(64), 1);
                Ok(())
            });
        graph
            .pass("textures/need copy")
            .buffer(need, BufferAccess::TransferSrc)
            .buffer(readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(need_buffer, readback_buffer, bytes);
                Ok(())
            });
        graph
            .pass("textures/need copy")
            .buffer(readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// The needs the frame that last used `slot` wrote (done by now), summed into the run's
    /// totals with the sizes of `textures`, the scene's.
    pub(crate) fn take(&self, slot: FrameSlot, textures: Option<&TextureSet>) {
        if slot.frame_number < FRAMES_IN_FLIGHT as u64 {
            return;
        }
        let mut t = self.totals.borrow_mut();
        let now = t
            .start
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_secs_f32();
        if let Some(set) = textures
            && set.len() != t.known
        {
            t.learn(set);
        }
        let mut raw = vec![NONE; SAMPLED_IMAGE_CAPACITY as usize];
        self.readback[slot.index].read(0, &mut raw);
        for (i, &level) in raw.iter().enumerate() {
            if level == NONE || i >= t.last.len() {
                continue;
            }
            t.last[i][(level as usize).min(LEVELS - 1)] = now;
            t.finest[i] = t.finest[i].min(level);
        }
        let working = t.working_set(now);
        t.frames += 1;
        t.working_sum += working as f64;
        t.working_max = t.working_max.max(working);
        if t.frames.is_multiple_of(LOG_EVERY) {
            t.log();
        }
    }
}

impl Drop for TextureFeedback {
    fn drop(&mut self) {
        let t = self.totals.get_mut();
        if t.frames > 0 {
            t.log();
        }
    }
}

impl Totals {
    /// The sizes of `set`'s textures, by bindless index.
    fn learn(&mut self, set: &TextureSet) {
        let n = SAMPLED_IMAGE_CAPACITY as usize;
        if self.finest.is_empty() {
            self.finest = vec![NONE; n];
            self.last = vec![[-1.0; LEVELS]; n];
        }
        self.bytes = vec![Vec::new(); n];
        self.tail = vec![0; n];
        for (sampled, side, levels) in set.level_bytes() {
            let i = sampled as usize;
            if i >= n {
                continue;
            }
            self.tail[i] = (0..levels.len())
                .find(|&l| side >> l <= TAIL)
                .unwrap_or(levels.len());
            self.bytes[i] = levels;
        }
        self.known = set.len();
    }

    /// The bytes of texture `i` from level `from`, or from its tail where that is finer.
    fn bytes_from(&self, i: usize, from: usize) -> u64 {
        let levels = &self.bytes[i];
        levels[from.min(self.tail[i]).min(levels.len())..]
            .iter()
            .sum()
    }

    /// Every texture from the finest level wanted within the window before `now`.
    fn working_set(&self, now: f32) -> u64 {
        (0..self.bytes.len())
            .filter(|&i| !self.bytes[i].is_empty())
            .map(|i| {
                let from = self.last[i]
                    .iter()
                    .position(|&s| s >= 0.0 && now - s <= WINDOW)
                    .unwrap_or(LEVELS);
                self.bytes_from(i, from)
            })
            .sum()
    }

    fn log(&self) {
        let mib = |b: u64| b as f64 / f64::from(1u32 << 20);
        let textures: Vec<usize> = (0..self.bytes.len())
            .filter(|&i| !self.bytes[i].is_empty())
            .collect();
        let resident: u64 = textures
            .iter()
            .map(|&i| self.bytes[i].iter().sum::<u64>())
            .sum();
        let tail: u64 = textures.iter().map(|&i| self.bytes_from(i, LEVELS)).sum();
        let needed: u64 = textures
            .iter()
            .map(|&i| self.bytes_from(i, self.finest[i].min(LEVELS as u32) as usize))
            .sum();
        let seen = textures.iter().filter(|&&i| self.finest[i] != NONE).count();
        tracing::info!(
            textures = textures.len(),
            seen,
            frames = self.frames,
            resident_mib = format!("{:.0}", mib(resident)),
            needed_mib = format!("{:.0}", mib(needed)),
            working_mean_mib = format!(
                "{:.0}",
                mib((self.working_sum / self.frames.max(1) as f64) as u64)
            ),
            working_max_mib = format!("{:.0}", mib(self.working_max)),
            tail_mib = format!("{:.0}", mib(tail)),
            "texture feedback (#385): resident, needed over the run, the working set over {WINDOW} s"
        );
    }
}
