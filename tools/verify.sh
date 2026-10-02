#!/usr/bin/env bash
# Checks a change in tiers (issue #134; docs/PROCESS.md, "Tiers"). It reads the paths changed
# since the last accepted commit (committed, staged, unstaged and untracked), picks the tier
# from tools/impact.toml and says why, builds the workspace, runs the gate and the captures the
# tier asks for, compares them with the last accepted set, and accepts the new set when it
# passes.
#
#   tools/verify.sh [--tier gate|0|1|2] [--base COMMIT] [--committed] [--expect PATTERNS]
#                   [--timings BASE_BIN] [--no-accept] [--dry-run]
#   tools/verify.sh --accept DIR [COMMIT]
#
# The tiers:
#   gate  every changed path maps to no capture (docs, reports): fmt, clippy, the tests and the
#         credits check only.
#   0     every changed path is mapped: the gate, the sentinels and the sets the paths select, on
#         the mesh path; with GPU code or shaders also the fallback and their validation.
#   1     a path is not mapped (shared rendering, Cargo.lock, the toolchain, ...), or there is no
#         accepted set: the full batch on both paths, validation, and the timings when
#         --timings gives a baseline build.
#   2     a milestone, asked for with --tier 2: Tier 1, plus the batch again with FORGE_ASYNC=0
#         (it must match), tools/origins.sh, and the real-time tour for the owner to watch.
#
# --base COMMIT: the change since COMMIT, compared with COMMIT's accepted set (default: the
# newest commit of HEAD's history with one). --committed: only BASE..HEAD's paths, not the
# working tree's (to check a commit again). --expect PATTERNS: the images the change is meant to
# alter (FORGE_EXPECT of tools/compare.sh, for example 'mesh-island* mesh-shot-*'); name them in
# the report with their FLIP numbers. --timings BASE_BIN: tools/timings.sh against the binaries
# in BASE_BIN, after the tests (never beside them). --no-accept: keep the run, accept nothing.
# --dry-run: say the tier and what would run, then stop.
#
# A passing run becomes HEAD's accepted set, captures/accepted/<sha>/ (FORGE_ACCEPTED): the
# base set's images with this run's on top, and manifest.txt (the commit, the tier, the build,
# the driver, each image's hash and the commit it was captured at). It is accepted only when the
# working tree adds nothing but gate-only paths to HEAD; otherwise commit, then
# `tools/verify.sh --accept RUN`, which checks the commit holds what the run built. `--accept DIR
# COMMIT` also takes a plain tools/captures.sh directory, a baseline captured by hand.
#
# The GPU is shared: every demo run waits for the lock directory FORGE_GPU_LOCK (default
# %TEMP%/forge-gpu.lock), holds it and removes it when done. The runs and their logs stay in
# captures/verify/<time>-<sha>/.
set -uo pipefail
here=$PWD
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
impact=$root/tools/impact.toml
accepted_dir=${FORGE_ACCEPTED:-$root/captures/accepted}
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
usage() {
  sed -n '8,9p' "$0" | sed 's/^# //' >&2
  exit 2
}

forced="" base_arg="" committed=0 expect="" timings_bin="" no_accept=0 dry=0 accept_dir="" accept_commit=""
while [ $# -gt 0 ]; do
  case $1 in
    --tier) forced=${2:?--tier gate, 0, 1 or 2}; shift 2 ;;
    --base) base_arg=${2:?--base COMMIT}; shift 2 ;;
    --committed) committed=1; shift ;;
    --expect) expect=${2:?--expect PATTERNS}; shift 2 ;;
    --timings) timings_bin=${2:?--timings BASE_BIN}; shift 2 ;;
    --no-accept) no_accept=1; shift ;;
    --dry-run) dry=1; shift ;;
    --accept)
      accept_dir=${2:?--accept DIR [COMMIT]}
      shift 2
      if [ $# -gt 0 ] && [[ $1 != --* ]]; then accept_commit=$1; shift; fi
      ;;
    *) usage ;;
  esac
