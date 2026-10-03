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
| `sea` | buoyancy on the sea we render: crates, barrels, logs, balls and a Blender boat afloat, rocks that sink, a jetty | ✅ #138 |
| `walk` | a walking character: stairs, ramps, a moving platform, crates to push, blocks to jump onto | ✅ #139 |
| `drive` | a car on wheels and springs: a jump ramp, a slalom of barrels, a wall of crates | ✅ #140 |
| `fly` | an aeroplane on flying surfaces: a take-off from a runway, turns over a wide field | ✅ #141 |
| `break` | destruction: a brick wall held by mortar that breaks, a wrecking ball, a concrete column that shatters | ✅ #142 |
| creatures, fluids | the later steps of the plan | planned |

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

## `sea`: what floats (issue #138)

```
cargo run --release -p physics-lab -- --lab sea
```

The open sea the island is drawn with (the same three FFT cascades from the same seed, D-038),
a floor 12 m down, a jetty of planks on twelve concrete pillars, and dropped from 1 to 6 m over
the water beyond its end: 30 wooden crates (600 kg/m³), 30 barrels (60 kg), 16 logs
(700 kg/m³, 3 m), 20 balls, 18 rocks that sink to the floor, and a boat. **The arrow keys**
drive the boat: up and down its throttle, left and right its rudder; **C** puts the camera
behind it. Space throws balls into the water as in `drop`.

![What floats at tick 600, and the boat come about under its motor](images/physics-lab-sea.png)

**The waves on the CPU.** What floats must sit on the waves the GPU draws. `forge_procgen::Ocean`
already synthesises the cascades' spectrum on the CPU (the GPU's agree with it within 3 × 10⁻⁶,
the water check at frame 120); `Ocean::displacement` now gives the height and the horizontal
displacement at a tick for less than its full surface: the phases only where the spectrum has
energy (3 208 samples of 65 536), two real fields in each complex transform, the rows outside
the cascade's band left out, and the rows and the columns spread over the job system with the
same bytes on any number of workers. A cascade takes 0.53 ms; the physics takes the two whose
waves move a hull (the swell and the waves down to 4 m), not the ripples. `SeaHeights` samples
them as the GPU's linear filter reads its images (texel centres half a texel off the transform's
samples) and finds the point under (x, z) by going back by the displacement there, twice: a
point of the surface is found again within 2 cm.

