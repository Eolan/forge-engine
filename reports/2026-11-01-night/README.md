# The night of 2026-11-01: pbrt-v4, then the private district as a game in production

The owner's brief (2026-11-01, autonomous until 10:00, "follow recommendations"): the pbrt-v4
pass with the scenes already downloaded; a forest reference scene found (not downloaded); then,
before any other scene test, the private district of a shipped open-world city game worked "as if
we were working on a proper game engine", held to 120 fps at 2560 × 1440 through DLSS or FSR
(D-094). Everything private (the game's data, the viewer, its captures, the shot list) stays in
`reference/`, out of git; this page names it "the private district".

## What was done

| Step | Result | Where |
|---|---|---|
| pbrt-v4 as the third reference | In clay the three renderers agree within 0.05–0.07 (mean \|log2\| of 16 × 9 tiles) in all four rooms; Forge's path within 0.05–0.08 of pbrt-v4. Textured, pbrt-v4 reads 4–19 % darker: its coated diffuse and conductors against the converter's metal-roughness, not the light | D-083, `tools/pbrt-render.sh`, `tools/pbrt-variant.py`, `PBRT_OUT` in `tools/gi-compare.sh` |
| A forest reference, found | pbrt-v4's **Landscape** first (Laubwerk, CC BY 4.0, 23,241 plants, 3.1 billion instanced triangles, light through leaves under a sky); then Intel's Sponza Trees and Ivy, then Jungle Ruins. **Nothing downloaded**: waits for the owner | `research/foliage-reference-scenes.md` |
| The frame's target | 120 fps (8.33 ms) at 2560 × 1440 through DLSS or FSR, DLAA optimised too, 60 preferred and never under 30 for detailed cinematics; measured on fixed paths at speed (mean, p99, worst, hitches) | D-094 |
| The district's first viewer | ±256 m round one street: 262,017 instances of 6,959 meshes, 7.1 M unique and 156.5 M instanced triangles; the game's sun path (its exposure curve read, Forge's metering used); 6,640 of its lights as lamps; flat colours from its layered materials; its 549 turning meshes (fans, signs) turning | private; PROFILE.md "An authored city district" |
| Instances scaled per axis | The instance record 80 → 96 bytes (`stretch`): at most +0.04 ms on the public views | #380, PROFILE.md |
| Small meshes' pages shared | The district's pages 12,539 → 2,436, VRAM 3.64 → 2.38 GB, same frame | #387, PROFILE.md |
| Thousands of lamps | GI rays' hits sample their lamps past 32 in reach: the night 28.0 → 13.7 ms (ReSTIR DI); the Bistro's night the same light (×1.005), its shimmer unchanged | #386, D-065 |
| A lamp's shadow optional | `Lamp::shadows` for authored data whose lights cast none; default unchanged; the district's raster-mode shadows save 0.55 ms (below) | D-065's amendment, #386 |
| Movers keep their materials | `set_instance_material`: the district's turning fans and signs as movers with their own colours | PROFILE.md |
| Streaming by cell | The district streamed through `CellStreamer` on the game's own sector levels; the edits' worst frames found and fixed (below) | D-096 |
| Layered materials | Decided: layers evaluated in the resolve, four a texel; not built yet | D-095, #384 |
| The demos detached | A plain build is the engine and its tools; demos build on demand; the gate 299 → 137 s | `Cargo.toml` default-members, `tools/verify.sh` |

## The district's frame against D-094 (RTX 5070 Ti, 2560 × 1440)

| View | DLAA | DLSS Quality | Target |
|---|---|---|---|
| Street by day, still | 10.6 ms | 6.8 ms | 8.33 |
| Turning 120°/s, driving 25 m/s, flying 40 m/s (resident, p99) | | 8.0, 9.2, 8.7 | 8.33 |
| Flying 40 m/s, streamed in real time (p50 / p99) | | 5.79 / 7.65 (was 5.77 / 13.9) | 8.33 |
| Street at night, 6,640 lamps, ReSTIR DI | | 13.7–14.0 ms (14.2 with the fans turning) | 8.33 |

By day under DLSS Quality the district holds the target, streamed or resident; DLAA does not yet
(GI is half the frame). The night is the gap: `shadow/lamp restir` 2.9 ms, `gi/trace` 2.2,
`gi/probe rays` 1.8, `shading/reflections` 1.4 (reflections still weigh every lamp in reach).

## The streaming's worst frames (this morning)

The streamed flight's p99 sat at 13.7–14.0 ms against a p50 of 5.8. Ablations: without rays the
gap stayed 8.4 ms (not the top-level structure rebuilt per edit), and the frame thread spends
0.02 ms applying an edit. Per-frame GPU zones on the slow frames named `scene/edit`: 3–19 ms on
the copy engine, which the culls wait for, because an edit copied each changed instance record as
its own region (3,500–36,700 of them when cells came in, about 0.5 µs a region). Copies now fold
into the region before when they continue it at both ends (`forge_gpu::commands::push_copy` and
`write_buffers_staged`): p99 13.9 → 7.65 ms (two runs: 7.65, 7.68), no frame's GPU over 8 ms, the
worker's longest edit 47 → 23 ms.

## Where the night's time goes (this morning)

- **Shadows are not the lever.** In the source's own data only 518 of 6,426 static lights (8 %)
  cast raster shadows (fading out past 10–15 m); its ray-traced modes shadow every light. Forge now
  takes a lamp's shadow as optional (`Lamp::shadows`, D-065's amendment; every lamp shadowed by
  default, its images unchanged). The district's night with only the raster mode's 465 shadowed
  lamps of 6,640: 14.6 → 14.0 ms. The time is in weighing the lamps, not in their rays. Whether
  the district matches the raster look (lights then reach through walls) or the ray-traced one is
  the owner's call; the measures keep every lamp shadowed.
- **The turning fans cost 0.45 ms at night** (14.02 → 14.48): every ray also walks the movers'
  structure. Not its build (refitted, the same frame) nor the fans' hits (seen by no ray, the same
  frame). Rays now skip the movers' structure when no mover is met by them, and the district
  traces its fans at rest in the static structure while drawing them turning: 14.18 ms (0.17 over
  no fans at all, from 0.45).
