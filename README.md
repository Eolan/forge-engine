# Forge

A professional-grade game engine for very large procedural worlds, built one system at a time,
each with its own research, demo and tests.

- Language: Rust everywhere (client, server, tools). Vulkan through `ash`, shaders in Slang.
- Docs: [docs/RESEARCH.md](docs/RESEARCH.md) (research index and per-system bibliographies),
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (system map and principles),
  [docs/DECISIONS.md](docs/DECISIONS.md) (numbered decisions), [docs/ROADMAP.md](docs/ROADMAP.md),
  and one page per demo under [docs/demos/](docs/demos/).

## Requirements

- Stable Rust (see `rust-toolchain.toml`; 1.98 or newer).
- Vulkan SDK 1.4.357 or newer: the demos compile Slang shaders with its `slangc` (found via
  `VULKAN_SDK`, `PATH`, or `FORGE_SLANGC=<path to slangc>`).
- A GPU with `VK_EXT_mesh_shader` for the rendering demos (any RTX, RDNA 2+, Arc). The
  Khronos validation layer is used automatically in debug builds and with `--validate`.

## Layout

```
crates/forge-core     deterministic math, seeds, hashes, handles
crates/forge-task     job system (work stealing, counters, scopes, task graphs, blocking pool)
crates/forge-gpu      Vulkan layer: device, memory, swapchain, Slang shaders, bindless set, pipelines, frames
crates/forge-geom     meshlets and the cluster LOD DAG (meshoptimizer), procedural test meshes, shared GPU layouts
crates/forge-render   meshlet renderer (compute culling, mesh shaders or an indirect-count fallback, two-pass HZB occlusion, visibility buffer + compute resolve), TAA, starfield,
                      physical exposure (luminance histogram, EV100), display transform (AgX / ACES / PBR Neutral), blit
crates/forge-app      window, input, frame loop, capture, fly camera, Tracy hooks
crates/forge-world    world frames (phase 2): f64 reference frames in an integer sector grid, positions in cells for the GPU (#93),
                      the flat grid and the cube sphere cut into u64-named cells, cell streaming plans with hysteresis
crates/forge-procgen  procedural generation on the CPU (phase 2): fields, lattice noise, the island's terrain genesis, its rivers, lakes, coast distance and sea spectrum
                      (mask, uplift, priority flood, drainage, stream-power erosion), PNG previews
shaders/              Slang sources (bindless, meshlet, barycentrics, vis64, hzb, starfield, atmosphere, atmosphere_luts, sky, skyview, sh, bloom, gtao, noise, dust, taa, exposure, aces2, tonemap, display, overlay, mipcheck, tonecheck)
demos/task-bench      job-system benchmarks and the frame-pacing demonstration
demos/meshlets        culling test bench: every culling stage switchable and measurable
demos/asteroids       the ballad: a scripted flight through an asteroid field (living showcase)
demos/city-blocks     a million GPU-placed instances on a 4 km terrain, cluster pages streamed, the 300 m/s flight
tools/imgdiff         pixel and perceptual (LDR-FLIP) comparison of captures (golden images)
tools/contact-sheet   lays captures out on one image of thumbnails (optionally cropped and enlarged)
tools/credits         the Rust crates in the build, their licences and authors (docs/credits-crates.md; CI checks it)
tools/genesis         the terrain genesis pipeline over a 16 km island, with timings and PNG previews (docs/demos/island.md)
tools/*.sh            the verification batch (docs/PROCESS.md): captures, their comparison, validation, timings
docs/                 ARCHITECTURE, DECISIONS, ROADMAP, RESEARCH + research/ and demos/
CREDITS.md            the people, libraries, assets and published techniques Forge builds on
```

## Build and test

