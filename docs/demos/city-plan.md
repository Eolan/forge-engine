# City plan — a city's layout before any building (`forge-procgen::city`)

The first step of D-059: a city's plan made from a site and a parameter record, judged as a map
image before any building exists (`docs/research/city-generation.md`, "Recommendation for
Forge", stages 1–5, and its extension §7–§19). It is engine-generic. The example below is an
invented coastal city made for the engine's tests and docs.

```
cargo run --release -p city-map -- --size 4000 --scale 2 --out captures/city
```

![The example city's plan](images/city-plan-example.png)

*The example city at 4 m a pixel.*
- **Water:** the sea, a beach, a river with its quays.
- **Blocks by district:** an organic old town on the river, perimeter blocks round courtyards
  (the pale insides), a waterfront, an eco-quarter, courtyard blocks.
- **Parks:** a river park, and the parks the 3-30-300 rule added.
- **Streets:** traced as streamlines of a tensor field. Eight bridges.
- **Transit:** two tram lines with their stops, a boat crossing.
- **Landmarks:** two red diamonds.

## What it does

| Stage | How | Module |
|---|---|---|
| Districts | each land cell to the anchor nearest relative to its radius, edges roughened by noise | `city::district_field` |
| Street pattern | a tensor field: a grid per district at its angle, rings at radial squares, the water's tangents near the coast and rivers (Chen et al. 2008) | `city::tensor` |
| Streets | main roads traced first (bridging rivers up to 260 m), then streets at each district's spacing; each stops half a block short of a parallel street, and its end joins the nearest street ahead | `city::streets` |
| Road graph | streets cut where they cross or meet, ends snapped, dead ends under 60 m pruned; blocks as the graph's bounded faces | `city::graph` |
| Blocks | quays where a face holds water; parks from park districts, then from the 3-30-300 rule (a park of 1 ha within 300 m of every built block); a courtyard per block, inset by half the widest street round it and a building's depth, once the streets' profiles are known | `city::blocks` |
| Lots | recursive splits across each piece's long axis (Vanegas et al. 2012); only the pieces on a street are kept where a block has a courtyard | `city::blocks` |
| Forest | ground above a height or steeper than a slope goes to the first reserve district (a real site's hills) | `city::district_field` |
| Transit | tram lines by the shortest path through their waypoints, main roads preferred (a waypoint far from any main road snaps to the nearest street); stops every 450 m moved to nearby junctions; lines added until a target share of blocks has a stop within 400 m (`transit::cover`); boat crossings where a river runs long without a bridge; street profiles (tram avenue, avenue, shared, pedestrian, path) | `city::transit` |
| Masses | each lot's building: the lot cut by the building line of each street it touches (half that street's profile width from its centreline), less the courtyard's hull (or a building's depth along its street where no courtyard fits); floors from the district's range, capped by the style bible's height-to-width ratio on the street it fronts; green, solar or terrace roofs | `city::mass` |
| Style bible | one TOML record of the city's rules (height to street width, green roofs, green area per block, park and stop reach, the streets' share, per-district floors, lots, spacing and walls), put into the plan's parameters and measured by a checker that names what fails | `city::bible` |
| Trees | rows along the streets by profile (two on avenues every 9 m, one on shared streets every 11 m, every 18 m in lanes), none within 10 m of a junction; a jittered grid in parks (13 m, two thirds filled), on the quays (14 m, under half) and in courtyards (16 m, half) | `city::trees` |
| Lamps | along every street by profile (both sides of avenues every 28 m at 8 m, one side of shared streets every 30 m, lanes and paths lower), at the pavement's edge, none in junctions: where step 5's lights will stand | `city::lamps` |
| Maps and numbers | a small rasteriser: the plan (with the districts', landmarks' and lines' names in the engine's font, kept from overlapping), a checks map (blocks by what they lack), an oblique massing view of any window; the plan's numbers and its digest | `city::map` |

Everything is a pure function of the site and the parameters. Costs are integers in the
router, and `forge_core::dmath` does the trigonometry. Two runs give the same digest (D-016).

## Numbers (the example, 4 km, release build, one thread)

| | |
|---|---|
| Land | 14.3 km² |
| Main roads / streets | 59 km / 149 km, 8 bridges |
| Built blocks / parks | 674 (mean 1.66 ha) / 48 |
| Lots / blocks with a courtyard | 13 323 / 563 |
| Parks' share of the land | 10.5 % |
| Built blocks with a 1 ha park within 300 m | 100 % |
| Built blocks within 400 m of a tram stop | 45 % (two lines, no line added) |
| Streets' share of the land | 21.5 % |
| Tram lines | 7.8 km, 19 stops |
| Masses / floor area | 14 700 / 20.5 km² (14 ms; floors capped by street width) |

