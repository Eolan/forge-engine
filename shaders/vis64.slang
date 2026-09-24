// Layout of the software rasteriser's 64-bit samples (meshlet.slang): one `depth << 32 | id`
// per pixel, row-major, 0 where it kept nothing (`crate::meshlet::vis64_bytes` sizes the
// buffer). A tiled layout (8×4-pixel tiles, a 2×2 quad per 32-byte sector) measured the same
// on the RTX 5070 Ti, so the simple one stays.
module vis64;

// Index of `pixel`'s sample in a buffer for a target `width` pixels wide.
public uint vis64_index(uint width, uint2 pixel)
{
    return pixel.y * width + pixel.x;
}

// The depth of sample `index` alone: its high 32-bit word, half the bytes of the sample.
public uint vis64_depth_bits(uint64_t* buffer, uint index)
{
    return ((uint*)buffer)[2 * index + 1];
}