```
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Running the demos

All rendering demos share these options: `--vsync`, `--validate` (Vulkan validation layer),
`--frames N` (exit after N frames, for headless runs), `--capture out.png --capture-frame N`
(save frame N as a PNG). Right mouse drag looks around, WASD/QE move, Shift is fast, Esc quits.

### `asteroids` — the ballad

```
cargo run --release -p asteroids
```

A 90-second scripted flight through 10 000 asteroids (rock chunks and ice blocks) in 42
procedural shapes (531 M source triangles) over a procedural sky with the sun and an Earth-like planet under a
physical atmosphere, with temporal anti-aliasing. Keys: **F1** profiling overlay (off →
compact → full: GPU time per pass and CPU time per zone, grouped by subject; **1**–**9**
open or fold a group; the same zones go to Tracy with `--features profiling`), **P** pause
the path and fly freely, **T** TAA,
**O** occlusion culling, **C** cone culling, **L** cluster LOD, **K** LOD colours, **[** /
**]** LOD threshold, **X** culling-error view (culled meshlets drawn in red: any red pixel
is a bug), **M** meshlet colours, **R** software rasteriser (auto → on → off), **H** tint
what it drew, **Tab** wireframe, **B** bloom, **J** the sun's ray-traced shadows, **Z** soft shadows, **N** ambient occlusion, **V** the belt's dust, **Y** translucent ice, **G** tone curve (ACES → PBR Neutral → ACES 2.0 →
AgX), **-** / **=** exposure compensation (half an EV per press), **U** TAA or a DLSS mode
(built with `--features dlss`: Windows, the Streamline SDK in `streamline-sdk/`, an RTX GPU).
Options: `--count N` asteroids, `--length M` belt length, `--duration S` seconds per pass,
`--origin M` (the field moved M metres from the world's origin along every axis, the camera
with it: the far-origin check, issue #93), `--sun-dir x,y,z`, `--planet-dir x,y,z`, `--planet-angle DEG`, `--planet-march [STEPS]` (the planet by a march per pixel, 16 segments, instead
of its planet-view table: the reference), `--planet-view-steps N` (the table's segments, 64), `--fixed-step` (path advances
per frame, for deterministic captures), `--no-taa`, `--no-shadows`, `--no-textures` (the
untextured Phase 0 rock), `--no-ao`, `--ao-radius M`, `--soft-shadows`, `--no-dust`, `--dust E`, `--no-translucency`, `--round-rocks`, `--no-occlusion`, `--no-cone`,
`--show-culled`, `--taa-blend F` (1 = jitter without history), `--capture-every N` (a
sequence of PNGs), `--overlay` / `--no-overlay` (the profiling overlay is on by default in
interactive runs and off in scripted ones), `--lod-error PX` (1.0), `--no-lod`,
`--lod-colors`, `--no-group-window`, `--tonemap aces|agx|neutral|aces2|aces2-analytic`, `--ev100 EV` (fixed
exposure instead of automatic), `--exposure-compensation EV`, `--sun-lux LUX` (128 000),
`--exposure-log file.csv` (EV100 per frame), `--look x,y,z` (hold the view direction: stills
of the sky), `--upscaler taa|dlaa|quality|balanced|performance|ultra-performance`,
`--force-fallback` (the device without mesh shaders: the geometry is drawn through
`vkCmdDrawIndexedIndirectCount`, pixel-identical), `--sw-raster auto|on|off` (the software
rasteriser for dense clusters; auto runs it when a frame holds enough of them),
`--sw-raster-area PX` (2: pixels of a cluster's bounding rectangle per triangle below which
it is dense), `--show-raster`, `--instance-occlusion auto|on|off` (instances the previous
frame's depth pyramid hides wait for this frame's; auto while most of a large crowd is hidden),
`--no-instance-cells` (no cells of 64 instances first; they apply from 65 536 instances).
Numbers: [docs/demos/asteroids.md](docs/demos/asteroids.md);
where the time goes: [docs/PROFILE.md](docs/PROFILE.md).

The culling A/B check (expects 0 differing pixels; see `docs/demos/asteroids.md`):

```
cargo run --release -p asteroids -- --fixed-step --no-taa --frames 601 --capture a.png --capture-frame 600
cargo run --release -p asteroids -- --fixed-step --no-taa --no-occlusion --no-cone --frames 601 --capture b.png --capture-frame 600
cargo run --release -p imgdiff -- a.png b.png
```

With Tracy (start `tracy/tracy-profiler.exe`, then):

```
cargo run --release -p asteroids --features profiling
```

### `meshlets` — culling test bench

```
cargo run --release -p meshlets
```

A grid of 1152 asteroids (127 M triangles). Keys: **F1** profiling overlay, **F** freeze
culling and move the camera to see what was culled, **V** frustum, **C** cone, **O**
occlusion, **L** cluster LOD, **K** LOD colours, **[** / **]** LOD threshold, **M** meshlet
colours, **R** software rasteriser (auto → on → off), **H** tint what it drew, **Tab**
wireframe, **G** tone curve. Options: `--side N`, `--detail N`, `--roughness R`,
`--no-occlusion`, `--lod-error PX`, `--no-lod`, `--orbit` (scripted motion), `--overlay`,
`--ev100 EV` (fixed exposure, 15), `--tonemap agx|aces|neutral|aces2|aces2-analytic` (AgX), `--force-fallback`
(the indirect-count path of GPUs without mesh shaders), `--sw-raster auto|on|off`,
`--sw-raster-area PX`, `--show-raster`, `--instance-occlusion auto|on|off`, `--no-instance-cells`, `--mip-check` (the resolve's texture level of detail
against a fragment shader's, logged at exit), `--tone-check` (ACES 2.0's GPU table and
per-pixel transform against the CPU's port, logged at exit).
Numbers and the correctness proof: [docs/demos/meshlets.md](docs/demos/meshlets.md).

### `city-blocks` — the Phase 1 closing demo, in steps

```
cargo run --release -p city-blocks
```

A city on a 4 km terrain: a compute pass places a million instances of twenty procedural
props (buildings with real window recesses, towers, lamp posts, fountains, columns, and
rocks and rubble over the hills around it; 25.8 M triangles of props, 8 M of terrain),
cooked into cluster DAGs on the job system and cached in `mesh-cache/` (13 s the first
time, under a second after), then streamed: 128 KiB cluster pages read from the cache files
as the LOD cut asks for them, through a 512 MiB pool. Every prop is made of textured
materials (brick, plaster, concrete, glass windows, marble, rock), and the ground of layers:
asphalt streets, sidewalks, paved plazas, grass, rocky slopes (issues #20, #41, #42), under a
physical sky with haze by distance (`--sun-elevation DEG`, issue #43), lit by the sun with
ray-traced soft shadows (issues #45, #54) and by the sky's light (issue #47), occluded by GTAO (issue #48) and, at the scale of the streets, by probes updated by ray queries that also bounce the light (DDGI, issue #53), reflected in the glass, coated on the towers (issues #49, #50, #56). Keys: **L** / **K** LOD and its
colours, **M** cluster colours, **O** occlusion, **R** software rasteriser, **H** its pixels,
**[** / **]** LOD threshold, **T** TAA, **B** bloom, **J** shadows, **I** sky light, **N** ambient occlusion, **V** its view, **F** sky reflections, **Y** mirror rays, **Z** soft or hard shadows, **P** probes, **U** their light alone, **Tab** wireframe, **G** tone curve. Options:
`--gallery` (the twenty props side by side), `--focus NAME` (frame one of them),
`--instances N`, `--recook`, `--orbit`, `--fly` (a loop at 300 m/s), `--fixed-step`,
`--stream-pool MIB` (0: every page resident), `--stream-upload MIB`, `--width W --height H`,
`--no-taa`, `--no-shadows`, `--no-sky-light`, `--no-ao`, `--ao-radius M`, `--show-ao`, `--no-probes`, `--show-gi`, `--probe-rays N`, `--probe-cascades N`, `--no-reflections`, `--no-ray-reflections`, `--hard-shadows`, `--no-lod`, `--no-occlusion`, `--lod-error PX`,
`--sw-raster auto|on|off`, `--instance-occlusion auto|on|off`, `--no-instance-cells`, `--show-culled`, `--ev100 EV`, `--day S` (a day in S seconds, automatic exposure),
`--view x,y,z,yaw,pitch` (start the camera there, e.g. in a street), `--origin M` (the city
moved M metres from the world's origin along every axis, the camera with it: the far-origin
check, issue #93), `--island SEED` (the 16 km island of `forge-procgen` instead of the city,
`docs/demos/island.md`; `--island-spacing M` (8), `--island-steps N` (150), `--island-wind W` (a
compass point, orographic rain) with `--island-rain-contrast C`),
`--force-fallback`. At
1440p the flight at 300 m/s runs at 3.38 ms of GPU with everything on, its p99 frame 3.8 ms
(2.6 ms without the probes; 2026-09-25, after #77 and #92). Numbers:
[docs/demos/city-blocks.md](docs/demos/city-blocks.md).

### `task-bench` — job system

```
cargo run --release -p task-bench
```

Prints throughput, latency and the frame-pacing comparison (6 workers vs every hardware
thread with a real-time thread alongside). Options: `--workers N`, `--frames N`, `--pin`,
`--quick`. Numbers: [docs/demos/task-bench.md](docs/demos/task-bench.md).

### `imgdiff` — golden images

```
cargo run --release -p imgdiff -- a.png b.png --out diff.png --tolerance 2
```

Exit code 1 when more than `--max-different` pixels differ. `--report N` prints the first N
differing pixels with both colours; `--crop x,y,w,h --zoom K --crops out.png` writes the two
crops and the diff side by side, enlarged, for looking at a difference.
`--then next_a.png next_b.png` counts the pixels whose change to the next frame differs between
the two sequences: LOD pops against a full-detail reference
([docs/demos/asteroids.md](docs/demos/asteroids.md), issue #65).

When pixels differ it also prints the perceptual error LDR-ꟻLIP (NVIDIA's metric, issue #75):
its mean, weighted quartiles, p50/p99/p99.9 and largest value. `--flip-map map.png` writes
the error map, `--ppd` sets the viewing distance, and `--max-flip X` / `--max-flip-mean Y`
make the exit code judge by ꟻLIP instead of the pixel count. Which check applies where:
[docs/PROCESS.md](docs/PROCESS.md), "The perceptual check".

### The verification batch

```
tools/captures.sh captures/base          # before the change (after cargo build --release)
tools/captures.sh captures/new           # after it
tools/compare.sh captures/base captures/new
tools/validate.sh
tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]
tools/origins.sh captures/origins        # the far-origin check (issue #93)
tools/report.sh 2026-09-26-batch         # the logs and summaries into reports/…, to commit for a cloud session
```

What each script does and how to build a baseline from an older commit:
[docs/PROCESS.md](docs/PROCESS.md) (issue #74).

### Environment variables (all demos)

| Variable | Effect |
|---|---|
| `FORGE_MONITOR` | `secondary` (default: the first non-primary monitor), `primary`, or a monitor index. Scripted runs (`--frames`) never take keyboard focus. |
| `FORGE_OVERLAY` | `off`, `compact` or `full`: the profiling overlay's start mode (default: compact when interactive, off in scripted runs; F1 cycles at run time). |
| `FORGE_OVERLAY_FONT` | TTF/OTF for the overlay (default `assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf`; a built-in pixel font if unreadable). |
| `FORGE_OVERLAY_FONT_PX` | Overlay font size in pixels (default 14). |
| `FORGE_VRAM_BUDGET_MB=N` | cap the device-local memory budget the overlay's memory group measures against (rehearsing a smaller card; the group turns red past 90 %). |
| `FORGE_SYNC_VALIDATION=1` | with `--validate`: the validation layer's synchronization (hazard) checks. Slow. |
| `FORGE_GPU_AV=1` | with `--validate`: GPU-assisted validation (out-of-bounds device-address and descriptor accesses). Slow. |
| `FORGE_WAIT_IDLE=1` | wait for the GPU after every frame (debugging). |
| `FORGE_FRAME_BARRIER=1` | a full memory barrier at the start of each queue's first batch of every frame (debugging). |
| `FORGE_ASYNC=0` | no async compute or transfer queue: every pass on the graphics queue, as before issue #77 (the serial reference). |
| `FORGE_PARANOID_BARRIERS=1` | a full memory barrier before every dispatch, draw and blit (debugging); a comma-separated list of pass-label prefixes (`ao/,dust/`) limits it to those passes. |
| `FORGE_FP_PRECISE=1` | compile the shaders with `-fp-mode precise`: no contraction into FMAs (debugging, issue #71). |
| `FORGE_GRAPH_LOG=1` | log the render graph's compiled plan (batches and their queues and waits, passes, derived barriers, transient placement) whenever it changes. |
| `FORGE_KEEP_LOGS=1` | the batch scripts (`tools/*.sh`) keep every run's log and a summary under their output directory, for `tools/report.sh` and a cloud session; off, they print as before. |
| `FORGE_GRAPH_NO_ALIAS=1` | give every transient image its own memory instead of the aliased heap (debugging). |
| `FORGE_STALL_MS=N` | sleep N ms after every frame (debugging; it was the workaround for the TAA run-to-run difference the render graph resolved, see `docs/demos/asteroids.md`). |
| `FORGE_NO_TITLE=1` | never update the window title (debugging). |
| `FORGE_TRACE_FRAMES=file` | `asteroids`: append every frame's CPU-side inputs and culling counters to `file` (to diff two runs). |
| `FORGE_HASH_IMAGES=1` | `asteroids`, with `FORGE_TRACE_FRAMES`: hash on the GPU the scene colour after each pass that writes it and the frame's other images, into the trace, without waiting (where two runs part, issue #71). |
| `RUST_LOG` | tracing filter (`info` by default). |
