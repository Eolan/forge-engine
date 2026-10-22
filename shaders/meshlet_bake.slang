// The resolve of an impostor's bake (D-075 step 4, crates/forge-render/src/impostor.rs):
// meshlet.slang with FORGE_IMPOSTOR_BAKE, so `sun_light` and `leaf_transmission` answer the
// frame's BAKE_ALBEDO and BAKE_NORMAL flags. Only the bake's renderer compiles its resolve from
// here (`MeshletRenderer::for_bake`); the frame's entries never see the code.
#define FORGE_IMPOSTOR_BAKE 1
#include "meshlet.slang"
