#!/usr/bin/env bash
# Builds Forge's C shim over NVIDIA's NRC (Neural Radiance Cache, D-088's trial) into nrc-sdk/bin,
# which git ignores, beside the NRC library and its CUDA runtime:
#
#   tools/fetch-nrc.sh           clone (or check) nrc-sdk/src, build nrc-sdk/bin/forge_nrc.dll
#   tools/fetch-nrc.sh --clean   start over from a fresh clone
#
# NRC is a binary release under the NVIDIA RTX SDKs License (nrc-sdk/src/License.md), not Forge's
# MIT/Apache: none of it ever enters the repository. tools/forge_nrc.cpp is Forge's own code,
# compiled against NRC's headers and import library; crates/forge-gpu/src/nrc.rs loads the shim at
# run time (the `nrc` feature), and the shim loads NRC_Vulkan.dll and CUDA 13's runtime from the
# same folder. A game that ships NRC ships those DLLs under NVIDIA's terms (CREDITS.md, "When a
# build ships").
#
# Windows: the build uses Visual Studio's C++ tools (found with vswhere) and the Vulkan SDK's
# headers. The clone is NVIDIA's repository at the release Forge pins (about 240 MB, CUDA's runtime
# most of it).
set -euo pipefail

release=v0.16.0.0
commit=7de30e4
here=$(cd "$(dirname "$0")/.." && pwd)
sdk=$here/nrc-sdk
case ${1:-} in
  --clean) rm -rf "$sdk/src" "$sdk/bin" ;;
  "") ;;
  -h | --help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "unknown option $1" >&2; exit 2 ;;
esac

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *) echo "tools/fetch-nrc.sh builds on Windows only for now" >&2; exit 1 ;;
esac

src=$sdk/src
if [ -d "$src/.git" ]; then
  have=$(git -C "$src" rev-parse --short=7 HEAD)
  if [ "$have" != "$commit" ]; then
    echo "nrc-sdk/src holds $have, not $release ($commit): run with --clean" >&2
    exit 1
  fi
  echo "NRC $release already in nrc-sdk/src"
else
  echo "cloning NRC $release from https://github.com/NVIDIA-RTX/NRC"
  git clone --quiet https://github.com/NVIDIA-RTX/NRC.git "$src"
  git -C "$src" checkout --quiet "$commit"
fi

vswhere="${ProgramFiles:-C:/Program Files} (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
[ -x "$vswhere" ] || vswhere="C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
vs=$("$vswhere" -latest -prerelease -products '*' \
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | tr -d '\r')
[ -n "$vs" ] || { echo "no Visual Studio with the C++ tools (vswhere found none)" >&2; exit 1; }
[ -n "${VULKAN_SDK:-}" ] || { echo "VULKAN_SDK is not set (its headers)" >&2; exit 1; }

bin=$sdk/bin
build=$sdk/build
mkdir -p "$bin" "$build"
script=$sdk/build.bat
# The C runtime linked in (/MT), as NRD's; NRC's own DLL keeps its own. NRC_Vulkan.dll is
# delay-loaded: the shim checks its signature before loading it (tools/forge_nrc.cpp).
cat > "$script" <<EOF
@echo off
call "$vs\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul || exit /b 1
cd /d "$(cygpath -w "$build")"
cl /nologo /LD /MT /O2 /EHsc /std:c++17 /W4 /WX ^
  /I "$(cygpath -w "$src/Include")" /I "$(cygpath -w "$VULKAN_SDK/Include")" ^
  "$(cygpath -w "$here/tools/forge_nrc.cpp")" /Fe:"$(cygpath -w "$bin/forge_nrc.dll")" ^
  /link "$(cygpath -w "$src/Lib/x64/NRC_Vulkan.lib")" /DELAYLOAD:NRC_Vulkan.dll delayimp.lib || exit /b 2
EOF
echo "building the shim (log: nrc-sdk/build.log)"
if ! cmd //c "$(cygpath -w "$script")" > "$sdk/build.log" 2>&1; then
  tail -20 "$sdk/build.log" >&2
  echo "the build failed" >&2
  exit 1
fi
cp "$src"/Bin/x64/*.dll "$src/License.md" "$bin/"
rm -f "$bin"/NRC_D3D12.dll "$bin"/forge_nrc.exp "$bin"/forge_nrc.lib
size=$(wc -c < "$bin/forge_nrc.dll")
echo "installed nrc-sdk/bin/forge_nrc.dll ($((size / 1024)) KB) beside NRC $release's NRC_Vulkan.dll and CUDA's runtime"