**Buoyancy** (`forge_physics::buoyancy`, after Jacques Kerner's model for boats, 2015). Each
floating body has a closed hull in its frame: a box cut into squares, a cylinder, a ball, or a
model's own shell. Every tick each triangle is cut where the surface crosses it, and each
submerged piece is pushed by the water's pressure at its depth along its normal; the pieces
moving into the water are dragged by it (pressure drag), all of them along it (skin friction),
and near the surface their motion makes waves that carry energy away (radiation damping, which
settles a raft in seconds where the drag alone left it bobbing). The pushes are worked out for
all the floaters in parallel and given to Jolt in one call; a body asleep wholly under the water
(a rock on the floor) is left asleep. The tests: a box under water is pushed by the weight of
the water it displaces, a raft half in by half of it and tilted it rights itself (a cube half in
does not: its metacentre lies under its centre of mass, and the code says so), a moving box is
slowed as a plate would be, a raft dropped into still water settles at its draft to 5 mm and
rests, and a stone sinks.

**The boat** was modelled in Blender 5.2 from code (`assets/blender/boat.py`, run headless) and
exported as glTF (`assets/models/boat.glb`, 280 KB): a 5 m open motorboat with a round-bilged
hull, a raised bow and a flat transom, a blue rim, three thwarts and an outboard motor, 10 248
triangles in four materials, and a second mesh, a coarse closed shell of 427 triangles, for its
buoyancy and its collision. `forge_geom::model::load_glb` (through the `gltf` crate) reads both
with their nodes' transforms; the drawn one becomes a prop (`PropKind::Imported`, cached by the
file's bytes) with a material row per glTF material. In the lab it weighs 420 kg with its weight
30 cm under its hull's centre; its motor pushes 2.6 kN at the propeller along the boat, turned by
the rudder, only while the propeller is under the surface: about 3.5 m/s ahead, and it comes
about with the rudder over. The throttle and the rudder are a command (`Steer`) through
`forge-sim`, so a session with the boat records, replays and goes through `--net` like the
throws; `--steer T,R` holds them from the first frame (the captures).

![The boat in Blender's renderer: drawn (left) and its buoyancy shell (right)](images/physics-lab-boat-blender.png)

Measured at 1600 × 900, `--fixed-step`, 600 ticks, 147 bodies (115 awake: all but the balls not
yet thrown): **1.8 ms a tick** (p99 2.3), of it about 1.1 ms the two cascades of waves and most of
the rest the pushes. A test replays 4 s of the scene (a throw, the boat ahead and turning) to the
same digests: the waves, the buoyancy and the motor are as deterministic as the rest.

## `walk`: a character in a playground (issue #139)

```
cargo run --release -p physics-lab -- --lab walk
```

A character walks the lab's floor: Jolt's `CharacterVirtual` through `forge-physics`, a capsule
1.8 m tall standing on its feet, swept through the world by its velocity. It slides along what it
meets, walks up steps of up to 40 cm, keeps to the ground going down, stops at slopes steeper
than 45°, stands on what moves and moves with it, and pushes what it walks into with up to
400 N. Its state is saved and restored with the world's (the C layer saves the characters after
the bodies, each world numbering its own: Jolt's numbers run across the process, and a server
and a client in one process must agree).

The playground: six stairs of 20 cm, ramps of 20°, 35° and 50° (their angles from
`forge_core::dmath`, so the ground is the same bits on every platform), a red platform
shuttling along x at 2 m/s, six light crates (100 kg/m³: it shoves them) and three heavy ones
(900 kg/m³: they stay), and a pyramid of four layers of blocks to jump onto. **WASD** walk
along the view, **Shift** runs (6.5 m/s against 3), **Space** jumps (5 m/s up, about 1.3 m),
**the right mouse button** turns the view round the player, **X** throws a ball; the camera
stays 5 m behind its head. The walk and the jumps are commands (`Walk`, held until it changes,
and `Jump`), so a session records, replays and goes through `--net`; `--walk X,Z` holds a walk
from the first frame (the captures).

![Halfway up the stairs at tick 150, and through the light crates at tick 240](images/physics-lab-walk.png)

The tests: up the stairs to the top step (0.996 m) and not up the 50° ramp; carried 1 m by a
platform moving at 1 m/s for a second, and the same second again from a saved world to the bit;
and the playground replays a walk up the stairs, a jump and a turn to the same digests. A tick
of the playground (72 bodies, the character, the platform): **0.08 ms** on average, p99 0.22 ms.

## `drive`: a car (issue #140)

```
cargo run --release -p physics-lab -- --lab drive
```

A car on the lab's floor: Jolt's `VehicleConstraint` with its wheeled controller, through
`forge-physics` (`World::add_vehicle`, `drive`, `wheels`, `engine`). The chassis is a 1200 kg
convex hull of the car's shell with its weight 25 cm low; four wheels of 31 cm on springs of
1.6 Hz (0.55 damped) travelling 30 cm, found by casting a cylinder down, so they ride over edges
a ray would drop through; the front two steer by up to 31° and are driven through a
differential by an engine of 320 N·m up to 6500 rpm and its gearbox; anti-roll bars front and
back. Brakes of 1600 N·m, the handbrake 4000 N·m on the rear wheels. The car and its wheel were
modelled in Blender from code (`assets/blender/car.py`, a hatchback with its wheel arches cut
from the lower body; `assets/models/car.glb`, 100 KB) and drawn as the boat is: the body moves
with the chassis, each wheel is a mover placed where Jolt puts it, turned by its steering and its
roll.

The track: a 14° jump ramp ahead, a slalom of eight barrels to the right, and a wall of 32
crates in four courses at 95 m. **The arrow keys** drive (up the throttle, down the brake and,
once stopped, reverse; left and right steer), **Space** holds the handbrake, **C** lets the
camera go from behind the car. The throttle and the steering are the boat's command (`Steer`)
and the handbrake one of its own (`Handbrake`), so a drive records, replays and goes through
`--net`; `--steer T,S` holds them from the first frame.

![Down the track at tick 300, and turning past the slalom at tick 600](images/physics-lab-drive.png)

A test drives off, turns and replays (the car 10 m on and upright, the same digests the second
time); the lab's test drives the track with a turn, the handbrake and a throttle astern and
replays it. A tick of the track (73 bodies): **0.06 ms** on average, p99 0.16 ms.

## `fly`: an aeroplane (issue #141)

```
cargo run --release -p physics-lab -- --lab fly
```

A light high-wing aeroplane (7.3 m long, 10 m span, 750 kg; `assets/blender/plane.py`,
`assets/models/plane.glb`, 124 KB) on a 520 m runway across a field of grass 5 km wide. It flies
on its surfaces (`forge_physics::aero`): each wing's half, the tailplane and the fin is a plate in
the aeroplane's frame, and the air past it, from the body's motion and its turning, gives it an
incidence. Its lift follows the thin-wing law (2π a radian) up to the stall near 14°, then falls
to a flat plate's by 20°; its drag is a parasitic part, the drag its lift induces (by its aspect
ratio) and the plate's broadside drag. The elevator, the ailerons and the rudder add to the
incidence of their surface. As in the buoyancy, the incidence is carried by its sine and cosine
taken from the airflow, so the forces are the same bits everywhere. The wing is rigged 2° up with
4° of dihedral (a high wing's effective dihedral), the tailplane 1° down; the propeller pulls
3 kN at full throttle at the nose, and the fuselage and the gear drag besides.

On the ground it rests on the hull of its three wheels, a tail skid and its fuselage, with its
weight just ahead of the main wheels (`Shape::with_center_of_mass_at`); its friction is its
wheels' (0.04), and off its wheels (low and banked or turned over past 45°) it scrapes on the
field with 0.6 of its weight. **W** and **S** open and close the throttle, **the arrows** are the
stick (down pulls the nose up: half of the elevator, all of it with **Shift**; left and right
roll), **A** and **D** the rudder; the camera follows 17 m behind. The controls are a command
(`Fly`), and `--pilot T,E,A,R` holds them from the first frame.

![Down the runway at tick 600, and climbing away at tick 1200](images/physics-lab-fly.png)

With full throttle and the stick 0.4 back it rotates near 24 m/s, lifts off near 30 m/s after
about 300 m, and climbs at 5 m/s; the ailerons roll it at up to about 37° a second, a banked turn
holds its bank and loses height unless the stick comes back, and a full pull from the keys
stalls it (hence half the elevator by default). The lab's test takes off, banks right, climbs
past 20 m and replays to the same digests. A tick of the field: **0.05 ms** on average, p99
0.10 ms.

## `break`: a wall, a wrecking ball, a column (issue #142)

```
cargo run --release -p physics-lab -- --lab break
```

Phase 3's step 6, its first part. A wall of 324 bricks (21.5 × 6.5 × 10.25 cm, 2.6 kg) in
running bond, 24 courses on a 3 m run, every brick a body held to the bricks beside it, under it
and over it, and the bottom course to the floor, by mortar: 912 joints, Jolt's fixed constraints
through `World::join_fixed`. After each step the lab reads what every joint carried in it
(`World::joint_loads`, one call) and where its bricks are, and breaks (`World::set_holding`)
the joints that carried more than 2.5 kN or 150 N·m, or whose bricks moved apart by 3 mm or
turned by 1.5° from where they were laid: an iterative solver's joints yield a little under a
blow and pass on less of it than rigid mortar would, so the strain catches the cracks the load
alone misses. The wall's joints take 30 velocity and 10 position iterations of the solver
(Jolt's per-constraint override; the world's 10 and 2 let 24 courses sag and lean); standing,
it settles by under a centimetre at its top in the first half second and goes to sleep. Jolt
saves whether each joint holds, and its impulses, with the world: a broken wall restores, replays
and goes through `--net` like the rest.

A 3 t steel ball (0.45 m) hangs on a 5.5 m chain (a distance joint) from a yellow gantry, held
back 60° by one more joint; **Space** lets it go (then throws balls, as elsewhere), and it meets
the wall at about 7 m/s just past the bottom of its swing. Behind the wall stands a concrete
column (0.4 × 2.4 m, 920 kg), cut ahead of time into 14 pieces: the Voronoi cells of points
spread up it (`forge_geom::fracture`: the column clipped by the planes halfway between each
point and the others, in `f64` without trigonometry, each piece a convex hull in the physics
and a mesh whose cut faces are drawn paler). The pieces wait asleep on a shelf far under the
floor; when a blow changes the column's velocity by more than 1.5 m/s in a step (gravity
aside), they take its place, each with the velocity of its part of the column, and the column
goes to the shelf. Nothing is added or removed from the world while it runs, so the state keeps
its size and its order. `--release N` lets the ball go at frame N (the captures).

