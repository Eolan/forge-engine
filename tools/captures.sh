#!/usr/bin/env bash
# Writes the capture batch every rendering change is checked with (issue #74; docs/PROCESS.md):
# 78 fixed-step captures of meshlets, the ballad (its HDR output too, #94), city-blocks and its
# island, the island demo's golden shots (#96) and the physics lab (#136), on the mesh path and
# on the fallback (`--force-fallback`). Compare two batches with tools/compare.sh.
#
#   tools/captures.sh OUT [BIN]
#
# OUT: the directory to write (created). BIN: the directory holding the demo binaries (default
# target/release; a baseline built in another tree, see docs/PROCESS.md). The demos open on the
# secondary monitor and never take focus (FORGE_MONITOR). The batch takes a few minutes; each
# capture prints a line as it lands, and a demo's errors and panics. With FORGE_KEEP_LOGS=1 each
# run's full log is kept in OUT/logs/ and its key lines in OUT/summary.txt, for a cloud session
# to read (tools/report.sh gathers them); a local session needs neither.
#
# Part of the batch (issue #134; tools/verify.sh picks them from tools/impact.toml):
#   FORGE_SETS   the sets to capture, separated by spaces or commas (default all): sentinels
#                (static60 orbit120 noocc120 ast-notaa600 ast240 ast240-noocc ast-hdr600 city60
#                city60-noocc gallery60), meshlets, ballad, city, island (the city's island and
#                the island demo's shots), lab (the physics lab's scenes), all. The images keep
#                their names.
#   FORGE_PATHS  the paths, mesh and fb (default both).
#   FORGE_RECOOK 1: make the cached meshes again. The props are cached in BIN's tree
#                (cache/meshes/) by their parameters' text, not the code that makes them: a
#                change to forge-procgen or forge-geom needs it (the island's products and tiles
#                follow their code, #208). The first run of each scene that cooks (the city, the
#                gallery, the island on its 2 m and 8 m grounds) gets `--recook`.
# OUT/batch.txt records the commit, the sets, the paths, the driver and the binaries' hashes.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/captures.sh OUT [BIN]}
bin=${2:-$root/target/release}
keep=${FORGE_KEEP_LOGS:-0}
sets=" ${FORGE_SETS:-all} "
sets=${sets//,/ }
paths=${FORGE_PATHS:-mesh fb}
paths=${paths//,/ }
recook=${FORGE_RECOOK:-0}
for set in $sets; do
  case $set in
    all | sentinels | meshlets | ballad | city | island | lab) ;;
    *) echo "unknown set $set: sentinels, meshlets, ballad, city, island, lab or all" >&2; exit 1 ;;
  esac
done
for path in $paths; do
  case $path in
    mesh | fb) ;;
    *) echo "unknown path $path: mesh or fb" >&2; exit 1 ;;
  esac
done
mkdir -p "$out"
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
meshlets=$bin/meshlets$exe
asteroids=$bin/asteroids$exe
city=$bin/city-blocks$exe
# The island demo (#96), which a baseline from before it lacks: its shots are then skipped.
island_demo=$bin/island$exe
# The labs' external models captured alone (#170), those of them tools/fetch-assets.sh fetched.
external="CesiumMan FlightHelmet Fox MetalRoughSpheres NormalTangentMirrorTest NormalTangentTest TextureCoordinateTest TextureSettingsTest TextureTransformTest WaterBottle"
# The physics lab (#136), which a baseline from before it lacks: its captures are then skipped.
lab=$bin/physics-lab$exe
for demo in "$meshlets" "$asteroids" "$city"; do
  [ -f "$demo" ] || { echo "missing $demo: build with cargo build --release" >&2; exit 1; }
done
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
# The commit BIN was built from: the tree holding it (BIN is TREE/target/release).
built=$(git -C "$bin/../.." rev-parse HEAD 2>/dev/null)
header="tools/captures.sh $out from $bin, $(date -u +%FT%TZ), commit ${built:0:7}"
[ "$sets" != " all " ] || [ "$paths" != "mesh fb" ] && header="$header, sets$sets(paths $paths)"
[ "$recook" != 0 ] && header="$header, recooked"
echo "$header"
if [ "$keep" != 0 ]; then
  mkdir -p "$out/logs"
  echo "$header" > "$summary"
