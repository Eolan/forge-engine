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
| `rocket` | a rocket off a launch pad: thrust vectoring, roll jets, fins as flying surfaces | ✅ #148 |
| `space` | a sci-fi spaceship in zero g over a planet under the stars: momentum kept through a crash into floating crates | ✅ #150 |
| `break` | destruction: a brick wall held by mortar that breaks, a wrecking ball, a concrete column that shatters | ✅ #142 |
| `creatures` | powered ragdolls: mannequins on stands and dogs modelled in Blender, their motors driving moving poses | ✅ #143 |
| `flood` | a dam break: the authoritative shallow-water model, drawn as fresh water, carrying what floats, which pushes it aside | ✅ #144, #151 |
| `tank`, `tank-bench` | a dam break in a glass tank: the GPU's particle liquid (D-044), drawn through the glass; the same tank as a bench to tune by | ✅ #156 |
| `dominoes` | an advanced test: a 300-domino run on a spiral that ends the same, replayed | ✅ #146 |
| `bridge` | an advanced test: a timber bridge that stands empty and collapses under a convoy of cars, replayed | ✅ #147 |
| `tug --net 100` | an advanced test: a tug-of-war on one sled, this player against the bot over a lossy link | ✅ #149 |
| skinned creatures, the GPU's shallow water, splashes | the later steps of the plan | planned |

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
`assets/models/plane.glb`, 124 KB) on a 520 m runway across a field of grass 40 km wide (5 km
until #148, when its corner showed from the rocket's height). It flies on its surfaces
(`forge_physics::aero`): each wing's half, the tailplane and the fin is a plate in the aeroplane's
frame, and the air past it, from the body's motion and its turning, gives it an
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

## `rocket`: a rocket off a pad (issue #148)

```
cargo run --release -p physics-lab -- --lab rocket
cargo run --release -p physics-lab -- --lab rocket --pilot 1,0.1,0,0
```

Step 5's rocket, on a concrete pad in the aeroplane's field.
- **The rocket:** a white body 13.5 m tall with an ogive nose (a lathe), four swept red fins, 3 t
  fuelled, its weight 5 m up its axis. Its meshes weigh their normals 16 times a hard surface's
  in the simplification. With the default weight, the line between its lit and shaded sides
  moved a pixel as the cluster levels changed while it turned, which the owner saw shimmer in
  `--lab space`: 108–138 px a frame changed differently from the full-detail drawing, now 0–15.
- **Its engine:** pushes 50 kN (1.7 times its weight) along its axis from the nozzle.
- **The stick and the roll jets:** the stick swings the engine up to about 6° (thrust vectoring),
  as for an aeroplane pitched up on its tail:
  - pushing tips the nose downrange (−z);
  - the rudder yaws it;
  - jets at the nose roll it, 2 kN·m at full aileron.
- **The air:** the fins are flying surfaces (`forge_physics::aero`, the aeroplane's), so they turn
  it into its wind. Its body drags along its axis and across it.
- **Its controls:** the aeroplane's, from the keys or `--pilot T,E,A,R`, so a flight records,
  replays and goes through `--net`.
- **The camera:** follows from 30 m off its right, its pitch downrange crossing the view.

![On the pad; climbing off it at full throttle, the stick a tenth pushed; pitched over at 6 s and 10 s](images/physics-lab-rocket.png)

The lab's test:
- **On the pad:** the rocket stands untouched.
- **The climb:** at full throttle it climbs 85.68 m in 5 s straight up, where thrust and weight
  alone (½ (T/m − g) t², the drag under 300 N) say 85.71. Jolt's default linear damping had taken
  7 m of it, so the rocket has none: its drag is the air's.
- **The stick:** pushed for a second, it tips the rocket 3.8° downrange, where the fins hold it,
  339 m up at 10 s.
- **The replay:** the flight replays to the same digests.

A tick: **0.04 ms** on average, p99 0.08 ms.

Not yet:
- fuel burning off (Jolt's mass would change in flight);
- the exhaust's flame (it needs particles or an emissive material);
- a spaceship in zero g: `space` below.

## `space`: a spaceship in zero g (issue #150)

```
cargo run --release -p physics-lab -- --lab space
cargo run --release -p physics-lab -- --lab space --pilot 1,0,0,0
```

Step 5's last vehicle: a spaceship in a world with no gravity (`WorldDesc::gravity`) and no air.
It was the rocket until the owner's look of 2026-10-03 ("make the space scene look like space,
and spaceship looks like a more sci-fi spaceship").
- **The sky:** space's, in place of the ground's. The asteroids' starfield
  (`forge_render::Starfield`) draws the stars, the sun's disc and an Earth-like planet low on the
  left under its air (D-023), seen from about 5 600 km up so its oceans, land and clouds show. Its
  nebula is at a quarter of the asteroids' (`Starfield::faint`): the chase camera looks along the
  Milky Way's band, which at full strength lay over everything like a cloudy sky. The sun is
  unfiltered white, from the ship's right. There is no floor and no cloud. The shaded sides take
  space's constant fill, occluded by GTAO: no sky's light, no probes.
- **The ship:** a 16.9 m fighter-shuttle with a 13 m span, modelled in Blender from code
  (`assets/blender/ship.py` → `assets/models/ship.glb`):
  - a faceted hull with a dark canopy and glowing strips along its sides;
  - swept wings with blades turned down at their tips;
  - two engine nacelles and a main engine in the tail;
  - running lights, red and green.
  Its glowing parts are emissive (the glTF reader now takes a material's emission and its
  strength). Its collision is the hull of a coarse shell in the model. Its meshes weigh their
  normals as the rocket's do (#157).
- **Its engines:** 60 kN together along its nose (4 t, 1.5 g). Each flame slides out of its nozzle
  with the throttle, hidden inside it when closed.
- **Its jets:** a flight computer's. The stick asks for a rate of turn (pitch and yaw 0.8 rad/s,
  roll 1.4 rad/s at full stick), and the jets push towards it (150 kN·m per rad/s off, at most
  80 kN·m in pitch and yaw, 60 in roll). Centred, it holds still. These are the aeroplane's
  controls, so a flight records and replays.
- **The crates:** 27 of them float ahead of it in a block (100 kg each, 1.2 m apart).
- **No damping:** the solver damps nothing, not the ship and not the crates. The jets push no
  linear momentum.

So what it tests is momentum:
- the ship must gain exactly what its engines give it;
- when it ploughs through the crates and scatters them, the ship and the crates together must keep
  that momentum.

![At full throttle over the planet, the crates ahead; through them at tick 150; turning and rolling over the planet at half throttle](images/physics-lab-space.png)

The lab's tests:
- **Untouched:** half a second leaves everything where it was.
- **The burn:** 60 kN for 1 s gives 60 000.004 kg·m/s along −z (60 000 by Newton).
- **The crash:** the ship coasts into the block and scatters it, 15 crates leaving at over 1 m/s.
  4 s on, the ship and crates together carry 59 999.973 along −z and 0.001 across: within half a
  part in a million.
- **The replay:** the flight replays to the same digests.
- **The handling:** half the stick to the right rolls it at 0.7 rad/s within 2 s about its nose
  alone, and let go it stops within 1.5 s.

The state log prints the momentum every few seconds (`momentum`). A tick: **0.061 ms** on average,
p99 0.142 ms. Its frame on the GPU: 0.55 ms at 1600 × 900, the starfield and the planet 0.14 ms of
it.

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

## `creatures`: powered ragdolls (issue #143)

```
cargo run --release -p physics-lab -- --lab creatures
```

Phase 3's step 7, its first part: D-012's physics layer, creatures as ragdolls whose motors
drive them to a pose. Two puppets were modelled in Blender from code
(`assets/blender/creatures.py`, `assets/models/creatures.glb`, 309 KB): a 1.8 m artist's
mannequin of pale wood with dark joints, and a 1 m dog, eleven rigid parts each, every part a
mesh and every joint an empty (the glTF loader now keeps the empties' positions,
`Model::point`). In the lab each is a Jolt ragdoll through `forge-physics`
(`World::add_ragdoll`): a body per part (the hull of its mesh), held to its parent by a ball
joint (shoulders, hips, neck, waist, the dog's legs and tail) or a hinge (elbows, knees, the
dog's lower legs) with its limits, a part not colliding with its parent. Every tick
`World::drive_ragdoll` sets each joint's motor to a target: the mannequins swing their arms,
bend their elbows and turn their heads, the dogs wag their tails and nod, each creature at its
own phase (the angles through `forge_core::dmath`, the same bits everywhere). The motors are
springs of a stiffness in N·m a radian (800 for the mannequin, 4000 for the dog's legs, which
carry its 45 kg torso) rather than of a frequency: Jolt scales a frequency spring by the light
part each joint turns, and the dogs folded under their own weight.

Three mannequins stand on poles, their pelvis held by a joint that lets go past 2 kN or
400 N·m; two dogs stand on their own legs in front. **Space** throws balls at them: a dog is
shoved and finds its pose again, a mannequin hit squarely comes off its pole. **↓** lets every
motor go (a `Limp` command, so it records, replays and goes through `--net`): the dogs fold to
the ground, the mannequins hang from their poles; **↑** powers them again. `--limp-at N` lets
them go at frame N (the captures).

![Posed at tick 120; struck by a ball every 50 ticks (the middle mannequin knocked off its pole); limp from tick 60](images/physics-lab-creatures.png)

The tests: an arm on a ball joint and a hinge holds out its pose on its motors to a few
millimetres, hangs when they let go (from a saved world, to the same bits twice) and bends its
elbow to a target; the lab's creatures stand for two seconds (the mannequins on their poles,
the dogs' torsos over 45 cm), fold when let go (under 35 cm) and replay to the same digests. A
tick (55 parts in five ragdolls, balls thrown): **0.11 ms** on average, p99 0.21 ms. Skinned
creatures that bend instead of being jointed (GPU skinning, Phase 7's first step) and a slime
as a soft body are the step's second part.

## `flood`: a dam break (issue #144)

```
cargo run --release -p physics-lab -- --lab flood
```

Phase 3's step 8, its first part: D-009's middle tier of water, the authoritative column model,
which the server runs and the clients predict like the rest. `forge_physics::shallow` keeps a
column of water on each cell of a grid and the water's velocities on the faces between cells,
after Matthias Müller-Fischer's height-field water (GDC 2008): each step the velocities are
carried along by themselves, the water flows through each face at its velocity times the depth
it leaves, each cell's outflow scaled so that it never gives more than it holds, and the slope
of the surface speeds the faces up; a face into a cell whose bed stands over the water carries
nothing. Two first-order fixes made its fronts run: a still face is traced back along the
velocity of the water arriving at it, and a dry cell the water runs into passes its speed on.
The volume is kept to the rounding and no column goes below zero; it is `f32` sums, products
and floors in a fixed order, saved, restored and digested with the world.

The scene: a basin 48 m by 24 m walled in concrete, a cell every 25 cm (192 × 96), a reservoir
2 m deep behind a red gate at its upper end, four concrete blocks and a brick hut downstream;
18 crates, 12 barrels and 9 logs, two thirds afloat behind the gate and a third lying on the
dry floor beyond it. **Space** (or `--release N`) lifts the gate: the water runs down the basin,
white where it is fast, round the blocks and the hut, and carries what floats; the buoyancy is
the sea's (#138) with the pool's surface and its flow in place of the waves (fresh water,
1000 kg/m³). **Enter** fills the reservoir again.

The water is drawn by the island's water pass as fresh water, a pool (#144): each frame the
lab hands the renderer its columns' surface, depth and velocity (`WaterSurface::set_pool`, 295
KB), two triangles between each four samples at the surface, faded in over the last 3 cm of
depth so its edge thins to nothing. A dry sample stands at its bed, so the front thins onto the
floor. Where its bed is the top of a block, the hut, a wall or the gate, it stands instead at
the water beside it, so the water runs on level into the obstacle, hidden there. Before
2026-10-03 the water climbed the obstacle's side in one cell, a grey sheet over the hut's lower
half (the owner's report). The rivers' shading (`fresh_water`) gives it the scene seen
through it, the sky and sun on it, its ripples carried on its flow and white water past 2 m/s.
The scene draws no sea (`WaterSurface::set_sea`).

![Before the gate lifts; the water running out at tick 75 and 150; spread round the blocks at tick 300](images/physics-lab-flood.png)

The tests: a dam break in a channel keeps its volume and never a negative depth; it follows
Ritter's solution where the dam stood (a depth of 4⁄9 of the reservoir's within 5 % and ⅔ √(g h₀)
within 10 %) and its front runs 3.25 m in the first second (Ritter's tip runs 6.3 m; a
first-order scheme lags where the water thins to nothing); a saved pool runs on to the same
bits; still water floats a box at its level. The lab's flood keeps its reservoir behind the
shut gate, then in four seconds puts a third of it down the basin, every drop kept, carries what
floats more than 5 m on, and replays to the same digests. A tick (18 432 cells in two half
steps, 39 floaters): **0.86 ms** on average, p99 1.1 ms. The GPU's own shallow-water layer
shadowing the model near the player, and particles for splashes, come next.

**What floats pushes the water aside (#151, two-way coupling).** The water no longer only pushes
what floats. After Müller-Fischer's height-field water (GDC 2008), as the column model already is:
- **The thickness:** each tick, each floater's volume under the water (its buoyancy over ρg) is
  spread as a thickness over the wet cells of its footprint (`Pool::displace`). The footprint is
  three quarters of its hull's reach round its centre of mass: a crate's about its side, a log's
  wider than the log.
- **The slopes:** that thickness adds to the surface the water's slopes see. The water flows out
  from under a body and rises round it: a body set down sends out a ring, and one carried along
  pushes water ahead of it.
- **The volume:** the depths keep it; the thickness is not water.
- **Buoyancy:** a body afloat reads the same surface, so at rest it sees the level it would have
  without it and sinks no deeper.
- **The state:** the thickness is saved with the pool, so a run still replays to the bit.

Each tick now pushes what floats by the water as it stands, then lets the water be pushed aside,
then steps the water.

The new test: 0.4 m³ set at once into still water 1 m deep in a 10 m square, over a footprint
0.6 m round.
- **The ring:** 1.6 m out, it rises over 5 mm within the second.
- **The volume:** kept.
- **Settled:** 40 s on, the water under the body is 0.6 m deep, and its surface and the far water's
  stand level, 4 mm over the first (0.4 m³ over 100 m²).

The lab's flood still carries what floats more than 5 m on and replays.

A tick: **1.12 ms**, against 0.90 for the same run before (measured the same day). The new order
alone gives 1.00 ms, so much of the rest is the floaters moving differently, more of them
jostling.

## `tank` and `tank-bench`: a dam break in a glass tank (issue #156)

```
cargo run --release -p physics-lab -- --lab tank
cargo run --release -p physics-lab -- --lab tank-bench
```

D-044's first milestone, after the owner's answers of 2026-10-03: the water is particles on a
grid, simulated and drawn on the GPU (`forge_render::liquid`, `shaders/liquid.slang`,
`shaders/liquid_draw.slang`), visual and lab-only.

**The scene.** A tank 1.6 m long, 0.6 m tall and 0.6 m deep inside, on a table, with 0.4 m of
water behind a red gate 60 cm from its left end. The owner asked for it larger than the first
1.0 × 0.6 × 0.5 m, with the same depth behind the gate, so 1.8 times the water. **Space** (or
`--release N`) lifts the gate at 3 m/s. The water runs out along the floor, climbs the far wall
to the rim, falls back, sloshes and settles. `tank-bench` is the same tank as a bench to tune by
(the owner's ask, after Sebastian Lague's fluid videos):
- no glass, frame or table to see;
- a floor of 10 cm squares in four tints, a darker line every metre, to read distances off;
- a plain violet background and the sun alone;
- the water tinted teal (`LiquidLook::tinted`) so its depth and motion show.

**The solver** (APIC on a MAC grid, Jiang et al. 2015):
- **The particles:** 589 824, eight a cell, on a 1.25 cm grid (128 × 48 × 48 cells). That spends
  the owner's budget of answer 3 (~640 000, "bigger or finer") on the larger tank;
  `--liquid-cell 0.01` gives 1.15 million.
- **The substeps:** four of 1/240 s per tick of the lab's, with the gate's height and speed each.
- **The transfers:** trilinear weights, the affine vector per component. The faces' sums are
  64-bit fixed-point atomics (the weight and the momentum packed in one add), so a run replays to
  the same bits.
- **The pressure:** 32 red-black Gauss–Seidel sweeps with over-relaxation 1.7, warm-started
  from the last substep. The still water starts at its hydrostatic pressure.
- **The volume:** each cell's density, against a full cell's, is the particles' crowding. The
  particles move down its gradient, a quarter of it undone a substep, inside the water only
  (the surface's part-full cells are left alone). A cell against the glass expects an eighth
  less per solid side.
- **Gravity:** `--liquid-gravity x,y,z`, 9.81 m/s² down by default. At `0,0,0` the block floats
  where it stands: there is no surface tension yet.

**What it took to be still.**
- **The volume correction as a velocity:** first written as Ten Minute Physics writes it, a target
  for the pressure's divergence. Still water then shook itself apart in half a second (the
  owner's report: "it's always moving for no reason"). The correction went into the particles'
  velocity every substep, a spring with nothing to damp it. As a move it adds no energy: the still
  water stays at rest to the bit (rms speed 0.000 m/s over 540 ticks).
- **Correcting crowding only:** water that splashed apart settled 20 % high, its sparse cells
  holding their volume like full ones.
- **Tiled pressure sweeps:** sweeps in groupshared tiles, 8 cells a side, cost a third as much.
  They left errors on the tiles' edges that kept the water sloshing (0.4 m/s rms after 12 s,
  against 0.05), so they were dropped.

**The drawing.**
- **The density field:** the particles' density, smoothed twice by a 3 × 3 × 3 binomial, with
  4-cell bricks of its least and most to skip air and the water's inside.
- **The march:** each pixel's ray through the glass (Fresnel's reflection, its tint), into the
  water where the density crosses one half. There it is bent by Snell's law, with Fresnel's share
  mirrored (the sky, the sun's highlight).
- **Through the water:** absorbed and scattered along its path (pure water by default), out through
  the surface or the glass. Past the critical angle it is mirrored whole and marched on: a side
  wall seen at a slant mirrors the inside.
- **Where it lands:** a short march over the screen against the depth, with a 25 cm thickness, so
  something in front of the ray is not taken for where it lands.
- **The floor:** the floor's glass lies on the table and mirrors nothing.
- **For TAA:** a reactive mask where the surface moves.

![The glass tank as the wave climbs the far wall; the bench as the gate lifts and as the wave climbs the far wall](images/physics-lab-tank.png)

**The checks** (fixed step, `--liquid-log N` prints the line every N ticks):
- **The particles:** none lost.
- **The still water:** at rest to the bit before the gate lifts (rms and greatest speed 0.000 m/s).
- **The settled level:** 149.2 mm nine seconds after the gate lifts, against the 150.0 mm its volume
  gives over the whole floor. D-044's check asks within 2 mm. The column model of `flood` fails it.
- **The replays:** three runs give the same digests at all 100 logged ticks (every 6, 600 frames).
- **The front:** from the gate to the far wall (0.98 m) in 0.47 s; between ticks 42 and 54 it
  runs 3.0 m/s, three quarters of Ritter's 2√(g h₀) = 3.96 m/s for a dam removed at once (this
  gate lifts at 3 m/s, letting the water go over a tenth of a second).

**The cost** on the RTX 5070 Ti, 1600 × 900, on the async compute queue (600 frames):

| Zone | ms a frame |
|---|---|
| `liquid/p2g` | 2.16 |
| `liquid/pressure` | 1.19 |
| `liquid/g2p` | 1.00 |
| `liquid/faces`, `cells`, `project`, `clear` | 0.13 |
| `liquid/draw` (graphics) | 0.16 |

That makes 4.5 ms of simulation, more than D-044's 2.5 ms estimate for 190 000 particles (answer
4: to be found by trying). The particles' sums to the grid are the most of it. Where the time
could go:
- sorting the particles by cell once a frame, and summing a workgroup's into groupshared memory
  first;
- a multigrid pressure in place of the sweeps.

## `dominoes`: a run that ends the same (issue #146)

```
cargo run --release -p physics-lab -- --lab dominoes
```

The first of the advanced tests the plan suggests ("a domino run that ends the same on two
machines"). 300 wooden dominoes (64 × 32 × 8 cm, 650 kg/m³) stand on a spiral from 3 m out to
8 m, 40 cm apart along it (five eighths of their height), placed and turned through
`forge_core::dmath`, so the run starts from the same bits everywhere. **Space** (or `--release
N`) tips the first. The fall runs round the spiral at about 2.5 m/s, six dominoes a second,
and the last is down at tick 2 940, 49 s on; the dominoes fall asleep where they lie.

![Standing; the fall a turn in at tick 900, two turns at 1800; all down at tick 3000](images/physics-lab-dominoes.png)

What it tests is the solver's determinism over a long chain of cause and effect, thousands of
contacts made and broken in turn, where one bit off anywhere would show at the end: the lab's
test runs the fall out (none falls untouched for a second; pushed, all 300 are down within 50 s)
and replays it from its recording to the same 25 digests; `--net` runs it through a server and a
client. That the world's hash does not depend on the workers, on a save and restore, or on the
platform is the pile's test (#136, Windows and Linux in CI). A tick: **0.21 ms** on average, p99
0.38 ms (332 bodies, those falling and fallen awake until the run is over).

## `bridge`: a convoy on a bridge that gives way (issue #147)

```
cargo run --release -p physics-lab -- --lab bridge
```

The second advanced test ("a bridge collapsing under a convoy"), where the car (#140) meets the
joints that break (#142). The bridge:
- **The deck:** 16 timber panels (4.4 m × 1 m × 12 cm, 600 kg/m³) spanning a 16 m gap between
  two banks 4 m high.
- **Its joints:** each panel is joined to the next, and the end ones to the banks, by fixed
  joints that break as the wall's mortar does.
  - That rule is now shared code (`lab/bonds.rs`): a joint breaks on its load (here 120 kN or
    78 kN·m) or its strain (12 mm or 1.3°), and Jolt saves whether each holds.
  - The panels settle under their own weight for five seconds as the scene is built: laid without
    their load, the joints swing past their load at rest on the way (93 kN·m against 55).
- **The convoy:** four of the track's cars wait on the near bank, held where they stand.
  - **Space** (or `--release N`) lets them go.
  - An autopilot keeps each on the middle line (against its offset and heading) at 4 m/s.
  - It stops each past a line on the far bank, or at once behind a car that went down.

The empty deck carries 55 kN·m at its worst joint. It holds the first car (up to 71 kN·m) and gives
way about 4.7 s after the cars set off, when the second is on it too:
1. the far bank's joint goes first;
2. the deck swings down from the near bank with both cars on it;
3. then the rest of its joints break as it lands.

The two cars behind stop on the near bank, the third 2 m short of the edge.

![The first car on the deck; the second on it too, the deck sagging; falling; in the gap, the two behind stopped](images/physics-lab-bridge.png)

The lab's test checks the deck standing for a second untouched with the cars held. Then it lets
them go and checks, ten seconds on:
- most of the joints broken (16 of 17 at the time of writing);
- two cars down and none across;
- the run replayed from its recording to the same digests.

The wall's and the track's images did not change with the shared code (0 px). A tick: **0.11 ms**
on average, p99 0.23 ms, max 0.34 ms (52 bodies, 4 vehicles).

## `tug`: a tug-of-war over the network (issue #149)

```
cargo run --release -p physics-lab -- --lab tug --net 100
```

The third advanced test ("a networked tug-of-war on one crate at 100 ms"): the netcode (#137) on
a body two players fight over.

**The game:**
- A 200 kg sled sits on the floor (friction 0.4, 785 N to start it moving) between two ropes, on
  the middle of three lines 3 m apart.
- Each team pulls along its rope with up to 2 kN: the left team as player 0 (this player) says,
  the right as player 1, each with a `Pull` command.
- The pulls are held in the world's state like the other controls, so they are saved, restored and
  digested with it.
- Both start holding at half. Once the sled's middle is over a line, that team has won and both let
  go.

**Locally:** ← pulls with all the left team's strength and → eases to a fifth, against a right
team that holds.

**Over `--net 100`:** the right team is the bot, pulling hard and easing off to 0.3 every 1.5 s.
This player's client predicts the sled, but learns of each change of the bot's a link late:
- every snapshot after one disagrees with the client's prediction (the pulls are in the digest,
  and so is where the sled went);
- the client goes back to the server's state and runs the ticks since again.

Holding at half, this player loses: the sled is over the right line by tick 300.

![Both holding; the bot pulling hard, the sled on its way right at tick 200; over the right line at 300; still there at 600, the game over](images/physics-lab-tug.png)

`--net 100`, 600 ticks, this player holding:

| | |
|---|---|
| snapshots taken by this player's client | 99: 93 predicted to the bit, 6 corrected (one for each change of the bot's pull), 84 ticks run again |
| a correction (the state restored, 14 ticks run again) | at most 0.55 ms |
| a tick: the server and the two clients | mean 0.12 ms, p99 0.30 ms, max 0.66 ms |
| commands taken by the server | the bot's 6, none late |
| bytes down to a client | 6.0 KB a snapshot (one sled), 60 KB/s |

The lab's tests:
- **Locally:** a second with both teams holding leaves the sled on the middle line. The left team
  pulling with all its strength (2 000 N against 1 000 and the friction) wins within 3 s, and the
  run replays to the same digests.
- **Over the lossy link:** this player eases and then pulls harder while the bot changes its pull
  six times. The server takes all eight commands in time, the client corrects at least six times,
  and it ends where the server is, to the bit.

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
twin; from the break scene (#142) `lab-break85` (the ball through the wall) with its twin and
`lab-break300` (the wall broken, the column in pieces); and from the creatures (#143)
`lab-creatures120` (posed) with its twin, `lab-creatures-throw240` (struck by balls) and
`lab-creatures-limp240` (let go); from the flood (#144) `lab-flood150` (the water running
down the basin) with its twin and `lab-flood300` (spread round the blocks); and from the
dominoes (#146) `lab-dominoes900` (a turn down) with its twin and `lab-dominoes3000` (all down);
and from the bridge (#147) `lab-bridge360` (the deck falling with two cars) with its twin and
`lab-bridge600` (in the gap); and the rocket (#148) at full throttle with the stick a tenth
pushed, `lab-rocket120` (climbing off the pad) with its twin and `lab-rocket600` (pitched over
downrange); and the tug-of-war (#149) through `--net 100`, `lab-tug-net200` (the sled on its way
right) with its twin and `lab-tug-net600` (over the line); and the spaceship (#150) at full throttle,
`lab-space90` (closing on the crates) and `lab-space150` (through them) with its twin;
and the glass tank (#156), the gate lifted at tick 31, `lab-tank90` (the wave climbing the far
wall) with its twin and `lab-tank-bench300` (the bench, the water settling).
