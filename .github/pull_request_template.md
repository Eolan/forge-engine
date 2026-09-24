## What

<!-- One paragraph: what changed and why. Link the issue. -->

Closes #

## How it was verified

- [ ] `cargo build --release`, `cargo test --release`, `cargo clippy --release --all-features --all-targets` (warning-free), `cargo fmt --all -- --check`
- [ ] Demo run with `--validate`: no validation errors
- [ ] Culling A/B harness (if culling/temporal code changed): occlusion, cone and jitter vs brute force = 0 differing pixels; `--show-culled` shows no red
- [ ] Profiler numbers before/after (if performance changed) and `docs/PROFILE.md` updated
- [ ] Docs updated (research / decisions / demo page / README options and keys)

## Numbers

<!-- Paste the relevant overlay lines or imgdiff output. -->

## Notes for the reviewer

<!-- Risky spots, follow-up issues created, anything left out on purpose. -->

🤖 Generated with [Claude Code](https://claude.com/claude-code)
