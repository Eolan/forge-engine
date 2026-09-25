#!/usr/bin/env bash
# Gathers what a cloud session needs to read a run of the batch (docs/PROCESS.md, "Reports for
# a cloud session"): the machine and the build, and from each capture directory its summary,
# its compare output, its logs and one contact sheet of its images (not the images themselves),
# into reports/NAME/, a few MB, to commit and push.
#
#   tools/report.sh NAME [DIR...]
#
# NAME: the report's name, e.g. 2026-09-26-origins. DIR: capture directories written by
# tools/captures.sh, origins.sh, validate.sh or timings.sh (default: every directory under
# captures/ holding a summary.txt). Then: git add reports/NAME && git commit && git push.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
name=${1:?usage: tools/report.sh NAME [DIR...]}
shift
cd "$root"
dirs=("$@")
if [ ${#dirs[@]} -eq 0 ]; then
  for d in captures/*/; do
    [ -f "$d/summary.txt" ] && dirs+=("${d%/}")
  done
fi
[ ${#dirs[@]} -gt 0 ] || { echo "no capture directory with a summary.txt under captures/" >&2; exit 1; }
report=reports/$name
mkdir -p "$report"
sheet=target/release/contact-sheet
[ -f "$sheet.exe" ] && sheet=$sheet.exe
{
  echo "report $name, $(date -u +%FT%TZ)"
  echo "branch $(git rev-parse --abbrev-ref HEAD), commit $(git rev-parse --short HEAD)"
  echo "== git status"
  git status --short
  echo "== toolchain"
  rustc --version
  cargo --version
  if command -v slangc > /dev/null 2>&1; then slangc -v 2>&1 | head -n 1; else echo "(slangc not on PATH)"; fi
  echo "== environment"
  env | grep '^FORGE_' || echo "(no FORGE_* variables)"
  echo "== GPU"
  grep -rh "selected GPU" "${dirs[@]}" 2>/dev/null | head -n 1 || echo "(no log names the GPU)"
  echo "== directories"
  for d in "${dirs[@]}"; do echo "$d: $(ls "$d"/*.png 2>/dev/null | wc -l) images"; done
} > "$report/env.txt" 2>&1
for d in "${dirs[@]}"; do
  base=$(basename "$d")
  mkdir -p "$report/$base"
  for f in summary.txt compare.txt; do
    [ -f "$d/$f" ] && cp "$d/$f" "$report/$base/"
  done
  [ -d "$d/logs" ] && cp -r "$d/logs" "$report/$base/"
  images=("$d"/*.png)
  if [ -f "$sheet" ] && [ -f "${images[0]}" ]; then
    "$sheet" "$report/$base/sheet.png" "${images[@]}" --columns 6 --width 320 > /dev/null 2>&1 ||
      echo "$base: no contact sheet (contact-sheet failed)"
  fi
done
echo "report in $report ($(du -sh "$report" | cut -f1)); commit it: git add $report && git commit -m 'Report $name' && git push"