- **The 112-byte lamp record costs nothing** (the Bistro's night 18.76 → 18.76 ms with the lists).

## Proposals for the owner (not built)

- **Per-object hide distances.** The source's objects carry their own hide distance (20 m for
  a rooftop fan): small props hidden past their own distance. Not read yet; a cheap win for the
  instance cull and the streamer once the frame needs it.
- **The night under DLSS Quality** needs about 6 ms back, from the lamps' weighing more than their
  shadows: the reflections' lamps sampled like the GI's once their speckle is handled (1.4 ms
  weighing every lamp in reach), fewer candidates per pixel in ReSTIR (16 now), and its spatial
  samples (3 → 1 saves rays but raised the Bistro's shimmer in #372).

## For the owner

1. **The shot list** (in the private plan under `reference/`): the game's pictures and frame rates
   at the same street, noon and midnight, DLSS Quality and DLAA, raster, RT Ultra and path tracing.
2. **The forest scene:** the go to download pbrt-v4's Landscape (part of `pbrt-v4-scenes`).
3. **Which of the game's looks the district matches:** its raster mode (a twelfth of the lights
   shadowed, light through walls) or its ray-traced modes (every light shadowed, as now).
4. **What comes next:** the night's lamps (about 6 ms to find), or the layered materials (D-095),
   or the package and texture streaming (D-096, #385).

## Caveats

- The district's frame is measured on one street and three paths; the game's own captures (the
  shot list, private) are not taken yet, so nothing is compared with the game's look.
- Flat colours stand in for the layered materials (D-095 not built), textures are not streamed
  (#385), the game's videos, flicker and particles are not drawn.
- The turning meshes' A/B (+0.13 ms) ran at 1600 × 900; the 35 stretched ones stand still.
- The night task and the day check stay off (the owner, 2026-10-29): changes were checked with the
  gate and targeted A/Bs only.
