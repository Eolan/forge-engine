// Fixed-point arithmetic for what the CPU, the GPU and a server must agree on to the bit
// (D-016's amendment, D-058's step 2): forge_core::fixed, the same code. Every operation is an
// integer one, exact by SPIR-V's definition; a 128-bit product is made of four 32-bit ones.
module fixed;

public static const int64_t ONE = int64_t(1) << 61; // one in Q61
public static const uint Q61 = 61;
public static const int64_t METRE = int64_t(1) << 32; // one metre in Q32
public static const uint Q32 = 32;
public static const uint CORDIC_STEPS = 56;

// `a × b >> shift`, rounded toward −∞, through 128 bits, wrapped to 64 (0 < shift < 64):
// forge_core::fixed::mul_shr.
public int64_t mul_shr(int64_t a, int64_t b, uint shift)
{
    uint64_t ua = uint64_t(a);
    uint64_t ub = uint64_t(b);
    uint64_t mask = 0xFFFFFFFFull;
    uint64_t a0 = ua & mask;
    uint64_t a1 = ua >> 32;
    uint64_t b0 = ub & mask;
    uint64_t b1 = ub >> 32;
    uint64_t p00 = a0 * b0;
    uint64_t p01 = a0 * b1;
    uint64_t p10 = a1 * b0;
    uint64_t p11 = a1 * b1;
    uint64_t mid = (p00 >> 32) + (p01 & mask) + (p10 & mask);
    uint64_t hi = p11 + (p01 >> 32) + (p10 >> 32) + (mid >> 32);
    uint64_t lo = (mid << 32) | (p00 & mask);
    // The signed product from the unsigned one: less b × 2⁶⁴ when a < 0, a × 2⁶⁴ when b < 0.
    if (a < 0)
    {
        hi -= ub;
    }
    if (b < 0)
    {
        hi -= ua;
    }
    return int64_t((hi << (64 - shift)) | (lo >> shift));
}

public int64_t mul61(int64_t a, int64_t b)
{
    return mul_shr(a, b, Q61);
}

public int64_t mul32(int64_t a, int64_t b)
{
    return mul_shr(a, b, Q32);
}

// The CORDIC's constants (forge_core::fixed::Cordic), from the CPU.
public struct Cordic
{
    public int64_t angles[56];
    public int64_t inverse_gain;
    public int64_t gain;
    public int64_t half_pi;
    public int64_t pi;
};

// (cos θ, sin θ) in Q61 for θ in Q61, |θ| ≤ π/2.
public int64_t2 cos_sin(Cordic* c, int64_t theta)
{
    int64_t x = c.inverse_gain;
    int64_t y = 0;
    int64_t z = theta;
    for (uint k = 0; k < CORDIC_STEPS; k++)
    {
        int64_t dx = y >> k;
        int64_t dy = x >> k;
        if (z >= 0)
        {
            x -= dx;
            y += dy;
            z -= c.angles[k];
        }
        else
        {
            x += dx;
            y -= dy;
            z += c.angles[k];
        }
    }
    return int64_t2(x, y);
}

// atan2(y, x) in Q61 and K √(x² + y²), for |x|, |y| ≤ 1 in Q61.
public int64_t2 atan2_fixed(Cordic* c, int64_t y, int64_t x)
{
    int64_t z = 0;
    if (x < 0)
    {
        int64_t t = x;
        if (y >= 0)
        {
            x = y;
            y = -t;
            z = c.half_pi;
        }
        else
        {
            x = -y;
            y = t;
            z = -c.half_pi;
        }
    }
    for (uint k = 0; k < CORDIC_STEPS; k++)
    {
        int64_t dx = y >> k;
        int64_t dy = x >> k;
        if (y >= 0)
        {
            x += dx;
            y -= dy;
            z += c.angles[k];
        }
        else
        {
            x -= dx;
            y += dy;
            z -= c.angles[k];
        }
    }
    return int64_t2(z, x);
}

// 1 / √r2 in Q61, 1/4 ≤ r2 ≤ 2: Newton's iteration from 1, eight steps.
public int64_t rsqrt61(int64_t r2)
{
    int64_t y = ONE;
    for (uint i = 0; i < 8; i++)
    {
        int64_t three = 3 * ONE - mul61(r2, mul61(y, y));
        y = mul61(y, three) >> 1;
    }
    return y;
}

// Perlin's quintic fade in Q32.
public int64_t fade32(int64_t t)
{
    int64_t inner = mul32(t, 6 * METRE) - 15 * METRE;
    inner = mul32(t, inner) + 10 * METRE;
    return mul32(mul32(mul32(t, t), t), inner);
}

// t clamped to 0..1 and eased, Q32.
public int64_t smooth32(int64_t t)
{
    t = clamp(t, int64_t(0), METRE);
    return mul32(mul32(t, t), 3 * METRE - 2 * t);
}