![The ball held back, through the wall at tick 85, and at ticks 100 and 300: the hole, the column in pieces](images/physics-lab-break.png)

At tick 300, five seconds on, 412 of the 912 joints are broken: the lower courses and the
ends stand round a ragged breach, the column's pieces lie behind; by tick 600, after the ball's
swings back, 349 joints hold. The tests: a joint carries its load (a beam
of two boxes out of a wall: each joint the weight and the turn it should) and lets go when
broken; a broken joint is mended by a restored state, which then runs to the same bits; a ball
on a chain swings at its length; the cuts keep a box's volume and the Voronoi cells fill the
column without overlap; and the lab's wall stands untouched for two seconds, then breaks under
the ball and replays to the same digests. A tick through the impact (372 bodies, 912 joints):
**0.99 ms** on average, p99 2.0 ms, at most 2.5 ms; the state is 87 KiB.

## Captures

The batch (`tools/captures.sh`, set `lab`) takes `lab-drop90` (the rain in mid-air), its A/B
twin with the occlusion off (`lab-drop90-noocc`, 0 px apart: the movers are culled like
everything else), `lab-drop600` (the pile at rest) and `lab-net300` (the client's view through
`--net 100 --throw-every 45`: thrown balls in flight, the bot's corrections behind it), on both
geometry paths; and from the sea (#138) `lab-sea300` (what floats and the rocks on the floor) with
its occlusion-off twin, and `lab-sea-steer600` (the boat under way with the rudder over); and from
the playground (#139) `lab-walk150` (halfway up the stairs) with its twin, and
`lab-walk-crates240` (through the light crates); from the track (#140) `lab-drive300` with its
twin and `lab-drive-turn600`; from the field (#141) `lab-fly1200` (climbing away) with its
twin; and from the break scene (#142) `lab-break85` (the ball through the wall) with its twin
and `lab-break300` (the wall broken, the column in pieces).