done
case $forced in "" | gate | 0 | 1 | 2) ;; *) usage ;; esac
# Paths given relative to where the script was called from.
[ -n "$timings_bin" ] && [[ $timings_bin != /* ]] && timings_bin=$here/$timings_bin
[ -n "$accept_dir" ] && [[ $accept_dir != /* ]] && accept_dir=$here/$accept_dir

# --- tools/impact.toml ---------------------------------------------------------------------

# glob_regex PATTERN: the extended regex of an impact.toml pattern.
glob_regex() {
  local g=$1 r="" c i
  for ((i = 0; i < ${#g}; i++)); do
    c=${g:i:1}
    if [ "${g:i:3}" = "**/" ]; then
      r+="(.*/)?"
      i=$((i + 2))
    elif [ "${g:i:2}" = "**" ]; then
      r+=".*"
      i=$((i + 1))
    elif [ "$c" = "*" ]; then
      r+="[^/]*"
    elif [ "$c" = "?" ]; then
      r+="[^/]"
    elif [[ $c == [.+\(\)\{\}\^\$\|\\\[\]] ]]; then
      r+="\\$c"
    else
      r+=$c
    fi
  done
  echo "^$r\$"
}

paths_re=() paths_sets=() paths_glob=() recook_re=() gpu_re=()
read_impact() {
  local table="" line pattern value regex set
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%%#*}
    if [[ $line =~ ^\[([a-z]+)\] ]]; then
      table=${BASH_REMATCH[1]}
      continue
    fi
    [[ $line =~ ^\"([^\"]+)\"[[:space:]]*=[[:space:]]*(.*[^[:space:]]) ]] || continue
    pattern=${BASH_REMATCH[1]}
    value=${BASH_REMATCH[2]}
    regex=$(glob_regex "$pattern")
    case $table in
      paths)
        value=${value//[\[\]\",]/ }
        for set in $value; do
          case $set in
            sentinels | meshlets | ballad | city | island) ;;
            *) echo "tools/impact.toml: unknown set $set for $pattern" >&2; exit 2 ;;
          esac
        done
        paths_re+=("$regex")
        paths_sets+=("$(echo $value)")
        paths_glob+=("$pattern")
        ;;
      recook) [ "$value" = true ] && recook_re+=("$regex") ;;
      gpu) [ "$value" = true ] && gpu_re+=("$regex") ;;
      *) echo "tools/impact.toml: unknown table [$table]" >&2; exit 2 ;;
    esac
  done < "$impact"
}

# classify PATH: sets "rule" to the index of the first [paths] pattern that matches, or -1.
classify() {
  local i
  rule=-1
  for i in "${!paths_re[@]}"; do
    if [[ $1 =~ ${paths_re[i]} ]]; then
      rule=$i
      return
    fi
  done
}
# matches PATH REGEX...: true when one REGEX matches PATH.
matches() {
  local path=$1 re
  shift
  for re; do
    [[ $path =~ $re ]] && return 0
  done
  return 1
}
# gate_only PATH...: true when every PATH maps to no capture.
gate_only() {
  local path
  for path; do
    classify "$path"
    [ "$rule" -ge 0 ] && [ -z "${paths_sets[rule]}" ] || return 1
  done
  return 0
}

# --- accepted sets -----------------------------------------------------------------------------

# latest_accepted COMMIT: the newest commit of COMMIT's history with an accepted set, or nothing.
latest_accepted() {
  local sha
  for sha in $(git rev-list -n 500 "$1" 2> /dev/null); do
    [ -f "$accepted_dir/$sha/manifest.txt" ] && echo "$sha" && return 0
  done
  return 0
}
# worktree_tree: the tree of the working tree as it is (tracked and untracked files git does not
# ignore), without touching the index.
worktree_tree() {
  local index
  index=$(mktemp -u)
  GIT_INDEX_FILE=$index git read-tree HEAD &&
    GIT_INDEX_FILE=$index git add -A . 2> /dev/null &&
    GIT_INDEX_FILE=$index git write-tree
  rm -f "$index"
}
# link SRC DST: a hard link when the file system allows one, else a copy.
link() { ln "$1" "$2" 2> /dev/null || cp "$1" "$2"; }

