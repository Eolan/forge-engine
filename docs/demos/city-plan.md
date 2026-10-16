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
| Blocks | quays where a face holds water; parks from park districts, then from the 3-30-300 rule (a park of 1 ha within 300 m of every built block); squares: the built block nearest each built district's centre and each landmark, and small ones through old towns 180 m apart (`add_squares`); once the streets' profiles are known, a buildable outline per block on each street's building line (`geom::inset_by`) and a courtyard a building's depth inside it | `city::blocks` |
| Lots | recursive splits across each piece's long axis (Vanegas et al. 2012); only the pieces on a street are kept where a block has a courtyard | `city::blocks` |
| Forest | ground above a height or steeper than a slope goes to the first reserve district (a real site's hills) | `city::district_field` |
| Transit | tram lines by the shortest path through their waypoints, main roads preferred (a waypoint far from any main road snaps to the nearest street); stops every 450 m moved to nearby junctions; lines added until a target share of blocks has a stop within 400 m (`transit::cover`); boat crossings where a river runs long without a bridge; street profiles between building lines (tram avenue 24 m with 6 m pavements, avenue 18 m with 6 m pavements (5 until the owner found the trees too close to the windows), shared street 10 m, pedestrian 8 m, path 4 m: narrowed on 2026-10-11 after the owner found too much road, the default height-to-width ratio raised from 1.5 to 1.75 to keep the homes) | `city::transit` |
| Masses | each lot's building: strips of its district's building depth along each street the lot fronts (lots are cut inside the block's building lines, half each street's profile width from its centreline); floors from the district's range, capped by the style bible's height-to-width ratio on the street it fronts; green, solar or terrace roofs | `city::mass` |
| Style bible | one TOML record of the city's rules (height to street width, green roofs, green area per block, park and stop reach, the streets' share, per-district floors, lots, spacing, building depth, tiled roofs and walls), put into the plan's parameters and measured by a checker that names what fails | `city::bible` |
| Trees | rows along the streets by profile (on avenues one a side by the kerb, 0.8 m into the pavement, in slots every 8 m that alternate with the lamps, so a tree every 16 m; one down the middle of shared streets every 14 m and of lanes every 18 m), each crown keeping 2.5 m from the building line, none within 10 m of a junction; a jittered grid in parks (13 m, a little over half filled), on the quays (14 m, under half) and in courtyards (16 m, half). None within 3 m of a lamp or 4 m of a river's water, and the parks', quays' and courtyards' none within 1.5 m of a road or path (the owner saw trees on the water, the roads and the paths, and a few too many). The demo keeps them 6 m clear of the tram stops | `city::trees` |
| Lamps | along every street by profile: on avenues in the kerb rows' free slots between the trees, a lamp every 16 m a side, staggered, 8 m tall with two arms (over the carriageway and over the pavement), lit only underneath; one side of shared streets every 30 m, lanes and paths lower, at the pavement's edge; none in a junction (within its widest street's half width and 4 m) and no two within 10 m (two streets' lamps stood side by side in the corners, the owner's second walk): where step 5's lights will stand | `city::lamps` |
| Maps and numbers | a small rasteriser: the plan (with the districts', landmarks' and lines' names in the engine's font, kept from overlapping), a checks map (blocks by what they lack), an oblique massing view of any window; the plan's numbers and its digest | `city::map` |

Everything is a pure function of the site and the parameters. Costs are integers in the
router, and `forge_core::dmath` does the trigonometry. Two runs give the same digest (D-016).

## Numbers (the example, 4 km, release build, one thread)

| | |
|---|---|
| Land | 14.3 km² |
| Main roads / streets | 59 km / 149 km, 8 bridges |
| Built blocks / parks / squares | 667 (mean 1.66 ha) / 48 / 12 |
| Lots / blocks with a courtyard | 11 224 / 605 |
| Parks' share of the land | 10.5 % |
| Built blocks with a 1 ha park within 300 m | 100 % |
| Built blocks within 400 m of a tram stop | 45 % (two lines, no line added) |
| Streets' share of the land | 18.0 % |
| Tram lines | 7.8 km, 19 stops |
| Masses / floor area | 12 614 / 19.0 km² (13 ms; floors capped by street width) |

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

**Superblocks and open blocks (both off in the example).** Where the traced streets run far
apart, a block's courtyard became a park of a hectare or more, and its fronts ran for hundreds
of metres. The owner found the courtyards unrealistically large, and the long rows of big
buildings too close, on their second walk (2026-10-11). A plan can now set two options:
- `CityParams::max_block`: a built block longer than this many times its district's street
  spacing gets a shared street across its middle, again until none is (four rounds at most).
  In the private sandbox, at 1.6: 2 148 → 2 764 built blocks, the mean 1.67 → 1.31 ha, homes
  919 k → 1.07 M people with the open blocks below.
- `DistrictSpec::gaps` (open blocks, Portzamparc's *îlot ouvert*): a lot between a block's
  corners left as a garden at the district's share, and wherever a block's front has run
  `DistrictSpec::run` (120 m when not given) since the last gap; never two side by side. Under
  60 m a corner lot may be a gap too: separate buildings round a shared green rather than a
  ring (the owner's photos of open blocks: open, organic, not oppressive). Light, views and
  people reach the courtyard, the walls beside a gap get windows, and in 3D a paved path runs
  from each gap to the court's middle. It is paved with the block's forecourt as one surface off
  the streets. Drawn over the paving, it began on the street's centreline, lay across the
  carriageway on some seeds, and showed as a patch on the pavement (the owner). Test:
  `the_court_paths_are_paved_with_the_forecourts_off_the_carriageways`, four seeds. The sandbox
  opens its modern districts (runs of 35–55 m, gaps 10–25 %) and keeps its centre and old town closed: about
  1.05 M people (977 k once the stair cores took their floor, step 3).

**Footprints.** A footprint's corner sharper than 55° is chamfered where its wedge is 6 m
wide, or at 0.45 of its shorter side: a building came to a point at a block's sharp corner by
a bridge (the owner: an impossible building). Each footprint is then cleaned of repeated and
collinear points within a centimetre (`geom::clean`): ear clipping stalled on them, and 1 372
roofs lost big triangles (the owner saw them). Tests: `no_building_comes_to_a_point`,
`every_footprint_triangulates_whole`.

## In 3D

```
cargo run --release -p city-plan
cargo run --release -p city-plan -- --village 600   # the first map's village (D-062, #266)
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
| Meshes / triangles / trees | 484 / 720 k (309 k before the afternoon of 2026-10-11: dropped kerbs, paved corners, quay walls, the rivers' banks) / 27 379 instances of one tree (36 717 before the avenues' trees alternated with lamps) |
| Built and cooked | 1.3 s |
| GPU frame at 2800 × 1575, this view | 3.84 ms (3.33 without trees or facade textures) |
| p50 / p99 frame | 3.67 / 7.28 ms |

![Closer, 35 m up](images/city-plan-3d-near.png)

*35 m up: the walls' window bays and shopfronts are two generated textures
(`forge_render::textures::facade` and `shopfront`) read through the walls' coordinates, a shop
band on the ground floor along avenues and lanes, window bays above, four render tints of the
palette chosen by lot. Whole bays only, centred: the wall left at each end, and all of a wall
shorter than a bay, is plain plaster (the end's part of a bay put a window and its shutter at
the corner, the owner); walls onto a neighbour's lot are plain too. Variety in the facades
themselves comes with D-039's modules.*

![At eye height](images/city-plan-street.png)

*At eye height on the river avenue (`--street`), late afternoon and midnight: buildings behind raised
pavements with kerbs, shopfronts and shuttered window bays, a row of plane-like trees each side,
tram tracks, the tree-lined quay and the river; at night the shopfronts and windows glow (lamps
stand along the streets, their heads in the crowns, and light nothing yet: step 5). Next at this
height: finer trees, street furniture, the modules (#247).*

![Night, three ways](images/city-plan-night.png)

*Midnight (`--time-of-day 1.5`), 35 m up: about 40 % of the open windows lit, warm or cool, and the
shopfronts, from emissive maps beside the facade textures. Left, metered; middle, a fixed EV 3;
right, without the probes. The probes took the windows' light into the city many times over
(the walls and courtyards glowed as by day), so night views ran with `--no-probes` (#254). The
emissive was scaled by the frame's reference illuminance for a run held at one time of day: a
stopgap.*

**Emissive in cd/m² (2026-10-15, #254).** The windows', shopfronts', screens' and lamps' rows
give their luminance in cd/m² (`RenderLayer::emissive_absolute`). The renderer converts it each
frame by the reference illuminance it shades in (D-046: L·π / E). A running day (`--day 40`) at
21:00 now shows the lamps and the lit windows as `--time-of-day 1.25` does. Before, they kept the
noon reference's scale, about nothing at night. At a fixed hour nothing changes: 72 pixels of
1.4 M.

**The probes at night (2026-10-15, #254).** The probes' rays read each hit's emission at the
row's full value, but the frame no longer shows it so:
- a window's glass shows its room lit at a share of the lamp (4 % then, 25 % since the residents
  light their own; twice that for a shop);
- a far facade emits only through its lit windows, the emissive map's few texels.

The hits now emit as the frame shows them: the emissive map's average (its last mip, as the
albedo), and a window's lamp share over walls about half as bright as white. At 21:00 from 30 m,
the facades that glowed as by day are dark now. The street lamps light their streets and the
facades beside them, and the lit windows dot the city. Night views run with the probes again.

## Buildings from the grammar and the kit (D-039, #247)

```
cargo run --release -p city-plan -- --street --detail 450
cargo run --release -p city-plan -- --rules my-style.rules   # another style set's rules
```

![The kit's buildings](images/city-plan-kit.png)

*Within `--detail` metres of the view (450 here), each mass is built from the first style set's
rules and kit; beyond, it stays a textured box. Top left, a lane at 16:40: French windows with
louvred shutters, balconies on consoles with wrought-iron railings on the first floor,
shopfronts with fascias, a string course over the ground floor, the cornice, sun shadows
across it all. Top right, the same lane at night, lit windows, without the probes (#254).
Bottom left, the river avenue's shops, entrances and awnings. Bottom right, from 60 m: the
kit's quay front, the textured masses behind.*

Three stages, each generic:
- **`forge_procgen::city::facades`, the component split.** Each mass's walls become faces that
  look onto:
  - the **street**: outside the wall is street, up to the building line;
  - the **courtyard**: the wall looks into its own lot or block;
  - a **party wall**: another lot's building stands within 3 m of it.

  Walls between two pieces of one lot are inside the building and get no face, if the lot's
  other pieces cover them all along: each wall is probed at five points. A single probe at
  its middle dropped 816 of 2 627 walls covered only in part, and the courtyards saw into
  the buildings, their window frames floating (the owner's second walk). A wall with another
  lot's building within 3 m straight out is a party wall, blank, whatever lot lies between: a
  slit between two buildings had windows facing a wall a metre away. Farther, the wall looks
  over the neighbour's garden and gets its windows: a wall onto a neighbour's lot was blank
  for the law's 1.9 m to a direct view, and the courts showed blank walls on two sides of four
  (the owner, 2026-10-15: "would not match if real apartments were inside"). A street face
  knows the corners it turns, and each building's longest street face holds its entrance.
  `city-map` draws the split as `faces-centre.png`.
- **`forge_procgen::grammar`, a split grammar** with CGA's operators (Müller et al. 2006):
  - scopes split along x or y into parts sized absolute, relative (`'0.5`) or floating (`~1`),
    which may name an attribute;
  - repeats `{ ... }*`, whose members know their index, count, first, last and middle;
  - conditions on the context (`[street && level.index == 0]`);
  - weighted choices, per building or per attribute;
  - `let` values each building draws from its seed;
  - a text form with line-numbered errors.
- **`forge_geom::modules`, the kit:** wall, party wall, string course, cornice, parapet, French
  window (with or without shutters, a quarter of them closed), low window, courtyard window, balcony, entrance, shopfront
  with or without an awning (its valance 2.3 m up, so walkers pass under it). Each module is a small part program of boxes and quads with one
  section layout, so eight material groups dress any of them: four shutter colours, each with
  dark and lit glass (each window its own mix of dark glass and curtains). Walls, bands and
  cornices change size with every scope and are merged into
  the tiles; openings repeat at a few sizes and are instanced. With `--railing-cards` a
  railing's bars are one card between its rails, a grating the cut-out raster draws by the
  share of each pixel the bars cover, so they cannot flicker; off by default, it costs 0.09 ms
  at street level and 0.43 ms from the air until #265 (`research/trim-sheets.md` §2.9).
- **Thin sheets** (2026-10-15, the owner's sightings in a running test). The awnings, the stalls'
  valances, the Market Hall's glass roof, the terraced hall's canopies, the belvedere's dome and
  the trams' roof lights are drawn from both sides as `modules::sheet`, their back face 2 cm
  behind the front.
  - **The fault:** the two faces stood on one plane, and the coarser levels of detail folded
    them into each other. From the air the awnings vanished but for a cheek's thin blue blade,
    while the ray-traced shadows still cast them on the pavement: "shadows on each side of the
    building" with nothing over them.
  - **Also:** a moving tram's roof flickered.
  - The stalls' canvas had been fixed this way that morning.

The engine's example style set is `crates/forge-procgen/src/city/mediterranean.rules`, about
fifty lines. Each building picks its bay (2.8–3.4 m), its balconies, its shutters and whether its
top floor is an attic storey of small windows, and
neighbours differ.

| The kit within 450 m of the avenue view (example city) | |
|---|---|
| Buildings / faces / terminals | 1 021 / 4 466 / 198 335 |
| Instances / unique modules | 35 985 / 32 (9.8 M instanced triangles; four terrace layouts a width) |
| Merged triangles added (walls, bands, cornices) | 416 k |
| Derived and assembled | 80 ms (one thread) |
| Longest run of one opening along a floor of a street front | 10 |
| GPU frame at 2800 × 1575, this view, best of 3 | 3.78 ms (3.42 ms with `--detail 0`) |
| Scene built and cooked | 2.79 s (2.35 s) |

![Faces round the centre](images/city-plan-faces.png)

*The component split round the example's centre: street faces red, courtyard faces blue, party
walls grey, entrances as dots, corners in yellow.*

**Perimeter blocks, closed.** The first street views showed the avenues lined with gardens and
the backs of buildings. Lots were cut from the block's outline on the streets' centrelines, so
a lot on a 28 m avenue kept a metre or two behind the building line, and the courtyard's
convex hull ate the buildings along concave sides. Now:
- each block gets a buildable outline, moved in to each street's building line
  (`geom::inset_by`);
- lots are cut from that outline;
- every lot is built as strips of its district's building depth along each street it fronts:
  a lot between two streets has two fronts, and a corner lot wraps its corner;
- the courtyard is the buildable outline moved in by that depth.

The blocks close round their courtyards and the avenues are lined on both sides:
- masses: 12 935 (was 14 807);
- floors: 18.3 km² (was 20.5, now one building deep rather than up to 20 m);
- blocks with a courtyard: 602 (was 563);
- blocks under their green area: 83 (was 143).

**No slivers.** One mass in 17 was a slab 2–4 m thick, a blank party wall seen from the street.
Two causes:
- **Short street edges set far back.** Where a short edge between two wide streets turned over
  when moved to its building line, the block fell back to a uniform inset by its narrowest
  street. Its lots along the wide street then kept up to 11 m of street, leaving a 2–3 m
  building. The building lines now drop each edge the move turns over
  (`geom::inset_by_collapsing`, the straight skeleton's edge event).
- **Corner lots.** A corner lot a little deeper than its first strip left a sliver along its
  second street. The first strip now takes that sliver in. A sliver it cannot take joins
  another strip of its lot where the two make a convex piece, or is dropped.

No building is now under 4 m thick:
- masses: 12 373 (was 12 877);
- floors: 18.4 km² (was 18.3);
- lots: 10 985 (was 11 048).

A neighbour's garden is a view, not a neighbour: a wall onto a lot whose building stands more
than 3 m off is a court face with windows (2026-10-16; before, an empty lot only).

**Parcels for the first map** (`city::parcels`, 2026-10-16; `research/parcels-and-rooms.md`):
the lots above are cut across the piece's longer side, which on a tensor-field block gives
wedges and slivers (the owner: "non-rectangle shaped buildings for no reason, spaces between
them that don't make sense"). `parcels::strips` cuts a block the way a cadastre reads: each
street side a strip of one depth (the block inset, mitred at the corners), the strip into lots
by cuts square to its street at widths a type draws (a village house 5–8 m on 14 m; a
perimeter block's building 12–25 m on 18 m), the corner lots taking the corners (an acute one
gives a flatiron), a strip's end short of a lot's least width going to the last lot, a party
side left to the court. The block's middle is the court. A block too thin or concave for the
depth all round takes no strips, and the caller falls back on the cut above. Tested on a
rectangle and a trapezoid (every lot's frontage on the outline, no lot under the least width,
every inner party side square to the street); `PARCELS_PNG=path cargo test -p forge-procgen
the_trapezoid` draws one. The first map's village takes it first; the city, block by block.

**Houses from their rooms** (`city::rooms`, 2026-10-16; the research's §4): the owner's
principle that a house is thought from its rooms. `rooms::programme` gives a household's
rooms from its size and its means (poor: one or two rooms at the minimums, the kitchen in the
living room; modest: a living room, a kitchen, a bedroom a couple and one a child or two;
comfortable: a bedroom a person, a dining room, a study, two bathrooms; rich: a salon, suites
with their bathrooms, service rooms), each with its area; `rooms::plan` lays them on a lot of
a given width, a hall with the stair 2.95 m wide from the party wall's middle the whole depth, the rooms
beside it in bands from the front to the back: a band as deep as its first room is square
(2.6 to 6.5 m), its rooms side by side across the width, the largest first and the small ones
filling the bands' ends; the day rooms on the ground floor, the bedrooms upstairs once the day
rooms take half the depth, a new storey when one is full. A band's rooms share its width and
every storey's last band stretches to the house's depth, so the bands tile each storey and the
storeys stack; no room goes under what it asked nor under the 9 m² the decency decree wants
for a main room. The footprint's depth and the storeys come out of the rooms. (The first
solver, the same day, stacked every room across the whole width at 2.6 m deep at least: a
rich house on a wide lot wasted its width and climbed to nine storeys.) Tested on a modest
family of four on a 6 m lot (two storeys, the ground floor the day's), a poor household of
three (the minimums kept), a rich house of five on 14 m (its suites upstairs), each storey's
rooms tiling it with none over another. **On their lots** (`city::houses`): each strip's lot gets a household's plan at the
lot's width and the strip's depth less a 1.5 m yard; the footprint runs from the street to the
plan's depth, cut to the lot where a corner lot narrows, the rest of the lot the garden. The
house takes only the stretch of the front where the lot is at least a room deep (2.6 m,
sampled every quarter metre): a corner lot's bisector cuts the strip to a wedge, and a house
across the whole front had a window at its end with the roof right behind it and no room (the
owner, 2026-10-16, the sandbox's item 26); the wedge stays garden, and a front left under a
hall and a room wide keeps no house.
`HOUSES_PNG=path cargo test -p forge-procgen houses` draws the trapezoid block's houses, their
footprints shaded by their storeys.

**The village** (`city::village`): a site a few hundred metres across planned as the city is
(streets, blocks, parks: the example's site at 600 m gives 30 nodes, 44 edges, 15 blocks, 9
built), each built block cut into strips of village lots and a house from its rooms put on each,
the households drawn by the seed (one to five people; 15 % poor, 50 % modest, 30 % comfortable,
5 % rich). The lots are cut to the houses, the owner's way round: each lot as wide as its
household's rooms need on two storeys at the strip's depth (`parcels::strips_with`, which
hands back each lot's key, so a corner sliver merged into its neighbour does not shift the
households after it), never under the type's 5 m; a thin block takes a shallower strip (11 m,
then 8 m, a house with no yard), one too thin for that keeps no houses. Each house is also one
of the city's masses on its own lot, appended to the plan's (old stone, a tiled roof, homes
only), so the structures, the facades and the kit draw it as they draw the city's buildings.
The example village on the city's parameters: 8 blocks with strips round their courts, 1 too
thin, 384 houses of one to four storeys (7, 209, 146 and 22), 1 200 people, every house with
a door and 346 with a stair (`reports/2026-10-16/village-600m.png`; `VILLAGE_PNG=path cargo
test -p forge-procgen village` draws it; 406 houses before the corner wedges lost their
slivers). The first cut (2026-10-15) drew the widths at random: 191 houses, 586
people, and once the houses were masses the rich households on 5 m lots stood nine storeys.
**Its own plan** (`village::params`, the same day): one village district on the site's middle
with lanes that wander (`Pattern::Organic`), one main road, no trams, boats nor landmarks, a
green within 200 m of every block. The traced lanes make blocks of many short edges, which the
strips refused (the inset by the depth lost corners): a strip is now the block within the
depth of its street side between the bisectors at the side's two corners (a reflex corner's
too), and the court is the inset that drops the edges it turns over
(`geom::inset_by_collapsing`); every built block takes strips. **In its forest** (the same
day): the district reaches 1.6 times its radius of 120 m and the land beyond is countryside,
which `CityParams::forest` plants, 12 m clear of the roads (`trees::Setting::Forest`). Since
D-063 (the same afternoon) the forest is `forge_flora`'s: the Mediterranean wild species placed
by their habitats on the site's ground, once for the whole map, never round the player
(`place::place`), each tree keeping its species (`Tree::species`). On flat ground the flow's
wetness index read every sample as a valley floor and the holm oak took the whole forest, so
there moisture, soil and warmth follow the distance from the water, an assumption of this map:
deep moist alluvium by the river, the thin dry soil of a limestone plateau 300 m from it. At
600 m: 278 Aleppo pines, 870 holm oaks, and 21 002 garrigue shrubs (2 291 rosemary, 7 999
cistus, 10 712 kermes oaks). The demo grows three individuals of each species once
(`forge_flora::grow`) and instances them, each scaled to its plant's height and crown, in its
species' bark and foliage (`grown_mesh`): a tree's wood 5 cm thick or more as tapered limbs,
its leaf clusters gathered on a grid of a sixth of its crown (an eighth of a broadleaf's, a third
of a shrub's, a tenth of its height at least) into lumps sized by how many clusters they hold,
a shrub's of 20 triangles. A tree takes 8 000 to 33 000 triangles, a shrub about 600 (the full
graphs' 2 000 to 250 000; the demo's test keeps them under 40 000 and 1 000). The map check's
views take 2.2–2.6 ms of GPU where they took 1.6–1.9. The foliage is lumps until D-013's leaf
cards, which need a leaf atlas (drawn procedurally, or a CC0 one downloaded on the owner's
go). The road through the forest: `reports/2026-10-16/village-forest.png`. A village's whole network is under the 3 km a cut-off
network had to reach to be kept (`graph::MIN_NETWORK`), so the plan came out empty under
twice the radius: a network is now kept over 3 km or over half of all the streets. The example
site at 600 m on its own plan: 16 nodes, 22 edges, 7 blocks, 5 built and all with strips, 171
houses, 517 people, 3 118 trees in the forest at the time (`reports/2026-10-16/village-own-organic.png`;
`VILLAGE_OWN_PNG=path` draws it). The first cut on the whole site, without the forest, had
559 houses on 13 blocks. **Its rings** (`research/cadastre-villages-and-towns.md` §2, the
same day): a block's lot type follows its distance from the centre against the district's
radius. Within 0.9 of it the old core (`ParcelType::CORE`): lots 6–9 m wide on 12 m, the
house the whole lot, a court only where the rooms leave one. Within 1.35 the faubourg
(`FAUBOURG`): a block on the road out takes its lanières on that side only, 6–12 m wide and
40 m deep (32, 25 or 18 where the block is thinner), the house on the street line and the
garden behind it; its lane sides take no lots and are the gardens' ends (`parcels::strips_by`
cuts with a depth per edge and splits a corner where its two depth lines meet, so the corner
lot keeps its depth). A faubourg block off the road out is cut on all its sides, 30 m deep, a
garden of 12 m at least behind each house. Spacing the lanes wider towards the rim was tried
first: the tracer then kept 8 junctions of 16 and the village two blocks. Beyond, the lotissement (`LOTISSEMENT`): lots 15–25 m wide on 35 m (500 to
900 m²), the house free on four sides, 5 m back from the street, 3 m off the sides, 8 m of
garden behind; its front still counts as a street wall (`facades::SETBACK`), so it gets its
door and windows. Each ring falls back on shallower strips where a block is thin. The traced
lanes gave the blocks short edges along their curves, and each edge took a strip of its own
with one narrow lot: the buildable outline is simplified within 0.6 m before the cut. A lot
whose front is cut short (a corner's wedge) and whose household's rooms would pile over three
storeys stays garden. **The rich house on the square** (the research's §5): the village's
rich household (five people, rich) takes the lot whose front lies nearest the square, and its
block is cut again for that household's width; on the example site it stands 9 m wide and
three storeys on the square's block, the village's tallest. **Its distribution** (the owner's
question, the test prints it): the core block's 25 houses on lots of 82 m², more than half
of them with land left beside the house (a court of 17 m² on average); the two faubourg blocks
on the road out, 10 and 5 houses on lanières of 252 and 372 m², each with a garden (159 and
249 m² on average); the faubourg block off it, 34 houses on lots of 161 m², nine in ten with a
garden of 81 m²; the lotissement block's 8 houses on lots of 406 m², 309 m² of garden each.
The village at 600 m: 82 houses
on 5 blocks (one core, three faubourg, one lotissement), 255 people (on the city's
parameters 97 houses, 325 people, 55 of one storey, 33 of two, 9 of three; one in twenty
vacant as the city's). The lots grew wider when the houses' halls took their cores' width
(below, "Its insides"). Two strips could overlap where a short edge between them collapsed or
a block narrowed under two depths (a lotissement's lots crossed on its acute corner): each
now keeps to its side of the line where the two depths stand in proportion, and strips that
only meet are left alone. **Its garden walls** (`city::walls`, the same day): wherever no
house stands on a lot's or a court's outline, a stone wall 0.4 m thick closes it, 1.8 m high
along the streets and lanes, 1.5 m between neighbours, 1 m on a lotissement's front; each
garden with a side on the street keeps a gate 1.2 m wide, level with the street (before the
house on a lotissement's front), and each stretch of a court along a lane 6 m or more one in
its middle. Where walls meet one butts against the other, never overlapping, since two tops
on one plane would flicker: a wall stops at the face of one passing through, two walls in line
meet halfway, mitred, and at a corner the lower (then the shorter) stops at the higher's face;
walls left sharing ground step back, and two lots back to back across a sliver of court keep
one wall. The village has 662 m of wall along its streets and lanes, 1 779 m between gardens
and 42 gates; the kit draws them in stone and the walker meets them
(`reports/2026-10-16/village-garden-walls.png`, the map check's garden view along the longest
of them). **Its insides** (`city::interior`, the same day; the owner: "a house or structure
should be thought by thinking about the rooms"): a house's structure now follows its rooms'
plan. Its core (`cores`, the straight flight of 1.1 m with handrails beside a corridor of
1.2 m a wheelchair passes, walls of 0.2 m) stands at the start of its front where the plan
keeps the hall, the flight on the party wall's side and the corridor on the rooms' side; a
house of one storey gets its street door in the same hall (`structure::hall_door`). The plan
counts the party walls' halves and the core's full width (`rooms::HALL` 2.95 m from the party
wall's middle), so village and core lots start at 6 m, and a house of two storeys or more is
made deep enough for its flight (8.3 m outside for old stone, `Plan::deepen`). Inside the
walls, `interior::house` stretches the plan's bands between the hall and the far wall, front to
back: the hall's wall with a doorway (0.90 m clear, 2.10 m high) into each band's first room,
plastered partitions between the bands, and between the rooms of a band with a doorway each,
so every room of a storey is reached from the hall; where the core stood by the far corner the
plan runs mirrored. The kit draws them near the view: the partitions floor to ceiling with a
lintel over each doorway, the flight a stone step a riser, each upper floor and the ceiling
under it open over the treads below (`without` cuts the slab round the hole), the landing at
the top kept; the ground floor now stands 2 cm over the walking level its door's sill is on (5
cm up, a garden's lawn showed through it). 77 of the 82 houses get their insides, 419
partitions and as many doorways, every room reached (the village test); the five left had their
core in the middle of the front or none fitted. The map check's seventh view stands a metre
behind the first storeyed house's door, in its hall, the flight beside it
(`reports/2026-10-16/village-hall.png`). **Walking in** (the same day): a house whose insides are
drawn opens its street door, the kit's `door_open` with both leaves swung in against the reveal,
and the walker's wall pieces are cut at its ground doors; its partitions, its flights' steps
(the character steps up 0.4 m) and its upper floors round the stairwell are the walker's pieces
too. Every kit door now has a stone threshold 2 cm proud, level with the floor inside, where it
had a 12 cm step (the owner: no step where people walk). A house from its rooms gets a garden
door in its back wall at the end of its hall (`structure::garden_door`, in place of the court
door at the back's middle): 73 of the 77 houses with their insides, the others' backs on a
party wall. The floors and ceilings are inset by each wall's own thickness (`inner_ring`; the
thickest wall all round left a strip of the garden's lawn showing by the party walls). The
map check's second walk starts 3 m out before that house's door and walks through it into the
hall (`reports/2026-10-16/village-way-in.png`). **No wall behind a window** (the owner's item 26
the other way round): the facade's windows stand by its bays, the rooms by the plan, and 169
ends of the hall's wall met the front or the back wall in the middle of a window. A house from
its rooms now keeps its windows off the hall's wall (`structure::clear_hall`): a window standing
across it moves along its wall to the rooms' side, every storey alike, or goes where its
stretch has no room; and a partition between two rooms of a band that meets the front or the
back wall within an opening moves to the nearer pier (`interior::house`). Four ends are left,
walls between bands meeting a corner house's side windows (`interior::at_openings`, the
village test). **Its furniture** (`city::furnish`, the first map's props): each room takes its
pieces by what it is for, in order: a bedroom its bed (a double 1.60 by 2.05 m in a room of 11
m² or more, else a single; head to the wall, else along it, else the single), a wardrobe and a
desk; a living room its sofa with a low table before it, shelves and a table with two chairs,
and the kitchen's counter where the house has no kitchen; a kitchen its counter (a worktop
0.62 m deep with its sink and hob) and its table; a bathroom its bath, basin and toilet; a
study its desk with its chair and shelves; a salon two sofas. Each stands against a wall
facing into the room, from the middle of the longest wall outwards, clear of the doorways
and the openings to the floor (1.1 m before them), the tall ones (over 0.95 m) clear of the
windows, none over another and 0.6 m kept before each; a piece with no place is left out. The
kit draws them near the view (`forge_geom::furniture::home`, a mesh a piece in the kit's
sections) and the walker meets all but the chairs. In the village: 137 of 151 bedrooms with a
bed, 89 of 94 bathrooms with a toilet, 71 sofas, 137 tables and 302 chairs (the test checks
none overlaps another or stands in a doorway). The map check's eighth view looks into the first
room off the hall (`reports/2026-10-16/village-room.png`); without lamps inside, the rooms are
lit by their windows and the hall's doors only. **Its streets:** a village's main road is a shared street (10 m of
paving, no kerbs, bike lanes nor lamp rows) and its lanes are pedestrian, as an old town's,
through the forest too (a road beyond every district's reach takes the nearest district's
kind, `transit::assign_profiles`); the city's avenues stay. The narrower streets gave the
blocks ten more houses.
**In 3D:** `cargo run --release -p city-plan -- --village 600` draws the village on its own
plan in place of the city, every house from the kit (`reports/2026-10-16/village-air.png` from
the air, `village-street.png` from the first house's street, which the log names for
`--view`). **The map check** (`tools/map-check.sh`, D-062's check per commit): the village
from the air and from the views the log names (the first house's street, the green, the main
road, the busiest junction, the longest garden wall along a lane, the first storeyed house's hall and a room off it inside) and two walked paths, along the first house's street and through the first storeyed house's open door into its hall, each compared
with `captures/map/accepted` by LDR-ꟻLIP, about two minutes; `--accept` keeps a run
(`docs/PROCESS.md`). **One home a house** (`Mass::home`, the same day): a village house is
one unit over all its storeys, entered from the street, with its household's main rooms (1.5
a person, so the structures' residents match the village's): on the city's parameters, 368
households and 1 232 residents by the structures for 384 houses and 1 200 people by the
village (one home in twenty vacant, as the city's). Before, the units cut each storey into a
flat. Next: the rooms themselves as the units' plan, a square with the rich house, species
for the trees (#257), the player's objective.

**The trees, sized to their streets.** Trimmed street trees of 9–12 m line the avenues, 7–10 m trees
the shared streets and 5–7 m trees the lanes. Each instance is scaled by its own crown
across and its height up. The one model is a trunk forking into three limbs under a crown
of seven smooth lumps (about 620 triangles), so the crowns no longer hide the facades from
the pavement.

The rows stood 2.5 m from the building lines, and their crowns went into the facades (the
owner's first walk, 2026-10-11). An avenue's trees now stand by the kerb, 0.8 m into the
pavement, and shared streets and lanes have theirs down the middle, off the walkers' lines.
Each crown keeps 2.5 m from the building line (`trees::CROWN_CLEAR`), the avenues' pavements
are 6 m and their crowns' radii 2.4–3.2 m: 4 m from the facades, radii of up to 4.8 m reached
within half a metre of the windows (the owner: no one living there would want that).

**The riverbanks and the shore.** The site's water mask is 10 m cells, and drawn as cells every
quay and beach was a staircase (the owner's plan view). The shore is now one smooth line
(`city::shore::Shore`): the sea's and the lagoons' cells blurred over two cells (a 5-tap tent),
the rivers measured from their centrelines, one wetness per sample whose half-way contour,
bilinear between the samples, is the shore. The ground's squares the shore crosses are cut
along it (marching squares; where two opposite corners are wet the water joins across), the
rest drawn whole in runs. The map draws its water the same way, and its beaches 40 m from the
sea by a chamfer distance field. Artificial edges (a quay wall, a building in the water) may
keep straight, stepped lines later; the natural shore is smooth.

The sea, the lagoons and the rivers share one level, 0.6 m under the ground; the river lay over
the ground and met the harbour 0.6 m higher, a dark step between the two waters (the owner's
second walk). For the ground the rivers' beds are widened by three quarters of a cell each side,
so no land juts into them. Along each bank a stone wall drops to 1.2 m under the ground, and a
stone promenade 13 m wide follows the river over the widened bed; where a street runs by the
river, the street lies over it. The promenade is laid on land only, by the smooth shore the
ground follows (its land edge and middle out of the open water): where the river runs into the
harbour it floated over the open water. Testing its river edge too, half wet on the shore, or
the sea's cells, which reach past the drawn shore, dropped it by a bridge (the owner: a missing
pavement next to the bridge). It runs in stretches of 60 m or more: a shorter one stood alone
by a lawn's bank. Stone quay walls drop the same way along the shore.
A quay's land is lawn under its trees, the dry squares and pieces inside it. The harbour's
water itself is still a flat mirror: the island's FFT sea is the next step.

## Buildings carry their structure (D-060, step 1: #264)

`city::structure` gives every mass its structure, as data the next steps build on (the façade
kit reading the walls, collision, interiors, destruction):

- **Eras.** Each district's buildings have an era, with the research note's dimensions
  (`docs/research/building-structure.md` §5): old stone (the old towns and villages), the 1800s
  (the centres and seafronts), modern (the rest).

  | Metres | Old stone | 1800s | Modern |
  |---|---|---|---|
  | Façade / party (shared) / rear wall | 0.60 / 0.50 / 0.45 | 0.45 / 0.40 / 0.30 | 0.35 / 0.20 / 0.35 |
  | Slab | 0.30 | 0.30 | 0.27 |
  | Ground storey, shops / homes | 3.6 / 3.6 | 4.2 / 4.2 | 4.2 / 3.0 |
  | Upper storeys | 3.2, 3.1, 3.0, then 2.9 | a noble 3.4, 3.1, 2.9, an attic of 2.8 | 2.8 |
  | Window bay / window | 3.3 / 1.0 | 3.0 / 1.2 | 2.7 / 1.4 |

- **Storeys.** A mass's storeys come from its era, and its height is their sum (`Mass::storeys`):
  the street's ratio caps how many fit (`StyleBible::max_floors`). The grammar's `ground` and
  `floor` follow each mass. A modern block now fits more storeys of 2.8 m under the same
  height: in the sandbox's city the floors went from 67.5 to 77.5 km² gross.
- **Walls.** Each footprint edge is a wall with its thickness inwards and its role (street,
  court, party, inside the building). It is probed every 0.5 m, 0.75 m out (another mass) and
  2.5 m out (another building that close leaves a slit), and each storey keeps the stretches
  nothing stands against: a lower neighbour covers only the storeys below its roof.
- **Openings** go on those stretches only, bays centred, so they line up storey over storey and
  no window is cut by a neighbour (the owner: "not realistic at all"). Shopfronts on lively
  ground floors, windows elsewhere, the doors (1.2 m, step-free) where the stair cores stand
  (below), or in the middle of the longest street stretch.
  - **Room for the shutters.** Each window keeps room either side for its shutters folded back,
    each leaf half its width.
    - **How they hang** (the owner, 2026-10-16: "the shutters should be on the border exterior
      so it can rotate", "there are supposed to be hinges"). A leaf turns on pins sealed in
      the masonry at the opening's outer edge, by iron straps across its face, and folds back
      flat against the facade over the surround (the makers' fitting notes: straps on the
      leaf, pins screwed or sealed into the masonry, a stop to hold it open [Ehret][PROVOBAT]).
      The kit hung them beyond the 0.15 m surround, where they could not swing shut: they now
      stand at the opening's edges, 2 cm off the surround, two straps and their pins a sixth
      of the leaf from its top and bottom (`modules::shutters`). Closed, they sit in the
      reveal; a French window's guard rail moved into the reveal behind them (11.5 cm in),
      out of the leaves' swing.
    - **Balconies** span the opening and both open leaves with 0.1 m to spare, and stand out
      further than a leaf is wide (0.8 m, more past 1.2 m windows), so a leaf swings shut
      inside the railing (the owner, item 27 of the sandbox's list: the balcony was 0.45 m
      wider than the window each side, the leaf hung past its end;
      `reports/2026-10-16/village-balconies.png`).
    - A pier of 0.4 m of wall remains between a shutter and a corner or a stretch's end; the
      bays keep half a window and the pier off the ends.
    - 0.3 m remains between two neighbours' shutters, so their balconies stay 0.1 m apart.
    - Windows narrow (by the 10 cm) to fit, those the doors and the cores gave back too
      (`structure::fit_shutters`).
    - Before, a shutter wrapped round a building's corner (the owner), and the 2.7 m bays'
      1.4 m windows had their shutters over each other.
    - Test `shutters_fold_back_clear_of_corners_and_of_each_other`: 795 190 windows in the
      example, none within the pier, none over a neighbour (3 and 161 before).
- **Courts.** A passage of 3.0 by 3.6 m runs through one building of each block whose court no
  gap reaches, on its street and court walls (the owner: a door into every inner court). The
  sandbox: 2 082 of 2 087 courts reached, 201 by a passage.
- **Areas.** Gross to the outer faces; net inside the walls (a party wall counts half), less the
  stair cores below on every storey, less the partitions and flues (5–10 %, the 1800s' with
  their service stairs). Net over gross on the example: modern 0.777, the 1800s 0.732, old stone
  0.655. The note expected about 0.82 modern and 0.70–0.75 old stone, but without lifts: here
  every building of four storeys or more has one (the owner's accessibility), where old houses
  rarely do. With the stairs counted as a flat 12–24 m² a storey and one lift a building, it
  read 0.828, 0.809 and 0.722.
- **The population** counts homes from net area at INSEE's 32.4 m² a person, jobs at 25 m²: the
  sandbox's city housed 1 301 827 people at step 1 (1 048 538 before); about 977 k today, after
  the open blocks, the narrower streets and the cores.
- **Cost:** 0.62 s for the sandbox's 39 048 masses at step 1 (3.4 M openings); 2.25 s today for
  its 31 538 (3.1 M openings) with the reach check and the cores.

### Inside the walls (D-060 step 1, in 3D)

A camera through a wall saw the city beyond the culled walls, the neighbours' party walls lit
as if outdoors with the sun's shadows blurred over them, and the block's grass for a floor (the
owner, 2026-10-15: "many useless things rendered inside the empty buildings"; the second
flight's item 21). Since 2026-10-16 the kit's buildings carry their insides (`insides`): each
street, court and party wall's inner face at the wall's thickness, run the thickness past each
end so two meet at a corner; a stone floor a storey (the ground floor a hand over the block's
ground) and a plaster ceiling under each slab and under the roof, on the footprint inset by
the thickest wall. Over a passage into the court the face starts at the passage's head and the
ground floor keeps clear of the passage's width (whole, they closed every passage behind its
mouth with a black wall: the sandbox's `inside/passage-before.png` and `-after.png`). Nothing
else of it shows from outside (the sandbox's day check differs by a ꟻLIP mean of 0.015 at
most, scattered over the far city: the probes inside the buildings no longer see the sky
through their walls' backs), and inside a building one sees a dark room (the sandbox's
`reports/2026-10-15/owner-views/inside/`, before and after). The sandbox's 4 205 kit buildings
take 143 k triangles for it, and the view inside one draws in 2.20 ms as before (2.21).
The windows show nothing from inside yet (the glass is drawn for the street), and the far
masses beyond the kit's reach stay hollow; going inside builds the real rooms (step 3).

### Stair cores (D-060, step 3)

`city::cores` puts the stairs and lifts in, by the French rules (the research note's §1):

- **The core.** Behind a street door: a hall 1.50 m wide, where a wheelchair turns, and the
  landing on every storey, where the flats' doors open. A dog-leg stair beside it: flights 1.00 m
  between handrails, risers of 17 cm at most on 28 cm goings (2h + g between 0.61 and 0.62 m,
  Blondel's comfortable stair), a landing as wide as a flight at each end. Its length follows the
  tallest storey: 4.6 m for 2.8 m storeys, 5.8 m under a 4.2 m shop. From four storeys (R+3) a
  lift: a type 2 car (1.10 × 1.40 m) in a 1.60 × 1.90 m shaft. The lift stands beside the hall
  (6.0 m along the street), or on a floor under 150 m² behind it (4.4 m).
- **How many.** One every 20 m of street front, and enough that every floor lies within about 25 m
  of a landing. A wing under 100 m² beside another mass of its building is reached from that
  one's core.
- **Where.** On a ground bay of the street walls nearest each core's share of the front, else
  anywhere along an open stretch, else on a court wall. The core must fit inside the walls and
  stand clear of the others. Its door replaces the openings there. The shopfronts its walls
  stand behind become windows, so no shop window shows a stair. A court passage keeps its
  stretch: a core in its way moves aside (it took the only place, and courts went unreached). A
  core whose door opens onto a court no one reaches is dropped.
- **On the roof.** Under a garden or a terrace, the core climbs on to the roof. A head house
  2.7 m tall stands over it, with a door out from the hall (the owner: a garden on the roof
  means a way up). Every lift's overrun rises 1.2 m over the roof or the head house. Under a
  pitched roof nothing shows.
- **Checks** (`every_building_has_stairs_and_lifts_that_fit_and_reach_its_floors`), on the
  example:
  - 15 502 cores, 15 171 with a lift, 10 294 up to the roof;
  - each core's door is in its hall, on its wall's ground storey, and the core lies inside the
    walls;
  - 11 of 11 614 masses that need a core have none: slivers;
  - 117 of 12 613 upper masses lie more than 25 m from a landing at a far corner.
- **The sandbox:**
  - 41 041 cores, 39 748 with a lift, 29 252 up to the roof;
  - 201 passages, 2 082 of 2 087 courts reached;
  - no door unreached on foot;
  - 48.0 km² net of 61.7 gross (0.78, 0.83 before): homes for about 977 k people (1 045 k
    before);
  - the structure in 2.25 s (2.08 s before).

- **A house's stair** (2026-10-16, for the first map's village): where no dog-leg fits anywhere
  along the walls (a 5 m house holds no hall of 1.5 m beside two flights), a house's core: one
  straight flight of 1.1 m along the party wall beside a corridor of 1.2 m, the hall the whole
  depth, no lift (`Core::house`, `cores::flight_length` for its depth); the units take it as a
  compact core, one home a storey. The dog-legs keep the first choice everywhere, so the
  city's cores stay as they were. (Of the village's 371 storeyed houses, 38 had a stair before:
  the clips that cut a footprint to its lot repeated the corner they passed through, the
  repeated corner made a wall of no length classed inner, and a small mass with an inner wall
  counts as a wing served from its neighbour. Cleaned, 342 have one, one of them the straight
  flight.)

### Flats and shops (D-060, step 3)

`city::units` cuts each storey into units round its cores, the French way, two to four homes a
landing:

- **Territories.** Each core serves the floor nearer to it than to any other core, cut by the
  bisectors between their middles. Cores on a corner's two walls share the floor too.
- **The landing.** Behind the core a lobby 1.40 m deep continues the hall, and the doors open
  there:
  - on each side of the core, a flat runs through from the street to the court, with two
    aspects;
  - behind the lobby, where the building is deep enough (25 m² or more), a smaller flat looks
    onto the court;
  - a side too big for one flat (over 110 m² net) gets a corridor continuing the lobby, with
    single-aspect flats either side, drawn from the French mix of new homes (T1 10 %, T2 25 %,
    T3 38 %, T4 22 %, T5 5 % at 30, 46, 66, 86 and 100 m²) and 3.6 m wide at least;
  - a narrow house's compact core serves one flat a storey;
  - a unit under 25 m² joins its nearest neighbour.
- **Uses.** A lively ground storey holds shops, entered from the street. The storeys a building
  keeps for work hold offices (below). A unit with no window on its storey is a store room. A
  home's main rooms follow its net area (under 40 m² a T1, 56 a T2, 76 a T3, 93 a T4, then T5).
- **Checks:**
  - **Areas** (`every_floor_is_cut_into_homes_that_add_up_and_open_on_a_landing`): the units and
    landings add up to the net area within 5 % in all 12 610 masses with storeys. 19 of 229 174
    homes have their door more than 0.6 m from their landing.
  - **The walk** (`people_reach_every_home_from_the_street_by_stairs_and_lift`): people go from
    the street, on foot to the core's door (the reach check), up its stairs, and from R+3 by
    lift, to every home's door. None is stranded. 5 424 homes sit in small wings reached
    through their building's other mass.
- **The example:** 229 174 homes, 98 277 of them through flats; T1 16 %, T2 31 %, T3 26 %, T4
  13 %, T5 13 %. There are more large flats than in the new-build mix: the centres' and the
  waterfront's 14–16 m deep buildings make big through flats, as 1800s blocks have. Also 8 955
  shops and 4 634 store rooms.
- **The sandbox:** 673 593 homes, 38 % of them through flats; T1 16 %, T2 34 %, T3 25 %, T4
  13 %, T5 12 %. Also 19 640 shops and 25 374 store rooms. The structure takes 2.9 s with the
  units (2.25 s without). These are the counts before the offices (below).

### Offices and homes upstairs (D-060, step 3)

The upper floors are no longer all homes. Uses go by floor and frontage
(`research/city-generation.md` §14): offices over the shops on the main streets, whole office
buildings by the tram interchanges, as Euronantes and Part-Dieu stand by their stations.
- **Which buildings** (`mass::office_chance`, all estimates): a chance by the street (a central
  tram avenue 0.6, a tram avenue 0.4, a central avenue 0.5, an avenue 0.3, a central lane or
  shared street 0.3, elsewhere 0.15), and by the district (the converted halls ×1.3, the old
  town and the eco-quarters ×0.6, the villages ×0.2). It falls off away from the centre and
  rises by a stop (+0.1 within 100 m).
- **How many storeys:** from R+2 up, a lot draws:
  - offices on its first floor (and the second over five storeys), at that chance;
  - or the whole building, at 0.35 of it plus +0.35 within 150 m of an interchange (+0.15
    within 400 m). An interchange is a stop with another line's within 150 m.

  The ground floor of an office building without shops is its reception, with no garden flats
  and no homes' planters.
- **Drawn by block, then a little by the lot,** so offices gather. The draw is made even again
  (`mass::even_mix`): mixing a block's number with the lot's had turned a 15 % chance into 5 %.
- **The units:** a storey kept for work is cut as the homes are (offices to let) and marked
  `Use::Office`.

**The population counts by home.** Each home holds one household, unless it stands empty: 5 %
do, the design value of `research/city-generation.md`.
- **Its people:** its main rooms by INSEE's 1.5 rooms a person in flats, at least one, the
  fraction drawn (`units::occupants`). That is about 1.9 a flat, INSEE's 2.9 rooms over 1.5
  (`research/building-structure.md`).
- **The jobs:** the offices' floor at the British Council for Offices' 15 m² an employee, and the
  shops' at 25 m² (`units::occupancy`).
- **The population:** it takes these counts (`population::Homes`, `Building::jobs`) in place of
  the old two thirds of the floor area at 32.4 m² a person. Each home's residents are dealt
  among its households in turn (`Population::household`), so every counted household has
  someone. The crowd, the taxis' bookings and the residents' lights all count this way.

| | the example | the sandbox |
|---|---|---|
| upper floors as workplaces | 13.6 % (42 % within 400 m of an interchange, 12 % elsewhere) | 13 % fewer homes |
| buildings with offices upstairs, office buildings | 3 104, 995 | 7 883, 3 932 |
| homes | 183 618 | 585 900 (was 673 593) |
| office units | 26 632 | 122 733 |
| households, people (a home) | 174 416, 331 645 (1.90) | 556 765, 1 043 k |
| jobs | 133 837 | 575 990 |

The sandbox keeps its million: the old count by floor area gave 977 k to 1 045 k. Test
`offices_gather_upstairs_by_the_interchanges_and_the_homes_count_the_people`. From the street an
office building still looks like its neighbours, shutters and balconies included: offices'
facades and rooms come later. One in the kit is logged (`one at --view=`).

### Rooms behind the windows (D-060, step 3)

The kit's glass shows a room behind it, seen from outside (interior mapping, Van Dongen 2008;
`interior_room` in `shaders/meshlet.slang`). A material row with `RenderLayer::interior` set
traces the view ray into a box in the window's own space: 4 m wide, 3.5 m high and 4.5 m deep,
the glass on its front face, one box per cell when the glass is wider or taller (a shopfront's).
The ray leaves the box by a wall, the floor, the ceiling or the back wall:

- walls in one of four colours (cream, grey-blue, sand, sage), a floor of dark wood, oak or
  tiles, a white ceiling, each window's own draw;
- a picture on the back wall, a door in one side wall, a low piece of furniture (a sofa, a
  sideboard) against the back, which the ray can meet first;
- the depths darker, and net curtains on 45 % of the windows, from sheer to drawn.

No geometry is added: the glass's shading does it all. By day the room takes a third of the
daylight that falls on the window, and the curtains, at the glass, all of it: a room as bright
as the wall outside matched the walls from the air, and the facades lost their windows (a real
room gets a few per cent; a third keeps it in sight from the street). At night a lit row's room
takes a quarter of the row's luminance (3–8 cd/m²), 0.75 to 2 cd/m².
- The flat glow was too strong and all alike (the owner): at the full value the lit panes were
  white.
- At 4 % the walls sat a stop or two over the moonlit night's grey and the room showed. Once
  the probes saw the street lamps (#254), the exposure rose to the walls they light, and from
  the pavement no window looked lit.
- At 25 % a lit room is level with a lamp-lit wall and still shows. A real one is brighter
  still.
- The textured boxes beyond the kit light their windows at 6 % of the glow. Matching the rooms
  would take 15 %, but at the horizon the facades stack with no street between and the far city
  read as a white band again (the owner, 2026-10-11). Where a pixel covers more than 6 cm of the room its
edges would flicker as the room moves behind the glass, so the room fades into its mean colour
by 25 cm. A turn of 2.5 pixels a frame shows the same shimmer as without the fade (138 695 and
138 629 pixels over ten frames): a turn does not move the room behind the glass, so this
measures the facades, not the rooms.

**Lit by their residents (D-060 step 3).** With `--time-of-day`, each home's windows light by
what its people are doing at that hour (the owner: "lit windows from the people living there").
The share of the hour came first, as an estimate.
- **The people:** the crowd's (`crowd::people`, one seed), on a Tuesday with its Monday evening
  counted. Only the residents of the kit's buildings are made (`population::generate_in`), each
  the same person as in the whole population.
- **Home and awake:** `population::home_awake`. Each person's day plan says where they are. A
  sleep model was added (`population::sleep`):
  - they wake 45 to 90 minutes before their first outing;
  - they go to bed at their role's hour (22:30, students 23:30), later for the extraverted and
    before a day off, or half an hour to an hour after coming home;
  - a night worker on shift sleeps from coming home in the morning.
- **A household's flat:** a window finds its unit from a point half a metre inside it, among its
  storey's (`residents::unit_at`). Each occupied flat holds its own household, since the
  population counts by home ("Offices and homes upstairs"); an empty flat stays dark. A window in
  no unit (a landing, a wing) is lit in the share of its building's households up.
- **Its windows:**
  - a household at home and awake lights 55 % of its windows after dark (the rooms in use), and
    15 % in full day;
  - a household out or asleep lights 2 % (a lamp left on, a hall light).
- **The offices:** lit by the working day (`residents::office_lights`, estimates): 70–75 % from
  8:00 to 17:00, fewer at lunch, a quarter at 19:00 as the cleaners go round, a tenth at 21:00,
  4 % all night.
- **The shops:** lit in the share of the hour (`lit_share`): while open, 8:00 to 19:00, and a
  tenth of them all night for their displays.
- **The far facades:** their texture's windows are lit in the share the kit's residents lit
  theirs (`lit_share` without a kit). Dimming a fixed two in five instead left them lit at 3:00,
  the exposure rising to meet them.
- **The trams:** they pause from 1:30 to 4:30, as French networks run from about half past four
  to half past one (`tram_pause`). The running ones are hidden and every depot place is taken.

| The sandbox, `--place old-centre` | 21:00 | midnight |
|---|---|---|
| households at home and awake (33 243 in the kit, one a flat) | 96 % | 13 % |
| homes' windows lit (145 497) | 51 % | 9 % |
| windows that found their flat | 80 % | 80 % |
| offices' windows lit (49 212) | 10 % | 4 % |

The residents take 180 to 280 ms at start-up: 1.1 s when the whole city's million people were
made. Without `--time-of-day`, two windows in five are lit as before. Tests
`homes_are_awake_in_the_evening_and_asleep_in_the_small_hours` (forge-sim, on its test town: 3:00
under 5 % awake, 20:30 over 60 %, night workers asleep in the morning),
`some_homes_people_are_the_whole_populations`,
`the_residents_light_their_flats_in_the_evening_and_sleep_in_the_small_hours` (the example city:
96 % up at 21:00, none at 3:00, every unit found from inside it) and `windows_light_by_the_hour`.

**Shops behind the shopfronts.** The shopfronts' glass has its own section
(`modules::SHOP_GLASS`) and row (`RenderLayer::shop`): no curtains, which 45 % of the shops had,
and no sofa. A shop's floor is level with the glass's foot and it runs 6 m deep. Its pale walls
are lined with shelves of goods, boards every 0.45 m from 0.3 to 2.1 m with boxes 0.3 m wide
between them. It has a tiled floor and a counter at the back. A lit shop takes 8 % of its row's
luminance, twice a home's (50 % since the residents light their own windows).

The rooms stay outside views: the doors are never see-through (Celsys's claims, D-060), and
going inside builds the real rooms.

Next in step 3: the units' rooms behind the windows (an office shows a home's sofa for now),
then the near rooms drawn for real with people in them, and F to enter. Offices and homes upstairs
and the residents' lights were done on 2026-10-15.

## Trams on their timetables (#253)

```
cargo run --release -p city-plan -- --street --tram=X,Z,LEAD   # a tram passes (X, Z) LEAD s in
```

![Trams](images/city-plan-tram.png)

*A five-section tram on the river avenue from the pavement and from 40 m up (a stop's platform
and shelter beyond it), and a lane's café terraces: white bodies in
the line's colour, articulated round the curve, on the right-hand track.*
- **The timetable** (`forge_sim::transit`) runs vehicles out along their line and back:
  - each leg from stop to stop on a trapezoidal profile (50 km/h at most, 1.1 m/s²);
  - 25 s at each stop, 3 minutes at each end, a vehicle every 6 minutes;
  - a run's place is a pure function of the time (GTFS's model of trips and frequencies), so a
    server, its clients and every stop display compute the same tram from nothing sent.
- **The model** (`forge_geom::vehicles`) is a part program like the kit's: five sections of
  6.4 m (two cabs with raked windscreens, three passenger sections), glazed doors, the livery's
  stripe, roof pods, bellows between sections. It takes its power from the ground, as in
  Bordeaux and Nice, so the streets have no overhead wires. Inside: a low floor 25 cm over the
  rails, level with the platforms; hollow sides; facing pairs of seats by the windows. It was a
  solid body to 1.05 m with no inside faces, and the owner found it small.
  - **Modernised (2026-10-15).** The owner asked for more futuristic trams with more glass and
    a dome-like glazing, and more comfortable-looking seats:
    - the glass runs from 0.95 to 2.8 m between slimmer pillars;
    - roof lights slope in from the eave to the roof over the passengers' heads, a ceiling
      0.5 m in from each side under the pods;
    - the cabs' windscreens wrap round their sides;
    - the seats have deep cushions, tall backs and headrests.
    - *Fixed the same evening* (the owner saw a moving tram's roof flicker): the roof lights'
      inside face stands 2 cm in from their outside one. Each section's front bellows is 2 cm
      smaller than the back one, so on a curve it nests in the next section's: their tops and
      sides lay on one plane there.

    Rounded windows wait for curved parts in the part programs. The cars are 2.55 m wide (were
    2.4; the owner: a little wider, more comfortable inside). The platforms' edges stand 1.35 m
    from the track's middle, 7.5 cm from the car. Two trams pass 0.65 m apart on the 3.2 m track
    centres.
- **Riding:** C boards the tram section nearest the camera (within 10 m), standing in its
  aisle and facing the way it runs; C again gets off on the platform's side, walking again if
  the rider walked. `--ride SECONDS` boards the nearest tram anywhere that far into a scripted
  run.
- **In the scene,** each line gets two tracks 3.2 m apart, one each way. Each section is a
  mover set every frame by its two ends on the track (the bogie points), so a tram bends
  through curves.
- **Stops and turns** (2026-10-15). A tram stands with its middle at its stop both ways, so the
  two platforms face each other. Before each end's platform, a crossover takes a tram coming in
  across to the track it leaves by: an S-curve 30 m long, its rails drawn 3 mm over the straight
  ones. The tram then turns where it stands.
  - Before this, a tram turning at an end jumped 3.2 m across and a whole tram's length along:
    its front stood at the stop, and the other way's platform lay beyond it.
  - Test `trams_turn_where_they_stand_having_crossed_over_on_the_way_in`: over whole cycles, no
    section moves more than a quarter second at the cruise allows (3.8 m on the outer track of
    an 18 m curve).
- **Crossover pairs along the lines:** about every 3 km, two crossovers one each way, 10 m
  apart. A tram can pass a broken-down one on the other track and come back. They stand on
  straight track on land, clear of the junctions and of the stops' platforms and ramps.
  - The example has 6 on 26 km of line over three seeds; the sandbox 18.
  - The trams don't take them yet.
  - Test `crossover_pairs_stand_every_few_kilometres_clear_of_stops_and_junctions`.
  - Tail tracks are next (`research/transit-and-vehicles.md` §10).
- **Tram depots** (`city::tram_depot`, 2026-10-15; the owner asked for depots for maintenance
  and stabling). About one for three lines, 2–4 for a network of 5–8, near a line's end where
  a tram avenue's block holds a yard of 6 trams or more behind its building line. Industrial
  land comes first, and the depots stand 1.5 km apart.
  - **The link:** a single track leaves the line's near track on an S-curve (25 m curves). It
    rises up the verge on a ramp from the carriageway to the walking level, so the footway it
    crosses stays level, and becomes the yard's first track.
  - **The yard:** parallel stabling tracks 4.6 m apart, in asphalt behind a low wall along the
    street. The next two tracks leave the first on S-curves; the deeper ones turn off a 45°
    ladder on 20 m curves. Each place is a tram and 5 m (38 m).
  - **The hall:** over the three deepest tracks' far ends, 50 m long and 9 m high, open towards
    the yard, with solar panels on its roof.
  - **The trams kept:** two places in three hold a tram in its line's livery, standing; the
    lines' timetables run round the clock, so none goes in or out yet.
  - Only the lots under a yard lose their buildings, and the depot's block has no court
    gardens. Stops keep off the links as they keep off junctions.
  - The example has one depot of 8–13 trams in each of three seeds; the sandbox has 4,
    keeping 42 trams.
  - Tests:
    - `a_depot_s_tracks_curve_no_tighter_than_a_tram_turns_and_hold_its_places`;
    - `tram_depots_stand_behind_the_building_lines_near_the_lines_ends`;
    - `a_depot_s_trams_stand_on_its_tracks_in_their_places`.

The example's two lines run 10 trams.

Lines share their tracks where they share an avenue, and nothing yet keeps their trams apart.
Lines ending at one hub stood their trams on the same spot for their three-minute turns. Each
further line ending within 30 m of another's terminus now ends 45 m before it, along its own
track. The log counts the trams of two lines within 30 m on one track, every 5 s over a game
hour:
- a 21-line city: 685 such pairs, down from 985;
- the example: 4.

The rest are trams turning at a terminus that another line runs through, and chance meetings
on shared avenues. Spacing trams on a shared track needs the lines' timetables planned
together (holding at stops, as real networks do).

**River shuttles** (Nantes' Navibus, smaller): an electric boat for about twenty people
(`vehicles::shuttle_boat`: a hull with bows at both ends, a glazed cabin under a solar roof, ramps
at both ends) runs out and back at each of the plan's boat crossings. It uses the same
timetable as the trams, with two stops at the water's edges: 3.5 m/s at most, 45 s at each
landing. `--clock SECONDS` sets the service's clock for a capture.

![A river shuttle](images/city-plan-boat.png)

*The example's crossing from mid-river: the shuttle by its north landing.*

**Street furniture** (`forge_geom::furniture`) in the kit's section layout:
- **café terraces:** round tables in two staggered rows, a little off their places and turned
  their own ways, two to four bistro chairs round each, a parasol over every other table. There
  are four layouts a width, chosen by the shopfront, so neighbours differ (the owner asked for
  several tables round each café, not aligned, a bit organic). They stand within 2.65 m of the
  wall, before a third of the kit's shopfronts on the avenues' pavements and in the lanes,
  and two thirds of those facing a square, a park or a quay. They are coloured as the
  building's awnings.
- **Which ground floors are shops** (`Mass::active`, redone 2026-10-15). The owner: not every
  ground floor on a pavement is a shop or a café; shops go on the main roads, near the sights,
  in the centre, along the coast and the rivers. `research/ground-floors.md` gives the model, all
  its numbers estimates calibrated on Paris and Barcelona.
  - **A chance by street:**
    - a central tram avenue 0.9, another 0.65;
    - a central taxi road 0.75, another 0.35;
    - an old town's lane 0.55, another pedestrian street 0.1;
    - a shared street 0.35 in the old towns, 0.12 elsewhere.
  - **Or by what it faces:** a square 0.9 in the centre and 0.6 elsewhere; the water 0.8 and
    0.4; a park 0.35.
  - **The neighbourhood's parades:** at least 0.5 within 400 m of each district's heart.
  - **Then:**
    - ×0.4 in industrial districts;
    - less far from the centre (×0.8 at 3 km, ×0.6 past 5 km);
    - +0.15 within 100 m of a tram stop (+0.07 within 250 m);
    - +0.15 near a sight;
    - at most 0.95.
  - **Parades, not scatter:** the draw is shared by stretches of 120 m of a street (70 %), a
    little by the lot.
  - **The example:**
    - 22 % of the ground floors are active, 96 % along the tram avenues and 13 % along the shared
      streets;
    - an active building's neighbour is active 65 % of the time;
    - 30.9 shopfronts per 1 000 people (Paris 28, Barcelona 37);
    - the old rule made every avenue, lane and open front shops.
  - **Tests:** `shops_gather_where_people_pass_and_leave_most_ground_floors_homes` and
    `a_city_has_about_as_many_shops_as_a_real_one`.
- **What the homes keep on the ground** (`city::gardens`, 2026-10-15; the research's §4.6–4.8).
  All of it is the generator's data, at the walking level and step-free; the scene draws it.
  - **The courts' paths** (2026-10-15, after the owner's review): one network round each
    court's middle. The owner: close paths should first join the paths round the garden, like
    a hub; separate ones side by side waste the ground.
    - The middle is the kitchen garden in its paved ring 1.8 m wide, or a round paved place
      7 m across in the courts without one.
    - A path comes in from each gap between the buildings and each passage (2.4 m wide), and
      from each court door (1.8 m). Each runs straight out past the private gardens, then to the
      nearest point of the network laid so far, the nearest entrances first. A path goes to
      the middle itself where joining would turn it more than 75° away from it.
    - In the example, 3.9 % of the paths' length lies within 4 m of another path, away from
      the junctions, against 50.7 % when each ran to the middle. They are about a quarter as
      long.
    - They are paved with the forecourt, one surface.
  - **The garden flats** (the *rez-de-jardin*): 8 m of a court wall each, laid out between the
    doors and passages, where the ground is open court or the building's own lot.
    - Half have a private garden 4.5 m deep: a terrace of 2.5 m before the flat, a lawn, a
      hedge of 1.4 m between neighbours, and a hedge of 0.8 m along the court with a 1.2 m gate
      and a paved way to it.
    - The others have a terrace of 1.8 m between two short hedges.
    - Each opens onto its terrace through a glazed sliding bay to the floor (the owner's
      "baie vitrée"): up to 2.4 m wide, its threshold flush. It is the style's `GardenBay`, the
      kit's `bay` module, with no shutters.
  - **Planters** 0.9 m deep and 0.45 m tall, against the facades of the homes on streets 8 m wide
    or more. They are broken 0.8 m either side of each door and passage, and none stand before
    a shop. The footway keeps 2.6 m on an avenue, and the walkers pass 3 m out.
  - **A shared kitchen garden** in each green court of 600 m² or more, on paved ground at the
    court's middle, where its paths meet, else as near it as fits on its sunniest (north)
    side, clear of the private gardens. It has eight beds (four where only those fit),
    1.2 × 4 m and 0.7 m tall for a seated gardener, on paths of 1.4 m.
  - **Benches round each court's middle,** on the green 10 cm off its paving and facing it:
    - by a kitchen garden, on its long sides' middles first, then its ends and quarters;
    - by a round place, a quarter turn apart first;
    - up to four a court, 3 m apart at least, none where a path arrives or a wing reaches in.
    - The example has 2 001 in 535 courts. A court's benches are logged as a view.
    - Test `court_benches_face_their_middles_paving_from_the_green`.
  - **The gaps between the buildings** (open blocks) get a low hedge (0.8 m) along their street
    side, just behind the 2 m paved forecourt, broken a metre either side of the court's
    paths. They showed a plain lawn; the owner said a gap would "match the sidewalk or have a
    decoration".
    - With open blocks on, the example has 3 091 such hedges in 1 770 gaps, none in a building
      or on a path; the sandbox has 16 476.
    - A gap's first stretch of hedge 3.7 m long or more gets a bike rack before it on the
      forecourt: three Sheffield stands in a row along the hedge, 1.65 to 1.95 m in from the
      building line, so the footway keeps its width. With open blocks on, the example has
      1 644 in 1 775 gaps.
    - Test `open_blocks_gaps_are_hedged_behind_their_forecourts_but_where_their_paths_go_in`.
  - **The walker** walks round the planters, hedges, beds, benches and bike racks. No tree
    stands on any of them, nor on a path or a terrace.
  - **Plants:** within 400 m of the kit's centre, shrubs every metre in the planters and two
    rows of vegetables every 0.8 m in the beds. They are small trees sunk in their boxes to their
    crowns' middles.
    - The boxes' flat green tops read as carpet; standing on them, the plants showed their
      trunks like bonsais.
    - Out to 1.2 km, a pixel or two each, they speckled the facades' feet from the air.
    - The sandbox has 4 226; its planters' view takes 2.40 ms GPU.
  - **The example:**
    - 8 880 court-side flats, 4 464 with a garden, every one with its glazed bay;
    - 400 kitchen gardens of 3 200 beds;
    - 20 377 planters (two gave way to a door's path round a building's corner);
    - none of the 15 701 street doors and passages and none of the 9 544 shopfronts behind a
      planter;
    - no court door or passage opening into a garden.
  - **Tests:** `court_gardens_keep_clear_of_the_buildings_the_paths_and_the_doors`,
    `court_paths_join_round_the_middle_instead_of_running_side_by_side` and
    `planters_line_the_homes_on_the_streets_and_leave_their_doors_clear`.
- **tram stops:** a 36 m boarding area by each track, level with the trams' floor (28 cm over
  the road), from the tram's side across the carriageway to the kerb and half a metre into the
  pavement (3.55 m on a tram avenue, `boarding_deep`), and a glass shelter with a bench and a
  lit totem. Its screen shows the next tram, amber on black: "Due", "1 min" to "14 min", or
  "15+ min", from `Timetable::next_arrival`. The plates are one atlas row each
  (`signs::screen_plate`), twelve of each kept as movers and set each frame on the totems within
  150 m of the camera, nearest first (the owner asked for the screens).
  Ramps of 1 in 12 run down to the road at each end, and along its back down to the pavement,
  so someone on the pavement rolls onto it anywhere: the platform was first reached by a 28 cm
  step, then stood 0.75 m off the kerb with the road between (the owner: not accessible). The
  bikes' strip by the kerb runs over it behind the shelter, at its height, as a cycle track
  crosses a stop's boarding area in Dutch and German streets; it was a 0.75 m channel. Each stop
  stands at the nearest place along its route where its platforms and ramps, 37.4 m either way,
  are clear of every junction and on straight track, so the whole tram stands level with the
  platform (searched 400 m either way in 2.5 m steps, 60 m apart at least). Snapped to a
  junction, they stood across the crossings and bikes rode through them; set in the middle of
  the stretch between junctions, some still jutted into the crossroad or lay on a curve (the
  owner).
- **tram lines** turn on arcs: each line is simplified to 0.5 m, then every corner is rounded to
  18 m, about a low-floor tram's tightest in the street (`geom::fillet`, `transit::TRAM_RADIUS`).
  The tracks turned at right angles in the junctions.
- **benches** keep 2.2 m clear of the trunks (the owner saw a tree through one).
- **chess tables** (`furniture::chess_table`): four round the fountain of each square of
  0.15 ha or more without a landmark, 9 m from its middle on the diagonals. Each is a stone
  pedestal table with an inlaid board in stone and wood, close in tone so it does not shimmer,
  and two stools. Its players sit as the terraces' guests do. Round a game being played, two to
  five people stand and watch, a step back on the players' sides (the owner: a few play, more
  watch, some pass by).
- **markets** (`furniture::market_stall`, 2026-10-15; `research/ground-floors.md` §4.9: open-air
  markets on the boulevards and squares; the owner's programme against monotony):
  - **Where:** one square in two of 0.25 ha or more, without a landmark or a water mirror.
  - **Layout:** rows of stalls facing each other across 3 m aisles along the square's long axis,
    down its middle and 11 m either side where it is wide enough, 3.2 m apart.
  - **Kept clear:** they stay inside the benches and clear of the trunks, and the fountain keeps
    a ring of 7.5 m. Market squares have no chess tables.
  - **A stall:** 3 × 2 m, a trestle table with crates along its front, four poles and a canvas
    sloping from 2.5 to 2.3 m, in the kit's four awning colours. The walker walks round it.
  - **The sandbox:** 147 stalls on 5 squares; the nearest is logged as a view.
  - **The people** (with `--people`): within 220 m of the camera, each stall has its
    stallholder standing behind the table and nought to three shoppers in the aisle looking at
    the crates, as the chess games have their watchers.
  - **Mornings:** with `--time-of-day` outside 6:30 to 14:00 the stalls are packed away and the
    chess tables are back. Market days come with the week's rhythm.
- **fingerposts** (`signs.rs`; the owner got lost on their second walk): at each junction where
  two avenues meet, within the kit's reach, a dark pole by the kerb just past the dropped kerb,
  clear of the trunks. Up to three blue blades, each pointing the way to one of the nearest
  landmarks or built districts' centres (200 m to 4 km, at least 35° apart). Each carries its
  name in white, read from its left end on both faces, from one 1024² atlas of names drawn
  with the engine's JetBrains Mono (16 rows). 23 in the sandbox's kilometre round the harbour.
- **the street ground as one map** (D-061, `city::ground`; the owner, 2026-10-14: "a lot of
  bugs" in the roads and pavements). Every surface of the streets is drawn once, at one level.
  The old pieces were stacked a centimetre apart (0.16, 0.17, 0.18, 0.30) and patched case by
  case: paved corners, swaps, straight bands, bend wedges, square borders, aprons.
  - **Sections:** a street's cross-section is data (`Profile::section`: footway, verge, cycle,
    drive, tracks, shared), so its width, carriageway and pavements are sums of its strips.
  - **Streets:** each street is its centreline stroked to its width, and its carriageway to the
    carriageway's width (`i_overlay`, integer mode). Arms end flat at their junctions, a mitre
    fills a reflex gap, and dead ends are round. The union is the street ground.
  - **Corners:** between two driven arms the kerb turns on 6 m, shorter where an arm is short.
    Where a tram turns, the carriageway takes in its swept path (the outer track, half a car and
    0.5 m: `transit::SWEPT`), so the kerb keeps clear of it.
  - **Crossings:** a zebra 4 m wide on each driven arm of every junction, a metre past its corner
    or past a pedestrian way's mouth. A dropped kerb at each end slopes 1.6 m (1 in 12, #263) down
    to a 1 cm lip, its sides flared at 1 in 12 too, so there is no edge to step or roll over (the
    owner: "gentle slopes all around"): 862 crossings and 1 724 dropped kerbs in the example.
  - **Raised tables** on the taxi roads (the owner's go): a crossing over an avenue without trams lies
    at the walking level from kerb to kerb, flat for a wheelchair, and taxis take a ramp of 1.5 m
    up and down either side (14 cm, walking pace). Tram avenues keep dropped kerbs: rails take no
    table. A kerb's face drops to nothing along a table. Test
    `a_taxi_road_s_crossings_are_raised_tables_flush_with_the_pavements`.
  - **Pedestrian streets:** the shared streets are for people only (bikes, deliveries at set
    hours, no taxis; the owner). A row of bollards closes each walking way's mouth onto a road,
    1.5 m behind its building line, 1.5 m apart at most (`furniture::bollard`, a steel post with a
    pale band), and no café terrace stands within 8 m of them.
  - **Taxi stands** (`city::taxi`, the owner's model: automated taxis booked from a communicator,
    waiting at stands when idle). A lay-by 14 m long and 2.5 m deep cut into a taxi road's verge at
    the carriageway's level, its ends splayed, and a lit totem behind it (`furniture::taxi_totem`).
    Only on avenues without trams, so no platform meets one. They go first by the tram stops and
    squares (within 150 m), then wherever a built block is left without one within 300 m, 200 m
    apart at least. Trees and lamps keep out of the lay-bys. The example has 81 (15 of 610 built
    blocks are farther than 300 m: no taxi road near them), the sandbox 189. Test
    `taxis_wait_within_a_short_walk_of_every_block`.
  - **The taxi fleet** (`forge_sim::taxi`, the owner's model: like Uber or inDrive, with no
    drivers). The taxi roads make a two-way network. Each stand has two bays.
    - **Trams first** (the owner: not everybody takes a taxi, and it has to be useful to the one
      who does):
      - the trips by tram or taxi are 0.68 a person a day (D-059's shares: of some 2.8 trips,
        22 % by tram, 2 % by taxi), over a daily profile with morning and evening peaks;
      - a trip goes by tram when a line stops within 400 m of both ends, or a line near one end
        meets one near the other (stops within 150 m: one change);
      - of the trips the trams do not serve, a share books a taxi, set so that taxis carry 0.056
        trips a person a day; the rest walk, cycle or change more;
      - trips shorter than 700 m walk.
    - **Bookings:** places weighted by floor area (built blocks) or 5 000 m² (squares).
    - **Fleet size:** three trips a taxi in the busiest hour. One taxi for 90 trips a day left
      527 bookings waiting at 9:00 (402 s on average).
    - **Dispatch:** the nearest free taxi by road (Dijkstra in millimetres):
      1. it drives to the booking, waits 25 s while the passenger gets in;
      2. carries them, waits 25 s again;
      3. returns to the nearest stand with a free bay.
    - **Busy hours:** the hour's rate is above 1.3 times the day's mean. The taxis behind a
      stand's first leave to cruise round the busy places, one at a time as the road clears.
    - **Traffic:**
      - 8.3 m/s (30 km/h), 1.5 m/s² to start;
      - each taxi follows the one ahead in its lane along its route (the Intelligent Driver
        Model: 2 m standing, 1.2 s at speed, looking 60 m ahead);
      - one taxi at a time crosses a junction, and only with room past it; the others wait at a
        stop line 10 m before its node, behind the crossings (after 20 s they go anyway: no two
        queues wait on each other for ever);
      - pick-ups and stands keep 15 m from junctions;
      - a taxi pulling out from the kerb goes before one standing level with it;
      - a taxi stops with its front 3 m short of anyone on foot on the carriageway ahead (within
        3.2 m of the road's centreline: crossing at a zebra or a junction), the people drawn
        round the camera (`Fleet::set_people`). Test
        `a_taxi_stops_for_someone_crossing_and_goes_on_once_they_are_over`.
      - the walkers cross the streets on the zebras: at a junction, each driven street on the
        way's right that has one is crossed on it, along the pavement to its near end and back
        from its far end, at the road's level between the kerbs (or the table's); they crossed
        in the junction's middle, where the taxis drive. Test
        `walkers_cross_the_side_street_on_its_zebra`.
    - **Where each cab stands:**
      - in its lane, 1.5 m right of the centreline;
      - at the kerb, 2.3 m right, while it picks up or sets down;
      - waiting: in its lay-by, two bays;
      - parked: in a bay of its depot's deck, seen through the fins.
    - **The vehicles** (`vehicles::taxi_cab`, `taxi_shuttle`): ivory bodies, dark glass and an
      amber band, drawn as movers after the trams.
      - A cab for four, 3.6 × 1.8 × 1.65 m: most trips.
      - A shuttle for eight with a ramp, 5.0 × 2.0 × 2.2 m: 5 % of the fleet, one a depot at
        least (26 in the example).
      - A booking is for a party: one in 68 %, two in 20 %, three in 6 %, four in 3 %, five to
        eight in 3 % *(estimates)*.
      - A group goes to the nearest free shuttle. It waits without holding up the bookings
        behind it. The lanes' gaps take each vehicle's own length. Test
        `groups_ride_in_the_shuttles`.
    - **Depots** (`city::taxi::depots`, `forge_geom::depot`): the parking decks where the taxis
      no stand holds wait and charge, one for 25 stands (the example 4, the sandbox 9).
      - **Sites:** behind the building line of a taxi road, 48 by 32 m in a built block.
        - Clearances: 13 m from the block's other streets, 45 m from junctions, 40 m from
          stands, 150 m from the landmarks.
        - Each cluster of stands takes its nearest site, an industrial district's counted 800 m
          nearer; depots stand 1 km apart.
        - Their blocks hold no flats and no court paths.
      - **The deck** (redone 2026-10-15). The owner asked whether the cabs could drive in, about
        the door's height and the ramps, and how car parks keep the flows in and out apart. The
        first deck had no ramps, a column in its way in, and 2.1 m under its lintel. The new
        deck follows the usual practice (`docs/research/transit-and-vehicles.md` §9):
        - **Split levels:** the front half's floors at 0, 3 and 6 m, the back half's 1.5 m
          higher. Ramps of 1.5 m over 10 m (1 in 6.7) in the side strips join each half to the
          other's next floors.
        - **One way round:** the front aisle runs east and the back aisle west. Each strip has
          an up lane and a down lane, so no ramp is shared both ways.
        - **In and out apart:** the way in at the front's middle passes every floor's bays going
          up. The way out leaves at the front's west corner, at the foot of the last down lane.
          Each has its own one-lane driveway.
        - **Clear ways:** columns stand 7.5 m apart, only between bays, at the strips' edges and
          in the facades. Each level has 2.7 m clear, and the way in's fascia 2.3 m.
        - **People:** a stair and lift core in the front's east corner opens onto the pavement
          for the staff who keep the chargers and the cabs. The fleet would stop its cabs on a
          floor they work (ISO 23374); that is not simulated.
        - **Dress:** timber fins every half metre, concrete slabs, upstands round the ramps and
          the half-level edges, solar panels over both roofs.
        - **Room:** 177 bays, 15 to a row, four rows to a level of both halves.
      - **The driveways** climb the 2.5 m verge from the carriageway (ramps flared at their
        sides), then cross the footway at the walking level: nobody walking along steps down.
      - **The sim:**
        - the taxis past the stands' bays start parked, shared out by room;
        - a free taxi goes to a stand with a bay free within 1.5 km, else to the nearest depot
          with room;
        - every 30 s the depots send one to each stand short of taxis (all its bays at a quiet
          hour, one at a busy one);
        - at busy hours they call their taxis out, one at a time;
        - a cab drives through the deck in 45 s, in to its bay or out from it, and leaves 4 s
          after the one before;
        - an arriving taxi takes its stand's first free bay (counting the waiting ones put two
          in one bay).
      - **In the scene:** each parked cab keeps its own bay until it has driven out. Entering or
        leaving, it follows the deck's way (`depot::way_in`, `way_out`): up the driveway, round
        the floors the one way they run, nose into its bay, backing out of it.
    - **Numbers:**
      - the example: 511 taxis, 349 of them parked in 4 depots at 4:00, 336 an hour on, none out
        of place. From 8:00, at 9:02: 2 646 trips by tram, 1 372 by taxi, none waiting, a mean
        wait of 40 s, no taxi stuck;
      - the sandbox: 1 727 taxis in 9 depots; the trams serve half of the trips by tram or taxi,
        17 % of the rest book a taxi.
    - **Not yet:**
      - batteries and charging times;
      - shared rides.
    - **Tests:**
      - `waiting_taxis_stand_in_their_lay_bys_and_moving_ones_on_the_roads`: each on a
        carriageway or its depot's deck, none drawn one on another, at the start and an hour on;
      - `taxi_depots_stand_behind_the_building_lines_with_level_footways` and
        `the_taxis_no_stand_holds_wait_in_the_depots_and_refill_the_stands`;
      - `forge_geom::depot`'s: 177 bays clear of the columns, every bay reached one way round
        and left by the way out, no step over 1.5 m nor slope over 1 in 6.25;
        `the_plans_depots_are_the_decks_drawn`;
      - `at_the_morning_peak_the_taxis_keep_moving`: none stuck, a mean wait under 2 minutes,
        the trams carrying more than the taxis;
      - `forge-sim`'s tests: fetch-carry-return, determinism, busy-hour cruising, routes that
        follow the roads, a taxi stopping behind one standing, no two overlapping in a lane
        (6 122 pairs, the closest 2.0 m apart, 6 pulling out), one at a time in a junction
        (19 463 samples, none shared), and trips one tram line serves taking the tram.
  - **No hairlines:** the junctions' pieces (corner wedges, gap mitres) reach 0.3 m into the
    strips they join, and the carriageway's gaps close among the driven arms only (a pedestrian
    street's mouth between an avenue's arms left a sliver of pavement on the asphalt).
  - **One pattern across the tiles:** the ground's rows are projected from the scene frame
    (`RenderLayer::world_projected`), not from each 250 m tile mesh's origin. A pavement polygon
    kept in the next tile's mesh showed as a lighter band: its brightness noise (and the grass's
    hex tiles, offset per instance) started from another origin (the owner).
  - **One walking level** (0.30): pavements, shared and pedestrian streets, paths, squares,
    gardens and courts. Only the avenues' carriageways lie lower (0.16, a 14 cm kerb). A
    pedestrian way meeting an avenue runs on across its pavement (a continuous footway), and
    the doors' and shopfronts' sills all stand at the walking level (`structure::outside`).
  - **Faces:** a kerb's face along each carriageway, following a dropped kerb's slope, the
    ramps' sides, and a face down from the street ground's edge to the open ground (none over
    the water, where the bridges' sides stand).
  - **Blocks:** what is left of each face off the streets (`GroundMap::off_streets`). Their building
    lines are the pavements' real back edge, so every street front stands on it. Moved in by a
    width read from the nearest street, they missed it here and there, a strip of garden between
    the paving and the wall (the owner: "buildings have no foundations"). A built block's ground
    is paved 2 m deep along its streets (`GroundMap::forecourt`), its gardens behind: plain grass
    had met the pavement in the gaps between the buildings.
  - **Checks:** `the_street_ground_is_one_map_with_every_surface_once` (the example's 0.403 km² of
    carriageway and 2.047 km² of walking ground, none of it both), and the crossroads' and
    pedestrian street's unit tests in `city::ground`. The map takes 0.1 s for the example.
- **the network cleaned before it is drawn** (D-061, the owner: "junctions, road intersections
  and bridges are still a mess"; the map drew the network faithfully, and the network was the
  mess).
  - **Counted** (`graph::defects`): close junctions, short streets, sharp meetings, nodes on the
    water, bridges close together, doubled streets.
  - **Junctions:** those within 20 m, or joined by a street shorter than that, become one
    (`graph::tidy`). The groups are bounded (every member within 20 m of the middle), nearest
    pairs first; joined unbounded, junctions chained across a dense grid and its streets
    collapsed. Each street's moved end is cut back by twice the move and 5 m, and runs straight
    on to the new junction.
  - **Streets dropped:** of two drawn twice between the same junctions, of two leaving a node
    within 25°, and of two running doubled side by side, the lesser goes.
  - **Dog-legs smoothed** (`graph::smooth_jogs`, 2026-10-16): a traced street joined to the one
    ahead by a stub took two right angles 30 m apart inside one line (the owner: "one gentle
    turn would be enough"). A corner over 60° and, within 45 m, one the other way give way to
    one cubic curve from 12 m before the first to 12 m past the second, tangent to the street
    at both ends, unless it would leave the land or the street is a bridge. The example keeps
    no jog (`no_street_is_left_with_a_jog`).
  - **Bridges planned first** (`streets::bridge_sites`): one every 450 m along each river
    between built districts, square to it at its narrowest within 80 m. Main roads cross only
    there, arriving within 12 m of its axis and 25° of its way. A site no road took gets one
    straight on inland along its axis to the first street on either side, a T-junction there.
  - **The sandbox:** close junctions 353 → 0, short streets 342 → 0, sharp meetings 81 → 0,
    doubled streets 22 → 3, nodes on the water 1 → 0, close bridges 2 → 0, 12 bridges; the
    homes unchanged (about 980 k people). Test: `the_network_has_one_place_for_each_junction`.
  - **Checked over seeds** (the owner: "or else it will just go back if we change a param or the
    seed"): `the_rules_hold_on_other_seeds` builds the example city on four seeds, and the
    sandbox's `tests/seeds.rs` the real site on three. Each must have no network defect, every
    street polygon covered by its triangles, and every door and court reached on foot. The real
    site's seeds first showed what the example's did not:
    - junctions straightened last came close again (straightening now runs first);
    - narrow loops of one street traced up and back (two streets between the same junctions
      under 25 m apart are one);
    - streets judged at their width before a tram widened them into one another (judged at a
      tram avenue's 24 m);
    - a court with no wall stretch facing one (passages may run slant);
    - a street door onto a pocket the block's buildings close (back to a window when its
      building keeps another door).
- **quays, promenades and the seafront** (D-061). With no street along a bank, the land between
  the last street and the water was no block at all: bare ground with streets crossing it (the
  owner's "gigantic paved areas").
  - **Quays** (`streets::quay_lines`): a pedestrian walk along each river bank through the built
    districts, 6 m from the water's edge.
  - **Seafront** (`streets::coast_lines`): a promenade along the sea's and the lagoons' shores,
    by marching squares on the distance to the sea. It runs 34 m inland, a beach before it (6 m in
    the industrial quay).
  - Both close the faces into blocks and are pedestrian whatever their district.
  - **Promenades** (`streets::promenades`): 13 m along each river bank on land. They are walking
    ground in the street map, so they meet the quays and streets with no step. They had been drawn
    apart at 5 cm, a 25 cm edge (the owner: "avoid step and edge for accessibility, think
    wheelchair all the time"). Their edge over the water gets the bank's stone wall and
    balustrade.
  - **Blocks and quay lawns** lie at the walking level, cut round the streets, the walks and the
    rivers' water. A park had covered a river's strip by the mouth.
  - **The promenade follows the real bank.** Where the land starts within 12 m of the river's
    strip, it covers the edge cells as an embankment from the strip itself. Farther off (at a
    mouth), it starts at the bank. It needs no open sea under it and its back on land.
  - **Quays share the promenade's line.** Its runs break where the setback jumps, and the
    backtracks are dropped.
  - **Mouth fixes.** All strokes join round. A mitre at a near reversal shot out as a walled spike
    in the water at the mouth (the owner).
  - **The rivers' water** is drawn from the same strokes, tile by tile. Strips between two offset
    lines can fold.
  - **The shore** counts the rivers' cells as well as their strip. Past the strip, at a mouth,
    river cells had been dry.
  - **The sandbox's site** opens its land by two cells (`Site::open_land`, after
    `Site::clear_slivers`), so no spit thinner than about 40 m is left.
- **the city's outer limit** (`CityParams::reach`): land farther than `reach` radii from every district's
  anchor stays countryside, with no streets or blocks (main roads still cross it). Every land
  cell had taken its nearest district however far: islands of city grew wherever the hills left
  room, with empty ground nearer in (the owner). The sandbox uses 1.5, with denser districts
  and hillside quarters on the south bank's gentler slopes to keep its million. Courtyards under
  100 m² are none: the block is built through, as such light wells had no passage to them.
  Tram lines end in the city: a line's end waypoint out in the countryside moves in along it to
  where the built districts begin, or goes (lines drawn to the frame's edges ran on into empty
  land, the owner).
- **open land as nature:** the ground outside the city is a dry, olive scrub. The reserves and
  any land outside every block carry a loose wood: a jittered grid of 16 m, under half filled, 8 m
  clear of the streets. They were gravel, which the owner took for paving.
- **tram alignments** (`transit::alignment`): the route straightened to within 1.5 m of its
  streets, then each corner rounded, a junction's turn on 18 m and a gentle bend on a longer arc
  (up to 120 m). Each little jog had been rounded on its own into a run of S-bends (the owner:
  "trams wouldn't make a series of small undulating turns").
- **one track, one stop, where lines share a street:**
  - **Tracks** (`transit::share_tracks`): each line in turn is drawn onto the alignments of the
    lines before it wherever it runs beside one within 3 m, the same way or the other. It moves
    in full within 1.5 m and blends off to 3 m, so a line leaves the trunk on a curve. Only the
    first line draws the shared rails. In the sandbox, 96 % of the shared stretches now lie
    within 5 cm (56 % before), and the rest are the turnouts. Each line had laid its own rails
    beside the other's, up to a metre or more apart.
  - **Stops:** a line takes a stop the lines before it placed on its track within 150 m, and
    each platform is drawn once. Several lines' stops stood one over another (the owner).
  - **Placement:** stops keep clear of a junction's corners and crossings (a platform's ramp
    met a zebra) and are never on a bridge. A stop with no clean place within 400 m is dropped.
    A line left with fewer than two stops searched 800 m for a place clear of the junctions on
    a curve before falling back on the stop's own place, which is a junction as often as not
    (the owner, 2026-10-15: a stop over the intersection, the cross street's taxis driving
    through its platforms). A new stop never stands within 120 m of another line's stop on the
    same track: it shares that one or moves on (two stops stood one after the other).
- **the tiles' LOD** weighs normals (`CookOptions { normal_weight: 2.0 }`): at the coarser levels
  a pavement's kerb and back faces folded into its top, long tilted triangles where two paving
  patterns seemed to blend on the pavements and the bridges (the owner guessed an LOD issue).
  The far walls (`tile.masses`) also weigh their texture coordinates (`CookOptions::uv_weight`,
  a unit the facade texture's four bays, 12 m): the permissive simplifier collapsed across the
  facade's texture seams (plain band to window bays, ground floor to the storeys), and from afar
  the lower storeys' windows smeared into diagonal streaks, with a triangle on a facade that went
  away closer (the owner, 2026-10-15). Only the city's walls set it, so no other demo's cooking
  changes. View: `reports/2026-10-15/facade-seams-before-after.png`.
- **bridges:** a stone parapet a metre over the pavement along both edges, 0.35 m thick, and
  the deck's side below it down to the water; the walker walks against the parapets. The
  pavements stopped at a bare edge over the river (the owner: the bridges' pavements looked
  broken). The parapets stand wherever the street ground's own edge borders the water as drawn
  (a river's strip or the smooth shore's open water), so they follow the deck to the banks and
  a quay street's edge over the water too (D-061). Run along the bridges' centre lines' sides,
  they went on past a bridge's end into the junction at its head, a wedge on the quay (the
  owner); cut by the site's cells, they first ran over the land, then stopped short. Where two
  walls met at a corner, each a box with square ends, they left a notch and showed an open end
  (the owner, a quay's corner, 2026-10-15): each wall's inner line is now mitred against its
  neighbour's at a shared end, and a free end is capped. View:
  `reports/2026-10-15/parapet-corner-before-after.png`.
- **street-workout parks:** one in each park of a hectare or more, 14 by 10 m on a rubber
  floor:
  - pull-up bars at three heights, monkey bars, parallel bars, a dip station;
  - wall bars, push-up handles, an inclined sit-up bench.

  The park's trees keep clear of it. There are no playgrounds: the owner wants no children in
  the cities.

## Squares (#245)

![A square and the market hall](images/city-plan-piazza.png)

*Left: a square by the river avenue at 18:00, from the pavement, paved in limestone, its
fountain in the middle, an alley of planes round it (each tree in its own tones), shops round
it. Right: the market hall on its square under the planes, benches round it.* Squares come from
three rules:
- the block nearest each built district's centre and each landmark becomes a square (within
  150 m, at most 1 ha), unless one already opens within 120 m;
- so do the old towns' blocks of 600–3 000 m² that stand 180 m from any other square: their
  small squares;
- slivers too small to build on.

Squares are paved, with a row of trees every 10 m, 3 m inside their building lines. Those of
0.5 ha or more get a second row 11 m further in, an alley of planes (a Provençal *cours*). A
fountain stands in the middle of those of 0.15 ha or more (`furniture::fountain`), grander on
grander squares, and benches stand every 15 m inside the trees, facing the middle
(`furniture::bench`). Squares of 1.5 ha or more are gardens: a lawn in the middle, 14 m inside
their building lines, a paved border round it under the trees. The example has 12 squares,
slivers included.

**Water mirrors** (2026-10-15; the owner: "water mirrors as in Nantes"). Two centimetres of still
water over dark granite, level with the paving, as in Bordeaux's (130 × 42 m) and Nantes'
mirrors. The sky and the facades show in it through the mirror rays. They go on the squares
without a landmark:
- every other square of 1.5 ha or more, in place of its lawn: up to 80 × 30 m;
- every third square of 0.15 ha or more, in place of its fountain: up to 30 × 9 m, 14 m inside
  the building lines from 0.5 ha and 8 m below, clear of the trees and the benches. The chess
  tables move out of the water.

The lawns and the mirrors are cut out of the paving (`GroundMap::off_streets_split`): one laid
2 cm over the other could fight it. The example's squares of 0.15 ha or more mostly hold its
landmarks; four seeds give one mirror. The test is
`water_mirrors_lie_level_in_their_squares_clear_of_the_trees_and_benches`.

**The landmarks** (`LandmarkKind`) get their own square: the built block of 0.15–3.5 ha
nearest their anchor becomes one. A crane gets none: it stands on a quay. Each then stands on the nearest square of 0.15 ha or more
within 160 m (`Landmark::square`). A terraced hall without one takes the nearest quay within
250 m. A gantry crane looks for a quay first, the nearest within 600 m, and a square after.
A weir bridge needs no block: it spans its river at its anchor. Five have models
(`forge_geom::landmarks`):
- **the market hall,** the iron and glass halls the Midi's old towns built on their squares:
  - a stone plinth, iron columns every bay on stone bases, girders at the eaves;
  - a pitched glass roof on iron rafters, with a glazed lantern along the ridge;
  - two rows of stalls under striped canvas, the sides left open;
- **the terraced hall,** a long, low civic hall whose roof steps down in four public terraces
  towards the nearest water, each step wrapped in a glazed band, glass balustrades and benches
  on the terraces, a glazed ground floor under a colonnade (the example's harbour house);
- **the gantry crane,** a shipyard crane kept: A-framed legs on rail bogies, the girder, a jib
  over the nearest water with its stays, the trolley, its cab and its hook. It is drawn only
  where it finds a site;
- **the pavilion,** a belvedere's domed rotunda on a stepped octagonal platform, eight
  columns, a green copper dome and its lantern, larger on larger squares;
- **the weir bridge,** square to the flow where the river passes nearest its anchor:
  - a stepped stone weir from bank to bank, its crest just above the water;
  - stone piers every 10 m or so carrying a railed footbridge 1.2 m up, a ramp down to each
    bank;
  - a sluice gate across each bay upstream, and over each pier a steel portal with its
    gate's hoist.

  The example has one upstream of the old town's bridge.

![The weir bridge](images/city-plan-weir.png)

*The example's weir bridge seen from downstream, the old town on the left bank.*

Each stands inside its square's outer tree row (the inner row and, on a quay, the trees within
45 m give way), along the square's longest side, in its own material group, in place of the
fountain. The others are placed but not drawn yet.

![The harbour house](images/city-plan-harbour-house.png)

*The example's harbour house on its square by the beach, its terraces stepping down towards
the water.*

## Tiled roofs in the old towns

![The old town's roofs](images/city-plan-roofs.png)

*The old town on both banks from 90 m up: the terracotta roofs of a row share one ridge along
the street, stepping where the heights change; beyond it, the green, solar and terrace roofs.*
A district's share of tiled roofs is its kind's unless its style bible sets it
(`DistrictSpec::tiled_share`): 85 % in an old town, 90 % in a village, none elsewhere. The
bible's green-roof check counts the flat roofs only.

Each roof rises at about 29° from its street and court walls to a ridge, those edges moved in
as far as they go (the straight skeleton's roof for a convex footprint, near enough). A wall
onto a neighbour (`facades::edge_kinds`), across the main front and at most 18 m long, carries
a gable instead when the neighbour stands as tall as the eaves all along it
(`facades::neighbour_heights`, probed a pace outside the wall at five points). So the houses of
a row share one ridge. A house taller than its neighbour showed its gable above theirs as a
plaster wedge, which the owner circled on 2026-10-15 ("should not exist"): such a wall slopes
like the others now, and only the blank party wall below the eaves stands over the lower roof.
A free-standing building is hipped on every side. A roof stops rising after an 8 m run (4.4 m
up), its top then a terrace. The tiles are the brick courses tinted terracotta, read by texture
coordinates that run along the eaves (u along the eave, v up the slope, 2.5 m a repeat):
projected from the world, their rows ran askew to the building (the owner, the same night).

## The people on foot (D-059's step 6, a proof of concept)

```
cargo run --release -p city-plan -- --street --people 16.3   # from 16:18
```

![People](images/city-plan-people.png)

*Top: from 16:18, the park's visitors standing about it; at 08:12, the morning's cyclists in the
river avenue's lanes and walkers on its pavements. Bottom: at 20:00, a lane's café terraces; the
evening's walkers.* `--people HOUR` builds the plan's population from the masses
(`forge_sim::population`, 307 283 people, 2 s) and runs its day to that hour. Each frame:
- **who is drawn:** people within 220 m of the camera, as placeholder figures
  (`forge_geom::figures`: blocky, in eight palettes, walking, standing, riding a bike or
  seated). A walk is a 1.4 m gait cycle of eight poses: a left stride, a half stride, passing,
  a half, a right stride, a half, passing, a half, a new pose every 17.5 cm, each person at
  their own phase. Four poses held 0.7 m each read as jerky (the owner); skinned humans
  replace them with #251;
- **those out at a café:** seated at the kit's terraces. Each seat is taken in the share that
  the people out at the cafés within 300 m make of the seats there, up to nine in ten. The
  same seats stay taken while the share holds.
- **those on their way,** as the simulation's trip goes (`population::Trip::mode`):
  - on foot or by bike: the shortest way along the streets (A* on the road graph) from the
    street nearest their start to the street nearest their goal, at the pace their trip's time
    gives, walking on the pavement or riding in the carriageway's right lane, by the kerb and
    at the carriageway's level (they rode through the tram stops' platforms, 13 cm up). At a junction a
    walker goes on to the corner where the pavements' lines meet and crosses straight over,
    as at a crossing; they jumped up to 17 m from one side to the other before;
  - by tram: waiting for the trip's first five minutes at a platform of a line going their
    way, found once per trip: among the platforms within 500 m of their start, one of a line
    with a stop within 600 m of their end farther along its way (outbound if that stop's index
    is the higher), nearest by the walks at both ends, else the nearest platform (they waited
    at the nearest whatever its line and way: a crowd on a platform no tram came to, the
    owner, 2026-10-15). Until a tram of that platform's line stands there going its way: then
    they board. Aboard, they are not drawn yet (the trams' windows show no one).
- **those in a park:** they stand about it, kept 9 m inside its streets, spread over its own
  extent. Squares are not places people go and stand in: that filled one with a crowd, and the
  owner pointed out people would rather be at the cafés, the pubs and the tea rooms, or
  shopping. The squares have chess tables instead, with their players.

The drawn crowd runs on the real clock, a game second a second: at the game's three (an hour
in 20 real minutes, `population::GAME_SPEED`), its walkers would cross the screen at 4 m/s.
Near the player people walk at their real pace, which the owner found right. #251's humans
replace the figures.

**What a frame costs.**
- A scan of the population over 30 frames keeps who may be drawn round the camera (the
  reach plus 300 m) and who is at each place. Each frame draws from the last whole scan.
- Walking the million every frame took 77 ms on the CPU, about 15 frames a second (the
  owner's first walk). A street view with people now takes 2.3 ms a frame.
- **Who gets a slot.** A mesh (a palette in one pose) shows 60 figures. It shows the nearest
  of those it could, so a full mesh drops the farthest. Before, it dropped whoever the scan
  met last: in a busy street a walker ahead blinked in and out as their pose's mesh filled
  (the owner's second walk).
- **Motion vectors.** Someone keeps their slot while they stay in its mesh, and moves with it,
  so TAA and DLAA follow them. A slot that changes hands has no motion of its own that frame
  (`set_movers_still`): a walker's pose is another mesh every 0.7 m, and people come and go.
  The first fix gave every figure no motion of its own, only the camera's. A walker keeping
  pace ahead was then reprojected as the background and faded.

## Walking the streets (D-059's step 4)

```
cargo run --release -p city-plan -- --street        # then Enter: walk; Enter again: fly
cargo run --release -p city-plan -- --street --walk-start --walk=1.0,-1.4   # a scripted walk
```

Enter puts a Jolt character (1.8 m, the island's walker's) on the ground where the camera
stands, and the camera at its eyes, 1.65 m up (Alt+Enter is the app's borderless fullscreen). WASD walks along the view, Shift runs, and the
mouse turns the view. The masses round it are its walls: each footprint's convex hull raised to
its parapet, kept within 80 m as it goes. The ground is one slab at the streets' level, and the
pavements' 14 cm are stepped. A walk east into a lane's facade stops at its render.

![A walk up a lane](images/city-plan-walk.png)

*Three seconds up a lane at eye height, walked by the script: café terraces, a street tree, the
first floor's balconies.*

## Next (#245–#247)

- Squares at the radial districts' centres; the landmarks' sightlines.
- More checks on the checks map; a legend.
- The style bible's palette into the materials; more rules (cornice lines, opening rhythm) now
  that facades exist (#246).
- The kit, next (#247):
  - the 0.5 m module grid and its footprint checks;
  - a merged proxy per building (#38) in place of the textured boxes;
  - corner pieces and quoins;
  - roofs from the straight skeleton;
  - trims and materials of release quality (#249).
- Trees by species (vegetation-materials.md §1, the grower) in place of the one lumpy model.

[Ehret]: https://www.ehret.com/sites/default/files/content/downloads/bon_de_commande_volets_battants_-_suisse_romande.pdf
[PROVOBAT]: https://www.d-c.fr/wp-content/uploads/2024/04/PROVOBAT_Notice-de-montage_Volets-battants-standards-PVC-ALU_-2024.pdf
