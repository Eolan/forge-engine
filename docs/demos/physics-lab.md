# Demo: physics-lab

Phase 3's first demo (`docs/ROADMAP.md`, "Phase 3"): small scenes of bodies on a flat floor,
each one a test of the physics with its numbers and its determinism hash. The owner's plan
(2026-10-02): simple dedicated environments first, one per kind of object or effect (rocks,
barrels, boats, planes, rockets, fluids, creatures, destruction), then a demo where everything
comes together, which is the island.

```
cargo run --release -p physics-lab
cargo run --release -p physics-lab -- --lab drop --fixed-step
```

`physics-lab` is `city-blocks --lab drop` in a window of its own: it shares that demo's
renderer, keys and options (the sun, the probes, TAA, `--view`, `--capture`). **Space** throws a
ball from the camera at 25 m/s; **Enter** takes the scene back to its start.

| Scene | What it tests | State |
|---|---|---|
| `drop` | the binding: boxes, cylinders, spheres and convex hulls falling, stacking, rolling and going to sleep; the determinism hash | ✅ #136 |
| `drop --record`, `--replay`, `--net MS` | inputs as commands, recordings replayed to the same digests, a server and predicting clients over a lossy link | ✅ #137 |
| `pool` | buoyancy: barrels, logs, crates and a boat on the water we render | step 3 |
| walking, vehicles, flight, destruction, creatures, fluids | the later steps of the plan | planned |

## The binding (`forge-physics`, issue #136)

Jolt Physics 5.6.0 (D-009) is vendored in `third_party/jolt` and built from source by the
crate's `build.rs` through `cc`, on Windows (MSVC) and Linux (gcc) alike, with the flags Jolt's
CMake sets for `CROSS_PLATFORM_DETERMINISTIC` and `DOUBLE_PRECISION`: precise floating point, no
fused multiply-add, AVX2 without FMA on x86-64. The build takes about 25 s on the 9800X3D and is
cached after that.

