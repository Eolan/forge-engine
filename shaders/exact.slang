// Arithmetic that a precise entry keeps one way (#281). `-fp-mode precise` (`PRECISE_ENTRIES` in
// crates/forge-gpu/src/shader.rs, #277) marks every multiply and add `NoContraction`, but `lerp`
// compiles to GLSL.std.450's `FMix`, an instruction the mark does not reach. NVIDIA keeps two
// variants of a pipeline and evaluates `FMix` differently in them, so TAA's display pass still
// parted from one run to the next on its `lerp` of the history and the bloom. Written out, the
// multiply and the add are the entry's own and stay unfused.
module exact;

// `lerp(a, b, t)` as (b − a) t + a (aces2.slang's `lerpf`), for the passes that compile precise.
public float lerp_exact(float a, float b, float t)
{
    return (b - a) * t + a;
}

public float2 lerp_exact(float2 a, float2 b, float t)
{
    return (b - a) * t + a;
}

public float3 lerp_exact(float3 a, float3 b, float t)
{
    return (b - a) * t + a;
}
