# #187: the car sinks and slows in mud

`physics-lab --lab yard --fixed-step` (`docs/demos/physics-lab.md`, "`yard`"). The car's beds
are now ground: the car rides on the layer and sinks into the ruts it presses. Its mud is a deep
puddle (15 cm of soft mud), and it slows there.

`mud.png`: tick 240, from the yard's camera, the car in its mud, sunk 10 cm into it, its
berms beside it.

- **The beds as ground:** each of the car's beds is a Jolt height field of every other point of
  its layer (4 cm). It is edited in place (`Shape::height_field_editable`,
  `World::set_heights`) box by box, wherever a sample moved over a millimetre. It sits in a
  layer only vehicles' wheels feel. A restore or a reset gives it its layer whole, as Jolt's
  saved state holds no shape.
- **Sinking to a level:** a press sinks to the material's depth less its pressure over its
  stiffness, wherever it lands. A wheel standing still therefore sinks no further.
- **A wheel's press** is flat across its tread and 2 cm wider than the tyre each side. Along its
  heading it follows the wheel's round, from where the wheel was at the step's start to where it
  will be at the next one's end.
  - **The first try** pressed only to where the wheel was. The wheel then climbed the front of
    its own rut every step, rode 5 cm high in the snow, and stuck at the mud's edge.
  - **With a rounded tread,** the tyre's edges rode on the rut's sides and berms.
- **Slowed:** each wheel in a ground is held back against its travel, at its contact, by its load
  times 0.45 · √(z / 2r), where z is how far it sank. Each ground grips as its material does:
  sand 0.4, snow 0.2, mud 0.15, against the floor's 0.2.
- **On the autopilot** (3 m/s, throttle at most 0.6) the car keeps its pace through the snow,
  slows from 2.9 to 1.2 m/s in the mud, and is back over 2.4 m/s past the sand.

**Costs:** a tick 0.236 ms over 1 200 (p99 0.71), 0.37 ms over the first 600 while the car
crosses. Three fixes on the way brought it down from 0.83 ms:
- **Height-field updates:** a box per wheel, and only samples that moved, instead of one box
  round all four (0.2 → 0.1 ms).
- **Ground resolution:** the grounds at 4 cm instead of the layer's 2 cm. The wheels' cylinder
  casts against 2 500 triangles had cost 0.4 ms a step.
- **Slump:** at most 32 passes, on a box shaped to the pad.

The frame is 1.90 ms at 1600 × 900.

**Tests:**
- a crate sinks where its height field is lowered under it;
- a pad pressing again where it stands sinks no further;
- the car's pace and sinking through its beds, replayed to the bit.

Left: wheels spinning and digging in, rocking out of a rut, the dogs' beds as ground, tread
patterns, water pooling in the ruts.

**Tier 0** (`captures/verify/20261008-131814-0abc932`, 421 tests): the yard's three images changed
as meant (FLIP mean 0.09 to 0.29: the car rides on its beds, lower in the mud), everything else
0 px but the #71 flake.