# accept RUN COMMIT BASE_SHA HOW: RUN's images become COMMIT's accepted set, over BASE_SHA's
# (none: nothing under them). The images compare.sh called the flake keep the base's.
accept() {
  local run=$1 commit=$2 base_sha=$3 how=$4
  local dest=$accepted_dir/$commit tmp=$accepted_dir/$commit.part image name origin
  local base_set="" flaked=" " changed=" " short
  short=$(git rev-parse --short=12 "$commit")
  if ! ls "$run"/*.png > /dev/null 2>&1; then
    echo "no images in $run: nothing to accept (a gate-only run keeps the base's set)" >&2
    return 1
  fi
  [ -n "$base_sha" ] && [ "$base_sha" != none ] && base_set=$accepted_dir/$base_sha
  if [ -f "$run/compare.txt" ]; then
    flaked=" $(sed -n 's/^\([^:]*\): .*: FLAKE #71$/\1/p' "$run/compare.txt" | tr '\n' ' ')"
    changed=" $(sed -n 's/^\([^:]*\): .*: expected$/\1/p' "$run/compare.txt" | tr '\n' ' ')"
  fi
  rm -rf "$tmp"
  mkdir -p "$tmp"
  : > "$tmp/images.txt"
  if [ -n "$base_set" ]; then
    for image in "$base_set"/*.png; do
      [ -f "$image" ] || continue
      name=$(basename "$image" .png)
      if [ -f "$run/$name.png" ] && [[ $flaked != *" $name "* ]]; then continue; fi
      link "$image" "$tmp/$name.png"
      origin=$(awk -v n="$name.png" '$2 == n { print $3 }' "$base_set/manifest.txt")
      [ -n "$origin" ] || origin=${base_sha:0:12}
      if [[ $flaked == *" $name "* ]]; then
        echo "$name.png $origin flake #71, the base's kept" >> "$tmp/images.txt"
      else
        echo "$name.png $origin" >> "$tmp/images.txt"
      fi
    done
  fi
  for image in "$run"/*.png; do
    [ -f "$image" ] || continue
    name=$(basename "$image" .png)
    [ -f "$tmp/$name.png" ] && continue
    link "$image" "$tmp/$name.png"
    if [[ $changed == *" $name "* ]]; then
      echo "$name.png $short changed, expected" >> "$tmp/images.txt"
    else
      echo "$name.png $short" >> "$tmp/images.txt"
    fi
  done
  {
    echo "commit: $(git log -1 --format='%H %s' "$commit")"
    echo "accepted: $(date -u +%FT%TZ), $how"
    [ -f "$run/state.txt" ] && grep -E "^(tier|sets|paths|recook|expect):" "$run/state.txt"
    echo "base: ${base_sha:-none}"
    echo "build: $(rustc -V 2> /dev/null), release, $(grep '^commit:' "$run/batch.txt" 2> /dev/null | cut -c9-)"
    grep -E "^(driver|binary):" "$run/batch.txt" 2> /dev/null
    echo "run: $run"
    echo "images (sha-256, name, the commit it was captured at):"
    while read -r name origin rest; do
      echo "$(sha256sum "$tmp/$name" | cut -c1-64)  $name  $origin${rest:+  $rest}"
    done < "$tmp/images.txt"
  } > "$tmp/manifest.txt"
  rm -f "$tmp/images.txt"
  rm -rf "$dest"
  mv "$tmp" "$dest"
  echo "accepted: $dest ($(ls "$dest"/*.png | wc -l) images, $(grep -c "  $short" "$dest/manifest.txt") from this run)"
}

read_impact

# --- --accept DIR [COMMIT] -------------------------------------------------------------------

if [ -n "$accept_dir" ]; then
  [ -d "$accept_dir" ] || { echo "no directory $accept_dir" >&2; exit 1; }
  accept_dir=$(cd "$accept_dir" && pwd)
  commit=$(git rev-parse --verify "${accept_commit:-HEAD}^{commit}") || exit 1
  if [ -f "$accept_dir/state.txt" ]; then
    grep -q "^verdict: pass" "$accept_dir/state.txt" ||
      { echo "$accept_dir did not pass: nothing accepted" >&2; exit 1; }
    tree=$(sed -n 's/^tree: //p' "$accept_dir/state.txt")
    mapfile -t extra < <(git diff --name-only --no-renames "$tree" "$commit")
    if ! gate_only "${extra[@]}"; then
      echo "$commit is not what $accept_dir built: these paths differ and can change images:" >&2
      for path in "${extra[@]}"; do gate_only "$path" || echo "  $path" >&2; done
      exit 1
    fi
    base_sha=$(sed -n 's/^base: //p' "$accept_dir/state.txt")
    accept "$accept_dir" "$commit" "${base_sha:-none}" "verified by tools/verify.sh" || exit 1
  else
    [ -n "$accept_commit" ] || { echo "a plain captures directory needs its COMMIT" >&2; exit 1; }
    base_sha=$(latest_accepted "$commit^")
    accept "$accept_dir" "$commit" "${base_sha:-none}" "by hand from $accept_dir" || exit 1
  fi
  exit 0
fi

# --- the change and its tier -------------------------------------------------------------------

start=$SECONDS
head=$(git rev-parse HEAD)
if [ -n "$base_arg" ]; then
  base=$(git rev-parse --verify "$base_arg^{commit}") || exit 1
else
  base=$(latest_accepted HEAD)
fi
base_set=""
[ -n "$base" ] && [ -f "$accepted_dir/$base/manifest.txt" ] && base_set=$accepted_dir/$base

{
  [ -n "$base" ] && git diff --name-only --no-renames "$base" HEAD
  if [ $committed = 0 ]; then
    git diff --name-only --no-renames HEAD
    git ls-files --others --exclude-standard
  fi
} | sort -u > "${TMPDIR:-/tmp}/verify-paths.$$"
mapfile -t changed < "${TMPDIR:-/tmp}/verify-paths.$$"
rm -f "${TMPDIR:-/tmp}/verify-paths.$$"
# The working tree's own changes, which the build includes whatever --committed says.
mapfile -t dirty < <({ git diff --name-only --no-renames HEAD; git ls-files --others --exclude-standard; } | sort -u)

unmapped=() selected=" " recook=0 gpu=0 by_rule=()
for path in "${changed[@]}"; do
  classify "$path"
  if [ "$rule" -lt 0 ]; then
    unmapped+=("$path")
  else
    by_rule[rule]=$((${by_rule[rule]:-0} + 1))
    for set in ${paths_sets[rule]}; do
      [[ $selected == *" $set "* ]] || selected="$selected$set "
    done
  fi
  matches "$path" "${recook_re[@]}" && recook=1
  matches "$path" "${gpu_re[@]}" && gpu=1
done
selected=$(echo $selected)

if [ -n "$forced" ]; then
  tier=$forced
  why="asked for with --tier"
elif [ -z "$base" ]; then
  tier=1
  why="no accepted set in HEAD's history ($accepted_dir): the full batch makes the first"
elif [ ${#unmapped[@]} -gt 0 ]; then
  tier=1
  why="${#unmapped[@]} changed paths are not in tools/impact.toml (shared): ${unmapped[*]:0:8}"
  [ ${#unmapped[@]} -gt 8 ] && why="$why ..."
elif [ ${#changed[@]} -eq 0 ]; then
  tier=gate
  why="nothing changed since $(git rev-parse --short "$base")"
elif [ -z "$selected" ]; then
  tier=gate
  why="every changed path maps to no capture"
else
  tier=0
  why="every changed path is mapped; they select: $selected"
fi
[ -n "$base" ] && [ -z "$base_set" ] && [ "$tier" != gate ] && [ "$tier" != 1 ] && [ "$tier" != 2 ] && {
  tier=1
  why="$(git rev-parse --short "$base") has no accepted set to compare with"
}

# What runs.
case $tier in
  gate) sets="" paths="" ;;
  0)
    sets="sentinels${selected:+ $selected}"
    paths=mesh
    [ $gpu = 1 ] && paths="mesh fb"
    ;;
  *) sets=all paths="mesh fb" ;;
esac
validate_sets=""
if [ "$tier" = 0 ] && [ $gpu = 1 ]; then
  validate_sets=$selected
elif [ "$tier" = 1 ] || [ "$tier" = 2 ]; then
  validate_sets=all
fi
timings_sets=""
[ -n "$timings_bin" ] && case $tier in 0) timings_sets=$sets ;; 1 | 2) timings_sets=all ;; esac

echo "tools/verify.sh at $(git rev-parse --short HEAD)$([ ${#dirty[@]} -gt 0 ] && echo " with ${#dirty[@]} uncommitted paths"), $(date -u +%FT%TZ)"
if [ -n "$base" ]; then
  echo "base: $(git log -1 --format='%h %s' "$base")${base_set:+ (accepted set)}"
else
  echo "base: none"
fi
echo "changed: ${#changed[@]} paths$([ $committed = 1 ] && echo " (committed only)")"
for i in "${!paths_glob[@]}"; do
  [ -n "${by_rule[i]:-}" ] && echo "  ${by_rule[i]} under ${paths_glob[i]}: ${paths_sets[i]:-gate only}"
done
[ ${#unmapped[@]} -gt 0 ] && echo "  ${#unmapped[@]} unmapped: ${unmapped[*]:0:12}$([ ${#unmapped[@]} -gt 12 ] && echo " ...")"
echo "Tier $tier: $why"
if [ "$tier" != gate ]; then
  echo "captures: $sets, paths $paths$([ $recook = 1 ] && echo ", recooked (forge-procgen or forge-geom changed)")"
  echo "validation: ${validate_sets:-none}; timings: ${timings_sets:-none}"
fi
since=0 milestone=""
for sha in $(git rev-list -n 500 HEAD); do
  [ -f "$accepted_dir/$sha/manifest.txt" ] && grep -q "^tier: 2" "$accepted_dir/$sha/manifest.txt" &&
    milestone=$sha && break
  since=$((since + 1))
done
if [ -z "$milestone" ]; then
  echo "Tier 2 is due: no milestone check (--tier 2) accepted in HEAD's history yet"
elif [ $since -ge 5 ]; then
  echo "Tier 2 is due: $since commits since the last milestone check"
fi
if [ $committed = 1 ] && ! gate_only "${dirty[@]}"; then
  echo "note: the build includes uncommitted changes that can move pixels; the run will not be accepted"
fi
[ $dry = 1 ] && exit 0

# --- the run -------------------------------------------------------------------------------------

run=$root/captures/verify/$(date -u +%Y%m%d-%H%M%S)-$(git rev-parse --short HEAD)
mkdir -p "$run"
tree=$(worktree_tree)
{
  echo "head: $head"
  echo "tree: $tree"
  echo "base: ${base_set:+$base}"
  echo "tier: $tier ($why)"
  echo "sets: $sets"
  echo "paths: $paths"
  echo "recook: $recook"
  echo "expect: $expect"
} > "$run/state.txt"
locked=0 gate_pid=""
unlock() {
  [ $locked = 1 ] && rm -rf "$lock"
  locked=0
}
cleanup() {
  [ -n "$gate_pid" ] && kill "$gate_pid" 2> /dev/null
  unlock
}
trap cleanup EXIT
trap 'exit 130' INT TERM
lock_gpu() {
  [ $locked = 1 ] && return 0
  until mkdir "$lock" 2> /dev/null; do
    echo "the GPU is busy ($(cat "$lock/owner" 2> /dev/null || echo "no owner file")): retrying in 30 s"
    sleep 30
  done
  echo "tools/verify.sh in $root, pid $$, since $(date -u +%FT%TZ)" > "$lock/owner"
  locked=1
}
# step NAME: prints a step's heading with the time so far.
step() { echo "== $1 ($((SECONDS - start)) s)"; }
failed=()

step "build"
cargo build --release > "$run/build.txt" 2>&1
build_status=$?
tail -n 2 "$run/build.txt"
[ $build_status = 0 ] || { echo "the build failed"; echo "verdict: fail" >> "$run/state.txt"; exit 1; }
step "fmt"
cargo fmt --all -- --check > "$run/fmt.txt" 2>&1 || { failed+=(fmt); head -n 20 "$run/fmt.txt"; }
# Clippy, the tests and the credits check in the background, beside the captures (never beside
# the timings).
step "clippy, tests and credits (in the background, $run/gate.txt)"
(
  status=0
  cargo clippy --release --all-features --all-targets -- -D warnings || status=1
  cargo test --release || status=1
  cargo run --release -q -p credits -- --check || status=1
  exit $status
) > "$run/gate.txt" 2>&1 &
gate_pid=$!

if [ "$tier" != gate ]; then
  lock_gpu
  step "captures: $sets, paths $paths"
  FORGE_SETS=$sets FORGE_PATHS=$paths FORGE_RECOOK=$recook tools/captures.sh "$run" ||
    failed+=(captures)
  step "compare with ${base_set:-nothing (no accepted set)}"
  compare_base=${base_set:-$run/no-base}
  mkdir -p "$compare_base"
  FORGE_KEEP_LOGS=0 FORGE_EXPECT=$expect tools/compare.sh "$compare_base" "$run" | tee "$run/compare.txt"
  [ "${PIPESTATUS[0]}" = 0 ] || failed+=(compare)
  if [ "$tier" = 2 ]; then
    step "the batch again with FORGE_ASYNC=0"
    FORGE_ASYNC=0 FORGE_SETS=all FORGE_RECOOK=0 tools/captures.sh "$run/serial" > "$run/serial.txt" 2>&1 ||
      failed+=(serial-captures)
    FORGE_KEEP_LOGS=0 tools/compare.sh "$run" "$run/serial" | tee "$run/serial-compare.txt" | tail -n 1
    [ "${PIPESTATUS[0]}" = 0 ] || failed+=(serial)
    step "tools/origins.sh"
    tools/origins.sh "$run/origins" | tee "$run/origins.txt"
  fi
  if [ -n "$validate_sets" ]; then
    step "validation: $validate_sets"
    FORGE_SETS=$validate_sets FORGE_PATHS="mesh fb" tools/validate.sh target/release "$run/validate" |
      tee "$run/validate.txt"
    # A clean run prints its commands, durations and the checks' verdicts only.
    if grep -E '^ +[0-9]+ ' "$run/validate.txt" | grep -vE '^ +[0-9]+ s, [0-9]+ log lines$' |
      grep -vE 'mip check passed|tone check passed' | grep -q . ||
      grep -q "the run printed nothing" "$run/validate.txt"; then
      failed+=(validation)
    fi
  fi
fi

step "waiting for clippy, the tests and credits"
if wait "$gate_pid"; then
  grep -E "^test result:" "$run/gate.txt" | awk '{ passed += $4; failed += $6 } END { print "   tests: " passed " passed, " failed " failed" }'
else
  failed+=(gate)
  grep -E "^(error|warning)|FAILED|panicked|test result: FAILED|out of date" "$run/gate.txt" | head -n 20
fi
gate_pid=""

if [ -n "$timings_sets" ]; then
  lock_gpu
  step "timings: $timings_sets against $timings_bin"
  FORGE_SETS=$timings_sets TIMINGS_OUT=$run/timings tools/timings.sh "$timings_bin" target/release |
    tee "$run/timings.txt"
fi
unlock

# --- the verdict -----------------------------------------------------------------------------

left="the full batch on both paths, validation, timings, tools/origins.sh, FORGE_ASYNC=0 and the real-time tour"
case $tier in
  0)
    left="the other sets"
    [ $gpu = 0 ] && left="$left, the fallback path, validation"
    left="$left, timings, tools/origins.sh, FORGE_ASYNC=0 and the real-time tour"
    ;;
  1) left="tools/origins.sh, FORGE_ASYNC=0 and the real-time tour" ;;
  2) left="the real-time tour, to watch for shimmer and pops: target/release/island --tour" ;;
esac
[ "$tier" != gate ] && [ "$tier" != 0 ] && [ -z "$timings_bin" ] && left="timings (--timings BASE_BIN), $left"
step "verdict"
echo "Tier $tier in $((SECONDS - start)) s, run in $run"
if [ "$tier" = 2 ]; then echo "left for the owner: $left"; else echo "left for Tier 2: $left"; fi
if [ ${#failed[@]} -gt 0 ]; then
  echo "verdict: fail" >> "$run/state.txt"
  echo "FAILED: ${failed[*]}"
  exit 1
fi
echo "verdict: pass" >> "$run/state.txt"
echo "PASSED"
if [ "$tier" = gate ]; then
  exit 0
elif [ $no_accept = 1 ]; then
  echo "not accepted (--no-accept): tools/verify.sh --accept $run"
elif gate_only "${dirty[@]}"; then
  accept "$run" "$head" "${base_set:+$base}" "verified by tools/verify.sh, Tier $tier"
else
  echo "not accepted yet: the working tree changes paths that can move pixels."
  echo "commit them, then: tools/verify.sh --accept $run"
fi
exit 0
