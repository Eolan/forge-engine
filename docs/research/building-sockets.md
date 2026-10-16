# Building pieces with sockets: snapping, stability, checking

The research behind the owner's "snap" proposal (2026-10-16 evening, the village walk of #266):
"would using some kind of snap system with the module parts help, if it does not snap
everywhere, it means something is wrong, we need a way to detect anomalies for building and
probably roads, walls and other things"; "if one day we allow players to build structures we would
need a snap system anyway"; "the shape of the module piece and its type in itself would authorize
or reject what's possible to join it to"; "look at Enshrouded"; "apply it to sidewalks and roads as
well". Desk research of the same evening (a remote research agent). It adds to
[cosy-builders.md](cosy-builders.md) (grammars, WFC) and [building-structure.md](building-structure.md)
(destruction, Jolt), whose entries are referenced, not repeated. Labels: **[web] [docs] [paper]
[talk] [patent]**; **read** (opened) or **seen** (a search result only). Google Patents refused
automated access for most of the session (503, then a bot check, not passed): claims marked
"extract" come from search summaries. Not a clearance search, not legal advice.

> **In five sentences.** Shipped games join pieces three ways: snap points or typed sockets
> (Valheim, Rust, Conan Exiles, Space Engineers), grid cells (Fortnite, Minecraft, 7 Days to Die,
> The Sims) or voxels with prefab stamps (Enshrouded); Townscaper and Bad North generate with
> modules whose edges are labelled and let WFC choose. **Enshrouded has no structural integrity
> or building physics**: no source says otherwise (its 1.0 changelog of 2026-10-15 included), a
> tower whose base is destroyed stays floating; its strength is freedom (voxels, prefab shapes,
> terraforming). The useful connector models are Space Engineers' rectangles a face with veto
> masks and coupling tags, Conan Exiles' typed sockets with a stability cost, Tessera's painted
> faces with a matching matrix, and marian42's symmetric/flipped connectors; games' "stability"
> is connection to the ground, support decaying a piece or a metre, or a mass budget a face, not
> statics. The owner's rule becomes a test: every required socket matched exactly once by a
> compatible one, beside four checks sockets cannot express (overlaps and coplanar faces,
> support, rooms and openings, networks). Live patents met: two Nintendo US patents on bonding
> objects (Tears of the Kingdom's "Ultrahand" family), NCSOFT's Korean patent on WFC over graphs,
> Sony's stress nodes (already in `building-structure.md`); none on Valheim-like support or on
> typed sockets for building pieces.

**Contents**

1. [How shipped games build](#1-how-shipped-games-build)
2. [Socket models in generation](#2-socket-models-in-generation)
3. [Stability in real time](#3-stability-in-real-time)
4. [Sockets as an anomaly checker](#4-sockets-as-an-anomaly-checker)
5. [Patents](#5-patents)
6. [Recommendation for Forge](#6-recommendation-for-forge)
7. [Sources](#sources)

---

## 1. How shipped games build

| Game | Pieces | Joining | Support | Lesson |
|---|---|---|---|---|
| **Enshrouded** (Keen Games) | an in-house voxel engine drawing polygons from the voxels [6]; blocks (wall, roof, terrain; 0.5 m listed [1], 1 m in a guide [4]: unconfirmed) and prefab "building shapes" of 2 and 4 m: foundations, walls, window and door frames, columns, stairs, roof sides, corners, ridges [1][3]; terrain shapes and brushes [1] | blocks and shapes turn by 90°, props freely [7]; snapping a mode, unreliable at times [4] | **none**: no structural integrity [4], floating towers reported and defended [5]; the 1.0 changelog adds free-flight building, a collision system, a "brick renderer", explosions and "Sparkworks" logic, nothing on support [2] | freedom on voxels; a stamp has no sockets to check: Forge's terrain and gardens, not the checker |
| **Valheim** (Iron Gate) | prefabs with untyped snap points (tagged child transforms) | the nearest pair, ghost to placed piece, within about 0.5 m [11][11b]; the held piece's point cycled since 2023 [10] | ground, rock or tree gives the material's maximum; else the parent's × (1 − loss × distance), the loss blending vertical and horizontal by the angle; the best parent, not the sum (two parents over 100° apart average); breaks at the material's minimum. Wood 100/10/⅛/⅕, stone 1000/100/⅛/1 (no cantilever), iron 1500/20/1⁄13/1⁄13. Every 0.5 s, 50 pieces a cycle: failures ripple for seconds [8]; a calculator from the decompiled code has a per-hop form [9] | readable support; recompute it at once, not in a ripple |
| **Fortnite** (Epic) | walls, floors, ramps, roofs on 512-unit tiles [12][13] | the grid; openings as a cell mask (3 × 3 on walls and floors, 2 × 2 on ramps and roofs; the centre a window, centre and below a door) [12] | connected to the ground or breaks at once [12] | openings as masks on a piece |
| **Minecraft** | blocks | the grid | none, but scaffolding counts its distance from a supported column, 0–7, and falls at 7 [14] | Valheim's rule in integers |
| **Rust** (Facepunch) | building blocks with sockets | 2015: stability by physics queries "slow and unreliable", blocks not knowing their sockets nor which were taken, the weight on them ignored [15]; fixed with sockets that track their occupancy and "entity links" between pieces [16]: "much more predictable", fewer exploits [17] | along the links | joints are graph edges, never overlap queries at run time |
| **Conan Exiles** (Funcom) | a socket component a piece [18] | each socket a position, a rotation, one or more **types** (building, castle wall, fence foundation, fence wall, siege foundation, gate, rampart, defense, corner, hatch, strut); a colour-facing convention for a normal or turned join | an **attach cost** a socket: the stability lost joining through it, 100 forbids; foundations on the ground 100 % | typed sockets with their cost |
| **7 Days to Die** | blocks with a mass, a vertical support yes/no, a horizontal capacity a face (wood 40, concrete 120; a wood block weighs 5) [19] | the grid | a column down to bedrock is stable; an overhang's mass against the faces holding it; over the limit the attached series falls at once; previews colour strain | a capacity model, cheap and readable |
| **Space Engineers**, **Medieval Engineers** (Keen Software House) | blocks with **mount points**: axis-aligned rectangles on each face [20] | two blocks join if rectangles on touching faces overlap; property and exclusion bits (1 extruding, 2 thin, the rest free) veto both ways; a coupling tag; full cover makes a face airtight | a grid cut in two becomes two; static pieces split off may turn dynamic [21]; Medieval Engineers simulates integrity from weight, materials and torque, with a weak-point view [22][23] | rectangles a face, masks and tags |
| **The Sims** | walls on tile edges | the grid | — | rooms derived from walls (a floor or full enclosure) [24]: the checker's semantic layer |
| **Townscaper**, **Bad North** (Oskar Stålberg) | Townscaper: a hex grid split into quads and relaxed, tiles as corner segments (marching-cubes style), WFC choosing each cell's module, a pass for windows, chimneys, bushes, "recipes" (gardens) first [25]; Bad North: 300+ tiles modelled in Maya, their **edges scanned** in Unity for the adjacencies, some over several cells [25] | WFC over labelled edges; the dual grid (tiles on the grid's vertices) cut one tool's set from 14 tiles to 5 [28] | — | Townscaper's cells "allowed to fail silently" leave thin structures hanging; Bad North checks each island's navigability and retries; long thin tile sets failed often [25]. Talks: [26][27] |

## 2. Socket models in generation

- **WFC and model synthesis.** Merrell's model synthesis (2007–2010) learns adjacencies from an
  example and generates 3D shapes; Gumin's WFC (2016) re-derived it, "nearly identical" for
  Merrell [29][30][31]. Both take the cell with the fewest options, propagate, and fail at a cell
  with none ("surprisingly rarely" with suitable tiles [29][33]). Karth and Smith: WFC is constraint
  solving [32]; DeBroglie adds whole-output constraints (border, fixed tile, count, separation,
  symmetry, paths) and backtracking [35].
- **What a socket encodes:**

| Property | Found as |
|---|---|
| Type | a number a face [37]; a colour a face cell [36]; named types, several a socket [18]; a tag [20] |
| Compatibility | equality; a matrix of which colours match ("black does not match with itself, but does match with white") [36]; veto masks both ways [20] |
| Size, shape | a 3 × 3 grid of colours a face, all nine matching [36]; overlapping rectangles [20]; edges scanned from meshes [25] |
| Orientation, symmetry | horizontal connectors flipped, unflipped or symmetric, vertical ones with a rotation 0–3 or free [37]; tile symmetry classes [29]; normal or turned [18] |
| Cost | stability lost a socket [18] |
| Exceptions | pairs that fit but "just don't look nice next to each other" [37] |

- **Authored, scanned or learned:** painted or declared a piece (Tessera, marian42, Conan, Space
  Engineers), scanned from geometry (Bad North), learned from an example (Merrell). Bounded worlds
  pin their borders ("air" on top, "solid" below) [37]; chunked worlds contradict at chunk borders,
  and marian42 moved to one map with backtracking [37]. Tile sets grow fast: 16 corner-matched
  squares (6 with rotation), a 47-tile "blob" set for edges and corners [34].
- **Graphs and kits:** NCSOFT's WFC on graphs built from navigation meshes [38], patented (§5). A
  2025 HICSS paper composes buildings from commercial modular kits through connectors, with rules
  at a domain level and a kit level, so a new kit needs only its own rules [39]: the nearest match
  to the owner's idea. CGA, which Forge's grammar follows, has context rules keeping windows and
  doors off walls' intersections [40].
- **For Forge:** the kit's modules fill scopes of any size (`forge_geom::modules`, the grammar's
  terminals), so a socket sits on a scope's boundary with an **extent** (an interval or a rectangle,
  as Space Engineers') and a **profile** (a cross-section), not at a point. `cosy-builders.md` keeps
  WFC to small discrete problems ("a quiet failure is not acceptable in a realistic street");
  Townscaper's silent failures [25] agree.

## 3. Stability in real time

Four tiers on the piece graph the sockets make, cheapest first:

1. **Connectivity** (Fortnite, Space Engineers): union-find or a search from the anchors (the
   ground-bearing sockets), linear in the pieces touched; finds floating pieces exactly: the
   checker's.
2. **Support decay** (Valheim, Minecraft's scaffolding, Rust, Conan): the best parent's value less
   a loss a joint or a metre; the best product from the anchors is a widest path, one Dijkstra-like
   pass with no ripple [8]. Readable (it colours the ghost), not physics: a second support adds
   nothing, an arch is no better than a beam.
3. **Capacity and load paths** (7 Days to Die, Medieval Engineers): weights on pieces, capacities on
   joints [19]; in general a flow from every weight to the anchors, overloaded when the maximum flow
   [47] falls short of the weight, the minimum cut naming the weakest joints; no bending moments
   (a lower capacity on horizontal joints stands in).
4. **Equilibrium and stress:** procedural masonry made to stand by tuning a grammar's free
   parameters (Whiting, Ochsendorf, Durand 2009 [41]); Blast's stress solver on a bond graph,
   iterations a frame, overstressed bonds reported [42]; BrickSim's internal forces of snap-fit
   brick assemblies in about 5 ms, predicting 150 real assemblies' stability [45]; Legolization,
   StableLego [46]. Offline for generated buildings, at run time only for small player builds; Sony's
   patent (§5) claims stress nodes breaking over a threshold until about 2030.

**With Jolt:** buildings stay static; a component an edit or damage frees (tier 1) goes with debris
(Valheim, Fortnite) or becomes a body. A body created static needs `mAllowDynamicOrKinematic`
first; constraints break by their impulse ("lambda") after a step [43]. `building-structure.md`
adds `MutableCompoundShape` for edits by piece, and that Jolt has no fracture or stress solver.
Chaos breaks connections over an impulse limit and pins pieces with anchor fields [44]: the same
split between the piece graph and the physics.

## 4. Sockets as an anomaly checker

Little validates generated architecture: BIM rule checking in four stages (interpret the rules,
prepare the model, execute, report) [48]; answer-set programming for yes/no validity [49]; Bad
North's validate-and-retry [25]; DeBroglie's path constraints [35]; bug detection on rendered
frames [50], a complement later. No tool checks a procedural city through its connectors.

The checks, each on data the generator has:

1. **An unmatched required socket:** a wall seam with no neighbour, a road end with no junction or
   end cap, a garden wall's end touching nothing: gaps and holes.
2. **A mismatched pair:** types, profiles (wall thickness, cornice height), polarity or
   orientation differ.
3. **A double match:** a socket matched twice: overlapping pieces.
4. **Overlaps:** solids intersecting past a tolerance; **coplanar faces from two pieces** (the
   owner's z-fighting rule, `forge-geometry-pitfalls`); intrusions into keep-out volumes (doorways,
   passages, trunks).
5. **Floating or overloaded pieces** (§3, tiers 1 and 3).
6. **Openings against rooms:** a door joins two spaces whose floors meet level (the wheelchair
   rule); a window faces open air; every flat is reached from the street.
7. **Networks:** road and path ends matched, widths and levels equal, slopes ≤ 1:12 where people
   walk.

Each anomaly a record (kind, pieces, sockets, place); the counts by kind a gate in the map check
(`city::anomalies` already counts seven kinds without sockets); an overlay colouring sockets
(green matched, red unmatched, magenta overlapping, orange unsupported).

## 5. Patents

| Patent | Holder, jurisdiction | Status | Claim 1 | Alternative |
|---|---|---|---|---|
| KR 10-2310107 B1 [P1] (read) | NCSOFT; Korea, no foreign family listed | **live**, granted 2021-10-08, to about 2039-11-12 | adjacency rules between tiles; nodes generated and their connections extracted; tiles placed by picking an unplaced node, placing an allowed tile and propagating | WFC on a lattice or in grammar slots, no node generation; Merrell 2007, Gumin 2016 earlier; Korea only |
| US 12,440,766 B2 [P2] (extract) | Nintendo; US (PCT priority 2022-03-03) | **live**, granted 2025-10-14, to about 2042 | movable bondable objects; the user selects one; a "bonding object" marks positions on it and a second, updated as the first moves; they bond there on instruction; a dependent claim on preferred bonding points | pieces placed from a menu onto authored sockets with a ghost (Valheim, Rust, Conan earlier); no carry-and-glue of free objects. **Ask the owner before any vehicle or contraption builder.** |
| US 12,667,784 B2 [P3] (abstract) | Nintendo; US | **live**, granted mid-2026 (dates conflict) | an assembled object; selecting a member with a release input detaches what is connected to it | static pieces removed one at a time, support recomputed; read the claims first |
| US 8,152,613 B2 [P7] | Sony Interactive; US | **live**, to about 2030-07-26 | claim 1: tiles in a damage radius removed, those it crosses split; claim 9: stress nodes a tile under gravity, breaking over a threshold | tiers 1–2 at run time. **Ask the owner before any stress-and-break pass.** |
| US 12,364,930 [P4] | GREE; US | **live**, 2025 | a city builder's templates moving existing contents to the template's places by the shortest distance | blueprints that place new pieces and never move existing ones |
| US 12,220,640 B2 [P5] | Electronic Arts; US | **live**, to about 2041–43 | a player's bounds and budget; a floor-plan generator; furniture by a graph neural network | rule-based rooms (Forge's) |
| US 11,911,697 B2 and three continuations [P6] | Tencent; US (Chinese priority 2019, an EP member) | **live**, to about 2040 | an object built by filling a 3D model's outline with voxel blocks | no "import a model and fill it" feature |
| US 10,964,122 B2 [P8] | Microsoft; US | live, claims not read | snapping a virtual object to a target surface | socket-to-socket snapping |
| US 12,164,741 [P9] | holder not confirmed | live, claims not read | a snap point activated in AR | as above |
| US 7,979,251 B2, US 2007/0262984 A1 [P10] | LEGO; US | 2007 filing, if in force to about 2027 (unverified) | instructions by reversing a disassembly; the application describes virtual bricks whose connection elements join only compatible pairs | prior art for typed virtual connectors |

Searched and not found: patents on Valheim-like support propagation, 7 Days to Die's capacities,
typed sockets for building pieces, Townscaper's dual-grid modules. Seen in listings, not read:
CN 119003677 A (2024, "wave" in its cut title), Unity's US 11,887,229 B2. `cosy-builders.md` found
CMU's shape-grammar patents expired and no CGA patent; NCSOFT's corrects its "no WFC patent found".

## 6. Recommendation for Forge

**The data model:**

```rust
pub struct PieceDef {
    pub name: &'static str,     // kit module, structural part, road piece
    pub family: Family,         // Wall, Opening, Slab, Roof, GardenWall, Gate, Road, Path, Prop
    pub sizing: Sizing,         // Fixed(Vec3) | Scope { min, nominal, max } per axis
    pub material: Material,     // stone, rubble, brick, concrete, timber, tile, iron
    pub density: f32,           // weights
    pub sockets: Vec<SocketDef>,
    pub solids: Vec<Volume>,    // overlaps, Jolt shapes
    pub faces: Vec<FacePlane>,  // the coplanar check
    pub keep_out: Vec<Volume>,  // a door's passage, a gate's 1.2 m, the eaves
}
pub struct SocketDef {
    pub kind: SocketKind,
    pub profile: Profile,       // "wall 0.40 rubble", "eave 30° tile"
    pub frame: ScopeFrame,      // origin, outward normal, up, in scope coordinates
    pub extent: Extent,         // Point | Span | Rect
    pub fit: Fit,               // Exact | Overlap
    pub polarity: Polarity,     // Symmetric | Plug | Receptacle | Left | Right
    pub turns: Turns,           // allowed turns about the normal
    pub need: Need,             // Required | Optional | Cap(piece)
    pub bearing: Bearing,       // None | Carries { capacity, loss } | Anchor
    pub tags: u32, pub excludes: u32,
}
```

One `compatible(a, b)` for every user: the kinds allowed together in a static matrix (Tessera's),
equal profiles or a listed transition piece between them, complementary polarity, opposite
normals within 1° and positions within 1 cm, extents agreeing by `fit`, an allowed turn, no veto.

**Sockets by element:**
- **Walls:** `WallSeam` on the left and right edges (thickness, material, band heights),
  `WallFoot`, `WallHead`, optional face `Anchor`s (signs, lamps, an abutting garden wall); corners
  are pieces with two seams at an angle, placed first.
- **Openings:** a wall panel offers a `Hole` (width, sill, head); windows, doors and shops are
  `Fill`s of its extent; doors and gates a `Threshold` each side, meeting a floor or the street at
  one level.
- **Slabs:** `SlabEdge` bearing on wall feet and heads; `Floor` surfaces make the rooms.
- **Roofs:** `Eave` (pitch, overhang, onto a `WallHead`), `Ridge` (symmetric), `Hip` and `Valley`
  (left and right), `Verge` (a gable end on a gable or a party wall), `Penetration` (chimneys).
- **Garden walls:** `GardenEnd` meeting a house's `Anchor`, another wall's end or a pier; gates as
  holes with a street-side threshold; heights (1.8, 1.5, 1.0 m) as profiles.
- **Roads and paths:** `RoadEnd` (class, width, lanes, kerb, level) met by a segment, a junction or
  an end cap; `Kerb`; `PathEnd` with its slope limit.
- **Ground:** `Bearing` anchors with a terrain tolerance.

**One data set, three users:** generation (the grammar stays, its terminals make pieces with
sockets, a join pass places corners, end caps and transitions to meet the required sockets; WFC
through the same `compatible` only for roofscape clutter, chimneys and light wells); checking (a
lot after generation, the city in the map check); player building (free sockets in a spatial
hash, candidate pairs within about 0.5 m filtered by `compatible` and ranked with manual cycling
[10], the ghost coloured by tier-2 support, the same checker on each placement).

**In order:** (1) the schema and sockets on the 16 kit modules, the structural walls and slabs,
the garden walls and the road graph's nodes; (2) the checker as a tool and a test, its counts in
the map check, the overlay; (3) on the first map and the city, against the owner's anomaly inbox;
(4) connectivity and support on the piece graph; (5) a player-building prototype in the sandbox;
(6) later, capacity flows; stress only offline, after the patent question.

**Measure:** anomalies by kind a thousand pieces (the required and structural ones at zero);
recall against the owner's inbox, precision on 50 random flags; checker time a lot and a city; end
caps and fallbacks a thousand sockets, WFC contradictions; support recompute a edit (p50, p99) and
collapse sizes; snap-query time and wrong snaps in play; a hash of the socket graph a seed
(determinism).

## Sources

1. Enshrouded Wiki, Building — https://enshrouded.wiki.gg/wiki/Building — [web] read
2. Enshrouded Wiki, 1.0 Update (changelog part 1) — https://enshrouded.wiki.gg/wiki/1.0_Update — [web] read
3. MMORPG.com, Enshrouded preview (2023) — https://www.mmorpg.com/previews/enshrouded-preview-keen-games-plans-to-redefine-voxel-based-building-in-this-arpg-survival-game-2000128419 — [web] read
4. GameLuster, Enshrouded building guide (2024-02-13) — https://gameluster.com/enshrouded-basic-custom-base-building-guide/ — [web] read
5. Steam discussion, "Gravity in objects and NPCs" (2024) — https://steamcommunity.com/app/1203620/discussions/0/4142816719801826551 — [web] read
6. PC Games Hardware, Enshrouded 1.0 tech test (2026-10-14) — https://www.pcgameshardware.de/Enshrouded-Spiel-74267/Specials/1-0-Technik-Test-Benchmarks-Release-1556566/ — [web] read
7. Sportskeeda, 0.8.0 notes — https://www.sportskeeda.com/mmo/enshrouded-0-8-0-patch-notes-all-changes-pact-flame-update-5 ; Deltia, v0.1.0.8 notes — https://deltiasgaming.com/?p=142033 — [web] seen
8. Valheim Wiki (Weird Gloop), Building stability — https://valheim.weirdgloop.org/w/Building_stability — [web] read
9. Physgun, Valheim build calculator — https://physgun.com/tools/valheim-build-calculator/ — [web] read
10. PCGamesN, Valheim snapping update (2023) — https://www.pcgamesn.com/valheim/building-snapping-update — [web] read
11. Thunderstore, Extra Snap Points Made Easy — https://thunderstore.io/c/valheim/p/Searica/Extra_Snap_Points_Made_Easy/v/1.0.4/ — [web] read; 11b. Thunderstore, Sinka — https://thunderstore.io/c/valheim/p/Ezomic/Sinka/ — [web] seen
12. Game Informer, Building and crafting in Fortnite (2014) — https://gameinformer.com/b/features/archive/2014/04/28/building-and-crafting-in-fortnite.aspx — [web] read
13. Epic, grid snapping in UEFN — https://dev.epicgames.com/documentation/en-us/fortnite/using-grid-snapping-in-unreal-editor-for-fortnite — [docs] read
14. Minecraft Wiki, Scaffolding — https://minecraft.wiki/w/Scaffolding — [docs] read
15. Rust Devblog 84 — https://rust.facepunch.com/news/devblog-84 — [web] read
16. Rust Devblog 85 — https://rust.facepunch.com/news/devblog-85 — [web] read
17. Rust Devblog 87 — https://rust.facepunch.com/news/devblog-87 — [web] read
18. Conan Exiles Devkit wiki, building sockets — https://conanexiles.com/wp-content/wiki/2693562384.html — [docs] read
19. 7 Days to Die Wiki, Structural Integrity — https://7daystodie.wiki.gg/wiki/Structural_Integrity — [docs] read
20. Space Engineers Wiki, MountPoints — https://spaceengineers.wiki.gg/wiki/Modding/Reference/SBC/MountPoints — [docs] read
21. Keen support, static grids splitting — https://support.keenswh.com/spaceengineers/publictest/topic/unknown-version-static-grids-not-falling-when-disconnected-from-voxel — [web] seen
22. Keen support, Medieval Engineers structural integrity — https://support.keenswh.com/medievalengineers/topic/structural-integrity — [web] read
23. Wikipedia, Medieval Engineers — https://en.wikipedia.org/wiki/Medieval_Engineers — [web] seen
24. Carl's Sims 4 Guide — https://www.carls-sims-4-guide.com/tutorials/building/houses.php ; Prima Games — https://primagames.com/eguides/the-sims-4-eguide/architecture-and-design/building-a-house — [web] seen
25. T. Thompson, How Townscaper Works (Game Developer, 2022) — https://www.gamedeveloper.com/game-platforms/how-townscaper-works-a-story-four-games-in-the-making — [web] read
26. O. Stålberg, Organic Towns from Square Tiles (IndieCade Europe 2019) — https://www.youtube.com/watch?v=1hqt8JkYRdI — [talk] seen via https://en.wikipedia.org/wiki/Townscaper (read); not watched
27. 80.lv, Stålberg's EPC 2018 talk, WFC in Bad North — https://80.lv/articles/using-wave-function-collapse-algorithm-for-dioramas — [talk] listing read, not watched
28. TileWorldCreator docs, the dual grid — https://giantgrey.gitbook.io/tileworldcreator-v4-documentation/dual-and-standard-grid — [docs] read
29. M. Gumin, WaveFunctionCollapse — https://github.com/mxgmn/WaveFunctionCollapse — [web] read
30. P. Merrell, Model Synthesis — https://paulmerrell.org/model-synthesis/ — [web] read
31. P. Merrell, Comparing Model Synthesis and WFC (2021) — https://paulmerrell.org/wp-content/uploads/2021/07/comparison.pdf — [paper] not read (cited in `procedural.md`)
32. Karth & Smith, WFC is Constraint Solving in the Wild, FDG 2017 — https://doi.org/10.1145/3102071.3110566 — [paper] seen
33. Boris the Brave, WFC Explained — https://www.boristhebrave.com/2020/04/13/wave-function-collapse-explained/ — [web] read
34. Boris the Brave, Classification of Tilesets — https://www.boristhebrave.com/2021/11/14/classification-of-tilesets/ — [web] read
35. DeBroglie, Constraints — https://boristhebrave.github.io/DeBroglie/articles/constraints.html — [docs] read
36. Tessera, Palettes — https://www.boristhebrave.com/docs/tessera/6/articles/palettes.html — [docs] read
37. marian42, Infinite city with WFC — https://marian42.de/article/wfc/ — [web] read
38. Kim et al., Graph Based WFC, IEICE Trans. 2020 (NCSOFT) — https://global.ieice.org/en_transactions/information/10.1587/transinf.2019EDP7295/_p — [paper] seen
39. Strodick, Schattkowsky & Albertz, HICSS-58 (2025) — https://aisel.aisnet.org/hicss-58/st/digital_twins/4/ — [paper] abstract read
40. Müller et al., Procedural Modeling of Buildings (2006) — https://www.doi.org/10.1145/1141911.1141931 — [paper] seen
41. Whiting, Ochsendorf & Durand, SIGGRAPH Asia 2009 — https://people.csail.mit.edu/ewhiting/projects/siggasia09.html — [paper] project page read
42. NVIDIA Blast, Stress Solver — https://archive.docs.nvidia.com/gameworks/content/gameworkslibrary/blast/1.1/api_docs/files/pageextstress.html — [docs] read
43. Jolt Physics documentation — https://jrouwe.github.io/JoltPhysics/ — [docs] read
44. Unreal Engine, Destruction Overview — https://dev.epicgames.com/documentation/en-us/unreal-engine/destruction-overview — [docs] seen
45. Wen et al., BrickSim, arXiv 2603.16853 (2026) — https://arxiv.org/abs/2603.16853 — [paper] abstract read
46. Luo et al., Legolization (2015) — https://history.siggraph.org/?p=210389 ; StableLego — https://arxiv.org/abs/2402.10711 — [paper] seen
47. Wikipedia, Maximum flow problem — https://en.wikipedia.org/wiki/Maximum_flow_problem — [web] seen
48. Eastman et al., Automatic rule-based checking of building designs (2009) — https://doi.org/10.1016/j.autcon.2009.07.002 — [paper] seen
49. Smith & Mateas, ASP for PCG (2011) — https://course.khoury.northeastern.edu/cs5150f14/readings/smith_asp4pcg.pdf — [paper] seen
50. Wilkins & Stathis, World of Bugs (2022) — https://arxiv.org/abs/2206.11037 — [paper] abstract read

Patents:
- P1. KR 10-2310107 B1 — https://patents.google.com/patent/KR102310107B1/en — [patent] read
- P2. US 12,440,766 B2 — https://patents.google.com/patent/US12440766B2/en — [patent] seen (extract)
- P3. US 12,667,784 B2 — https://patents.google.com/patent/US12667784B2/en (503); listed at https://futureofgaming.com/publications/monthly/2026-06-gaming-patents-granted (read) — [patent] seen
- P4. US 12,364,930 — https://www.patentarcade.com/video-game-patent/US12364930/computer-control-method-for-implementing-a-computer-game-template-arrangement — [patent] record read (the site's dates unreliable)
- P5. US 12,220,640 B2 — https://www.patentarcade.com/video-game-patent/US12220640/generative-interior-design-in-video-games — [patent] record read
- P6. US 11,911,697 B2 — https://patents.google.com/patent/US11911697B2/en — [patent] seen
- P7. US 8,152,613 B2 — https://patents.google.com/patent/US8152613B2/en — [patent] read for `building-structure.md`
- P8. US 10,964,122 B2 — https://patents.google.com/patent/US10964122 — [patent] seen
- P9. US 12,164,741 — https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12164741 — [patent] seen (a PDF, not opened)
- P10. US 7,979,251 B2 — https://patents.google.com/patent/US7979251 ; US 2007/0262984 A1 — https://patents.google.com/patent/US20070262984 — [patent] seen
