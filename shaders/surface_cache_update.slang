// D-088's surface cache's passes over its entries, before and after their rays
// (`surface_cache_trace_main`, meshlet.slang): their ageing and their blend. The data, the lookup
// and the credits (Bevy Solari's world cache, kajiya's irradiance cache) are surface_cache.slang's.
import surface_cache;

struct Push
{
    SurfaceCacheField* field;
};

[[vk::push_constant]]
Push push;

// Frees the entries unread for CACHE_LIFE frames (Solari's `decay_world_cache`), those whose cell
// has left its level's window as the camera moved (the slot now stands for another cell), and
// after a reset every one: its slot empties and its number goes back on the stack. Before any
// lookup of the frame, so a slot only ever holds its own cell's entry. A freed entry's light is
// cleared: a lookup in a later pass that takes its number and one that reads it before the new
// entry's stores reach it both find no light. The entries alive here are this frame's rays'
// (`fresh` 0); one made from here on waits for the next frame (-1).
[shader("compute")]
[numthreads(64, 1, 1)]
void surface_cache_age_main(uint3 id : SV_DispatchThreadID)
{
    SurfaceCacheField* c = push.field;
    uint e = id.x;
    if (e >= c[0].pool)
        return;
    uint slot = c[0].entries[e].slot;
    if (slot == CACHE_EMPTY)
        return;
    int4 cell = c[0].entries[e].cell;
    int3 local = cell.xyz - c[0].origins[cell.w].xyz;
    bool inside = all(local >= 0) && all(local < int(CACHE_CELLS));
    uint life = c[0].entries[e].life;
    if (inside && life > 1u && c[0].reset == 0u)
    {
        c[0].entries[e].life = life - 1u;
        c[0].entries[e].fresh = float3(0.0);
        return;
    }
    c[0].index[slot] = CACHE_EMPTY;
    c[0].entries[e].slot = CACHE_EMPTY;
    c[0].entries[e].irradiance = float4(0.0);
    c[0].entries[e].fresh = float3(-1.0);
    uint at;
    InterlockedAdd(c[0].free[0], 1u, at);
    c[0].free[1u + at] = e;
}

// Blends each live entry's sample of this frame into its light (`cache_blend`); an entry made
// during the rays' pass has none yet (`fresh` negative). The first thread counts the entries in
// use for the host (`SurfaceCache::most_used`): nothing takes or frees one in this pass.
[shader("compute")]
[numthreads(64, 1, 1)]
void surface_cache_blend_main(uint3 id : SV_DispatchThreadID)
{
    SurfaceCacheField* c = push.field;
    uint e = id.x;
    if (e == 0u)
        c[0].used[0] = c[0].pool - c[0].free[0];
    if (e >= c[0].pool || c[0].entries[e].slot == CACHE_EMPTY || c[0].entries[e].fresh.x < 0.0)
        return;
    c[0].entries[e].irradiance = cache_blend(c[0].entries[e].irradiance, c[0].entries[e].fresh, c[0].most_samples);
}
