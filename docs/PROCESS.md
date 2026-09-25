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
3. Verify: build, tests, clippy, fmt, validation, the A/B harness for anything touching
   culling or temporal code, the profiler numbers for anything touching performance.
4. Update the docs the change affects (research, decisions, demo pages, `PROFILE.md`,
   README options and keys).
5. Open the PR with the template, `Closes #<n>`, numbers and captures where relevant.
6. Answer review comments in the same session (`claude --resume task-<n>`), then stop. The
   session does not merge.

## The verification batch (`tools/`, issue #74)

Every rendering change is checked with the same batch. It runs on the owner's machine: the
demos need the RTX 5070 Ti and open on the secondary monitor without taking focus. It writes
under `captures/`, which git ignores.

1. **Build the whole workspace first:** `cargo build --release`. Rebuilding only the demo you
   changed leaves the other demos stale. A stale binary writes an older frame block, and every
   capture then "differs" for the wrong reason.
2. **Capture the baseline before changing anything:** `tools/captures.sh captures/base`.
   - This writes 26 captures: meshlets, the ballad at fixed steps, and city-blocks, each on the
     mesh path and on the fallback.
   - To capture an older commit, build it in a tree of its own:
     `git worktree add --detach ../forge-base <commit>`, then `cargo build --release` in that
     tree, then `tools/captures.sh captures/base ../forge-base/target/release`.
   - Each tree must be built in place, because the shader, shader-cache and mesh-cache roots
     are compiled into the binaries from `CARGO_MANIFEST_DIR`.
   - Remove the tree afterwards with `git worktree remove ../forge-base`.
3. **After the change, capture again and compare:**
   - `tools/captures.sh captures/new`, then `tools/compare.sh captures/base captures/new`.
   - The script prints the pixels that differ per image, then checks the pairs within the new
     batch:
     - the A/B harness: occlusion off and cone culling off against on, and `--show-culled`
       against the plain frame, where red shows as a difference;
     - the mesh path against the fallback.
   - Every line must read `0 px`, unless the change is meant to alter the image. In that case,
     the report names the images, says why they changed, and gives their ꟻLIP numbers (see
     below).
4. **Validation:** `tools/validate.sh` runs every demo and path with the validation layer,
   synchronization validation included. A clean run prints only its header lines and the
   verdicts of the mip check ("mip check passed") and of the ACES 2.0 check ("tone check
   passed").
5. **Before pushing:** `cargo test --release`, and
   `cargo clippy --release --all-features --all-targets -- -D warnings` exactly as CI runs it.
   A plain clippy run hides a lint that CI then fails on.
6. **Leave a trace a cloud session can read.** Every script keeps its runs' full logs under
   `OUT/logs/` and what it printed in `OUT/summary.txt` (`validate.sh` and `timings.sh` under
   `captures/validate` and `captures/timings`), and `compare.sh` its lines in
   `NEW/compare.txt`. `tools/report.sh NAME [DIR...]` gathers them, the machine and the build
   (`env.txt`) and one contact sheet per directory into `reports/NAME/` (a few MB; the
   full-size captures stay out), to commit and push: `git add reports/NAME && git commit -m
   "Report NAME" && git push`. A cloud session reads the report from the branch; it cannot see
   `captures/`. What the scripts' output means: `captures.sh` prints a line per capture and
   says "NO CAPTURE" when a demo did not write one (its log says why); `compare.sh` ends with
   "every line is 0 px: the pass" when nothing changed, which is the expected result of a
   change that must not move pixels; `validate.sh` prints each run's duration and log length,
   and a clean run shows no message under its header (the runs are short: a minute or two in
   all); `origins.sh` is the one script whose differences are expected (issue #93).

**Known flake (#71):** the ballad's TAA frame 600 (`fb-ast-taa600`, `mesh-ast-taa600`) can
differ from the same build by a few hundred pixels, each at most 22 levels off. How often
depends on the GPU's timing: on 2026-09-25 the mesh path's differed in nearly every run
(340–392 px), so a rerun proves nothing. Judge it by its signature instead: a few hundred
single pixels scattered on edges over the whole frame, ꟻLIP mean ≤ 0.0015 and largest
0.06–0.13 in seventeen flakes. A difference that does not look like that is real.

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
  table): the numbers go in the report. The proposed threshold (D-017, 🟡 until the owner
  accepts it) is every pixel below 0.15 and a mean below 0.02: `imgdiff --max-flip 0.15
  --max-flip-mean 0.02` judges by it. The mean alone does not tell visible from invisible:
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

**Performance:** `tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]` times the usual views. It
covers the city (still, orbit, flight, every page resident), meshlets (still, orbit,
`--side 700`) and the ballad at 900p and 1440p. Each view runs three times per build,
alternating between the two builds. ZONES is a regex that also prints the matching GPU
zones, for example `cull`. Put the numbers before and after, or the F1 overlay's, in the
report and in `docs/PROFILE.md`.

**Far from the origin (issue #93):** `tools/origins.sh OUT [BIN] [ORIGINS]` captures the
city's south view and the ballad's frame 240 with the scene moved 10⁴, 10⁵, 10⁶ and 10⁷ m from
the world's origin along every axis (`--origin`, the camera and everything anchored to the
scene going with it), and compares each with the same view at the origin: pixels, ꟻLIP, the
difference and the error map beside each capture. A renderer without a precision limit would
give 0 px at every offset; today's world-space `f32` instance table does not (the numbers in
`docs/research/large-worlds.md` §1). Run it for a change that touches how positions reach the
GPU, and put its lines in the report. It becomes a 0 px check once D-004's amendment is built.

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
