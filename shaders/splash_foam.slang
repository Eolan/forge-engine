// The foam the splashes leave on the water (#107's polish), driven by
// crates/forge-render/src/splashes.rs: a field of cells round the camera, each holding the foam
// of the drops that landed in it (`advance_main` in splashes.slang adds it) in whole units, so
// the sum is the same in any order. It fades over seconds. Cells are addressed by world cell
// modulo the field's size, so it scrolls with the camera without a copy; a cell that has just
// come into the window is cleared. The water's shading whitens by it (`splash_foam_at` in
// water.slang).
import bindless;

struct FoamPush
{
    uint* foam;     // cells × cells of units
    int2 origin;    // this frame's window: its first world cell
    int2 previous;  // last frame's
    float keep;     // the share of its foam a cell keeps this frame
    uint cells;     // the window's cells a side
    uint fresh;     // 1: no last frame, every cell cleared
    uint pad;
};

[[vk::push_constant]]
FoamPush push;

int wrap(int v, int n)
{
    int m = v % n;
    return m < 0 ? m + n : m;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void fade_main(uint3 id : SV_DispatchThreadID)
{
    int n = int(push.cells);
    if (int(id.x) >= n || int(id.y) >= n)
        return;
    // The world cell this slot holds in this frame's window.
    int2 slot = int2(id.xy);
    int2 cell = push.origin + int2(wrap(slot.x - push.origin.x, n), wrap(slot.y - push.origin.y, n));
    uint i = id.y * uint(n) + id.x;
    bool held = push.fresh == 0u && all(cell >= push.previous) && all(cell < push.previous + n);
    push.foam[i] = held ? uint(float(push.foam[i]) * push.keep) : 0u;
}
