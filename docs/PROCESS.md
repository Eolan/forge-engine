# Forge — How work flows

GitHub is the system of record: every piece of work is an issue; every Claude Code session
handles one issue. The owner steers through issues, labels and reviews; the sessions are
autonomous inside an issue.

**Current mode (2026-09-24): local `main`.** No branches or pull requests yet: sessions
commit on `main` locally and push when a step is working (a demo shows it, verification
passes). Reviews happen on the owner's machine and in the session's report. The
branch-and-PR flow below is the target and switches on when the owner says so.

## Repository settings (issue #16)

The repository is public; only collaborators can write. Settings that live outside git are
kept as files and applied by the owner with `tools/github-settings.sh` (an admin `gh`
login, safe to re-run):

- **Rulesets on `main`** (`.github/rulesets/`). *Keep history*, no bypass: no deletion, no
  force-push, linear history. *Pull request and green CI*: changes arrive by pull request
  with both CI jobs green and review threads resolved; the admin role bypasses it, which
  is how local-mode pushes to `main` keep working.
- **Secret scanning with push protection**: a pushed credential is refused.
- **Dependabot**: alerts and security-fix pull requests for crates; monthly grouped
  updates of the pinned CI actions (`.github/dependabot.yml`). Its pull requests are ours:
  a session merges them after CI when the owner says so.
- **CodeQL** default setup (Rust and the workflows) on pushes, pull requests and weekly.
- **Private vulnerability reporting** (`SECURITY.md`).
- **Workflows from forks** wait for approval unless the author is a collaborator; CI runs
  with a read-only token and actions pinned to commit hashes.

## Issues

