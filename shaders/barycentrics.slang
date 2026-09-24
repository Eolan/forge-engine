// Perspective-correct barycentrics and their screen-space derivatives, shared by the
// visibility resolve (meshlet.slang) and the mip check (mipcheck.slang).
module barycentrics;

public struct Barycentrics
{
    public float3 lambda; // perspective-correct barycentric coordinates of the pixel centre
    public float3 ddx;    // their change per pixel to the right
    public float3 ddy;    // their change per pixel down
};

// Perspective-correct barycentrics of a screen point from the three clip-space vertices, with
// analytic screen-space derivatives (Schied & Dachsbacher 2015; the form used by Hable 2021):
// b_i / w_i is affine in screen space, so it is reconstructed from its value at vertex 0 and
// its gradient; the sum gives 1 / w at the pixel, and lambda_i = w * b_i / w_i. Mirrored on
// the CPU in `forge_render::visibility` with a unit test.
public Barycentrics barycentrics(float4 c0, float4 c1, float4 c2, float2 ndc, float2 size)
{
    float3 inv_w = 1.0 / float3(c0.w, c1.w, c2.w);
    float2 n0 = c0.xy * inv_w.x;
    float2 n1 = c1.xy * inv_w.y;
    float2 n2 = c2.xy * inv_w.z;
    float inv_det = 1.0 / determinant(float2x2(n2 - n1, n0 - n1));
    float3 dbdx = float3(n1.y - n2.y, n2.y - n0.y, n0.y - n1.y) * inv_det * inv_w;
    float3 dbdy = float3(n2.x - n1.x, n0.x - n2.x, n1.x - n0.x) * inv_det * inv_w;
    float ddx_sum = dbdx.x + dbdx.y + dbdx.z;
    float ddy_sum = dbdy.x + dbdy.y + dbdy.z;
    float2 delta = ndc - n0;
    float interp_inv_w = inv_w.x + delta.x * ddx_sum + delta.y * ddy_sum;
    float interp_w = 1.0 / interp_inv_w;
    Barycentrics b;
    b.lambda.x = interp_w * (inv_w.x + delta.x * dbdx.x + delta.y * dbdy.x);
    b.lambda.y = interp_w * (delta.x * dbdx.y + delta.y * dbdy.y);
    b.lambda.z = interp_w * (delta.x * dbdx.z + delta.y * dbdy.z);
    b.ddx = interp_w * (dbdx - ddx_sum * b.lambda) * (2.0 / size.x);
    b.ddy = interp_w * (dbdy - ddy_sum * b.lambda) * (-2.0 / size.y);
    return b;
}