fi
{
  echo "$header"
  echo "commit: $built"
  echo "sets:$sets"
  echo "paths: $paths"
  echo "driver: $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>/dev/null | head -n 1)"
  for demo in "$meshlets" "$asteroids" "$city" "$island_demo" "$lab"; do
    [ -f "$demo" ] && echo "binary: $(sha256sum "$demo" | cut -c1-16) $(basename "$demo") $(date -u -r "$demo" +%FT%TZ)"
  done
} > "$out/batch.txt"

# sets_of NAME: the sets a capture belongs to, from its name without the path.
sets_of() {
  case $1 in
    static60 | orbit120 | noocc120) echo meshlets sentinels ;;
    nolod120) echo meshlets ;;
    ast-notaa600 | ast240 | ast240-noocc | ast-hdr600) echo ballad sentinels ;;
    ast*) echo ballad ;;
    city60 | city60-noocc | gallery60) echo city sentinels ;;
    city*) echo city ;;
    island* | water* | clouds* | shot-*) echo island ;;
    lab-*) echo lab ;;
  esac
}
# wanted NAME: true when FORGE_SETS asks for the capture NAME.
wanted() {
  [[ $sets == *" all "* ]] && return 0
  local set
  for set in $(sets_of "${1#*-}"); do
    [[ $sets == *" $set "* ]] && return 0
  done
  return 1
}
# The scenes already cooked again with FORGE_RECOOK=1.
recooked=" "
# scene DEMO ARGS...: the cooked scene a run draws (city, gallery, island2, island8), or nothing.
scene() {
  local demo=$1
  shift
  if [ "$demo" = "$island_demo" ] || [[ " $* " == *" --island "* ]]; then
    [[ " $* " == *" --island-drawn 8 "* ]] && echo island8 || echo island2
  elif [ "$demo" = "$city" ]; then
    [[ " $* " == *" --gallery "* ]] && echo gallery || echo city
  elif [ "$demo" = "$lab" ]; then
    echo lab
  fi
}

status=0
# Lines of a run's log worth keeping in the summary.
keys="forge_app: gpu:|ERROR|Error|panicked|selected GPU|checksum|world's origin"
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png; its errors printed,
# its log kept with FORGE_KEEP_LOGS=1. Skipped when FORGE_SETS leaves it out.
capture() {
  local name=$1 frame=$2 demo=$3
  shift 3
  wanted "$name" || return 0
  if [ "$recook" != 0 ]; then
    local cooked
    cooked=$(scene "$demo" "$@")
    if [ -n "$cooked" ] && [[ $recooked != *" $cooked "* ]]; then
      set -- "$@" --recook
      recooked="$recooked$cooked "
    fi
  fi
  local start=$SECONDS
  "$demo" --frames $((frame + 1)) --capture "$out/$name.png" --capture-frame "$frame" "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g' > "$log"
  local verdict="captured in $((SECONDS - start)) s"
  if [ ! -f "$out/$name.png" ]; then
    verdict="NO CAPTURE after $((SECONDS - start)) s"
    status=1
  fi
  echo "$name: $verdict"
  grep -E "ERROR|panicked" "$log" | head -n 5
  if [ "$keep" != 0 ]; then
    cp "$log" "$out/logs/$name.log"
    echo "== $name: $verdict ($(basename "$demo") $*)" >> "$summary"
    grep -E "$keys" "$log" | grep -v "GOG" | cut -c1-300 >> "$summary"
  fi
}

