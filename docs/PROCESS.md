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
     the report names the images and says why they changed.
4. **Validation:** `tools/validate.sh` runs every demo and path with the validation layer,
   synchronization validation included. A clean run prints only its header lines and the mip
   check's verdict ("mip check passed").
5. **Before pushing:** `cargo test --release`, and
   `cargo clippy --release --all-features --all-targets -- -D warnings` exactly as CI runs it.
   A plain clippy run hides a lint that CI then fails on.

**Known flake (#71):** `fb-ast-taa600`, the fallback's TAA frame 600, can differ from the
same build by a few hundred pixels, each at most 21 levels off. Rerun that capture; a second
difference is real.

**Performance:** `tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]` times the usual views. It
covers the city (still, orbit, flight, every page resident), meshlets (still, orbit,
`--side 700`) and the ballad at 900p and 1440p. Each view runs three times per build,
alternating between the two builds. ZONES is a regex that also prints the matching GPU
zones, for example `cull`. Put the numbers before and after, or the F1 overlay's, in the
report and in `docs/PROFILE.md`.

**Debugging aids:**
- `FORGE_TRACE_FRAMES=<file>` with `FORGE_HASH_IMAGES=1` writes per-frame hashes of the
  targets. The variable takes a path: `=1` writes a file named `1`.
- `FORGE_WAIT_IDLE=1`.
- Vulkan debug printf: `VK_LAYER_PRINTF_ENABLE=1 VK_LAYER_PRINTF_TO_STDOUT=1` with
  `--validate`.
- `FORGE_GRAPH_LOG=1` prints the render graph's plan.

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
  - Changes to rendering or shaders go to a branch, `cloud/<issue>-<slug>`, pushed without a
    pull request, and the issue gets a comment saying what is left to verify. The owner, or a
    local session, runs the verification batch and brings the branch into `main`.
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
