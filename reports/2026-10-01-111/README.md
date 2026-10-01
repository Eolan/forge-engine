# #111: the sand contour's cost in `shading/layered` (2026-10-01)

**The cause is registers, not the contour's arithmetic.** `FORGE_SHADER_STATS=resolve_layered`
(new: the driver's statistics for a pipeline, `docs/PROCESS.md`) shows the layered resolve:

| Variant | registers | shared memory a group |
|---|---|---|
| committed (the contour, three layers) | 127–128 | 0 |
| without the contour block | 96 | 4 864 B (≈ 19 registers a thread spilled there) |
| the contour, its third layer not shaded | 96 | 4 864 B |
| the contour, its third layer a flat colour (with or without a branch) | 128 | 0 |
| the contour without its noise / without its sort / ids packed / scalars after the textures / `[branch]` | 127–128 | 0–256 B |
| a loop over the layers (even without the contour) | 128 | 1 792–2 048 B |

The third layer the contour splits off tips NVIDIA's driver from 96 registers with a spill to
shared memory to 128 without one: an SM then holds 16 of the pass's warps instead of 21, on every
pixel of layered ground.

**The change** (`shaders/meshlet.slang`, `shade_layered`):
- **At most two layers:** the contour still splits the pair into up to three, but only the two
  heaviest are shaded. The lightest fades out, its weight taken off the other two, so the blend
  stays continuous where the two lightest change places.
- **No block where it changes nothing:** a pair with none of the contour's layers (the sea floor,
  the rock) skips the contour's block, which leaves such a pair as it is (exact).

**Cost** (`docs/PROFILE.md`, "The contour's third layer"): `shading/layered` before → now
(without the contour block), two rounds at 2560 × 1440, three at 1600 × 900:

| View | 1440p | 900p |
|---|---|---|
| coast (the first view) | 0.884 → 0.845 (0.847) | 0.337 → 0.323 (0.320) |
| the stream from 16 m | 0.937 → 0.927 (0.923) | 0.356 → 0.349 (0.346) |
| down a river from 3 m | 0.908 → 0.898 (0.899) | 0.348 → 0.341 (0.338) |
| the stone at the largest mouth | 1.199 → 1.141 (1.180) | 0.448 → 0.427 (0.445) |
| a lake from 3 m | 0.841 → 0.832 (0.823) | 0.322 → 0.320 (0.318) |
| the island from 2.5 km | 0.837 → 0.778 (0.766) | 0.330 → 0.309 (0.306) |

Within 0.012 ms of the pass without the contour on every view (the issue asked for 0.02). The
valleys of #118 gain too: up the steep river 0.696 → 0.671 ms, the slot 0.627 → 0.608.

**Images:** the capture batch changes the island's two views only, `island60` 47 px (ꟻLIP mean
0.00001, max 0.11) and `water60` 26 px (0.000004); the city's layered streets, the A/B harness
and mesh against fallback at 0 px; `validate.sh` clean (the ballad's TAA frame 600 is #71's
flake). The contour's own views (frame 60, `--water`):
the coast 26 px (ꟻLIP mean 0.000005, max 0.086), along the beach from 8 m and over the south
beach from 60 m 0 px, the coast east from 70 m 7 px, the stone 4 px, the island 34 px (mean
0.000016), the lake 0 px. `contour-coast-zoom.png` and `contour-island-zoom.png`: the largest
differences, 8× (before, now, the pixels that differ).

**Tried and dropped:** the contour's pixels in a pass of their own (`shading/contour`) over the
tiles the layered pass lists. It kept the layered pass at 96 registers, but only without
groupshared memory or wave operations in it (either took the driver back to 128). From the
stone's view the band covers most of the screen: the second pass took 0.30 ms, and the pixels
it takes over pay twice.
