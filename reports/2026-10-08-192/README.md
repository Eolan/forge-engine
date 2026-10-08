# #192: the spray a spinning wheel throws

`physics-lab --lab yard --fixed-step --view=5.6,1.2,-4.8,121,-27`. `spray.png`: tick 520, the car
floored in its deep sand (#191), its front wheel spinning some 10 m/s past it and throwing a fan of
sand grains back out of its arch.

- **A new splash kind** (`SplashSource::Thrown`, `SPLASH_GRAIN`): #107's particles, emitted and
  moved on the async compute queue and drawn as streaked sprites, now also grains of the ground.
  A grain carries its colour in its flags (5:6:5 bits of the colour's square root, so a mud's
  darks keep fine steps), is lit as a Lambertian lump (two thirds of a surface facing the sun times
  the Lambert sphere's phase law, through the sun's shadow ray, plus the sky's light) rather than
  scattering like a drop, falls with little drag (terminal 15 m/s), and is gone where it lands on
  its ground's level: no foam, no rings.
- **The yard's sources** (`yard::sprays`): each wheel slipping past 0.3 m/s on a bed throws its
  bed's grains, 300 a second for each m/s of slip, from behind its contact, back the way its tread
  slides, rising at 34° and leaning out of its arch (under the body they would pass through it),
  at 15 to 50 % of its slip and at most 6 m/s, with the wheel's velocity. Sand grains 4–12 mm, mud
  clods 6–18, snow 5–14, in the beds' colours.
- **Without water:** the demo makes the splashes for the yard too and runs them on the sea's clock;
  their update moved out of the water's block, unchanged for the scenes with water.

**Costs:** `splashes/emit` 0.002 ms, `splashes/advance` 0.003, `splashes/draw` 0.008 with some 3 000
grains in the air. Nothing on the physics tick: the spray is visual only.

**Tests:** a wheel throws coloured grains at its rate (a stream of 600 a second gives 60 in a tenth
of a second, its colour packed, none out of reach); the yard's car throws no spray while braked in
its sand and from both front wheels once floored.

Left: the grains landing on the beds as material (the dig heaps what it tears already).
