# #112: a coastal plain in the island's uplift (2026-10-01)

The owner's judgement (`owner-valley.png`, `owner-lake.webp`): the rivers read as small streams,
not natural or integrated. D-041 (accepted) shapes the uplift; this is its first part. Each
sheet is the island before (left, 5d443e1) and with the plain (right), the waves held at frame
60. `city-blocks --island 7 --island-plain 0` still gives the island before.

- `plain-far.png`: the island from 2.5 km (`--view -6500,2500,-1416,-90,-35`), and the first
  view (the beach it finds).
- `plain-mouths.png`: each island's own logged largest mouth (`into_sea`) from 4 m and from
  60 m up, and its gentlest mouth (`gentle_sea`) from 4 m.
- `plain-rivers.png`: on the plain island, down a lowland river (`--view -929,15.2,-4399,83,-10`),
  the logged lake entry (`--view -1718,15.0,-3072,89.9,-20`), and down a river from the hills
  (`--view -3083,48.4,-376,123.7,-10`).

## What changed

The uplift's square root lifted the coast's foothills from the shoreline, so the island's hills
rose straight out of the sea: the largest river's floor stood at 24.7 m only 160 m inland, and
every river fell 9–24 % over its last 160 m (#109). Now:
- over the first quarter of the radius inland the uplift stays at 3 % of the hills' starting
  rate (`IslandParams::plain`, `plain_uplift`);
- that width wanders along the coast over its 5 km scale (`plain_wander` 3), from none, where
  the hills still meet the sea as cliffs, to about twice as wide;
- the hills rise from the plain's inner edge on the square root eased in over its first
  eighth, so they leave the plain on a slope rather than a wall.

## Numbers (seed 7)

| | before | with the plain |
|---|---|---|
| 8 m: mouths | 25 | 19 |
| 8 m: falls over the last 160 m | 9–24 %, all over 5 % | 1–5 %, none over 5 % |
| 8 m: rivers, the widest | 43, 14 m | 50, 17 m |
| 8 m: lakes of a hectare or more | 14 | 11 |
| 8 m: the water most under its banks | 3.9 m (621 points over 2 m) | 3.5 m (125) |
| 8 m: heights | −60 to 534 m | −60 to 525 m |
| 16 m (`genesis`): rivers, to the sea, lakes | 44, 27, 7 | 46, 18, 3 |

`genesis --plain 0` gives the island before, the same digest (2d17199dba8598bf).

## For the owner's eye

- The mouths: calm water running out through the sand into the surf, where the largest river
  came down in a white cascade.
- The plain: rivers winding across it, joining before the coast; the massif keeps its torrents
  in V valleys.
- The sand rule (gentle ground under 2.5 m) reaches further inland around the mouths; its
  edge still shows the 4 m layer map's texels from 60 m up.
- Open, next: the water far up a valley from low (#113), the channels' banks and the bed's
  layer (#114), the mouths' seam from very low (#110), floodplains and the riparian strip.

## Checks

Listed in the commit; the batch changes the island's images only.

## Later: the riparian strip

`riparian.png`: before (left, da572f7) and with the banks' growth and #114's beds (right), from
300 m over the plain (`--view -1100,300,-4600,60,-35`) and 3 m over a lowland river
(`--view -929,15.2,-4399,83,-10`). The grasses within 6 m plus two of the river's widths of its
water become a deeper, bluer green of reeds and shrubs (`island_layer::RIVERBANK`); from 300 m a
darker corridor follows each river.
