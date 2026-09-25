// Irradiance from the first nine spherical harmonics (issue #47), shared by the sky's
// projection (sky.slang) and the resolve (meshlet.slang). Ramamoorthi and Hanrahan, "An
// Efficient Representation for Irradiance Environment Maps", SIGGRAPH 2001: radiance
// projected on the bands 0–2 and convolved with the clamped cosine gives irradiance within
// a few per cent for any normal.
module sh;

public static const uint SH_COEFFICIENTS = 9;

// The real SH basis at the unit direction `d`: band 0, band 1 (y, z, x), band 2.
public void sh_basis(float3 d, out float b[9])
{
    b[0] = 0.282095;
    b[1] = 0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = 0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = 1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = 1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
}

// The clamped cosine's factor per band, for coefficient `k`: π, 2π/3, π/4.
public float sh_cosine_band(uint k)
{
    return k == 0 ? 3.14159265 : k < 4 ? 2.0943951 : 0.785398163;
}

// Irradiance at the normal `n` from coefficients already multiplied by their band's cosine
// factor (rgb in xyz); never negative (ringing below the horizon of a bright sun side).
public float3 sh_irradiance(float4* coefficients, float3 n)
{
    float b[9];
    sh_basis(n, b);
    float3 e = float3(0.0);
    [unroll]
    for (uint k = 0; k < SH_COEFFICIENTS; k++)
        e += coefficients[k].xyz * b[k];
    return max(e, float3(0.0));
}
