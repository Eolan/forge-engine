#!/usr/bin/env bash
# Fetches NVIDIA's NRD (Real-time Denoisers) at the release Forge pins and builds the library
# its SIGMA shadow denoiser runs from (issue #172, D-049) into nrd-sdk/, which git ignores:
#
#   tools/fetch-nrd.sh           clone (or check) nrd-sdk/src, build, install nrd-sdk/bin/NRD.dll
#   tools/fetch-nrd.sh --clean   start over from a fresh clone
#
# NRD is under the NVIDIA RTX SDKs License (nrd-sdk/src/LICENSE.txt), not Forge's MIT/Apache:
# none of it ever enters the repository. A game that ships it ships the library as object code,
# under terms at least as protective of NVIDIA as its own (CREDITS.md, "When a build ships").
#
# Windows: the build uses Visual Studio's C++ tools, CMake and Ninja (found with vswhere) and the
# Vulkan SDK's DXC for the SPIR-V. NRD's CMake also downloads its two build dependencies,
# NVIDIA's ShaderMake and MathLib, at the versions it pins. Built as Forge reads it: SPIR-V
# only, normals as floats (NRD_NORMAL_ENCODING 4), linear roughness, the C runtime linked in, no
# quad intrinsics (they need VK_KHR_compute_shader_derivatives, which Forge does not enable).
set -euo pipefail

release=v4.18.0
commit=d3df3435c876c29346d4538500eddd4621ce451a
here=$(cd "$(dirname "$0")/.." && pwd)
sdk=$here/nrd-sdk
case ${1:-} in
  --clean) rm -rf "$sdk" ;;
  "") ;;
  -h | --help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "unknown option $1" >&2; exit 2 ;;
esac

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *) echo "tools/fetch-nrd.sh builds on Windows only for now" >&2; exit 1 ;;
esac

src=$sdk/src
if [ -d "$src/.git" ]; then
  have=$(git -C "$src" rev-parse HEAD)
  if [ "$have" != "$commit" ]; then
    echo "nrd-sdk/src holds $have, not $release ($commit): run with --clean" >&2
    exit 1
  fi
  echo "NRD $release already in nrd-sdk/src"
else
  # By commit: 4.18.0 is master's, not a tagged release yet (#210: SIGMA's tile classification
  # synchronises its threads there, 4.17.3 raced).
  echo "cloning NRD $release ($commit) from https://github.com/NVIDIA-RTX/NRD"
  git init --quiet "$src"
  git -C "$src" remote add origin https://github.com/NVIDIA-RTX/NRD.git
  git -C "$src" fetch --quiet --depth 1 origin "$commit"
  git -C "$src" checkout --quiet FETCH_HEAD
  have=$(git -C "$src" rev-parse HEAD)
  [ "$have" = "$commit" ] || { echo "fetched $have, not $commit" >&2; exit 1; }
fi

# Without material IDs (Forge's normal encoding 4), master's `CompareMaterials` is a scalar
# `true` that RELAX's shaders put in `float3(...)`, which DXC rejects: give it the comparison's
# dimension (`m == m`, true for any material ID). RELAX is not used; NRD builds all its shaders.
sed -i 's/#define CompareMaterials( m0, m, minm )     true$/#define CompareMaterials( m0, m, minm )     ( ( m ) == ( m ) )/' \
  "$src/Shaders/Common.hlsli"

vswhere="${ProgramFiles:-C:/Program Files} (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
[ -x "$vswhere" ] || vswhere="C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
vs=$("$vswhere" -latest -prerelease -products '*' \
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | tr -d '\r')
[ -n "$vs" ] || { echo "no Visual Studio with the C++ tools (vswhere found none)" >&2; exit 1; }
cmake_bin="$vs\\Common7\\IDE\\CommonExtensions\\Microsoft\\CMake"
[ -n "${VULKAN_SDK:-}" ] || { echo "VULKAN_SDK is not set (its DXC compiles NRD's shaders)" >&2; exit 1; }

build=$sdk/build
script=$sdk/build.bat
cat > "$script" <<EOF
@echo off
call "$vs\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul || exit /b 1
set "PATH=$cmake_bin\\CMake\\bin;$cmake_bin\\Ninja;%PATH%"
cmake -S "$(cygpath -w "$src")" -B "$(cygpath -w "$build")" -G Ninja -DCMAKE_BUILD_TYPE=Release ^
  -DNRD_EMBEDS_SPIRV_SHADERS=ON -DNRD_EMBEDS_DXIL_SHADERS=OFF -DNRD_EMBEDS_DXBC_SHADERS=OFF ^
  -DSHADERMAKE_FIND_DXC=OFF -DNRD_NORMAL_ENCODING=4 -DNRD_ROUGHNESS_ENCODING=1 ^
  -DNRD_SUPPORTS_QUAD_INTRINSICS=OFF ^
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded || exit /b 2
cmake --build "$(cygpath -w "$build")" --config Release || exit /b 3
EOF
echo "building NRD (log: nrd-sdk/build.log)"
if ! cmd //c "$(cygpath -w "$script")" > "$sdk/build.log" 2>&1; then
  tail -20 "$sdk/build.log" >&2
  echo "the build failed" >&2
  exit 1
fi
mkdir -p "$sdk/bin"
cp "$src/_Bin/NRD.dll" "$src/LICENSE.txt" "$sdk/bin/"
size=$(wc -c < "$sdk/bin/NRD.dll")
echo "installed nrd-sdk/bin/NRD.dll: NRD $release, $((size / 1024)) KB"
