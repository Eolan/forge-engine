// Debug overlay: a grid of character cells and bars composited over the swapchain image.
// One full-screen triangle; every pixel finds its cell in a buffer written by the CPU each
// frame and draws a glyph from a font atlas (one row of 95 ASCII cells, coverage in R8) or
// the fill of a bar (crates/forge-app/src/overlay.rs packs the cells; both sides share the
// layout below).
//
// Cell (uint): bits 0-7 glyph (ASCII, 0 = empty, 255 = bar), 8-11 foreground colour,
// 12-15 background colour (0 = transparent), 16-23 bar fill (0..255), 24-27 bar colour.
import bindless;

struct Push
{
    uint* cells;
    uint cols;
    uint rows;
    uint cell_w;
    uint cell_h;
    uint atlas;      // sampled image: 95 glyph cells of cell_w × cell_h, coverage
    uint pad;
};

[[vk::push_constant]]
Push push;

struct VertexOut
{
    float4 position : SV_Position;
};

[shader("vertex")]
VertexOut vert_main(uint id : SV_VertexID)
{
    VertexOut o;
    o.position = float4((id == 1) ? 3.0 : -1.0, (id == 2) ? 3.0 : -1.0, 0.0, 1.0);
    return o;
}

static const float4 PALETTE[16] = {
    float4(0.0, 0.0, 0.0, 0.0),      // 0 none
    float4(0.02, 0.02, 0.03, 0.55),  // 1 panel
    float4(1.0, 1.0, 1.0, 1.0),      // 2 white
    float4(0.80, 0.82, 0.86, 1.0),   // 3 grey
    float4(0.52, 0.54, 0.58, 1.0),   // 4 dim
    float4(1.0, 0.86, 0.30, 1.0),    // 5 yellow
    float4(1.0, 0.55, 0.20, 1.0),    // 6 orange
    float4(1.0, 0.30, 0.25, 1.0),    // 7 red
    float4(0.45, 0.90, 0.45, 1.0),   // 8 green
    float4(0.40, 0.85, 0.95, 1.0),   // 9 cyan
    float4(0.50, 0.65, 1.0, 1.0),    // 10 blue
    float4(0.90, 0.50, 0.90, 1.0),   // 11 magenta
    float4(0.22, 0.24, 0.28, 1.0),   // 12 bar track
    float4(0.0, 0.0, 0.0, 0.85),     // 13 panel, darker
    float4(0.08, 0.09, 0.11, 0.80),  // 14 header band
    float4(0.0, 0.0, 0.0, 0.0),      // 15 unused
};

[shader("fragment")]
float4 frag_main(float4 position : SV_Position) : SV_Target
{
    uint2 p = uint2(position.xy);
    uint cx = p.x / push.cell_w;
    uint cy = p.y / push.cell_h;
    if (cx >= push.cols || cy >= push.rows)
        discard;
    uint cell = push.cells[cy * push.cols + cx];
    uint glyph = cell & 0xFF;
    uint fg = (cell >> 8) & 0xF;
    uint bg = (cell >> 12) & 0xF;
    float4 color = PALETTE[bg];
    uint lx = p.x - cx * push.cell_w;
    uint ly = p.y - cy * push.cell_h;
    if (glyph == 255)
    {
        // A bar cell: the track over the middle half of the cell, filled from the left.
        uint fill = (cell >> 16) & 0xFF;
        uint bar_color = (cell >> 24) & 0xF;
        if (ly >= push.cell_h / 4 && ly < push.cell_h - push.cell_h / 4)
        {
            uint filled = (fill * push.cell_w + 127) / 255;
            color = (lx < filled) ? PALETTE[bar_color] : PALETTE[12];
        }
    }
    else if (glyph >= 33 && glyph < 127)
    {
        float coverage = g_sampled_images_r[NonUniformResourceIndex(push.atlas)].Load(int3(int((glyph - 32) * push.cell_w + lx), int(ly), 0));
        if (coverage > 0.0)
        {
            float4 ink = PALETTE[fg];
            color = float4(lerp(color.rgb, ink.rgb, coverage), max(color.a, coverage));
        }
    }
    if (color.a <= 0.0)
        discard;
    return color;
}
