//! Forge's Vulkan layer.
//!
//! Vulkan 1.3 baseline (dynamic rendering, synchronization2, timeline semaphores, buffer
//! device address, descriptor indexing) with mesh shaders and ray queries as optional
//! features detected at device creation. Shaders are Slang, compiled by `slangc` into a
//! content-addressed cache. Every buffer carries its device address so shaders can walk the
//! scene through pointers instead of descriptor sets (see `shaders/meshlet.slang`).
//!
//! `unsafe` is unavoidable here: every Vulkan call is `unsafe` in `ash`. Each block carries a
//! `SAFETY:` note stating the Vulkan-level invariant it relies on.

#![allow(unsafe_code)]

mod accel;
mod bindless;
mod commands;
mod device;
pub mod dlss;
mod error;
mod frame;
#[forbid(unsafe_code)]
pub mod graph;
mod instance;
mod memory;
mod memory_report;
mod pipeline;
mod shader;
#[cfg(all(feature = "dlss", windows))]
mod streamline;
mod swapchain;
mod timers;

pub use accel::{AccelerationStructure, BlasTriangles};
pub use ash;
pub use ash::vk;
pub use bindless::{
    BINDING_SAMPLED_IMAGES, BINDING_SAMPLERS, BINDING_STORAGE_IMAGES, SampledImageId, SamplerKind,
    StorageImageId,
};
pub use commands::Commands;
pub use device::{Device, DeviceFeatures, DeviceOptions, MeshShaderLimits, VENDOR_NVIDIA};
pub use dlss::{Dlss, DlssFrame, DlssImage, DlssImages, DlssMode, DlssToken};
pub use error::{GpuError, Result};
pub use frame::{FRAMES_IN_FLIGHT, FrameSlot, Frames};
pub use gpu_allocator::MemoryLocation;
pub use graph::{
    BufferAccess, BufferHandle, FrameGraph, GraphBuffer, GraphImage, GraphStats, ImageAccess,
    ImageHandle, RawImage, RenderGraph, ResolvedImage, ResourceState, Resources, TransientDesc,
};
pub use instance::{Instance, Surface};
pub use memory::{Buffer, BufferDesc, Image, ImageDesc, TransientHeap};
pub use memory_report::{BUDGET_WARNING, HeapReport, MemoryCategory, MemoryReport};
pub use pipeline::{
    ComputePipelineDesc, FullscreenPipelineDesc, MeshPipelineDesc, Pipeline, VertexPipelineDesc,
};
pub use shader::{ShaderCompiler, ShaderEntry, ShaderStage};
pub use swapchain::Swapchain;
pub use timers::{GpuTimerSlot, GpuTimers, GpuZone, MAX_MARKS_PER_FRAME};