Between Rust and Jolt sits a C layer of Forge's own (`crates/forge-physics/cpp/forge_jolt.h`),
written after JoltC (Second Half Games): opaque shape handles, the layer set-up, and calls that
read or write many bodies at once (one call returns every body's transform). It is bound by
hand: no bindgen, no libclang; a test checks the C structs' sizes against their Rust twins. The
safe API has shapes (box, sphere, capsule, cylinder, convex hull, static mesh, offset), bodies
(static, kinematic, dynamic; friction, restitution, damping, mass, swept collision, asleep at
the start), the step, transforms, velocities, sleep, impulses, forces, a ray cast, and the
whole state saved and restored.

## Determinism

The tests drop a pile of 256 bodies (boxes, balls, cylinders, rock hulls), their rotations made
without trigonometry, and hash every body's transform to the bit after 5 s:

| Check | Result |
|---|---|
| the same run twice | the same hash |
| 1, 2, 4 and 8 threads | the same hash |
| saved at tick 100, run on to 300, restored to 100 and run to 300 again | the same hash |
| MSVC on Windows, g++ 15 on Linux (WSL's Ubuntu) | the same hash, `0x3e65693b88928ad9` |

The last is a test against a constant, so CI's Windows and Linux runners check it on every
push. These are the first, second, sixth and the cross-platform tests of
`docs/research/physics-fluids.md` §6; the rest (bodies added far away, a different insertion
order, record and replay across runs) come with the fixed tick in step 2.

## `drop`

A stepped pyramid of 204 sandstone blocks (0.8 m, 2300 kg/m³, eight layers), and 260 bodies
dropped onto it and around it from 8 to 45 m, turned and spinning at random: 100 red barrels
(60 kg cylinders), 100 rocks in three sizes (convex hulls of the city's boulder meshes, 0.35 to
0.65 m), 60 orange balls (0.25 m, bouncing). 32 more balls wait asleep under the floor for
Space. 496 bodies in all on a 800 m floor; the movers of #79 draw them, with their shadows,
motion vectors and the probes they wake.

![The rain in mid-air at tick 90, and the pile at rest at tick 600](images/physics-lab-drop.png)

The physics ticks at 60 Hz whatever the frame rate; the movers are drawn between the last two
ticks. With `--fixed-step` a frame is a tick, so a capture at frame N shows tick N; the log
gives the hash at ticks 60, 300 and 600 and over the run. Jolt has no rolling resistance: the
barrels' and the balls' spin is damped (0.3 a second) to stand in for it, and a ball still rolls
far before it sleeps.

Measured on the 9800X3D and the 5070 Ti at 1600 × 900, `--fixed-step`, 600 ticks:

| | |
|---|---|
| the world ready (shapes, 496 bodies, the broad phase, the state saved) | 204 ms; the state 82 KiB |
| a tick, 5 workers and the caller | mean 0.56 ms, p99 0.80 ms, max 0.89 ms |
| awake at tick 60, 300, 600 | 260, 452, 340 of 496 |
| the frame (GPU and CPU) | p50 1.15 ms, p99 1.77 ms |

## Commands, recordings and the network (`forge-sim`, issue #137)

The lab's world is a `forge_sim::Simulation`: it ticks at 60 Hz, saves, restores and digests
itself (its bodies' transforms and velocities to the bit, the tick, the next ball to throw).
What a player does reaches it as commands stamped with the tick they act on, applied in a fixed
order (by player, then by number) whatever order they arrived in: Space is a `Throw` (a ball
from the camera), Enter a `Reset` (everything back to the start; the clock runs on).
`--throw-every N` throws as Space does every N frames.

**Recordings.** `--record FILE` writes the session's commands and a digest every second at
exit; `--replay FILE` plays them again in place of the keys and checks each digest. A session
of 601 ticks and 13 throws replays to all 10 digests, and to the same digests at ticks 60, 300
and 600 and at the end. A recording with one throw left out leaves it at the next digest
(the tests).

**The network, in one process.** `--net MS` runs the scene through a server and this player's
client, the second player a bot throwing at the pyramid every 2.5 s, over links of MS one way
with 10 % of it as jitter and 2 % of the packets lost, both ways, from seeds:
- the server owns the world and takes every player's commands at their tick; one that comes
  after its tick is taken at the next;
- a client runs ahead of the server by the delay and two ticks, so its commands arrive in time,
  applies its own at once, and sends every command it has not seen acknowledged in each packet,
  so a lost packet loses none;
- each snapshot (every 6 ticks) carries the server's state, its digest and the commands taken;
  the client compares the digest with the one it predicted for that tick. When they agree it
  does nothing; when they differ (the other player threw, a command came late) it goes back to
  the server's state and runs the ticks since again with its commands not yet taken (D-010's
  reconciliation).

A single-player game runs the same server in its own process: nothing changes when a second
player joins. With `--net 100 --throw-every 45`, 600 ticks:

| | |
|---|---|
| snapshots taken by this player's client | 99: 96 predicted to the bit, 3 corrected (the bot's throws), 42 ticks run again |
| a correction (the state restored, 14 ticks run again) | at most 9.9 ms |
| a tick: the server, the two clients | mean 2.0 ms, p99 10.9 ms (the corrections) |
| commands taken by the server | all 17, none late |
| bytes down to a client | 280 KB a snapshot (the bodies and the contacts between them), 2.8 MB/s: a whole state, uncompressed |

The last line is what Phase 5's netcode is for: snapshots against an acknowledged baseline,
quantised, only of what a client sees (D-010's budget is 24 KB/s). The tests check the scheme
on a toy world (a lone client is never corrected over 205 snapshots; two clients correct each
other and end where the server is; late commands) and on this one (a session over the lossy
link ends where the server is, to the bit).

## Captures

The batch (`tools/captures.sh`, set `lab`) takes `lab-drop90` (the rain in mid-air), its A/B
twin with the occlusion off (`lab-drop90-noocc`, 0 px apart: the movers are culled like
everything else), `lab-drop600` (the pile at rest) and `lab-net300` (the client's view through
`--net 100 --throw-every 45`: thrown balls in flight, the bot's corrections behind it), on both
geometry paths.
