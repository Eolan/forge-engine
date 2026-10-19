#!/usr/bin/env bash
# Type-checks and lints the repository's Luau game scripts with luau-lsp (D-072's tooling, #319):
# every `crates/forge-script/scripts/*.luau` against the game API's definitions
# (`crates/forge-script/defs/game.d.luau`). Part of the gate (tools/verify.sh) and of CI.
#
#   tools/luau-check.sh
#
# The binary: $LUAU_LSP, else luau-lsp/win64/luau-lsp.exe or luau-lsp/linux/luau-lsp in the
# repository (ignored by git; the owner's go for the download, 2026-10-19), else `luau-lsp` on
# the PATH. Without one it says so and passes: CI always has it (.github/workflows/ci.yml, a
# pinned release and its checksum).
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root" || exit 1
bin=${LUAU_LSP:-}
if [ -z "$bin" ]; then
  for candidate in luau-lsp/win64/luau-lsp.exe luau-lsp/linux/luau-lsp; do
    if [ -x "$candidate" ]; then
      bin=$candidate
      break
    fi
  done
fi
if [ -z "$bin" ] && command -v luau-lsp > /dev/null; then
  bin=luau-lsp
fi
if [ -z "$bin" ]; then
  echo "luau-check: no luau-lsp (set LUAU_LSP, or unpack a release into luau-lsp/): skipped"
  exit 0
fi
scripts=(crates/forge-script/scripts/*.luau)
if [ ! -e "${scripts[0]}" ]; then
  echo "luau-check: no scripts"
  exit 0
fi
"$bin" analyze --platform=standard \
  --definitions=@forge=crates/forge-script/defs/game.d.luau "${scripts[@]}"
status=$?
if [ $status = 0 ]; then
  echo "luau-check: ${#scripts[@]} script(s) clean ($("$bin" --version))"
fi
exit $status
