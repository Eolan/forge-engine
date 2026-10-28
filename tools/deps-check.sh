#!/usr/bin/env bash
# The dependency check (the owner, 2026-10-28: weekly): what Forge builds on, as installed here,
# against the latest releases, and the crate updates `cargo update` would take, without
# changing anything. Crates stay pinned in Cargo.lock on purpose (docs/PROCESS.md, Dependabot
# for security fixes); this lists what moved so a session can recommend what to take.
#
#   tools/deps-check.sh            write captures/deps/<date>.txt and captures/deps/latest.txt
#
# Needs the network (the crates index, GitHub's API through `gh`, LunarG's version file). No
# build, no GPU, no lock. Sends no personal data: requests go out under the tools' own agents.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root" || exit 1
out_dir=$root/captures/deps
mkdir -p "$out_dir"
stamp=$(date +%Y-%m-%d)
out=$out_dir/$stamp.txt

# latest REPO: the newest release's tag and date on GitHub, or the newest tag.
latest() {
  gh api "repos/$1/releases/latest" --jq '"\(.tag_name) (\(.published_at[0:10]))"' 2> /dev/null ||
    gh api "repos/$1/tags" --jq '.[0].name' 2> /dev/null || echo "?"
}
# define FILE NAME: the value of `#define NAME value` in FILE.
define() {
  [ -f "$1" ] && sed -n "s/^#define $2 *(\{0,1\}\([0-9]*\))\{0,1\}.*/\1/p" "$1" | head -1
}

{
  echo "dependency check, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD)"
  echo
  echo "== Toolchain and SDKs: installed here / latest release"
  printf '%-22s %-28s %s\n' "rustc" "$(rustc --version | cut -d' ' -f2)" "$(latest rust-lang/rust)"
  vulkan_here=$(ls /c/VulkanSDK 2> /dev/null | sort -V | tail -1)
  printf '%-22s %-28s %s\n' "Vulkan SDK" "${vulkan_here:-?} (all: $(ls /c/VulkanSDK 2> /dev/null | tr '\n' ' '))" "$(curl -s https://vulkan.lunarg.com/sdk/latest/windows.txt)"
  # The compiler the engine takes: FORGE_SLANGC's when set (a standalone release, newer than the
  # SDK's), else PATH's.
  slangc_bin=slangc
  [ -n "${FORGE_SLANGC:-}" ] && slangc_bin=$(cygpath -u "$FORGE_SLANGC" 2> /dev/null || echo "$FORGE_SLANGC")
  printf '%-22s %-28s %s\n' "slangc${FORGE_SLANGC:+ (FORGE_SLANGC)}" "$("$slangc_bin" -v 2>&1 | head -1)" "$(latest shader-slang/slang)"
  [ -n "${FORGE_SLANGC:-}" ] && printf '%-22s %-28s\n' "slangc (Vulkan SDK)" "$(slangc -v 2>&1 | head -1)"
  sl="streamline-sdk/include/sl_version.h"
  printf '%-22s %-28s %s\n' "Streamline" "$(define $sl SL_VERSION_MAJOR).$(define $sl SL_VERSION_MINOR).$(define $sl SL_VERSION_PATCH)" "$(latest NVIDIA-RTX/Streamline)"
  dlss=$(powershell -NoProfile -Command "(Get-Item 'streamline-sdk/bin/x64/nvngx_dlss.dll').VersionInfo.FileVersion" 2> /dev/null | tr -d '\r')
  printf '%-22s %-28s %s\n' "DLSS (nvngx_dlss.dll)" "${dlss:-?}" "(ships with Streamline)"
  nrd="nrd-sdk/src/Include/NRD.h"
  printf '%-22s %-28s %s\n' "NRD" "$(define $nrd NRD_VERSION_MAJOR).$(define $nrd NRD_VERSION_MINOR).$(define $nrd NRD_VERSION_BUILD)" "$(latest NVIDIA-RTX/NRD)"
  ffx=$(grep -rh "define FFX_SDK_VERSION_\(MAJOR\|MINOR\|PATCH\)\b" fidelityfx-sdk 2> /dev/null | head -3 | grep -oE "\([0-9]+\)" | tr -d '()' | paste -sd.)
  printf '%-22s %-28s %s\n' "FidelityFX SDK" "${ffx:-?}" "$(latest GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)"
  sa=$(grep -rh "define STEAMAUDIO_VERSION_\(MAJOR\|MINOR\|PATCH\)" steam-audio-sdk 2> /dev/null | head -3 | awk '{print $3}' | paste -sd.)
  printf '%-22s %-28s %s\n' "Steam Audio" "${sa:-?}" "$(latest ValveSoftware/steam-audio)"
  rtxdi=$(grep -h "RTXDI" CREDITS.md 2> /dev/null | grep -oE "v?[0-9]+\.[0-9]+(\.[0-9]+)?" | head -1)
  printf '%-22s %-28s %s\n' "RTXDI" "${rtxdi:-?}" "$(latest NVIDIA-RTX/RTXDI)"
  jolt=$(grep -rh "define JPH_VERSION_\(MAJOR\|MINOR\|PATCH\)" --include=Core.h . 2> /dev/null | head -3 | awk '{print $3}' | paste -sd.)
  printf '%-22s %-28s %s\n' "Jolt Physics" "${jolt:-?}" "$(latest jrouwe/JoltPhysics)"
  echo
  echo "== Crates: what 'cargo update' would take (dry run; nothing written)"
  cargo update --dry-run --verbose 2>&1 | grep -E "Updating|Unchanged|Locking|Adding|Removing" | grep -v "crates.io index"
  echo
  echo "== Pinned from git and vendored (Cargo.toml's [patch.crates-io]): the pin / upstream"
  # Outside the workspace, so that `cargo info` reads the registry, not our patched lock.
  released() { (cd / && cargo info "$1" 2> /dev/null | sed -n 's/^version: //p'); }
  ash_rev=$(sed -n 's/^ash = { git = "[^"]*", rev = "\([0-9a-f]*\)" }$/\1/p' Cargo.toml)
  if [ -n "$ash_rev" ]; then
    behind=$(gh api "repos/ash-rs/ash/compare/$ash_rev...master" --jq '"\(.ahead_by) commits behind master"' 2> /dev/null || echo "?")
    printf '%-22s %-28s %s\n' "ash (git)" "${ash_rev:0:8}" "$behind; last release $(released ash)"
  fi
  if [ -f third_party/gpu-allocator/Cargo.toml ]; then
    vendored=$(sed -n 's/^version = "\(.*\)"$/\1/p' third_party/gpu-allocator/Cargo.toml | head -1)
    printf '%-22s %-28s %s\n' "gpu-allocator (vendored)" "$vendored" "last release $(released gpu-allocator)"
  fi
  echo
  echo "== Security: open Dependabot alerts"
  gh api "repos/Eolan/forge-engine/dependabot/alerts?state=open&per_page=50" \
    --jq '.[] | "#\(.number) \(.security_advisory.severity) \(.dependency.package.name): \(.security_advisory.summary)"' 2> /dev/null ||
    echo "(could not read the alerts)"
  echo "(end)"
} > "$out" 2>&1
cp -f "$out" "$out_dir/latest.txt"
cat "$out"
