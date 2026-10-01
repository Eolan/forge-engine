# #114: the rivers' banks, beds and lake entries (2026-10-01)

Before (left, da572f7) and now (right), `city-blocks --island 7`, frame 60 at a fixed step.

- `beds-plain.png`: a river across the coastal plain from 40 m (`--view -929,45,-4399,83,-40`):
  the blurred, jagged strip of bed texels is gone, the river a clean band (ꟻLIP mean 0.020).
- `beds-lake.png`: the logged lake entry from 50 m looking down (`--view -1700,50,-3072,89.9,-62`)
  and from 45 m (`--view -1718,45,-3072,89.9,-40`): a crisp shore and silty water where there
  was a brown smear and a jagged grass edge (0.14 and 0.14).
- `beds-mouth.png`: the largest mouth from 45 m (`--view -5138,45,-258,118.3,-45`): the channel
  reads as shallow water over sand (0.054). The long dark line across the sand on the left is
  there before and after; not looked into yet.

What changed is in `docs/demos/island.md`, "The rivers' banks, beds and lake entries": the bed
drawn by the water per pixel instead of the 4 m layer map, banks shaped by the bend (point bar
inside, cut bank outside), channels shoaling into and out of lakes.

For the owner's eye: the lakes' shallows under a metre now show the ground under them turned to
silt (the map's mud only deeper), and the shallow river water over sand at the mouths is paler
and greener than it was over the old gravel texels.
