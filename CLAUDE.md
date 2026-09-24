# Forge — working rules for every session

Forge is a professional game engine in Rust (Vulkan 1.3+ through `ash`, shaders in Slang),
built system by system: research → decision → implementation → demo with numbers. Read
`docs/ARCHITECTURE.md` first, then `docs/DECISIONS.md`; the roadmap is `docs/ROADMAP.md`,
the research index `docs/RESEARCH.md`, where the time goes `docs/PROFILE.md`. The process
(issues, branches, reviews) is `docs/PROCESS.md`.

## One task per session, on `main`, locally

- Every session works on exactly one GitHub issue (the backlog on `Eolan/forge-engine`).
  Current mode (owner's choice, 2026-09-24): work **locally on `main`**, no branches, no
  PRs. Commit on `main` as the task progresses; **push only when a step is working** (a demo
  shows it, the verification below passes) and say so in the report. Mention the issue in
  the commit message (`Closes #n` on the commit that finishes it).
- Read the issue, plan, implement, verify (see below), update the docs that describe what
  changed, then report with numbers and captures.
- Keep the session's scope to the issue. Anything else you notice becomes a new issue
  (`gh issue create`), not a change in this task.
- If the issue asks for a decision the owner has not taken, write the proposal as a
  `docs/DECISIONS.md` entry marked 🟡 and stop there; do not build on an untaken decision.
- The branch-and-PR flow with reviews (`docs/PROCESS.md`) is the target once the owner
  turns it on; until then reviews happen on the owner's machine and in the report.

## Verification before a PR (all of it, every time)

```
cargo build --release
cargo test --release
cargo clippy --release --all-features --all-targets      # must be warning-free
cargo fmt --all -- --check
```

Rendering changes additionally: run the demo with `--validate` (no validation errors; the
GOG overlay layer's naming warnings are noise), and the culling A/B harness
(`docs/demos/asteroids.md`): `--fixed-step` captures with `--no-occlusion`, `--no-cone`,
compared with `tools/imgdiff`, must differ in **0 pixels**; `--show-culled` must show no red.
Report the numbers in the PR. Performance changes: paste the F1 overlay numbers (or a
capture) before and after, and update `docs/PROFILE.md`.

## Conventions

- Rust edition 2024, `unsafe` only in `forge-gpu` with a `// SAFETY:` comment, every public
  item documented (`missing_docs` is a warning we keep at zero).
- Coordinates: right-handed, +Y up, −Z forward, 1 unit = 1 metre, reversed-Z infinite
  projection; f64 frames and camera-relative f32 (D-004). Determinism rules in D-016.
- Shaders: one Slang file per pass in `shaders/`, buffers by device address (`T*` in a
  per-frame block), one global bindless set for images/samplers.
- Every pass is a render-graph pass (`graph.pass("group/name")`, D-020) declaring the images
  and buffers it reads and writes; the graph derives the barriers and the profiler zone.
  Nobody outside `forge-gpu` records a barrier; if a pass needs a resource the graph cannot
  express, extend the graph. `FORGE_GRAPH_LOG=1` shows the derived plan.
- Docs are part of the change: a new system gets a research file, a decision entry, a demo
  page with numbers; a changed number gets updated where it is quoted.
- Never copy credentials into the repo or the docs (`server-auth.md`, `.env`,
  `config/server-identity/` are ignored on purpose).

## Environment (this machine)

- Windows 11, RTX 5070 Ti, Vulkan SDK 1.4.357 (`slangc` on PATH), Tracy 0.14 in `tracy/`,
  Streamline SDK in `streamline-sdk/` (both ignored by git).
- Demo windows open on the **secondary** monitor (`FORGE_MONITOR`), scripted runs never take
  focus: the owner works on the main screen while sessions run.
- Never open pages in the shared browser pane (it plays sound on the owner's machine);
  verify links with WebFetch/WebSearch.
- Research agents run one at a time (parallel fan-outs hit the usage limit).

## Commits and PRs

- Small, focused commits with imperative messages; end each commit message with
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and each PR description (when
  PRs are in use) with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Never push a state that does not build or whose tests fail; CI runs on every push.