for path in $paths; do
  flag=()
  [ "$path" = fb ] && flag=(--force-fallback)
  # meshlets: the static view, the orbit, and the orbit without LOD or occlusion.
  capture "$path-static60" 60 "$meshlets" "${flag[@]}"
  capture "$path-orbit120" 120 "$meshlets" --orbit "${flag[@]}"
  capture "$path-nolod120" 120 "$meshlets" --orbit --no-lod "${flag[@]}"
  capture "$path-noocc120" 120 "$meshlets" --orbit --no-occlusion "${flag[@]}"
  # The ballad at fixed steps: frame 600 with and without TAA, frame 240 for the A/B harness.
  capture "$path-ast-notaa600" 600 "$asteroids" --fixed-step --no-taa "${flag[@]}"
  capture "$path-ast-taa600" 600 "$asteroids" --fixed-step "${flag[@]}"
  capture "$path-ast240" 240 "$asteroids" --fixed-step --no-taa "${flag[@]}"
  capture "$path-ast240-noocc" 240 "$asteroids" --fixed-step --no-taa --no-occlusion "${flag[@]}"
  capture "$path-ast240-nocone" 240 "$asteroids" --fixed-step --no-taa --no-cone "${flag[@]}"
  capture "$path-ast240-culled" 240 "$asteroids" --fixed-step --no-taa --show-culled "${flag[@]}"
  # Its HDR output (#94), drawn off-screen in HDR10 through ACES 2.0 at 1000 nits: each run
  # writes the SDR preview and the PQ codes (`-pq.png`, 16 bits, compared exactly).
  capture "$path-ast-hdr240" 240 "$asteroids" --fixed-step --no-taa --tonemap aces2 --hdr offscreen "${flag[@]}"
  capture "$path-ast-hdr600" 600 "$asteroids" --fixed-step --tonemap aces2 --hdr offscreen "${flag[@]}"
  # city-blocks: every page resident (streaming would make the start depend on timing), and
  # the gallery of the twenty props. At a fixed step, as every capture of city-blocks, island and
  # physics-lab is since the clouds are on by default (#145): they drift on the scene's clock,
  # which would otherwise run on the run's own timing.
  capture "$path-city60" 60 "$city" --fixed-step --stream-pool 0 "${flag[@]}"
  capture "$path-city60-noocc" 60 "$city" --fixed-step --stream-pool 0 --no-occlusion "${flag[@]}"
  capture "$path-city60-culled" 60 "$city" --fixed-step --stream-pool 0 --show-culled "${flag[@]}"
  capture "$path-cityorbit120" 120 "$city" --fixed-step --stream-pool 0 --orbit "${flag[@]}"
  capture "$path-gallery60" 60 "$city" --fixed-step --gallery "${flag[@]}"
  # The island (#96) from its first view on the coast without its water (`--no-water`): its
  # heightfield, rocks and the stand-in sea.
  # The software rasteriser pinned on: its automatic switch follows how much the culls let
  # through, so it would differ between the A/B runs (#101).
  # Streamed, as it starts (its 2 m ground's 4.2 GB of pages exceed a resident pool): the pages
  # of its first view's cut are loaded before the first frame (#121), so the fixed view reads
  # nothing more and a frame depends on no read's timing.
  island=(--island 7 --sw-raster on)
  capture "$path-island60" 60 "$city" --fixed-step "${island[@]}" --no-water "${flag[@]}"
  capture "$path-island60-noocc" 60 "$city" --fixed-step "${island[@]}" --no-water --no-occlusion "${flag[@]}"
  # The island with its water (#105, the default): at a fixed step, so the waves are the same.
  capture "$path-water60" 60 "$city" "${island[@]}" --fixed-step "${flag[@]}"
  capture "$path-water60-noocc" 60 "$city" "${island[@]}" --fixed-step --no-occlusion "${flag[@]}"
  # The cloud layer over it (#145, on by default at 0.45 since 2026-10-03, so in every image of
  # city-blocks, island and physics-lab): here a second coverage, half the sky,
  # sixty frames for its blend over frames to settle.
  capture "$path-clouds60" 60 "$city" "${island[@]}" --fixed-step --clouds 0.5 "${flag[@]}"
  capture "$path-clouds60-noocc" 60 "$city" "${island[@]}" --fixed-step --clouds 0.5 --no-occlusion "${flag[@]}"
  # Its 8 m ground streamed from the start view and resident (#121): equal, or the start view
  # missed pages the cut wants.
  capture "$path-island8-60" 60 "$city" --fixed-step "${island[@]}" --island-drawn 8 --no-water "${flag[@]}"
  capture "$path-island8-60-resident" 60 "$city" --fixed-step "${island[@]}" --island-drawn 8 --no-water --stream-pool 0 "${flag[@]}"
  # The walker on the southern beach (#196, #197) at tick 300, its footprints in the sand window
  # round it (the tiles' ground cut there, the window shaded as it), and its A/B twin.
  walker=(--walker --walk 0,-1.5 "--view=0,25,5214,0,-50")
  capture "$path-island-sand300" 300 "$city" --fixed-step "${island[@]}" "${walker[@]}" "${flag[@]}"
  capture "$path-island-sand300-noocc" 300 "$city" --fixed-step "${island[@]}" "${walker[@]}" --no-occlusion "${flag[@]}"
  # The island demo's golden shots (#96), each at its time of day, the exposure metered from the
  # scene: dawn over the largest mouth, the lake in the morning, the island from the sea in the
  # afternoon, dusk up a steep valley.
  if [ -f "$island_demo" ]; then
    for shot in mouth lake island valley; do
      capture "$path-shot-$shot" 60 "$island_demo" --shot "$shot" --sw-raster on --fixed-step "${flag[@]}"
    done
  fi
  # The physics lab (#136), a tick a frame: the rain in mid-air at tick 90, with the occlusion
  # off for the A/B harness (the movers are culled like the rest), and the pile at rest at tick
  # 600.
  if [ -f "$lab" ]; then
    capture "$path-lab-drop90" 90 "$lab" --lab drop --fixed-step "${flag[@]}"
    capture "$path-lab-drop90-noocc" 90 "$lab" --lab drop --fixed-step --no-occlusion "${flag[@]}"
    capture "$path-lab-drop600" 600 "$lab" --lab drop --fixed-step "${flag[@]}"
    # Through a server and a client over 100 ms (#137), balls thrown from the camera and by the
    # bot: the client's prediction, corrected by the bot's throws, from seeded links.
    capture "$path-lab-net300" 300 "$lab" --lab drop --fixed-step --net 100 --throw-every 45 "${flag[@]}"
    # The sea (#138): what floats on the waves and the rocks on the floor at tick 300, its A/B
    # twin, and the boat under way with the rudder over at tick 600.
    capture "$path-lab-sea300" 300 "$lab" --lab sea --fixed-step "${flag[@]}"
    capture "$path-lab-sea300-noocc" 300 "$lab" --lab sea --fixed-step --no-occlusion "${flag[@]}"
    capture "$path-lab-sea-steer600" 600 "$lab" --lab sea --fixed-step --steer 1,0.6 "${flag[@]}"
    # The playground (#139): the player halfway up the stairs at tick 150 and its A/B twin, and
    # through the light crates at tick 240.
    capture "$path-lab-walk150" 150 "$lab" --lab walk --fixed-step --walk 2,0 "${flag[@]}"
    capture "$path-lab-walk150-noocc" 150 "$lab" --lab walk --fixed-step --walk 2,0 --no-occlusion "${flag[@]}"
    capture "$path-lab-walk-crates240" 240 "$lab" --lab walk --fixed-step --walk 0,2 "${flag[@]}"
    # The track (#140): the car down it at tick 300 and its A/B twin, and turning into the
    # slalom at tick 600.
    capture "$path-lab-drive300" 300 "$lab" --lab drive --fixed-step --steer 1,0 "${flag[@]}"
    capture "$path-lab-drive300-noocc" 300 "$lab" --lab drive --fixed-step --steer 1,0 --no-occlusion "${flag[@]}"
    capture "$path-lab-drive-turn600" 600 "$lab" --lab drive --fixed-step --steer 1,0.3 "${flag[@]}"
    # The field (#141): the aeroplane climbing off the runway at tick 1200, and its A/B twin.
    capture "$path-lab-fly1200" 1200 "$lab" --lab fly --fixed-step --pilot 1,-0.4,0,0 "${flag[@]}"
    capture "$path-lab-fly1200-noocc" 1200 "$lab" --lab fly --fixed-step --pilot 1,-0.4,0,0 --no-occlusion "${flag[@]}"
    # The break scene (#142), the ball let go at the first tick: through the wall at tick 85
    # and its A/B twin, and the wall broken, the column in pieces at tick 300.
    capture "$path-lab-break85" 85 "$lab" --lab break --fixed-step --release 1 "${flag[@]}"
    capture "$path-lab-break85-noocc" 85 "$lab" --lab break --fixed-step --release 1 --no-occlusion "${flag[@]}"
    capture "$path-lab-break300" 300 "$lab" --lab break --fixed-step --release 1 "${flag[@]}"
    # The creatures (#143): posed on their motors at tick 120 and its A/B twin, struck by a
    # ball every 50 ticks at 240, and let go at tick 60, at 240.
    capture "$path-lab-creatures120" 120 "$lab" --lab creatures --fixed-step "${flag[@]}"
    capture "$path-lab-creatures120-noocc" 120 "$lab" --lab creatures --fixed-step --no-occlusion "${flag[@]}"
    capture "$path-lab-creatures-throw240" 240 "$lab" --lab creatures --fixed-step --throw-every 50 "${flag[@]}"
    capture "$path-lab-creatures-limp240" 240 "$lab" --lab creatures --fixed-step --limp-at 60 "${flag[@]}"
    # The skin pass's paths on request (#169), close on the mannequin's left arm: its forearm
    # twisted by dual quaternions, and its elbow bent with the morph targets' correctives; and
    # close on the slimes, on eight points a vertex.
    capture "$path-lab-creatures-dq60" 60 "$lab" --lab creatures --fixed-step --arm-pose twist --dual-quaternion "--view=0.42,1.05,0.75,0,-5" "${flag[@]}"
    capture "$path-lab-creatures-corrective60" 60 "$lab" --lab creatures --fixed-step --arm-pose bend --elbow-correctives "--view=0.42,1.05,0.75,0,-5" "${flag[@]}"
    capture "$path-lab-creatures-slime8-60" 60 "$lab" --lab creatures --fixed-step --slime-eight "--view=-0.2,0.5,3.2,0,-12" "${flag[@]}"
    # The dogs on their course (#167): up the steps and the ramp at tick 600, coming back down at
    # 1200.
    capture "$path-lab-course600" 600 "$lab" --lab course --fixed-step "${flag[@]}"
    capture "$path-lab-course1200" 1200 "$lab" --lab course --fixed-step "${flag[@]}"
    capture "$path-lab-flyer300" 300 "$lab" --lab flyer --fixed-step "${flag[@]}"
    capture "$path-lab-flyer600" 600 "$lab" --lab flyer --fixed-step "${flag[@]}"
    # The dogs over their beds of sand, mud and snow (#185, #186), the car through its own: in
    # the mud at tick 240; the dogs' prints pressed in on the way up at tick 600, and both ways
    # at 1200.
    capture "$path-lab-yard240" 240 "$lab" --lab yard --fixed-step "${flag[@]}"
    capture "$path-lab-yard600" 600 "$lab" --lab yard --fixed-step "${flag[@]}"
    capture "$path-lab-yard1200" 1200 "$lab" --lab yard --fixed-step "${flag[@]}"
    # The materials' patches (#203): the crates sliding off the snow's and the ice's ramps and
    # the balls bouncing at tick 60, and its A/B twin; the walker's prints behind it in the sand
    # at tick 360 (#205); the walker at rest where it slid on the ice at tick 800.
    capture "$path-lab-materials60" 60 "$lab" --lab materials --fixed-step "${flag[@]}"
    capture "$path-lab-materials60-noocc" 60 "$lab" --lab materials --fixed-step --no-occlusion "${flag[@]}"
    capture "$path-lab-materials360" 360 "$lab" --lab materials --fixed-step "${flag[@]}"
    capture "$path-lab-materials800" 800 "$lab" --lab materials --fixed-step "${flag[@]}"
    # The flood (#144), the gate lifted at tick 31: the water running down the basin at tick
    # 150 and its A/B twin, and spread round the blocks at tick 300.
    capture "$path-lab-flood150" 150 "$lab" --lab flood --fixed-step --release 31 "${flag[@]}"
    capture "$path-lab-flood150-noocc" 150 "$lab" --lab flood --fixed-step --release 31 --no-occlusion "${flag[@]}"
    capture "$path-lab-flood300" 300 "$lab" --lab flood --fixed-step --release 31 "${flag[@]}"
    # The flood's columns drawn themselves, without the GPU's finer layer (#162).
    capture "$path-lab-flood150-columns" 150 "$lab" --lab flood --fixed-step --release 31 --no-gpu-water "${flag[@]}"
    # The domino run (#146), the first pushed at tick 31: the fall a third of the way at tick
    # 900 and its A/B twin, and all down at tick 3000.
    capture "$path-lab-dominoes900" 900 "$lab" --lab dominoes --fixed-step --release 31 "${flag[@]}"
    capture "$path-lab-dominoes900-noocc" 900 "$lab" --lab dominoes --fixed-step --release 31 --no-occlusion "${flag[@]}"
    capture "$path-lab-dominoes3000" 3000 "$lab" --lab dominoes --fixed-step --release 31 "${flag[@]}"
    # The bridge (#147), the convoy let go at tick 31: the deck falling with two cars at tick
    # 360 and its A/B twin, and in the gap at tick 600.
    capture "$path-lab-bridge360" 360 "$lab" --lab bridge --fixed-step --release 31 "${flag[@]}"
    capture "$path-lab-bridge360-noocc" 360 "$lab" --lab bridge --fixed-step --release 31 --no-occlusion "${flag[@]}"
    capture "$path-lab-bridge600" 600 "$lab" --lab bridge --fixed-step --release 31 "${flag[@]}"
    # The rocket (#148) at full throttle, the stick a tenth pushed: climbing off the pad at tick
    # 120 and its A/B twin, pitched over downrange at tick 600.
    capture "$path-lab-rocket120" 120 "$lab" --lab rocket --fixed-step --pilot 1,0.1,0,0 "${flag[@]}"
    capture "$path-lab-rocket120-noocc" 120 "$lab" --lab rocket --fixed-step --pilot 1,0.1,0,0 --no-occlusion "${flag[@]}"
    capture "$path-lab-rocket600" 600 "$lab" --lab rocket --fixed-step --pilot 1,0.1,0,0 "${flag[@]}"
    # The tug-of-war (#149) through `--net 100`, the bot pulling hard and easing off every 1.5 s:
    # the sled on its way right at tick 200 and its A/B twin, over the right line at tick 600.
    capture "$path-lab-tug-net200" 200 "$lab" --lab tug --net 100 --fixed-step "${flag[@]}"
    capture "$path-lab-tug-net200-noocc" 200 "$lab" --lab tug --net 100 --fixed-step --no-occlusion "${flag[@]}"
    capture "$path-lab-tug-net600" 600 "$lab" --lab tug --net 100 --fixed-step "${flag[@]}"
    # The spaceship in zero g (#150) at full throttle: closing on the crates at tick 90, through
    # them at tick 150 and its A/B twin.
    capture "$path-lab-space90" 90 "$lab" --lab space --fixed-step --pilot 1,0,0,0 "${flag[@]}"
    capture "$path-lab-space150" 150 "$lab" --lab space --fixed-step --pilot 1,0,0,0 "${flag[@]}"
    capture "$path-lab-space150-noocc" 150 "$lab" --lab space --fixed-step --pilot 1,0,0,0 --no-occlusion "${flag[@]}"
    # The glass tank (#156), the gate lifted at tick 31: the wave climbing the far wall at tick 90
    # and its A/B twin; on the bench, the water settling at tick 300; through the holed gate, the jet
    # and the water white at the far wall at tick 120.
    capture "$path-lab-tank90" 90 "$lab" --lab tank --fixed-step --release 31 "${flag[@]}"
    capture "$path-lab-tank90-noocc" 90 "$lab" --lab tank --fixed-step --release 31 --no-occlusion "${flag[@]}"
    capture "$path-lab-tank-bench300" 300 "$lab" --lab tank-bench --fixed-step --release 31 "${flag[@]}"
    capture "$path-lab-tank-hole120" 120 "$lab" --lab tank-hole --fixed-step --release 31 "${flag[@]}"
    # The blocks in the tank (#156): the wave wrapping the cube and climbing the post at tick 56.
    capture "$path-lab-tank-blocks56" 56 "$lab" --lab tank-blocks --fixed-step --release 31 "${flag[@]}"
    # The sharpness room (#159): still at frame 60, and slid sideways at 2 m/s into the same view
    # (TAA's history resampled every frame: the edges across the motion softer).
    capture "$path-lab-room60" 60 "$lab" --lab room --fixed-step "${flag[@]}"
    capture "$path-lab-room-pan60" 60 "$lab" --lab room --fixed-step --pan 2 --view=-2,1.5,3,0,0 "${flag[@]}"
    # The same, supersampled 2 × 2 (D-045: SSAA for screenshots).
    capture "$path-lab-room-pan60-ssaa" 60 "$lab" --lab room --fixed-step --pan 2 --view=-2,1.5,3,0,0 --ssaa "${flag[@]}"
    # The labs' external models (#170, D-048), when tools/fetch-assets.sh has fetched them (never
    # in CI): the row at frame 60, then each alone, framed as its Khronos screenshot; Sponza, a
    # reference model (--reference-only), with and without the probes (#171).
    if [ -d "$root/assets/external" ]; then
      capture "$path-lab-models60" 60 "$lab" --lab models --fixed-step "${flag[@]}"
      for model in $external; do
        [ -d "$root/assets/external/$model" ] || continue
        capture "$path-lab-model-$model" 60 "$lab" --lab models --model "$model" --fixed-step "${flag[@]}"
      done
      if [ -d "$root/assets/external/Sponza" ]; then
        capture "$path-lab-model-Sponza" 60 "$lab" --lab models --model Sponza --fixed-step "${flag[@]}"
        capture "$path-lab-model-Sponza-noprobes" 60 "$lab" --lab models --model Sponza --fixed-step --no-probes "${flag[@]}"
      fi
    fi
  fi
done
closing="captures in $out: $(ls "$out"/*.png 2>/dev/null | wc -l) images"
[ "$keep" != 0 ] && closing="$closing; logs in $out/logs, summary in $summary" && echo "$closing" >> "$summary"
echo "$closing"
exit $status
