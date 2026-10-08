# #188: tyre treads in the ruts

`physics-lab --lab yard --fixed-step`. The car's tyres leave their tread in its ruts. On the
way, its berms were evened out further and the car stopped catching on its ruts.

`tread.png`: tick 600, along the car's mud ruts: chevron lugs in their floors, low smooth berms
beside them. `yard.png`: tick 600 from the yard's camera.

- **The tread** (`deform::Tread`): chevron lugs 9 mm deep every 8 cm (four of the car's beds'
  points a lug, coarse enough not to shimmer). A wheel's press lays them along the ground where
  it rolled, placed by their position along the path, so each step, and the wheel after it,
  press the same lugs.
- **A relief, not the thickness:** pressed into the layer's thickness, the lugs snagged the
  wheels, which stuck in the mud, and the smoothing wore them away. They now go into the layer's
  relief, which is drawn over the thickness, saved and digested with it, and nothing stands on.
  Where a rut reaches the floor (snow, sand), a lug is kept above half the material's least:
  there a 6 mm lug cut through it, and the floor showed beneath in jagged patches.
- **The wheels on rays** (`VehicleDesc::ray_wheels`): the trace of a stall showed every tyre's
  contact normal tipped 30–40° sideways in the mud. The tyres rode on the ruts' walls, which the
  ground's 4 cm samples coarsen inwards. Each wheel now finds the ground with a ray down from its
  middle. The crossing no longer hangs on small changes, and a tick costs 0.211 ms instead of
  0.271.
- **Berms evened further:**
  - **The weight, not the swinging load:** a wheel presses with its share of the car's weight.
    Its load this step swung as the car rocked, so its ruts and berms rose and fell 5 to 8 mm in
    waves 32 cm long.
  - **Smoothing kept off the wall tops:** the smoothing works only in the band where a press
    heaps its rim, not at that band's inner edge. Heaped against the top of a snow rut's wall,
    it was poured by the slump into the rut's foot in a 12 mm sawtooth, which the sun dashed.
  - **The result:** flanks step 0.1 to 0.6 mm a point (1 mm after #189, 2.4 mm before it), and
    the snow ruts' walls are even.
- **Hold-back:** with the wall scraping gone, the rolling resistance carries the slowing alone:
  0.6 · √(z / 2r). The car slows from 2.9 to 1.5 m/s in the mud.

**Tests:**
- a treaded wheel's lugs come a pitch apart, the ground under them smooth, and a second pass
  leaves them;
- the yard's walk asserts both mud ruts' berm flanks under 1 mm a point and the snow ruts' walls
  even, the crossing and its slowing.

Left: finer treads on finer beds, a tyre's tread smeared when it spins.

**Tier 0** (`captures/verify/20261008-142745-35f657a`, 422 tests): the yard's three images changed
as meant (FLIP mean about 0.05), everything else 0 px but the #71 flake.
