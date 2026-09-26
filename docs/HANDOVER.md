# Handover — the cloud session of the night of 2026-09-25 to 26

For the owner and the next local session. The cloud session had no GPU, so nothing below has
been seen on a screen: every change builds, passes the tests, clippy and fmt exactly as CI
runs them, and every changed shader entry compiles with `slangc` 2026.18.3; the captures, the
validation and the timings are yours to run. The branch is `claude/keen-sagan-vk91st` (the
harness's name for what `docs/PROCESS.md` calls the cloud branch). Each step is one commit, in
an order where every commit builds and tests on its own, so you can bring them into `main` one
by one (`git cherry-pick <sha>`), run the batch after each, and drop the one you do not want
without losing the others.

## The commits, in order, and how to test each

| # | Commit | What | GPU needed to verify |
|---|---|---|---|
| 1 | `25d12e3` Move the demos far from the origin to measure f32 positions | `--origin` in the city and the ballad, `forge_render::precision` (the model), `tools/origins.sh` | yes (the measurement) |
| 2 | `998d0f8` Keep the batch's logs and gather them into a report to commit | the scripts write `OUT/logs` and `summary.txt`; `tools/report.sh` | no (dry-run here) |
| 3 | `fab6d74` Store the instances in integer cells and draw relative to the camera | #93 step 2: the 80-byte record, camera-relative matrices, the scene frame | **yes, the whole batch** |
| 4 | `aaeeb63` Add the terrain genesis research for the island demo | `docs/research/terrain-genesis.md` | no |
| 5 | `264a9fb` Add forge-world: frames, sectors, cells, partitions and streaming plans | Phase 2's first crate, 16 tests, D-037 🟡 | no |
| 6 | `d2f0c3c` Add forge-procgen and genesis: the island's terrain on the CPU | Phase 2's terrain, stages 1–4 with PNG previews, `docs/demos/island.md` | no |
| 7 | `484911f` Keep the batch's logs only with FORGE_KEEP_LOGS=1; the island's 4 m numbers | the logs opt-in, as the owner asked | no |
| 8 | `42c4097` Draw the island in the engine: `city-blocks --island SEED` | `PropKind::Heightfield`, the island cooked like the city's ground, stage 6's first layer rule | **yes** |
| 9 | `8d0d54a` Add the dynamic-scenes research for moving geometry | `docs/research/dynamic-scenes.md` (#79, #69, #95) | no |
| 10 | `ebebcfd` Route the erosion through the basin graph, in parallel: the 4 m island in two minutes | #97: `forge_procgen::flow::drain`, the step on `forge-task`; 3.5–4× a step | no (the same field, faster) |
| 11 | `a5b7e2a` Build the stack in parallel and keep the erosion's buffers: the 4 m island in 50 s | #97's second part: `Drainage`, `Erosion`; 9.5× the first per-step time in all | no (the same field, faster) |
| 12 | `c2dbcb4` Print the eroded field's digest in genesis, for D-016 checks across machines | `Field2::digest`; seed 7's values recorded | no (compare the digest) |
| 13 | `7ccfd59` Trace the rivers as polylines with Strahler orders and widths (stage 4) | `forge_procgen::hydrology`, the network's numbers in `genesis` | no |
| 14 | `a8e1f1d` Add the water research for the island's sea, shores, rivers and lakes | `docs/research/water.md` (Phase 2 item 3) | no |
| 15 | (below) Bake the coast distance and the sea's spectrum on the CPU: the water's first fields | `forge_procgen::coast`, `forge_procgen::ocean`, `genesis`'s stage 5 | no |

### 1. `--origin` and the measurement (commit 1)

What: `--origin M` moves the city or the belt M metres from the world's origin along every
axis, with everything anchored to the scene; `forge_render::precision` predicts the drift of the
old record (`cargo test -p forge-render precision -- --nocapture` prints the table);
`tools/origins.sh OUT` captures both demos at 0, 10⁴…10⁷ m against the origin's image.

Test, on the commit before #3 if you want to *see* the problem (build that commit in a
worktree, `docs/PROCESS.md` step 2): `tools/origins.sh captures/origins-before <that build>`.
Expected: the city 2 m from the camera 0.5 px off at 10 km, 9 px at 100 km, 45 px at
1 000 km, 800 px at 10 000 km (`docs/demos/city-blocks.md`, "Far from the origin"). With
commit 3 applied, the same script is the acceptance test of the new record (below).

### 2. Logs and reports (commit 2)

What: with `FORGE_KEEP_LOGS=1`, every script keeps each run's full log under `OUT/logs/` and
what it printed in `OUT/summary.txt`, and `compare.sh` writes `NEW/compare.txt`;
`tools/report.sh NAME [DIR...]` gathers them, the machine and the build, and one contact sheet
per directory into `reports/NAME/` (a few MB), to commit and push so a cloud session can read
the outcome. Without the variable (a local session, as the owner asked) the scripts print as
before and keep nothing.

Test: nothing to test for a local session. When a cloud session needs the results,
`FORGE_KEEP_LOGS=1 tools/captures.sh …` and so on, then `tools/report.sh 2026-09-26-batch &&
git add reports && git commit -m "Report 2026-09-26-batch" && git push`. What "nothing
happened" means is now printed either way: `compare.sh` ends with "every line is 0 px: the
pass" when nothing changed, `validate.sh` prints each run's duration and log length,
`captures.sh` says "NO CAPTURE" when a demo wrote none.

### 3. The cells record (commit 3) — the one to test with care

What (`docs/demos/city-blocks.md`, "Far from the origin", *Built*; D-004's amendment in
`docs/DECISIONS.md`): `Instance` is an `int3 cell` (1 km cells), a `float3 local`, a unit
quaternion, a uniform scale, the bounding sphere's centre from the same cell, the radius: 80
bytes, from 96. The frame block carries the culling camera's cell and offset; every matrix is
camera-relative (`FlyCamera::view_rotation`); a shader gets a position as
`float3(cell − camera_cell) × 1024 + (local − camera_local)`. The TLAS, the probes and the dust
work in a **scene frame** anchored at the scene's origin (`MeshletScene::origin`; `--origin`
sets it), rays starting from `relative + camera_in_scene`. The placement writes cells and a
quaternion; the Morton order and the cells of 64 (#38) take the cells into account; TAA's and
DLSS's reprojection add the camera's step between frames. `sun_light`'s highlight keeps its old
world-space approximation on purpose (`legacy_camera_world`), so the ballad's look does not
move.

Test, in this order:
1. `cargo build --release`, then `tools/captures.sh captures/cells` and `tools/compare.sh
   captures/base captures/cells` against a baseline built from the commit before. **Expected:
   not 0 px.** The vertex transform rounds differently (a quaternion instead of a matrix,
   camera-relative instead of world), so a few hundred pixels move by 1–2 levels, like #38's
   reordering (399 px, ꟻLIP mean 0.0002, largest 0.053). Judge by D-017's thresholds: every
   pair with a ꟻLIP mean under 0.02 and a largest value under 0.15 is the same image. A pair
   far above that, or a frame that is plainly wrong (instances missing, at the wrong place,
   shadows detached), is a bug in the record: see "If commit 3 fails" below.
2. Within `captures/cells`, the A/B harness and mesh against fallback **must be 0 px** (the
   same table feeds both paths).
3. `tools/validate.sh` must be silent (a layout mismatch between `GpuInstance` and `Instance`
   would show as reads out of bounds under `FORGE_GPU_AV=1`, or as garbage instances).
4. `tools/origins.sh captures/origins`. **Expected: 0 px at every offset**, or a few pixels
   from the split's 0.1 mm rounding (the offset inside a cell is an `f32`), with a ꟻLIP mean
   well under 0.001. This is the whole point of #93: the image no longer depends on where the
   scene stands.
5. `tools/timings.sh <build of the commit before> target/release`. Expected: flat, or a
   little faster (the table is 80 MB instead of 96 and the culls read less). Put the numbers
   in `docs/PROFILE.md`.
6. Record the placement's new checksum from the log ("instances placed … checksum=") in
   `docs/demos/city-blocks.md` where the old `ed6454c65dd1e823` is quoted.

Then D-004's amendment is yours to decide: keep the commit (accept), change the cell size or
the frame (D-037's numbers are constants), or drop it (the measurement of commit 1 stays).

**If commit 3 fails**, the likeliest causes, in order: (a) a struct layout: `Instance` (80
bytes; Rust asserts the size), `Frame` (two `float4`s inserted after `planes`, `camera_pos`
renamed `camera_local`), `Placement` (the origin's cell and offset at the end), `TlasPush`
(the origin appended), `CellBounds` (32 bytes) — compare the Rust mirror with the Slang struct
field by field; (b) the quaternion's convention: `quat_matrix` in `meshlet.slang` is tested
against glam in `forge_world::cells` (the shader's formula ported to Rust), so a rotation error
would come from the placement's product (`place`) or from `Mat4::to_scale_rotation_translation`
handing back a mirrored decomposition for some instance; (c) the previous-frame view: pass 1's
occlusion test uses `prev_cull_view × T(camera − previous camera)`; if the city's first frames
show everything culled or everything drawn, look there; (d) the scene frame: shadows or
reflections detached from their objects mean `camera_in_scene` and the TLAS's origin disagree
(`frame_block` and `build_tlas` must use the same `scene.origin`). Reverting the single commit
restores the old record; everything else on the branch stands without it.

### 4. The terrain genesis research (commit 4)

`docs/research/terrain-genesis.md`, 1 130 lines, written by a research agent. **Caveat:** the
container's proxy reached github.com only; eleven sources were read there and the rest were
confirmed through the search engine's record of the primary page. The file's "Verification
notes" grade every entry; the numbers to re-check before they enter a spec are listed. A
session with a full network should re-verify the entries marked as confirmed by search only
(#99).

### 5. `forge-world` (commit 5)

`cargo test -p forge-world`: sixteen tests. The four numbers it fixes are D-037 🟡 in
`docs/DECISIONS.md` (sectors of 2⁴⁰ m, cells of 1 km, the cell id's bit layout, the clipmap of
cells with hysteresis); each is a constant. `forge-render` now depends on it for `CellPos`.
Nothing on the GPU changes with this commit.

### 6. `forge-procgen` and `genesis` (commit 6)

`cargo run --release -p genesis -- --spacing 16 --steps 150 --out captures/island` (20 s),
then look at `captures/island/overview.png` and `hillshade.png`; `--spacing 4` is the 4097²
target. `docs/demos/island.md` has the pictures the cloud session looked at, the numbers and
what is next: the hand-off to the cluster-DAG cook, so the island is drawn by today's renderer.
Nothing on the GPU changes with this commit.

### 7. The logs opt-in (commit 7)

`FORGE_KEEP_LOGS=1` for the batch scripts, as asked; nothing to test.

### 8. The island in the engine (commit 8) — the second one to test with care

What (`docs/demos/island.md`): `cargo run --release -p city-blocks -- --island 7`. A
`PropKind::Heightfield` in `forge-geom` takes any heightfield through the cook path the city's
terrain uses (`heightfield_mesh` is the same grid mesh, `terrain_mesh` now calls it); the
island's field (8 m, 2049², 8.4 M triangles by default; `--island-spacing 4` for 4097²) is
generated once into `mesh-cache/island-<key>.f32` (`forge_procgen::cached_island`), cooked and
cached like a prop, and drawn as the scene's one instance on the ground's layered material with
a slope-and-altitude layer rule (`forge_procgen::slope_layers`); the city's sky, shadows,
probes and TAA as they are. The city itself is untouched: without `--island` nothing changes,
and `terrain_mesh` gives the same vertices as before (the test
`the_terrain_faces_up_and_is_flat_in_the_city` still passes).

Test: `cargo run --release -p city-blocks -- --island 7`. The first start generates the field
(13 s of erosion at 8 m on the cloud's four cores after commits 10–11, less on the 9800X3D;
the log says `island heightfield … from_cache=false`) and cooks it (about the city's 13 s);
the next start loads both. Expected: the island seen
from the sea to the south, ridges and valleys, grass below and rock on the steep ground and
the peaks, its shadows and the sky; the F1 overlay's GPU time in the same range as the city's
south view (one mesh of the city's ground's size, no props). Then `--island-spacing 4`
(50 s of erosion once here, the cook of 33.5 M triangles, a bigger cache file), and a capture
for `docs/demos/island.md`.

If it fails: a crash in the cook is a `forge-geom` matter (the same code the city's terrain
takes, so unlikely); a black or missing island with a clean log means the layer map or the
material rows (compare `build_island` with `build_city`'s `CityMaterials::ground`); a wrong
camera is `--view`. The cook's cache key includes the island's parameters, so a change of
`--island-steps` or `--island-spacing` cooks a new mesh and leaves the old file (`--recook`
removes the current key's file only).

### 9. The dynamic-scenes research (commit 9)

`docs/research/dynamic-scenes.md` for #79 (moving instances), #69 (probes woken by movers) and
#95 (async overlap), with the same network caveat as commit 4. Its recommendation is the plan
for #79; it is summarised on the issue.

### 10. The erosion through the basin graph, in parallel (commit 10)

What (`docs/demos/island.md`, "Where the time goes"): the erosion step no longer floods the
whole field with a heap (Barnes' priority flood, 89 % of a step); `forge_procgen::flow::drain`
computes the D8 receivers on the raw field, labels each pit's basin, finds the lowest pass
between adjacent basins, takes the spanning tree of the passes from the sea and carves each
pit's way out through its pass (Cordonnier, Bovy & Braun 2019). The step runs on `forge-task`:
rows in parallel for the uplift, the receivers and the diffusion, the stack's segments (whole
drainage trees) in parallel for the implicit update. `genesis --threads N` picks the workers;
the result is the same for any `N` (a test runs it with none and with three). The island's
field changes slightly (water leaves a lake by one carved path instead of over the whole
flooded flat), so the cache key is the same but the samples differ: `city-blocks --island`
regenerates nothing by itself, delete `mesh-cache/island-*.f32` and the island's cooked
mesh (`--recook`) to see the new field, or keep the old one; both draw. The reference
routing (`priority_flood` + `route`) stays for the tests and the final lakes.

Test: `cargo run --release -p genesis -- --spacing 16 --steps 150` and read the `stage 3` line:
the per-step time and its breakdown; then `--spacing 4` (128 s on four cores here after this
commit, 50 s after commit 11); `--threads 0` gives the serial time and the same `river`/`lake`
counts. `cargo test -p forge-procgen` (nine tests) covers the invariants: every cell reaches
an outlet, the stack visits receivers first, the segments hold whole trees, the pit of a cone
leaves through the same pass as the flood's, and the parallel and serial runs give the same
bytes.

### 11. The stack in parallel, the buffers kept (commit 11)

What (`docs/demos/island.md`, "Where the time goes", items 2 and 3): `Drainage::build` makes
the donor lists by row bands (even bands together, then odd: a receiver is a neighbour, so the
lists come out in the sequential order), a parallel prefix sum, the trees below each band's
outlets walked on the workers into parts, the parts concatenated with the positions stored
through atomics, the areas per segment; and a `Drainage` / `Erosion` keeps every buffer from
one step to the next (paging in a dozen fresh 67 MB arrays a step cost more than the work on
them). `erosion::step` takes an `&mut Erosion` and the flow is read from it. The field is the
same to the bit (the counts of every run above are unchanged).

Test: as for commit 10; the 4 m run should take about 50 s of erosion here and well under a
minute on the 9800X3D (#97's first "done" box). `cargo test -p forge-procgen` also reuses a
drainage across two fields and compares with fresh buffers. The run's last line prints the
field's digest: seed 7, 150 steps, should give `0189d031eff0fb84` at 16 m and
`9eacfe0f827fa7dd` at 4 m on the 9800X3D as here (D-016); if not, that is a finding of its
own (`docs/demos/island.md`, "Digests").

### 12. The field's digest (commit 12)

`genesis` ends with the eroded field's FNV-1a digest (`Field2::digest`); the values for seed 7
are in commit 11's test above and in `docs/demos/island.md`. Nothing else changes.

### 13. The rivers as polylines (commit 13)

`forge_procgen::hydrology::trace_rivers`: from the flow, the river cells (0.5 km² of catchment)
become polylines head to mouth, the trunk following the largest tributary, the others joining
it (`Mouth::Junction`), with Strahler orders and a width from the catchment
(`hydrology::width`). `genesis` prints the network's numbers on its `stage 4` line
(`docs/demos/island.md`, "The network"). Nothing draws them yet; the water research (commit
14) says how they become ribbons with flow maps. Test: `cargo test -p forge-procgen` (a
V-shaped valley gives one river, a Y gives a trunk, a tributary and an order 2) and the
`stage 4` line of a `genesis` run.

### 14. The water research (commit 14)

`docs/research/water.md`, 1010 lines, for Phase 2's third item: the open ocean (TMA/JONSWAP
spectra, FFT cascades, foam, the clipmap mesh), shores (depth colour, shore waves from the
coast distance, wet sand), rivers and lakes from the network, shading, what genesis bakes for
the water, and the engines' water systems; its "Recommendation for Forge" is the build order
for the island's sea. Same network caveat as the other research files: the proxy reached
github.com only, everything else was checked through the search engine's record and graded per
entry in its "Verification notes" (#99). Two corrections it makes to the owner's prompt: there
is no GDC 2020 "Breaking Down Barriers" water talk (that title is Pettineo's 2019 barriers
tutorial; Ubisoft's water talks are St-Amour 2013, Wroński 2014, Grujic 2018 and a 2024
SIGGRAPH talk), and no public Frostbite ocean talk exists. It also flags that D-009's
"spectrum evaluated identically on CPU and GPU" is not free with an FFT and proposes a 🟡
decision when Phase 3 starts.

### 15. The water's first fields (commit 15)

The CPU side the water research says to build first: `coast_distance` (a signed Euclidean
distance to the coast, metres, exact, rows and columns in parallel) and `ocean` (the
JONSWAP/TMA directional spectrum with Horvath's spreading, Gaussian amplitudes from the seed,
the inverse FFT with `dmath`, into heights, choppy displacements, slopes and the Jacobian).
`genesis` prints a `stage 5` line and writes `coast.png`, `sea-height.png` and
`sea-hillshade.png` (`docs/demos/island.md`, "The water's fields"). The GPU's cascades
(`ocean.slang`, not written) are to be diffed against `Ocean::surface` sample by sample; the
CPU transform of the lowest cascade is one of the three options the research names for D-009.
Test: `cargo test -p forge-procgen` (a disc's coast distance is its radius less the distance
to the centre; the inverse transform of one wave vector is a cosine; a breeze gives metres of
waves, zero mean, the same bytes twice) and the `stage 5` line and pictures of a `genesis` run.

## How to give the cloud session its results

`tools/report.sh NAME` after the batch, then commit and push `reports/NAME/`. The cloud
session cannot see `captures/`; it reads the report from the branch. If a demo crashes, its
log under `reports/NAME/<dir>/logs/` is what it needs; if an image is wrong, the contact sheet
shows it at thumbnail size and a crop (`imgdiff --crop --crops`) committed next to it shows the
detail.

## What is proposed next, in order

1. **Decide D-004's amendment** after the batch on commit 3 (keep, change, drop) and, with it,
   D-037's numbers. D-017's ꟻLIP thresholds (max < 0.15, mean < 0.02) are what step 1 above
   judges by; they are still 🟡.
2. **The island in the engine** (#96, stage 7 of `docs/demos/island.md`): a
   `PropKind::Heightfield` next to `PropKind::Terrain` in `forge-geom`, the island's 4 m field
   cooked into a cluster DAG like the city's ground, `city-blocks --island` first, then an
   `island` demo with the sky, the probes and TAA. A local session can do it in a day; the
   renderer needs nothing new.
3. **#79, moving geometry**, following `docs/research/dynamic-scenes.md`: a movers range of the
   instance table, previous transforms for motion vectors, movers tested in pass 2, the TLAS
   rebuilt per frame or split, probes woken by the movers' spheres (#69).
4. **The erosion at 4 m** (#97, commits 10–11: 3.13 s to 0.33 s a step here): what remains
   sequential is the basin labelling (0.1 s of the 0.27 s drain) and the pass sort; the lake
   rule (a fill mode with a spill rule, or an area limit) is the open part of the issue.
5. **The water**, following `docs/research/water.md`'s recommendation: the CPU side first
   (the spectrum as a pure function of the seed, the coast distance and the water mask baked
   by genesis, the river ribbons from commit 13's polylines), then on the dev PC the FFT
   cascades on the compute queue and the forward surface pass under TAA.
5. **Re-verify the two research files** (#99) from a machine with a full network.

## What needs the owner

- The decision on D-004's amendment (after the batch), D-037, D-017.
- #71: close it, or allow a public minimal reproduction for NVIDIA.
- #67: the GPU selector, so a session can test on the AMD integrated GPU.
- Whether the cloud branch's commits go to `main` one by one (the cherry-picks above) or as one
  merge once the batch is green on the whole branch.
- `sun_light`'s highlight direction (#98): fixing the old approximation (object space taken
  for world space) moves the ballad's highlights; a look change to judge.

## Known limits of what was done

- Nothing was run on a GPU. Commit 3 is the only one that changes pixels; the others are CPU
  code with tests, scripts dry-run with stand-in binaries, and docs.
- Commit 3's captures are not bit-identical to the old record's at the origin (the rounding
  differs); the acceptance is D-017's thresholds and `origins.sh` at 0 px.
- DLSS's reprojection gets the camera's step through TAA's `previous_from_current`; it was not
  exercised (the `dlss` feature builds, Windows only).
- The two research files were verified through github.com and the search engine's records.
- The erosion's basin labelling is sequential (0.1 s of the 0.33 s a step at 4097²); the
  rest is parallel and memory-bound on four cores.
