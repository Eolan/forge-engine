# #184: a flyer, gulls on their wings' lift

`physics-lab --lab flyer --fixed-step` (`docs/demos/physics-lab.md`, "`flyer`: gulls on their
wings"). Three gulls modelled in Blender (`assets/blender/bird.py`) circle the lab 8 m up; the
camera follows the first.

`gull.png`: tick 300, from behind the first gull as it banks into its turn, its wings beating.

- Each gull is one Jolt body of 0.75 kg. Its clip's pose places five flying surfaces each tick
  (each arm, each hand, the tail). The air past each includes the surface's own beat, so a
  beating wing pulls the gull on: over a beat at 10 m/s the wings lift 6 to 7 N and pull 0.2 N,
  and the same pose held still only drags.
- A balance torque holds it facing its flight, its wings at an angle of attack, banked to turn
  onto its circuit, as the dogs are held up. Held to a pitch against the horizon, it sank until
  its wings stalled.
- It beats when low or slow, glides when high and fast. Over 60 s each keeps 5.5 to 10.6 m up
  and 21 to 44 m from the middle, beating 86 % of the time.

**Costs** (610 ticks): a tick 0.048 ms (p99 0.11) for the three; the frame 1.44 ms at
1600 × 900.

**Tests:** a beat pulls the gull on where the held pose drags, and a glide carries about its
weight; over 60 s the gulls keep their height and circuit and replay to the bit.

Left: the three fly the same flight turned a third of a turn apart; landing and taking off; the
wind.

**Tier 1** (`captures/verify/20261008-105305-a953e31`, 409 tests): the flyer's four images are
new, mesh against fallback 0 px; everything else 0 px but the #71 flake.
