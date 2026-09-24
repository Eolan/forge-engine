# Forge — How work flows

GitHub is the system of record: every piece of work is an issue, every change is a pull
request reviewed before it reaches `main`, every Claude Code session handles one issue in its
own worktree. The owner steers through issues, labels and reviews; the sessions are
autonomous inside an issue.

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

## A session's life

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

## Review and merge

- Every PR gets an automated review from a separate session (`/code-review --comment` on
  the PR, or the Claude GitHub App once installed) and the owner's read. Findings are fixed
  in the PR branch; the reviewer re-runs.
- The owner merges (squash) or says "merge" and the process session merges. `main` is
  protected: no direct pushes, PR required, checks green.
- CI (`.github/workflows/ci.yml`) builds, tests, lints and checks formatting on Windows and
  Linux; GPU demos and captures run on the owner's machine, not in CI.

## Context hygiene

- One issue per session keeps context small; long tasks are split into issues, not into
  long sessions. `/compact` when a session gets long; `/export` a transcript to attach to an
  issue if the reasoning matters later.
- What must survive across sessions lives in files: `CLAUDE.md` (rules), `docs/` (facts,
  numbers, decisions), the issue and PR text (why). Claude's auto-memory holds the owner's
  preferences and machine facts, not project state.