- **Types** (labels): `task` (engineering work with a definition of done), `feature`
  (owner-facing capability, may spawn tasks), `bug`, `research` (a research file or a
  decision), `decision` (needs the owner's yes).
- **Systems** (labels): `geometry`, `lighting`, `world`, `physics`, `netcode`, `audio`,
  `animation`, `materials`, `memory`, `task-system`, `tools`, `demo`, `docs`, `process`.
- **Milestones** are the roadmap phases (`Phase 1 — Render core`, …); the showcase demo
  work is tagged `demo` and lives in the phase it belongs to.
- An issue is ready when it states the goal, the definition of done (what must be true,
  which numbers, which docs) and the decision(s) it depends on. Sessions refuse issues that
  build on an untaken decision and ask for it instead.

## A session's life (target flow; in local mode steps 1 and 5–6 collapse to "commit on
## `main`, push when working, report")

1. Pick the issue (assigned or the top ready one of the milestone). Start a session in a
   worktree: `claude --worktree task-<n>` (or the desktop app's worktree option). The
   branch is `task/<n>-<slug>`.
2. Read `CLAUDE.md`, the issue, and the docs it points to. Plan in the issue if the plan is
   not obvious (a comment), then build.
3. Verify in the tier the change needs (below): `tools/verify.sh` picks it and runs it. Add
   the profiler numbers for anything touching performance.
4. Update the docs the change affects (research, decisions, demo pages, `PROFILE.md`,
   README options and keys).
5. Open the PR with the template, `Closes #<n>`, numbers and captures where relevant.
6. Answer review comments in the same session (`claude --resume task-<n>`), then stop. The
   session does not merge.

## Checking a change in tiers (issue #134, D-043)

A change is checked in the tier its paths need, not with the whole batch every time. The
owner approved this on 2026-10-02. `tools/verify.sh` reads the paths changed since the last
accepted commit (committed, staged, unstaged and untracked), picks the tier, says why, and
runs it:

| Tier | When | What runs |
|---|---|---|
| gate | Every changed path maps to no capture: `docs/`, `reports/`, Markdown, `tools/credits/` | fmt, clippy `-D warnings`, the tests, the credits check |
| 0 | Every changed path is mapped in `tools/impact.toml` | The gate, the sentinels and the sets the paths select, on the mesh path. `--recook` when `forge-procgen` or `forge-geom` changed. With GPU code or shaders (`forge-render`, `shaders/`), also the fallback and the validation of those sets |
| 1 | A path is not mapped: `forge-gpu`, `forge-app`, the render graph, a shared pass or shader, `Cargo.lock`, the toolchain, the scripts. Or no accepted set yet | The full batch on both paths, validation, and the timings with `--timings BASE_BIN` |
| 2 | A milestone: every ~5 commits, before a showcase, when a system closes (`--tier 2`) | Tier 1, the batch again with `FORGE_ASYNC=0` (it must match), `tools/origins.sh`, and the real-time tour for the owner to watch |

- **The sentinels** are ten captures on the mesh path, about a minute: meshlets (`static60`,
  `orbit120`, `noocc120`), the ballad without TAA (`ast-notaa600`, `ast240`, `ast240-noocc`)
  and its HDR output (`ast-hdr600`), the resident city (`city60`, `city60-noocc`) and the
  gallery (`gallery60`). They catch a change that reaches further than its paths say.
- **The sets** (`FORGE_SETS` of `captures.sh`, `validate.sh` and `timings.sh`): `meshlets`,
  `ballad`, `city`, `island` (the city's island and the island demo's shots), `sentinels`,
  `all`. `FORGE_PATHS` picks `mesh`, `fb` or both. The images keep their names.
- **`tools/impact.toml`** maps path patterns to sets. The first pattern that matches a path
  wins. A path no pattern matches sends the change to Tier 1: an unknown path counts as
  shared until someone maps it. Its `[recook]` and `[gpu]` tables add `--recook`, and the
  fallback with validation.
- **`--recook`:** the props, the island's tiles and its heightfield are cached in
  `mesh-cache/` by their parameters' text, not by the code that makes them. After a change to
  that code, `FORGE_RECOOK=1` removes the heightfield and passes `--recook` to the first run
  of each scene that cooks (the city, the gallery, the island's 2 m and 8 m grounds).
- **Accepted sets** replace the "before" batch. A passing run becomes HEAD's accepted set,
  `captures/accepted/<sha>/`: the base set's images with the run's on top, and
  `manifest.txt` (the commit, the tier, the build, the driver, each image's SHA-256 and the
  commit it was captured at). The next run compares with the newest accepted set of HEAD's
  history. `FORGE_ACCEPTED` points elsewhere, for example to share sets between trees.
  - A set is accepted only when the working tree adds nothing but gate-only paths to HEAD.
    Otherwise commit, then run `tools/verify.sh --accept RUN`: it checks that the commit holds
    what the run built.
  - The first set of a tree comes from a Tier 1 run. A baseline captured by hand is accepted
    with `tools/verify.sh --accept DIR COMMIT`.
- **Images a change is meant to alter:** `--expect 'mesh-island* mesh-shot-*'`. Their lines
  read "expected" and do not fail. The report names them and gives their ꟻLIP numbers. The
  pairs within the run (the A/B harness, mesh against fallback) must still read `0 px`.
- **The flake (#71)** is recognised by its signature and prints `FLAKE #71` (below). The
  accepted set keeps the base's image.
- **Options:** `--tier gate|0|1|2` forces a tier. `--base COMMIT` checks the change since
  COMMIT, and `--committed` leaves the working tree out (to check a commit again).
  `--no-accept` keeps the run without accepting it, and `--dry-run` only prints the tier.
- **One GPU, shared:** before any demo runs, the script takes the lock directory
  `%TEMP%/forge-gpu.lock` (`FORGE_GPU_LOCK`). It retries every 30 s while another job holds
  it, and removes it when done. Run demos by hand under the same lock.
- **Order:** build, fmt, then clippy, the tests and the credits check in the background, beside
  the captures, compare and validation. The timings wait for the tests: never both at once.
- **What it prints:** the base, the changed paths by pattern, the tier and why, each step with
  the seconds so far, and the verdict. It ends with what it left for Tier 2, and says when
  Tier 2 is due (5 commits since the last accepted Tier 2 run). The run and its logs stay in
  `captures/verify/<time>-<sha>/`.

Measured on 2026-10-02 (RTX 5070 Ti):

| Change | Tier | Time |
|---|---|---|
| a docs-only change | gate | 16 s (55 s with clippy's first run in a tree) |
| #133 (`def84e5`) against its parent | 0: sentinels, island, city; recooked | 233 s in all: 23 captures with the island cooked again (218 s), the gate beside them |
| #133 by hand, before this tool | 0 | about 4 min of captures |
| `ec4e626` (#133 to #135 since the last accepted set), `--tier 2 --timings` | 2 | 1 592 s (26.5 min) in all |

The Tier 2 run's steps, which also give Tier 1's (about 11 min, 20 with the timings):
- **The full batch:** 58 images (54 runs on both paths, recooked) in 390 s, then the compare.
  The 34 pairs within the batch read 0 px.
- **The batch with `FORGE_ASYNC=0`:** 333 s. It matched the async batch except for
  `fb-ast-taa600`, which flaked (245 px, recognised).
- **`tools/origins.sh`:** 39 s. **Validation:** 38 runs in 242 s, clean.
- **The gate:** beside the captures, done before them.
- **The timings:** 565 s.

Before #134 a check was quoted at 15–20 minutes of captures, a "before" batch and an "after"
batch. Accepted sets remove the "before" batch.

## The verification batch (`tools/`, issue #74)

Tier 1 runs the whole batch; Tier 0 a part of it. It runs on the owner's machine: the demos
need the RTX 5070 Ti and open on the secondary monitor without taking focus. It writes under
`captures/`, which git ignores. `tools/verify.sh` runs these steps; this is what each does.

1. **Build the whole workspace first:** `cargo build --release`. Rebuilding only the demo you
   changed leaves the other demos stale. A stale binary writes an older frame block, and every
   capture then "differs" for the wrong reason.
2. **The batch:** `tools/captures.sh OUT [BIN]`.
   - The whole batch writes 58 captures: meshlets, the ballad at fixed steps (and its HDR
     output, #94), city-blocks and its island, and the island demo's four golden shots (#96;
     skipped for a baseline without the `island` binary), each on the mesh path and on the
     fallback. The HDR runs write the preview and the PQ codes (`-pq.png`, 16 bits), which
     `compare.sh` compares to the code. `OUT/batch.txt` records the commit, the sets, the
     driver and the binaries' hashes.
   - To capture an older commit, build it in a tree of its own:
     `git worktree add --detach ../forge-base <commit>`, then `cargo build --release` in that
     tree, then `tools/captures.sh captures/base ../forge-base/target/release`.
   - Each tree must be built in place, because the shader, shader-cache and mesh-cache roots
     are compiled into the binaries from `CARGO_MANIFEST_DIR`.
   - Remove the tree afterwards with `git worktree remove ../forge-base`.
3. **Compare:**
   - `tools/compare.sh NEW` compares with the newest accepted set; `tools/compare.sh BASE NEW`
     with a batch of your own.
   - The script prints the pixels that differ per image, then checks the pairs within the new
     batch:
     - the A/B harness: occlusion off and cone culling off against on, and `--show-culled`
       against the plain frame, where red shows as a difference. It runs on the meshlets
       bench, the ballad, the resident city and the island. The island's runs pin
       `--sw-raster on`: the automatic switch follows how many dense triangles the culls let
       through, so occlusion off would move it (#101). They stream the island's pages, as it
       starts: the pages its first view's cut wants are loaded before the first frame, so the
       fixed view reads none after and its frames depend on no read's timing (#121). Its 8 m
       ground streamed that way must equal it resident;
     - the mesh path against the fallback.
   - Every line must read `0 px`, unless the change is meant to alter the image. In that case,
     the report names the images, says why they changed, and gives their ꟻLIP numbers (see
     below).
4. **Validation:** `tools/validate.sh` runs every demo and path with the validation layer,
   synchronization validation included. A clean run prints only its header lines and the
   verdicts of the mip check ("mip check passed") and of the ACES 2.0 check ("tone check
   passed").
5. **Before pushing** (the gate, in every tier): `cargo fmt --all -- --check`,
   `cargo test --release`, `cargo run --release -q -p credits -- --check`, and
   `cargo clippy --release --all-features --all-targets -- -D warnings` exactly as CI runs it.
   A plain clippy run hides a lint that CI then fails on.
6. **Leave a trace a cloud session can read, when one needs it.** With `FORGE_KEEP_LOGS=1`
   every script keeps its runs' full logs under `OUT/logs/` and what it printed in
   `OUT/summary.txt` (`validate.sh` and `timings.sh` under `captures/validate` and
   `captures/timings`), and `compare.sh` its lines in `NEW/compare.txt`; without it (a local
   session) they print as before and keep nothing. `tools/report.sh NAME [DIR...]` gathers
   the kept files, the machine and the build (`env.txt`) and one contact sheet per directory
   into `reports/NAME/` (a few MB; the full-size captures stay out), to commit and push:
   `git add reports/NAME && git commit -m "Report NAME" && git push`. A cloud session reads
   the report from the branch; it cannot see `captures/`. What the scripts' output means:
   `captures.sh` prints a line per capture and says "NO CAPTURE" when a demo did not write
   one; `compare.sh` ends with "every line is 0 px: the pass" when nothing changed, which is
   the expected result of a change that must not move pixels; `validate.sh` prints each run's
   duration and log length, and a clean run shows no message under its header (the runs are
   short: a minute or two in all); `origins.sh` shows the same image at every offset since the
   cells record (issue #93), and showed the drift before it.

**Known flake (#71):** the ballad's TAA frame 600 (`fb-ast-taa600`, `mesh-ast-taa600`) can
differ from the same build by a few hundred pixels, each at most 22 levels off. How often
depends on the GPU's timing: on 2026-09-25 the mesh path's differed in nearly every run
(340–392 px), so a rerun proves nothing. Judge it by its signature instead: a few hundred
single pixels scattered on edges over the whole frame, ꟻLIP mean ≤ 0.0015 and largest
0.06–0.13 in seventeen flakes. A difference that does not look like that is real.
`compare.sh` judges it so (#134): a difference of `*-ast-taa600` against its base with at most
500 pixels, a ꟻLIP mean at most 0.0015 and a largest value at most 0.15 prints `FLAKE #71`
and does not fail. The largest value tells scattered pixels from a shape: a line of one pixel
20 levels off reaches 0.169 and a 3 × 3 block 0.261 (the table below). The HDR output's frame
600 (`*-ast-hdr600`, a sentinel) has TAA on too and flakes the same way: on 2026-10-02 two
runs in six, from both builds, by 238–299 px (ꟻLIP mean ≤ 0.0015, largest ≤ 0.082). Its PQ
codes then differ by 91 000–125 000 pixels in the dark (HDR-ꟻLIP mean 0.0034–0.0044, largest
0.16–0.19). So the preview is judged by the same signature, and the PQ codes are a flake only
when their preview flaked too, with an HDR-ꟻLIP mean at most 0.005 and largest below 0.2 (a
line of 20 codes reaches 0.24).

**The perceptual check (issue #75).** For each differing pair, `imgdiff` prints LDR-ꟻLIP:
the error a person would see when flipping between the two images, from 0 (none) to 1. It is
computed at 67 pixels per degree by default, a 4K monitor 0.7 m wide seen from 0.7 m
(`--ppd`), with `a` as the reference. It prints the mean (ꟻLIP's usual number), the weighted
quartiles of NVIDIA's tool, p50, p99, p99.9, the largest value and where it is, and how many
pixels reach 0.1, 0.2 and 0.5. `--flip-map map.png` writes the error map: black is no error,
pale yellow the largest. `compare.sh` puts the mean and the largest value on each line that
differs.

Which check applies where:
- **The A/B harness, mesh against fallback, refactors and speed-ups:** 0 px. ꟻLIP is only
  there to help read a failure.
- **Changes that may move pixels invisibly** (an order, TAA history, the flake, a baked
  table): the numbers go in the report. The threshold (D-017, accepted 2026-09-26 with a
  margin) is a mean below 0.02 and every pixel below 0.15: `imgdiff --max-flip 0.15
  --max-flip-mean 0.02` judges by it. A largest value above 0.15 sends the reviewer to the
  error map and the crops; isolated pixels (a silhouette or a shadow edge moved by less than
  a pixel, as in #93) pass, a speck, a line or a patch fails. The mean alone does not tell visible from invisible:
  the ACES 2.0 table's 1-level differences over a whole frame reach 0.012, GTAO's 0.011.
- **Look changes:** the mean, p99, largest value and error map go in the report, and the
  owner judges.

Measured on 2026-09-25 at 1600 × 900. The port matches NVIDIA's tool (v1.7) to six decimals
on every statistic, and its error maps are identical (0 px), at 30, 67 and 120 ppd:

| Pair | Pixels that differ | ꟻLIP mean | Largest | Pixels ≥ 0.1 |
|---|---|---|---|---|
| city, the new instance order (#38) | 399 | 0.0002 | 0.053 | 0 |
| city orbit, the same | 238 | 0.0006 | 0.045 | 0 |
| #71's flake, seventeen flakes (fallback and mesh) | 236–396 | 0.0010–0.0015 | 0.061–0.129 | 0–5 |
| ACES 2.0, the table against the per-pixel transform, six views (#76) | 0 above 2 levels | 0.003–0.012 | 0.034–0.046 | 0 |
| ballad, GTAO on the fill off against on (#55) | 34 663 | 0.011 | 0.824 | 19 025 |
| city, GTAO off against on (#48) | 171 102 | 0.026 | 0.496 | 37 730 |
| ballad golden image, AgX against ACES | 1 396 860 | 0.374 | 0.561 | 1 340 244 |
| on grey 128: one pixel 20 levels brighter | 1 | — | 0.079 | 0 |
| a line of one pixel, 20 levels brighter | 128 | — | 0.169 | 380 |
| a 3 × 3 block, 20 levels brighter | 9 | — | 0.261 | 21 |
| one white pixel | 1 | — | 0.379 | 13 |

It costs about 0.2 s per differing 1600 × 900 pair. Identical pairs skip it.

**HDR captures (issue #126).** Two 16-bit images are the HDR output's PQ codes (`-pq.png`,
#94). `imgdiff` counts their pixels at 16 bits (`compare.sh` passes `--tolerance 0`, so one
10-bit code counts) and prints the largest error in codes. Their ꟻLIP is HDR-ꟻLIP
(Andersson, Nilsson, Shirley and Akenine-Möller, Eurographics 2021):
- both images are decoded to light: Rec.2020 to Rec.709, 1.0 = 100 nits, colours outside
  Rec.709 clipped as an sRGB display would;
- both are tone-mapped (ACES) at one exposure per stop, from the one that puts the
  reference's brightest pixel at 0.85 to the one that puts its median there;
- LDR-ꟻLIP runs at each exposure, and each pixel keeps its largest error.

`--exr a.exr b.exr` writes that light, for NVIDIA's tool or an HDR image viewer. The port
matches NVIDIA's tool (v1.7) to six decimals on 28 pairs at 30, 67 and 120 ppd: every
statistic, the exposures included, and error maps identical to the pixel. It costs about
0.8 s per differing 1600 × 900 pair (nine exposures for the ballad).

HDR-ꟻLIP looks at the image over a range of exposures, the ballad's up to 7 stops above its
display, so it sees differences in the dark that the display hides. On a whole-frame change
its numbers run about five times LDR-ꟻLIP's on the SDR frame; on a speck they are about the
same. Measured on the ballad (`asteroids --tonemap aces2 --hdr offscreen`, 1600 × 900,
2026-10-02):

| Pair | Pixels that differ | Codes | HDR-ꟻLIP mean | Largest | Pixels ≥ 0.1 | In SDR (LDR-ꟻLIP mean, largest) |
|---|---|---|---|---|---|---|
| ACES 2.0's HDR table against its per-pixel transform, frames 240 and 600 | 719 762–755 552 | 2 | 0.021–0.025 | 0.090–0.102 | 0–2 | 0.0044, 0.034 (frame 600) |
| one code brighter over the whole frame | 1 440 000 | 1 | 0.037 | 0.070 | 0 | — |
| two codes, three codes | 1 440 000 | 2, 3 | 0.062, 0.083 | 0.115, 0.148 | 9 108, 246 153 | — |
| the field 100 km and 1 000 km from the origin (#93), frame 240 | 26 546–27 158 | 292 | 0.0019 | 0.577 | 510 | 0.0005, 0.266 |
| the same at frame 600, TAA on | 179 876 | 81 | 0.0061 | 0.523 | 127 | 0.0019, 0.072 |
| one pixel 20 codes brighter, then 60 | 1 | 20, 60 | — | 0.083, 0.192 | 0, 7 | — |
| a line of 128 pixels, 20 codes brighter | 128 | 20 | — | 0.240 | 390 | — |
| a 3 × 3 block, 20 codes brighter | 9 | 20 | — | 0.311 | 21 | — |
| GTAO off against on, frames 240 and 600 | 320 677–357 581 | 54–120 | 0.013–0.018 | 0.59–0.91 | 20 932–51 139 | 0.0034, 0.135 (frame 600) |
| a quarter stop brighter (`--exposure-compensation 0.25`) | 1 439 480 | 70 | 0.277 | 0.410 | 1 423 602 | 0.119, 0.201 |
| AgX against ACES 2.0 | 1 439 973 | 176 | 0.831 | 0.984 | 1 436 998 | — |

The largest values far from the origin are single pixels in the dark space, a dust mote that
moved, invisible on the display and glaring seven stops up. The synthetic pairs edit frame
600's PQ codes at its centre. Four more runs of frame 600 matched it to the code (#71's flake
did not show). The error maps of the GTAO pair, SDR above and HDR below, are in
`reports/2026-10-02-126/`.

What HDR differences are acceptable (D-017's second amendment, accepted 2026-10-02): the same classes
as LDR-ꟻLIP. For class 2 the largest value stays at 0.15, with the same rule for isolated
pixels: the 20-code line (0.24) and 3 × 3 block (0.31) fail, one pixel passes. The mean
becomes 0.05: the HDR table (0.021–0.025) and one code over the whole frame (0.037) pass, two
codes (0.062) fail. `imgdiff --max-flip 0.15 --max-flip-mean 0.05` judges by it.

**Performance:** `tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]` times the usual views. It
covers the city (still, orbit, flight, every page resident), meshlets (still, orbit,
`--side 700`) and the ballad at 900p and 1440p. Each view runs three times per build,
alternating between the two builds. ZONES is a regex that also prints the matching GPU
zones, for example `cull`. `FORGE_SETS` times only some views (`city`, `island`, `meshlets`,
`ballad`; `sentinels` is the city, meshlets and the ballad at 900p). Put the numbers before
and after, or the F1 overlay's, in the report and in `docs/PROFILE.md`. The noise to beat:
the same build against itself (2026-10-02, all views, 565 s) stayed within 0.04 ms per view.
That is within 1 % except the island, at 1.400–1.439 ms (3 %).

**Far from the origin (issue #93):** `tools/origins.sh OUT [BIN] [ORIGINS]` captures the
city's south view and the ballad's frame 240 with the scene moved 10⁴, 10⁵, 10⁶ and 10⁷ m from
the world's origin along every axis (`--origin`, the camera and everything anchored to the
scene going with it), and compares each with the same view at the origin: pixels, ꟻLIP, the
difference and the error map beside each capture. A renderer without a precision limit would
give 0 px at every offset; the world-space `f32` instance table before #93 did not (the numbers
in `docs/research/large-worlds.md` §1). With the cells record (2026-09-26) an offset of whole
cells gives 0 px. The far offsets keep a residue that does not grow with the distance: the
offset's rounding inside a cell, turned into pixels by the traced shadows' edges and TAA. On
2026-10-02 (Tier 2 at `ec4e626`) it was 2 307–2 428 px in the city (ꟻLIP mean ≤ 0.0016, largest
0.093) and 1 518–1 519 px in the ballad (≤ 0.0005, 0.29), as in #98 (2 486–2 730 and 1 519).
A change of these numbers is what to look at. Run it for a change that touches how positions
reach the GPU, and in Tier 2, and put its lines in the report.

**Debugging aids:**
- `FORGE_TRACE_FRAMES=<file>` with `FORGE_HASH_IMAGES=1` writes per-frame hashes of the
  targets. The variable takes a path: `=1` writes a file named `1`.
- `FORGE_WAIT_IDLE=1`.
- Vulkan debug printf: `VK_LAYER_PRINTF_ENABLE=1 VK_LAYER_PRINTF_TO_STDOUT=1` with
  `--validate`.
- `FORGE_GRAPH_LOG=1` prints the render graph's plan: its batches, the queue of each and the
  timeline values it waits for, then each pass and its barriers.
- `FORGE_ASYNC=0` keeps everything on the graphics queue (no async compute, no transfer
  queue, `EXCLUSIVE` sharing): the serial reference to compare an async frame with (issue
  #77). Captures must match in both modes.
- `FORGE_FRAME_BARRIER=1` puts a full barrier at the start of each queue's first batch,
  which serialises frames on the GPU.
- `FORGE_SHADER_STATS=<text>` logs what the driver compiled for each compute pipeline whose
  entry point holds the text (`1` for all): on NVIDIA the registers, the local and shared
  memory and the binary's size (`VK_KHR_pipeline_executable_properties`, #111; the local
  memory's low 32 bits are its bytes). A pass's registers set how many of its warps an SM
  holds. NVIDIA's driver may also spill registers to shared memory, which shows as shared
  memory the shader never declared; it did not for a shader using groupshared memory or wave
  operations.

## Sessions handed over: a fresh local session or a cloud session

- **Where to start:** `CLAUDE.md` holds the rules and the owner's standing preferences.
  `docs/ROADMAP.md` §"Checkpoint 2" holds the order of work, and the issues hold the tasks.
  Claude's auto-memory stays on the owner's machine, so anything a session must know lives in
  these files.
- **Local session:** it works as above, on `main`, one issue at a time.
- **Cloud session:** it has no GPU. It can build, test, lint and format exactly as CI does,
  write docs and research, and do CPU-side work with unit tests. It cannot run a demo, a
  capture or validation. So:
  - Changes that need no GPU (docs, research, tools, CPU code covered by tests) follow the
    local rules and may be pushed to `main` once CI's checks pass.
  - Changes to rendering or shaders go to a branch, `cloud/<issue>-<slug>` (or the branch the
    cloud harness assigns, `claude/<name>`), pushed without a pull request, and the issue gets
    a comment saying what is left to verify. The owner, or a local session, runs the
    verification batch, commits its report to the branch (`tools/report.sh`, step 6 above) so
    the cloud session can read the outcome, and brings the branch into `main`.
  - Research runs as one agent at a time, as locally.

## Review and merge

- Every PR gets an automated review from a separate session (`/code-review --comment` on
  the PR) and the owner's read. Findings are fixed in the PR branch; the reviewer re-runs.
  The Claude GitHub App is not installed (it needs an API-billed key, issue #14).
- The owner merges (squash) or says "merge" and the process session merges. Once the flow
  is on, sessions stop pushing to `main` even though the admin bypass would let them.
- CI (`.github/workflows/ci.yml`) builds, tests, lints and checks formatting on Windows and
  Linux; GPU demos and captures run on the owner's machine, not in CI.

## Context hygiene

- One issue per session keeps context small; long tasks are split into issues, not into
  long sessions. `/compact` when a session gets long; `/export` a transcript to attach to an
  issue if the reasoning matters later.
- What must survive across sessions lives in files: `CLAUDE.md` (rules), `docs/` (facts,
  numbers, decisions), the issue and PR text (why). Claude's auto-memory holds the owner's
  preferences and machine facts, not project state.
