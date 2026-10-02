// The HDR calibration pages (issue #125, crates/forge-app/src/calibration.rs), drawn over the
// frame in the target's encoding without dither, in Unity's and HGiG's form. The mark is a
// 3 × 3 checker (its corners and centre lit) at `value` nits:
// - 0, peak: a square of a tenth of the screen at the signal's top (10 000 nits) on black, the
//   mark inside: once `value` reaches the brightest the display shows, the two clip alike and
//   the mark disappears (HGiG's MaxTML; a tenth of the screen, so that a panel's full-screen
//   limit does not lower it).
// - 1, black: the mark on black: the darkest value at which it still shows (MinTML).
// - 2, paper white: the left half at the display's peak, the right half black, the mark across
//   both at the UI's white.
import tonemap;

struct CalibrationPush
{
    uint page;
    uint encoding; // OUTPUT_PQ or OUTPUT_SCRGB
    float value;   // the mark's nits
    float peak;    // the display's peak in nits (paper-white page)
    uint width;
    uint height;
};

[[vk::push_constant]]
CalibrationPush calibration_push;

struct Varyings
{
    float4 position : SV_Position;
};

[shader("vertex")]
Varyings vert_main(uint id : SV_VertexID)
{
    Varyings o;
    o.position = float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    return o;
}

// Whether `p` lies on a lit cell of the mark of side `side` centred on `centre`.
bool on_mark(float2 p, float2 centre, float side)
{
    float2 q = (p - centre) / side + 0.5;
    if (any(q < 0.0) || any(q >= 1.0))
        return false;
    int2 cell = int2(q * 3.0);
    return ((cell.x + cell.y) & 1) == 0;
}

[shader("fragment")]
float4 frag_main(Varyings input) : SV_Target
{
    float2 p = input.position.xy;
    float2 size = float2(calibration_push.width, calibration_push.height);
    float2 centre = 0.5 * size;
    float nits = 0.0;
    float side = size.y / 3.0;
    if (calibration_push.page == 0)
    {
        float square = sqrt(0.1 * size.x * size.y);
        if (all(abs(p - centre) < 0.5 * square))
            nits = 10000.0;
        side = 0.5 * square;
    }
    else if (calibration_push.page == 2 && p.x < centre.x)
        nits = calibration_push.peak;
    if (on_mark(p, centre, side))
        nits = calibration_push.value;
    if (calibration_push.encoding == OUTPUT_PQ)
        return float4(pq_encode(float3(nits)), 1.0);
    return float4(float3(nits / 80.0), 1.0);
}
