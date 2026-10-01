# #115: a tributary's straight edge at a confluence (2026-10-01)

Before (left, 2a6ee33) and now (right), the logged `confluence` view (`--view
3047,19.3,1407,-107.5,-20`) and from 40 m higher (`--view 3047,59,1407,-107.5,-45`), frame 60:
`confluence.png`; `confluence-zoom.png`, the junction from the logged view, 4× (before, now, the
pixels that differ).

**What it was.** The channel is carved through: the ground under the tributary's last 24 m
stands 10.4–10.7 m, under its water's 11.25 m. The band across the junction was the
tributary's own water, thinning:
- its ribbon faded out per point (4 m apart) from 4 m outside the main river's reach, which is
  under the main's banks past its water;
- where its coverage was partial it blended with the dry bed (grass since the water draws its
  bed, #114);
- the main river's water could not cover it. The tributaries were drawn first and the
  tributary's surface stands at or a hair above the main's, so it kept the depth
  (`GREATER_OR_EQUAL`).

**What changed.**
- The rivers are drawn per river, the largest first. The chunks of 64 segments never span two
  rivers, and only a river's own neighbouring chunks merge into one draw (`river_chunks`, the
  runs in `water.rs`).
- A tributary's water is whole to a metre from the main river's water edge and gone 3 m inside
  it (`levels` in `river.rs`, measured from the main's half width, not its reach).

So the tributary's thinning water blends over the main's water: from the logged view the
tributary opens into the main river, and from 40 m the junction is joined without its box end.
ꟻLIP mean 0.0073 on the logged view, 0.0021 from 40 m, 0.0052 from 3 m.
