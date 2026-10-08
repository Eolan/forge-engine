# #185: footprints in sand and snow, a deformable layer (D-007)

`physics-lab --lab yard --fixed-step` (`docs/demos/physics-lab.md`, "`yard`: footprints in sand
and snow"). The course's two dogs walk their lanes on the plain floor, over a bed of damp sand
and then a bed of fresh snow, turn about past the snow and come back. Each footfall (#167's
foot-down events) presses its pad into the bed it lands on.

`yard.png`: tick 1200, under the yard's low sun (26°). Two rows of prints in the snow, one a
lane, pressed through to the floor; in the sand, shallower cups with lit rims round the near
dog's paws.

- **The layer** (`forge_physics::deform`): a bed is a grid of its material's thickness over the
  floor, a point every centimetre, thinning to nothing over its last 6 cm. A pad sinks as deep as
  its pressure over the material's stiffness. What it pushes out is packed (snow 85 %, sand 25 %)
  or heaped in a rim. Then the print slumps to the material's steepest slope: 45° for damp sand,
  72° for snow. No transcendental function; saved, digested and replayed with the lab.
- **Drawn by displacement:** a bed is a grid mesh cooked as a skinned mesh of one joint. The skin
  pass raises its vertices by the layer and turns their normals to its slopes, and the ray
  tracing's copy is refitted, so the prints take the sun's shadows.
- **A fix on the way:** the resolve built a skinned mesh's normal-map detail about its bind
  pose's normal. On a displaced ground that normal is flat up, so a textured bed lost its relief
  (the first sand showed no prints). It now turns that normal onto the raised surface's normal.
- **Another:** after a restore, the next footfall went to the print ring's first free place, not
  its slot (the ring was not saved). The ring now has its 96 slots from the first footfall, empty
  until filled. Before, it only misplaced a drawn print; now a print presses a bed.

Over 26 s the dogs press 24 footfalls into the sand and 26 into the snow. The sand's prints are
1.2 cm deep with rims up to 4 mm; in the snow the paws go through to the 4 mm left over the
floor, with rims up to 6 mm.

**Costs** (1 200 ticks): a tick 0.113 ms (p99 0.19) against the course's 0.100; `skin/vertices`
0.011 ms for 130 000 vertices, `skin/blas` 0.107 ms for the dogs' and the beds' four structures;
the frame 1.72 ms at 1600 × 900; the heights' upload 349 KB a frame.

**Tests:** five on the layer (a pad's sink, the rim against packing, the slump, the same bits),
one on the displaced cook (each cluster's own bounds), the yard's walk printing both beds and
replaying to the bit, and the lab's save, reset and recorded replay with the beds.

Left: the layer clip-mapped round the player on the island; mud; wheel tracks; the weather
refilling it; friction, sinking and sound from the material row; a print shaped like the paw.

**Tier 1** (`captures/verify/20261008-113938-bc0a82b`, 417 tests): the yard's four images are
new, mesh against fallback 0 px; everything else 0 px but the #71 flake. A Tier 2 milestone is
due (seven commits since the last).
