# Consistent, organic buildings: Tiny Glade and five other builders, and what Forge's city can take

Research note, 2026-10-11, at the owner's request: the techniques Tiny Glade uses to keep its
buildings always consistent, with Townscaper, Summerhouse, Outside the Blocks, Dystopika and
"Belatique". The city needs more realistic adaptation than these toys, but they may show how to
make its structures more integrated and organic.

**Sources.** None of the developers has published a full account of their generators. For
Tiny Glade the most useful primary sources are:
- Pounce Light's Steam dev diaries and patch notes, read through the Steam News API;
- the developers' posts on X;
- the 80.lv interview;
- the abstract of their Graphics Programming Conference 2024 talk. The talk video was not watched.

Points marked *inferred* are a reading of these sources, not the developers' own words.

## 1. Tiny Glade (Pounce Light, 2024)

**What the developers say:**
- **No grid.** In Tomasz Stachowiak's words: "WFC is grid-based, but there are no grids in Tiny
  Glade."
- **One generator per element, chained.** Many generators, each tailored to one element, each
  with its own pipeline (creation, assembly, morphing or deforming), each feeding the next. The
  project began as real-time procedural generation on the GPU.
- **Brick by brick.** Every brick is placed one by one, and the wooden walls added in 2026 plank
  by plank, split round arches, windows, doors and roof shapes. Rectangular and round buildings
  take horizontal or vertical planks; freeform walls take vertical ones only.
- **Walls** are loops and paths. A 2024 patch says many generators could not yet handle closed
  loops.

**Elements change kind with their room and their context: the core of the consistency.**
- A doorway too wide becomes a run of arches (Anastasia Opara, June 2023).
- A chimney with too little room becomes a small vent pipe.
- Gate widths adapt, so towers can be slimmer.
- A path through a fence makes a wooden arch; through a wall, a door.
- A window near the ground, the water or a building's foot becomes a door. A door gets railings
  when raised and steps when it stands in water.
- Raising a wall turns its lintels and posts into stone arches and pillars.
- Stairs become platforms or ladders by their steepness; round a tower they wind, with landings
  at the corners.
- Railings and merlons give way round stairs, and brackets give way to openings.

**"Glue" between elements** (Opara, January 2024): every element is joined to its neighbours.
- Where a tiled roof meets a flat one, a row of merlons, or (a 2024 option) of short bricks.
- Raised pieces get brackets, supports and pillars by themselves, re-fitted when they move.
- Two wall flags close together merge into a string of small ones.

**Windows** ("Our windows are chameleons", March 2024):
- They sit on roofs, on the edge between roof and wall, on corners, and re-fit when the roof
  changes. The half-timbering follows them.
- Each vertex keeps one float, its signed distance to the nearest window, which drives irregular
  plaster damage round the windows (80.lv, September 2023).
- How windows avoid one another is not stated. *Inferred:* openings are intervals on a wall,
  checked against limits and reclassified (the "becomes a door" and "becomes arches" rules).

**Roofs:**
- Tiled tower roofs, gables, flat roofs with battlements or wooden railings; the tiles sway when a
  tower is raised fast.
- *Inferred:* roofed buildings are rectangles and circles ("shapes"), freeform walls are apart.
  Roofs look parametric per primitive footprint, and complex outlines come from gluing
  primitives. No mention of a straight skeleton.

**Edits regenerate locally.** A changed path or terrain recomputes only the walls in the regions
it touches (March 2024); stone columns follow the ground under them.

**The organic look comes from rules:**
- ivy avoids the windows and doors;
- props stand by windows, doors and gates, chosen by context: a wall to align to, a path, grass;
- clotheslines join windows;
- the bricks' randomness is tuned;
- a wall's top either follows the ground's slope or stays level, its top course filled in.

**Engine**, briefly: Rust, its own engine with Bevy's ECS only, Vulkan (after OpenGL). The GPC
2024 abstract lists real-time GI, heavy ray marching, GPU-driven rendering, some ray tracing,
stable shadows through a moving time of day, and ten-year-old GPUs supported. Since 1.16 the
roofs are in the BVH, so light bounces off them.

## 2. Townscaper (Oskar Stålberg, 2020–21)

- **Its grid.** A hexagon filled with triangles, random pairs merged, everything subdivided into
  quads, then relaxed (Stålberg, 2019).
- **Modules sit on the cells' corners**, chosen as marching cubes chooses: which corners are
  filled decides the corner piece. The pieces deform to fit the irregular quads. Stålberg notes
  the skew can be strong before anyone at street level notices.
- **Consistency:**
  - Wave function collapse picks among the modules allowed beside the neighbours, in real time,
    solved again locally on every edit.
  - Modules have priorities: gardens rank high but form only in enclosed flat areas.
  - Large shapes are predictable and learnable; small details vary.
  - When the solve finds nothing, it fails quietly rather than starting over.
- **Windows** are cut by a stencil after placement, so none is split by a corner. Chimneys,
  benches and bushes come in a later pass, where there is room.

(Tommy Thompson's 2022 article on Game Developer, drawing on Stålberg's talks.)

## 3. The other four

