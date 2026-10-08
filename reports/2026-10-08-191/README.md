# #191: wheels spinning and digging in

`physics-lab --lab yard --fixed-step`, both images from `--view=5.6,1.2,-4.8,121,-27`, beside the
car's sand. `bogged.png`: tick 500, the car floored in its sand, its front wheels spinning at 8
m/s past it, sunk into the holes they dig, sand heaped round them. `dug.png`: tick 720, the car
gone: its ruts through the sand deeper and gouged where it bogged down, toward the far end.

- **Spin** (`World::wheel_spins`): each wheel's spin from Jolt. A wheel's slip is how much faster
  its tread runs over the ground than it travels along it. The front wheels drive through an open
  differential, so the one with less grip spins: at the start on the floor, and in the mud and the
  sand as the car crosses (to 7 m/s past its 1.9).
- **The dig** (`deform::Layer::dig`): past 0.3 m/s of slip, a wheel tears 4 mm from under its
  patch's middle for each metre it slips (never below the material's least), its hole's floor
  along its travel the wheel's round, and throws everything it tears the way its tread slides: a
  heap from the hole's end out to 60 cm, highest a third of the way, then slumped. Nothing is made
  or lost. Its lugs fade as its slip nears 1 m/s and are smeared past it.
- **Why round:** a flat hole slumped to the sand's 45° walls, which a wheel on a ray (#188) meets as
  a 45° slope under its middle. It needed a grip of 1 to climb out, and the car sat in its holes
  for good, wheels screaming at 23 m/s. Faster digging traps it the same way: from 5 to 8 cm down a
  round's climb asks more grip than the sand's 0.63 (7 mm a metre slipped and more).
- **The car's sand is deep** for it: 12 cm, a wheel's 105 kPa sinking 4 cm. Its 3 cm was pressed to
  its least already, so a spinning wheel had nothing to dig.
- **The autopilot** stops in that sand, all four wheels on it, and pulls away at full throttle a
  second later: the front treads run up to 21 m/s past the car, which bogs down at 0.3 m/s for most
  of a second while they dig 2.4 cm below the ruts and throw the sand back, then claws out and is
  at 2.3 m/s past the sand. In the deep mud a standing car could never pull away: its grip moves it
  with less than a quarter of its weight, which ploughing that mud takes. It crosses the mud on its
  way in, now slowed to 1.25 m/s (1.5) by its spinning front wheel tearing at it.
- **A save mid-spin replays** (a bug since #187): the car's grounds lag their beds by up to a
  millimetre, and a restore rebuilt them from the beds, so a car standing on them went its own way.
  The lab now saves and digests the grounds as Jolt holds them.

**Costs:** a yard tick 0.266–0.273 ms over 1 200 (p99 0.79–0.85), against 0.216–0.236 for a56c474
timed alike.

**Tests:** a spinning wheel digs its hole and throws it behind, nothing made or lost, never through
the base, a wheel's hole round along its travel where a flat one slumps steep; the yard's car stops
in its sand, its front treads run over 5 m/s past it, its holes reach 2 cm below its ruts, and it
gets out; a save taken while it spins goes on to the same digest.

Left: the spray a spinning wheel throws (particles), grip from the hard ground under a dug hole.
