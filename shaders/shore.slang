// The shore's waves as both the water's surface (water.slang) and the ground they wet
// (meshlet.slang's layered shading) see them (issue #105, D-038's shore): the sets along the
// coast, the crests' drift out of line, and each train's run-up at the shoreline. Both sides
// evaluate the same functions of position and time, so the sand is wet where the water's
// sheet ran up.
module shore;

// How far up the beach the swash's sheet may reach, metres from the shoreline.
public static const float SHORE_SWASH_REACH = 40.0;
// The swash's run-up against the trains' heights in deep water (a gentle, dissipative beach).
public static const float SHORE_RUN_UP = 0.3;

static const float TAU = 6.28318530718;

// A hash of an integer lattice point, in [0, 1).
float lattice_hash(int2 p)
{
    uint h = uint(p.x) * 0x8da6b343u ^ uint(p.y) * 0xd8163841u;
    h = (h ^ (h >> 15)) * 0x2c1b3c6du;
    h ^= h >> 12;
    return float(h & 0xFFFFFFu) / 16777216.0;
}

// Smooth value noise in [0, 1): the lattice's hashes blended by a smoothstep.
public float value_noise(float2 x)
{
    int2 i = int2(floor(x));
    float2 f = x - floor(x);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = lattice_hash(i);
    float b = lattice_hash(i + int2(1, 0));
    float c = lattice_hash(i + int2(0, 1));
    float d = lattice_hash(i + int2(1, 1));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

// Along the coast the sets rise and fall: a factor on the trains' heights at `world` (x, z).
public float shore_set(float2 world)
{
    return 0.55 + 0.45 * value_noise(world / 350.0);
}

// ... and the crests drift a little out of line: a phase, radians.
public float shore_drift(float2 world)
{
    return 2.2 * value_noise(world / 260.0 + 17.3);
}

// A train's swash at the shoreline at `time`, as a share of its run-up: a quick uprush over a
// fifth of its period, then a slow backwash. `train`: ω (rad/s), its height in deep water, a
// phase offset.
public float shore_rush(float4 train, float time, float drift)
{
    float u = frac((train.x * time + drift + train.z) / TAU);
    return u < 0.2 ? sin(0.25 * TAU * u / 0.2) : 0.5 + 0.5 * cos(0.5 * TAU * (u - 0.2) / 0.8);
}

// How far inland of the shoreline (`distance` metres) the sheet still reaches, 0..1.
public float shore_reach(float distance)
{
    return 1.0 - smoothstep(0.5 * SHORE_SWASH_REACH, SHORE_SWASH_REACH, distance);
}