- **Summerhouse** (Friedemann Allmenröder, Unity, 2024):
  - hand-made pieces (blocks, doors, roofs, windows, chimneys, mailboxes) placed freely in a
    side view with depth;
  - the snap grid gets finer as the piece gets smaller;
  - nothing adapts by itself: consistency comes from one coherent kit at one scale;
  - little technical material.
- **Outside the Blocks** (Michał Kubas, 2025): a diorama builder of modular blocks:
  - "Autobuild" edits whole sections of a building;
  - layers, terrain, weather, lighting, themed kits (a medieval town among them);
  - no technical write-up found.
- **Dystopika** (Voids Within, Unity, 2024). Its ModKit documentation explains:
  - each tile is chosen by district weight;
  - the tile's normalised height picks a group of prefabs from ordered height breakpoints;
  - its seed picks the variant, so the result is deterministic;
  - "procbits" hold decoration with an amount slider;
  - a density brush paints the small buildings, chosen from the districts of their neighbours;
  - large transport props clear the low density in their way.
- **"Belatique"**: no game of that name was found.
  - The nearest is **Belle Citique** (Willis Studios, April 2026), going by its store page:
    blocks on a grid that is "a starting point", structures fitting the land, buildings arching
    over rivers and roads into tunnels and gates.
  - *Inferred* from a patch note about a level with "a perfectly square grid": the other levels'
    grid is irregular.
  - No technical material.

| Game | Placement | Consistency | Organic variation | Public material |
|---|---|---|---|---|
| Tiny Glade | gridless curves and loops; rectangles and circles | a generator per element, chained; kinds that change with room and context; glue; local recompute | brick by brick, ivy, props by context, wear by distance to the windows | medium |
| Townscaper | irregular relaxed quad grid | corner modules, WFC with priorities, local re-solve | the grid's irregularity, deformed modules, a decoration pass | good |
| Summerhouse | free, snap by size | one hand-made kit | hand-drawn pieces | low |
| Outside the Blocks | modular blocks, Autobuild | the kit, automation by section | materials, layers, props | low |
| Dystopika | free buildings, ground tiles | district weights, height breakpoints, seeded variants, clearance | density painting, procbits | medium (ModKit) |
| Belle Citique | a grid, probably irregular | blocks fit the land, arches over rivers and roads | unknown | very low |

## 4. Related techniques

- **Split grammars, CGA** (Wonka 2003; Müller et al. 2006): facades split into floors, bays and
  tiles, with repeats fitted to the length. The right tool for one facade plane. It knows nothing
  of corners or neighbours.
- **Straight-skeleton roofs:**
  - Laycock and Day (2003): roofs from footprints with a modified straight skeleton;
  - Sugihara (2019): events that remove the acute shapes roof boards cannot follow;
  - Kelly and Wonka, Procedural Extrusions (2011), and Kelly's thesis (2014): a profile per edge
    over a skeleton sweep, which gives hipped, mansard and gabled roofs, dormers, and vertical
    gables on party walls, on any footprint.
- **Constraint-based facade layout** (Bao, Schwarz and Wonka 2013): facade variants at any size,
  by heuristic search alternating with quadratic programming under alignment and symmetry.
- **WFC and model synthesis** (Merrell 2007; Gumin 2016): discrete adjacency on a grid; weak on
  long thin structures (Stålberg), and it can fail.

## 5. What Forge's city could take, realistically

1. **A facade as a 1D layout along its building line.**
   - Each storey band is measured by arc length along the lot's line, straight or curved.
   - Its bays are fitted by a small solver (dynamic programming over module widths), with window
     axes aligned between storeys.
   - On a curve: flat modules on chords cut at the bays' bounds; only plaster bands and cornices
     bend.
2. **Fallback chains, as Tiny Glade's.** Each module has a least, best and greatest width, and a
   chain of downgrades:
   - balcony window, then plain window, then narrow window, then blind panel or pilaster;
   - a wide ground-floor opening becomes an arcade;
   - a chimney stack becomes a vent.

   No bay overflows its wall, and nothing appears unexplained.
3. **Corners and junctions first, then the bays.**
   - Classify each vertex of the building line: convex, concave, acute, a chamfer candidate,
     near straight.
   - Place quoins, chamfered or rounded corners and corner balconies first, reserving length on
     both sides.
   - Between lots, glue pieces: firewall caps, cornice returns, stepped parapets where heights
     differ, downpipes at the party walls.
4. **Roofs from a weighted straight skeleton per building:**
   - pitched edges on the street and courtyard sides, vertical ones (weight 0) on party walls;
   - a flat terrace with a parapet when the footprint is degenerate;
   - low tiled hips and flat roofs (azoteas) suit the Mediterranean style;
   - Tiny Glade sidesteps this by keeping to primitive footprints (*inferred*); Forge's lots are
     arbitrary.
