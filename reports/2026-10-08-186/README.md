# #186: mud and wheel tracks

`physics-lab --lab yard --fixed-step` (`docs/demos/physics-lab.md`, "`yard`: footprints and
wheel tracks in sand, mud and snow"). The dogs now walk over beds of damp sand, mud and snow.
Beside them the drive lab's car crosses its own three beds on an autopilot, from the snow. The
arrow keys take it over, and C follows it.

`ruts.png`: tick 600 from over the car's snow, its ruts through all three of its beds and the
dogs' prints beyond. `yard.png`: tick 600 from the yard's camera, the car parked past its sand.

- **Mud** (`Soft::MUD`): 5 cm of wet soil. A dog's paw would sink 4 cm. It packs 5 % of what a
  pad pushes out (its water does not compress) and heaps the rest, and its walls stand at 56°.
  Dark, with a sheen.
- **Wheel contacts** (`World::wheel_contacts`): each wheel touching the ground, its normal, the
  way it rolls, and its load from the suspension's impulse over the step. At rest the four
  carry the car's weight.
- **Wheel tracks:** each tick, each wheel on a bed presses its tyre's patch (20 cm by 14 cm, about
  135 kPa) along the stretch it rolled over in the step, its rim beside it only.
  - **The first try** pressed a patch at the contact each tick. It left a chain of separate
    stamps, with lumps heaped between them, like a tank's tracks (the owner's word).
  - **The contact also jumped:** Jolt's contact lies anywhere across a flat tyre's width, up to
    18 cm sideways from one tick to the next, which made the ruts twice the tyre's width. The
    patch is now pressed under the wheel's middle.
  - **Two more changes:** the stretch follows the wheel's own travel, so the car's drift leaves
    no steps. The slump now runs until nothing moves (64 passes at most), so no heap stays a
    spike.
- **Heights go up only when a bed changes,** and once more after: 584 KB at a time, 160 KB a frame
  on average.

Over 26 s the car's ruts reach the floor in all three of its beds. The berms stand 3.9 cm over
the sand, 2.2 cm over the snow and 9.4 cm over the mud. The dogs' prints are 1.2 cm deep in the
sand and 1.9 cm in the mud, and in the snow they reach the floor.

**Costs** (1 200 ticks): a tick 0.134 ms (p99 0.26); `skin/vertices` 0.021 ms for 206 000
vertices, `skin/blas` 0.130 ms; the frame 1.81 ms at 1600 × 900.

**Tests:** a car's wheels carry its weight where they touch; a rolling wheel ploughs one
unbroken rut with berms beside it; mud heaps the highest rim; the yard's dogs print their three
beds and the car ruts its three, two ruts under its wheels and none between, replayed to the bit.

Left: tread patterns; the physics feeling the beds (sinking, slipping, the car slowing in mud);
water pooling in the ruts; a bound on what a car driven back and forth heaps in the mud.

**Tier 1** (`captures/verify/20261008-123757-be5b311`, 419 tests): the yard's images changed as
meant (FLIP mean 0.32–0.33: mud, the car and its beds, a new view), `lab-yard240` new, mesh
against fallback 0 px; everything else 0 px but the #71 flake.
