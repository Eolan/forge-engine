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
| 6 | (below) Add forge-procgen and genesis: the island's terrain on the CPU | Phase 2's terrain, stages 1–4 with PNG previews, `docs/demos/island.md` | no |
| 7 | (below) Add the dynamic-scenes research for moving geometry | `docs/research/dynamic-scenes.md` (#79, #69, #95) | no |

The SHAs of 6 and 7 are in `git log` on the branch; they were committed after this file was
first written.

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

### 7. The dynamic-scenes research (commit 7)

`docs/research/dynamic-scenes.md` for #79 (moving instances), #69 (probes woken by movers) and
#95 (async overlap), with the same network caveat as commit 4. Its recommendation is the plan
for #79; it is summarised on the issue.

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
4. **The erosion at 4 m in seconds** (#97): the basin graph instead of a flood per step, and
   basins in parallel on the job system.
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
- The erosion is single-threaded and floods the whole field each step: 3 s a step at 4097².
