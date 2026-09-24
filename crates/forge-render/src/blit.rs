//! A whole-image blit as a graph pass (format conversion included): the bench's colour
//! target to the swapchain, captures of intermediate targets, and the like.

use forge_gpu::{FrameGraph, ImageAccess, ImageHandle, vk};

/// Declares a pass that blits every pixel of `src` onto `dst` (same size, `NEAREST`).
pub fn blit<'f>(
    graph: &mut FrameGraph<'f>,
    label: &'static str,
    src: ImageHandle,
    dst: ImageHandle,
    extent: vk::Extent2D,
) {
    graph
        .pass(label)
        .image(src, ImageAccess::TransferSrc)
        .image(dst, ImageAccess::TransferDst)
        .run(move |resources, commands| {
            commands.blit_image(
                resources.image(src).raw,
                resources.image(dst).raw,
                extent,
                vk::Filter::NEAREST,
            );
            Ok(())
        });
}