5. **Openings as shared data feeding a chain of generators.** Each wall keeps one list of
   openings (an interval along it, a storey, sill and head heights). That list feeds, in order:
   - the structure (D-060's walls with their thickness, real holes, lintels);
   - the dressing (frames, shutters, balconies on anchors of the structural panel);
   - ivy, signs and laundry lines, which avoid the openings;
   - a distance-to-opening vertex attribute, as Tiny Glade's, driving streaks under the sills,
     plaster wear and grime.

   When a wall chunk breaks, the dressing on it falls with it.
6. **Paths drive buildings:**
   - a footpath through a block's line makes a passage through the building (a sottoportego);
   - on a slope the storeys stay level and a stepped plinth takes the slope, with steps at the
     doors (Tiny Glade's "follow the slope or stay level").
7. **Local, deterministic regeneration** (Tiny Glade's region recompute, Dystopika's seeded
   variants):
   - each choice is seeded from stable keys (the lot, the storey, the bay counted from a corner);
   - an edit to one wall (a lot change, damage) regenerates that wall and its junctions only,
     and the neighbours do not reshuffle.
8. **Organic variation within a style:**
   - parameters drawn per building (storey height, window proportions, plaster tint, shutter
     colour);
   - a "regularity" per building, so vernacular ones let their window axes drift between storeys
     and formal ones keep them strict;
   - states per bay (shutters, plants, awnings);
   - ageing from geometry (distance to the ground, the roof's edge, the openings).
9. **WFC only for small discrete problems** (light wells, roofscape clutter), never for facades:
   a quiet failure is not acceptable in a realistic street.

## 6. Patents

- None found for Pounce Light, Townscaper, Dystopika, WFC or model synthesis, or straight-skeleton
  roofs. Web searches only; Google Patents' search did not render. **Not a clearance search.**
- **US 7,415,156 B2**, "Parametric shape grammar interpreter" (Carnegie Mellon University):
  expired (fees), adjusted expiry 2022-10-11.
- **US 7,502,511 B2**, "Shape grammar interpreter" (Carnegie Mellon University): expired (fees,
  per Google's assumption), anticipated expiry 2020-01-28.
- **CGA / CityEngine (Esri):** no patent found. The 2006 paper is public prior art. Any Esri or
  Procedural patent family is *unverified*: check Espacenet before building on CGA's specifics.

## Sources

- 80.lv interview (opened): https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games/
- 80.lv on the windows (opened): https://80.lv/articles/procedural-windows-in-pounce-light-s-tiny-glade-get-an-upgrade
- Tiny Glade's dev diaries and patch notes (opened through the Steam News API, app 2198150);
  "Our windows are chameleons": https://store.steampowered.com/news/app/2198150/view/5970156536060713334
- Tiny Glade's store page (opened): https://store.steampowered.com/app/2198150/Tiny_Glade/
- Steam forum, the developers on props and the FAQ (opened):
  https://steamcommunity.com/app/2198150/discussions/0/4844273660400957331 and
  https://steamcommunity.com/app/2198150/discussions/0/3709306945109116212/
- Posts on X (opened through api.fxtwitter.com):
  - https://x.com/h3r2tic/status/1695073463294120219
  - https://x.com/anastasiaopara/status/1674793900031242243
  - https://x.com/anastasiaopara/status/1747555860581068927
  - https://x.com/anastasiaopara/status/1682762738710241281
  - https://x.com/anastasiaopara/status/1740759863049822299
- GPC 2024 archive (opened): https://graphicsprogrammingconference.com/archive/2024/ (the talk
  video not watched: https://www.youtube.com/watch?v=jusWW2pPnA0)
- How Townscaper works, Game Developer (opened):
  https://www.gamedeveloper.com/game-platforms/how-townscaper-works-a-story-four-games-in-the-making
- Sylves' Townscaper grid tutorial (opened):
  https://boristhebrave.com/docs/sylves/1/articles/tutorials/townscaper.html
- IndieCade Europe 2019 talk (search result only): https://www.youtube.com/watch?v=1hqt8JkYRdI
- Summerhouse (opened): https://en.wikipedia.org/wiki/Summerhouse_(video_game)
- Outside the Blocks (opened): https://store.steampowered.com/app/2350220/Outside_the_Blocks/
  and https://80.lv/articles/tiny-glade-like-3d-diorama-builder-has-been-released
- Dystopika (opened): https://store.steampowered.com/app/2379910/Dystopika/ and its ModKit:
  https://voidswithin.atlassian.net/wiki/spaces/DYSMOD/pages/105545777/Create+a+Tile
- Belle Citique (opened): https://store.steampowered.com/app/3445040/Belle_Citique/
- Papers (search results only):
  - Müller et al. 2006, https://www.doi.org/10.1145/1141911.1141931
  - Kelly and Wonka 2011, https://peterwonka.net/Publications/pdfs/2011.TOG.Kelly.ProceduralExtrusions.TechreportVersion.final.pdf
  - Kelly's thesis, https://theses.gla.ac.uk/4975
  - Laycock and Day 2003, https://dspace5.zcu.cz/handle/11025/991
  - Sugihara 2019, https://otik.zcu.cz/handle/11025/35614
  - Bao et al. 2013, https://history.siggraph.org/?p=109156
- Patents (opened): https://patents.google.com/patent/US7415156B2/en and
  https://patents.google.com/patent/US7502511B2/en
