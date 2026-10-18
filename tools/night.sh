#!/usr/bin/env bash
# The night check (D-071): the full batch on what was pushed that day. By day a change to a
# shared path gets the sentinels only (tools/verify.sh); this runs Tier 1 on origin/main, or
# Tier 2 when a milestone is due, in a worktree of its own so nobody's work in progress is built.
#
#   tools/night.sh [--dry-run]
#
# --dry-run: make or update the worktree, say which tier would run, and stop.
#
# A Windows scheduled task starts it at 00:30 (docs/PROCESS.md, "The night check"). It writes
# captures/night/<time>/ (the verify log and summary.txt) and copies the summary to
# captures/night/latest.txt in this checkout: the first thing a morning session reads. A failed
# night is fixed before new work: find the commit with tools/verify.sh --base, then rerun
# tools/verify.sh --tier 1 (with --expect for images a change meant to move).
#
# The worktree (FORGE_NIGHT_TREE, default ../forge-night) is made on the first night, with
# junctions to this checkout's ignored SDK and asset folders. Its accepted sets are this
# checkout's (FORGE_ACCEPTED). It holds the build lock all night long; verify.sh takes the GPU
# lock for each run itself.
set -uo pipefail
dry_run=0
case ${1:-} in
  "") ;;
  --dry-run) dry_run=1 ;;
  *) echo "usage: tools/night.sh [--dry-run]" >&2; exit 2 ;;
esac
main=$(cd "$(dirname "$0")/.." && pwd)
tree=${FORGE_NIGHT_TREE:-$(dirname "$main")/forge-night}
accepted=${FORGE_ACCEPTED:-$main/captures/accepted}
stamp=$(date +%Y%m%d-%H%M%S)
out=$main/captures/night/$stamp
mkdir -p "$out"
exec > "$out/night.log" 2>&1
summary=$out/summary.txt
start=$SECONDS

# say LINE...: a line of the summary, also in the log.
say() {
  echo "$*" | tee -a "$summary"
}
finish() {
  say "time: $(((SECONDS - start) / 60)) min"
  cp "$summary" "$main/captures/night/latest.txt"
  exit "$1"
}
say "night check, $(date '+%F %H:%M'), log: $out/night.log"

if [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  build_lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-build.lock
else
  build_lock=${TMPDIR:-/tmp}/forge-build.lock
fi
deadline=$((SECONDS + 6 * 3600))
until mkdir "$build_lock" 2> /dev/null; do
  if [ $SECONDS -ge $deadline ]; then
    say "verdict: not run, the build lock stayed taken six hours ($(cat "$build_lock/owner" 2> /dev/null))"
    finish 1
  fi
  sleep 30
done
trap 'rm -rf "$build_lock"' EXIT
echo "night check (tools/night.sh), since $(date -u +%FT%TZ)" > "$build_lock/owner"

# The worktree at origin/main, its ignored folders joined to this checkout's.
git -C "$main" fetch -q origin || { say "verdict: not run, git fetch failed"; finish 1; }
if [ ! -d "$tree/.git" ] && [ ! -f "$tree/.git" ]; then
  git -C "$main" worktree add -q --detach "$tree" origin/main ||
    { say "verdict: not run, no worktree at $tree"; finish 1; }
fi
git -C "$tree" checkout -q --detach origin/main && git -C "$tree" reset -q --hard origin/main &&
  git -C "$tree" clean -fdq || { say "verdict: not run, $tree did not check out origin/main"; finish 1; }
for dir in streamline-sdk nrd-sdk rtxdi-sdk steam-audio-sdk fidelityfx-sdk tracy \
  assets/external assets/planets assets/sounds; do
  [ -d "$main/$dir" ] || continue
  [ -d "$tree/$dir/." ] && continue
  # Only an ignored folder: git clean above must never walk into an SDK through its junction.
  git -C "$tree" check-ignore -q "$dir/" || { say "note: $dir is not ignored, no junction"; continue; }
  rm -f "$tree/$dir" 2> /dev/null
  cmd //c mklink /J "$(cygpath -w "$tree/$dir")" "$(cygpath -w "$main/$dir")" > /dev/null ||
    say "note: no junction for $dir"
done
head=$(git -C "$tree" rev-parse HEAD)
say "commit: $(git -C "$tree" log -1 --format='%h %s' "$head")"

# Tier 2 when a milestone is due; Tier 1 unless the head already has a full batch.
cd "$tree" || finish 1
dry=$(FORGE_ACCEPTED=$accepted tools/verify.sh --dry-run --tier 1 2>&1)
full=""
manifest=$accepted/$head/manifest.txt
[ -f "$manifest" ] && grep -q "^tier: [12]" "$manifest" && full=$(sed -n 's/^tier: \([12]\).*/\1/p' "$manifest")
if grep -q "^Tier 2 is due" <<< "$dry" && [ "$full" != 2 ]; then
  tier=2
  say "tier: 2 ($(grep "^Tier 2 is due" <<< "$dry" | cut -c15-))"
elif [ -n "$full" ]; then
  say "verdict: nothing to do, $(git rev-parse --short "$head") already has a Tier $full set"
  finish 0
else
  tier=1
  say "tier: 1"
fi
grep "^base: " <<< "$dry" | tee -a "$summary"
if [ $dry_run = 1 ]; then
  say "dry run: tools/verify.sh --tier $tier would run now"
  finish 0
fi

FORGE_ACCEPTED=$accepted CARGO_BUILD_JOBS=8 tools/verify.sh --tier "$tier" > "$out/verify.log" 2>&1
status=$?
if [ $status = 0 ] && grep -q "^PASSED" "$out/verify.log"; then
  say "verdict: passed"
  grep "^accepted: " "$out/verify.log" | tail -1 | tee -a "$summary"
else
  say "verdict: FAILED (exit $status), the run: $(sed -n 's/^== clippy.*(in the background, \(.*\)\/gate.txt).*/\1/p' "$out/verify.log" | head -1)"
  grep -E "differ|NO CAPTURE|FAILED|error\[|panicked|test result: FAILED" "$out/verify.log" |
    grep -v "FLAKE #71" | head -40 | tee -a "$summary"
fi
grep -E "^left for (the owner|Tier 2)" "$out/verify.log" | tee -a "$summary"
finish $status