| Stage | Time |
|---|---|
| districts | 2.3 ms |
| tensor field | 7.5 ms |
| main roads | 2.3 ms |
| streets | 8.6 ms |
| road graph and faces | 10.3 ms |
| blocks and parks | 5.5 ms |
| transit | 0.2 ms |
| courtyards and lots | 5.8 ms |
| **the whole plan** | **≈ 42 ms** |

## Tests

`cargo test -p forge-procgen city`:
- the example has streets, bridges, blocks, parks, lots and trams;
- no block's reference point is in the water;
- every built block has its park within reach;
- the plan is the same every time;
- no two roads cross except at a node;
- lots sit inside their blocks;
- added lines raise the stop coverage by over 20 points;
- masses stand inside their blocks and outside their courtyards.

## The style bible's checks (the example)

```
ok   height at most the street's width times the ratio: 1.000
ok   green roofs' share: 0.507 (limit 0.500)
FAIL built blocks at their green area: 0.788 (limit 1.000); 143 failing, e.g. block 10: 0.06, …
ok   built blocks with a park within reach: 1.000
FAIL built blocks within 400 m of a tram stop: 0.454 (limit 0.900)
ok   streets' share of the land at most: 0.215 (limit 0.250)
```

The old town's small blocks have no room for a courtyard, so they miss the green area; the
example adds no tram lines, so it misses the stops. A city's own bible and its added lines
(`max_extra_lines`) close those.

## Massing

![The example's centre as masses](images/city-plan-massing.png)

*A kilometre round the example's centre, seen obliquely from the south: perimeter blocks of
4–8 floors round their courtyards, green, solar and terrace roofs, the river and a bridge, two
tram lines.*
## In 3D

```
cargo run --release -p city-plan
```

![The example city in 3D](images/city-plan-3d.png)

*The example city's masses, trees, gardens, parks, streets, tram tracks and river, drawn by
city-blocks' renderer (sky, clouds, ray-traced soft shadows, probes, TAA) through its scene
interface.* `demos/city-plan` turns any plan and its masses into meshes in tiles of 250 m: the
site's land and water as runs of cells, blocks and street ribbons, walls and roofs (green,
solar, terrace). The rivers are smooth strips along their centrelines; the sea and lagoons come
from the site's cells. A private sandbox hands its own plan to `PlanScenario::new`.

| | |
|---|---|
| Meshes / triangles / trees | 484 / 309 k / 36 717 instances of one tree |
| Built and cooked | 1.3 s |
| GPU frame at 2800 × 1575, this view | 3.84 ms (3.33 without trees or facade textures) |
| p50 / p99 frame | 3.67 / 7.28 ms |

![Closer, 35 m up](images/city-plan-3d-near.png)

*35 m up: the walls' window bays and shopfronts are two generated textures
(`forge_render::textures::facade` and `shopfront`) read through the walls' coordinates, a shop
band on the ground floor along avenues and lanes, window bays above, four render tints of the
palette chosen by lot. Variety in the facades themselves comes with D-039's modules.*

![At eye height](images/city-plan-street.png)

*At eye height on the river avenue (`--street`), late afternoon and midnight: buildings behind raised
pavements with kerbs, shopfronts and shuttered window bays, a row of plane-like trees each side,
tram tracks, the tree-lined quay and the river; at night the shopfronts and windows glow (lamps
stand along the streets, their heads in the crowns, and light nothing yet: step 5). Next at this
height: finer trees, street furniture, the modules (#247).*

![Night, three ways](images/city-plan-night.png)

*Midnight (`--time-of-day 1.5`), 35 m up: about 40 % of the open windows lit, warm or cool, and the
shopfronts, from emissive maps beside the facade textures. Left, metered; middle, a fixed EV 3;
right, without the probes. The probes take the windows' light into the city many times over
(the walls and courtyards glow as by day), so night views run with `--no-probes` until step 5
gives the renderer emissive light in absolute units and the probes take it right (#254). The
emissive is scaled by the frame's reference illuminance for a run held at one time of day: a
stopgap.*

## Next (#245–#247)

- Squares at the radial districts' centres; the landmarks' sightlines.
- More checks on the checks map; a legend.
- The style bible's palette into the materials; more rules (cornice lines, opening rhythm) once facades exist (#246).
- Then buildings from the grammar (D-039, #247).
