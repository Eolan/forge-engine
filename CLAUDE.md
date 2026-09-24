# Forge — working rules for every session

Forge is a professional game engine in Rust (Vulkan 1.3+ through `ash`, shaders in Slang),
built system by system: research → decision → implementation → demo with numbers. Read
`docs/ARCHITECTURE.md` first, then `docs/DECISIONS.md`; the roadmap is `docs/ROADMAP.md`,
the research index `docs/RESEARCH.md`, where the time goes `docs/PROFILE.md`. The process
(issues, branches, reviews) is `docs/PROCESS.md`.

## One task per session

- Every session works on exactly one GitHub issue, in its own git worktree and branch
  `task/<issue-number>-<slug>` (start with `claude --worktree task-<n>` or the desktop app's
  worktree option). Never commit to `main` directly; `main` only moves by reviewed PR.
- Read the issue, plan, implement, verify (see below), update the docs that describe what
  changed, then open the PR with the template filled in and the issue linked (`Closes #n`).
- Keep the session's scope to the issue. Anything else you notice becomes a new issue
  (`gh issue create`), not a change in this PR.
- If the issue asks for a decision the owner has not taken, write the proposal as a
  `docs/DECISIONS.md` entry marked 🟡 and stop there; do not build on an untaken decision.

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
  per-frame block), one global bindless set for images/samplers. Every pass ends with
  `commands.mark("group/name")` so it appears in the profiler.
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
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and each PR description with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Squash-merge after review; the PR title becomes the commit on `main`.
