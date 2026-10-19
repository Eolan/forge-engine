# Research — Navigation and traversal: path search with links, passages as walker data, crossings, patents

> Written 2026-10-19 for #317 (path search with doors, windows and fences as links, then
> steering and avoidance) and #316 (passages across wall pieces as walker data, and crossings
> played with a clip); related #276, #311, #90. A companion to `game-ai-ecosystems.md` §2 and §7
> (2026-09-26: Recast per tile, Polyanya, off-mesh links, ORCA, flow fields; no patent section).
> Labels: *read* (fetched today), *search-recorded* (a search engine's record only), *from
> memory*. Google Patents served four pages, then refused (503); other claim 1s come from KISTI
> ScienceON's record of each US grant, summarised by the fetch tool. Expiries Google did not show
> are estimates (20 years from the US or PCT filing). Not a clearance search, not legal advice.
> Forge's facts: `forge-procgen`'s `city/walk.rs`, `sweep.rs`, `structure.rs`, `walls.rs`,
> `forge-physics`, and the two issues, read today.

> **In five sentences.** Shipped engines carry traversal as data on links (Detour's off-mesh
> connections, Unreal's Nav Link Proxies and smart links, Unity's NavMesh Link, Godot's
> NavigationLink, `landmass`'s animation links): two ends, a width, a cost, who may pass and
> whether it is on, with game code playing the action when an agent arrives. The action is a
> clip whose root motion is warped onto targets on the obstacle (Unreal's Motion Warping, Gears
> of War 4's warp points); Dying Light instead detects "hooks" in the geometry at run time. No
> patent was found on Recast-style generation, tiled rebuilds, any-angle search, ORCA or flow
> fields; the live ones met sit at the edges (Techland's hook selection, the closest, for a
> player controller; Tencent's out-of-reach jump; Square Enix's waypoint regions; Disney's one
> mesh for all sizes, now Adeia's; Amazon's bottleneck sweep; EA's plane-cast detection), and
> each has a way round that Forge's plan can take. In Rust, `rerecast` (generation, deterministic
> with `libm`), `polyanya` and `landmass` (with animation links) are maintained and need no
> Bevy, `dodgy` and the C++ bindings are quiet, and a new Recast and Detour port is at 0.1. The
> first step is a run-time walk field (the walk grid's rasterisation and clearance, as spans per
> column, built from the prisms the capsule sweeps), searched by A* with links made from the
> pieces' passages, smoothed and checked by the sweep; Recast's polygons and Polyanya come later
> for open terrain, with the same link records.

**Contents**

1. [What Forge has](#1-what-forge-has)
2. [Traversal as data in shipped engines](#2-traversal-as-data-in-shipped-engines)
3. [Moving a body across a passage while a clip plays](#3-moving-a-body-across-a-passage-while-a-clip-plays)
4. [Patents](#4-patents)
5. [Rust crates in October 2026](#5-rust-crates-in-october-2026)
6. [Recommendation for Forge](#6-recommendation-for-forge)
7. [Verification notes, and what was left out](#7-verification-notes-and-what-was-left-out)

---

## 1. What Forge has

- **The walk grid** (`city/walk.rs`, #266) is Recast's first stage, in 2D, for one house. Each
  storey is rasterised at 5 cm. Walls block, partitions open at their doorways, furniture and the
  flights' runs block. A chamfer distance transform gives each cell its clearance (`dist`); a
  cell is free at 0.3 m or more. A flood starts inside the front door, and a reached flight's foot
  seeds its head on the storey above. It reports unreached rooms, flights and doors. It is a
  build-time check: no search, no costs, no outside, no links but the flights.
- **The capsule sweep** (`city/sweep.rs`, #276): `WALKER` (radius 0.3 m, foot 0.35 m, top
  1.95 m) swept along a polyline against `Prism`s, in steps shorter than the radius; `Cuboid`
  tests for vehicles. It reports hits; it is the practical check.
- **Pieces with #316's data:** each `Opening` (`city/structure.rs`) has its wall, storey,
  position, width, sill, head and kind (`Window`, `Shopfront`, `Door`, `Passage`);
  `GardenWalls` (`city/walls.rs`) has walls at street, party and front heights, gates and hedges.
- **The controller** (`forge-physics`): `CharacterDesc` over Jolt's `CharacterVirtual` (radius
  0.3 m, height 1.8 m, step 0.4 m), unlike `WALKER` (step 0.35 m, top 1.95 m).
- **What the issues saw:** pursuers walk straight at the target and slide along walls. The test
  scene finds the nearest window edge or low fence by distance and carries the body by script.

## 2. Traversal as data in shipped engines

**Recast Navigation. `dtOffMeshConnection`. Detour API reference.** [docs] *read*
<https://recastnav.com/structdtOffMeshConnection.html>

Two end points, an end radius, its polygon, internal flags, a side and a `userId` set at build
time. Who may pass is in the user flags of the connection's polygon. End A must lie in the tile.
*Bearing:* the smallest link record that works. Forge's `userId` is the passage id, which leads
back to the piece, its kind and its state.

**Epic Games. "Navigation Links in Unreal Engine" (5.5+), `ANavLinkProxy` (4.26 API), "Smart
Objects in Unreal Engine — Overview". Unreal Engine documentation.** [docs]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/automatic-navigation-link-generation>
*read* · <https://docs.unrealengine.com/4.26/en-US/API/Runtime/AIModule/Navigation/ANavLinkProxy/index.html>
*search-recorded* · <https://dev.epicgames.com/documentation/en-us/unreal-engine/smart-objects-in-unreal-engine---overview> *read*

Links join navigable areas with no walkable join. Proxies are placed by hand; since 5.5
(experimental) the generator adds jump-down and falling links along border edges, set by a jump
length, distance from the edge, depth, height and tolerance, and a proxy class adds behaviour.
In the 4.26 API, *simple* links (points and segments) are plain edges; a *smart* link raises an
event when an agent reaches it, and the game hands control back with `ResumePathFollowing()`.
Smart Objects hold data and no execution logic: slots, behaviour definitions, and agents that
find, claim, use and release a slot.
*Bearing:* two kinds of link suffice. A plain edge with a cost (a step, a ramp), and a smart link
that stops path following and runs a crossing (a window, a fence). A claim per link only matters
when two agents want one window in the same second.

**Unity Technologies. "NavMesh Link" (AI Navigation 2.0), `autoTraverseOffMeshLink`. — Godot
Engine. "Using NavigationLinks". Documentation.** [docs] *read*
<https://docs.unity3d.com/Packages/com.unity.ai.navigation@2.0/manual/NavMeshLink.html> ·
<https://docs.unity3d.com/ScriptReference/AI.NavMeshAgent-autoTraverseOffMeshLink.html> ·
<https://docs.godotengine.org/en/stable/tutorials/navigation/navigation_using_navigationlinks.html>

Unity's link is a line or a span with a width (crossed at the nearest point of the entry edge),
with an agent type, a cost override, a direction flag, an area type (Walkable, Not Walkable,
Jump) matched against the agent's mask, and an Activated flag; agents cross by themselves unless
told not to. Godot's joins the polygons nearest its ends, with costs, layers and an owner, and
"does not provide any specialized movement through the link": game code plays the crossing.
*Bearing:* a window is a span link as wide as its opening; who may use it is a mask per
archetype; a shut window is a deactivated link, not a rebuild; the crossing is #316's action.

**Andriy (andriyDev). `landmass` CHANGELOG, 0.9.0–0.9.3. GitHub, 2025–2026.** [code] *read*
<https://github.com/andriyDev/landmass/blob/main/CHANGELOG.md>

0.9.0 (2025-10-01) added animation links, its name for off-mesh links: the agent reports reaching
one and the user starts the transition (a jump, a teleport, a wall run). 0.9.3 (2026-10-11) fixed
links left stale after regions were recomputed, and paths that only ran through links.
*Bearing:* the same shape in Rust, and its bugs are a test list: links survive the rebuild of the
tiles under their ends.

**MCV staff. "The vaulting dead: implementing first-person parkour in Dying Light." MCV/Develop,
18 May 2016.** [web] *read*
<https://www.mcvuk.com/development-news/the-vaulting-dead-implementing-first-person-parkour-in-dying-light/>

Techland had placed about 50 000 hooks by hand in one segment of the map. A prototype that
analyses the geometry in front of the player in real time replaced them ("Natural Movement").
The geometric conditions are not given.
*Bearing:* the two roads are hand-placed data and run-time detection. Forge's kit knows its
pieces, so it can *generate* the data: no hand placement, no detection. Techland's patents (§4)
cover the hook side.

## 3. Moving a body across a passage while a clip plays

**Epic Games. "Motion Warping in Unreal Engine" and "Game Animation Sample Project". Unreal
Engine documentation.** [docs] *read* — **Bollo, D. (The Coalition). "High Performance Animation
in Gears of War 4." SIGGRAPH Talks, 2017.** [talk] *search-recorded*
<https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-warping-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/game-animation-sample-project-in-unreal-engine> ·
<https://history.siggraph.org/wp-content/uploads/2022/09/2017-Talks-Bollo_High-Performance-Animation-in-Gears-of-War-4.pdf>

Warp windows are notify states on a montage, each with a modifier, a target name and translation
and rotation settings. Game code sets the named targets before the montage plays. Skew Warp meets
the target's location and rotation by the window's end; Scale scales the root motion. The Game
Animation Sample vaults, climbs and drops from ledges; motion matching picks montages and entry
frames, and the animation scales to the obstacle's size. A known issue: warping inside a montage
can move the body when the offset is released. Gears of War 4 (search record) used warp points
whose timing animators set.
*Bearing:* a crossing clip carries named windows (entry, top, exit); the passage supplies the
targets. The method is Witkin and Popović's "Motion Warping" (SIGGRAPH 1995, *from memory*); the
live patents of §4 claim how hooks or ledges are found and chosen, not the warp.

**The crossing in Forge's terms.** Its frame is the passage's: entry on side A, the sill or top,
exit on side B. (1) Jolt's character moves kinematically along the warped root path, ignoring
the passage's own piece. (2) At the exit a capsule check (`sweep::meets`) confirms the body fits,
and the controller resumes. (3) Until #311 brings clips, a procedural curve over the sill with
the same three targets stands in.

## 4. Patents

Searched with Google Patents (until refused), KISTI ScienceON, Patent Arcade and web search, for
voxel navmesh generation, tiled rebuilds, any-angle search and string pulling, navigation links,
climbing, vaulting, ledges, parkour, motion warping, smart objects, velocity obstacles and flow
fields. Named searches covered Havok, Autodesk (Kynogon), Ubisoft, Sony, Epic, Unity, EA/Maxis,
Nintendo, Tencent and UNC; Google's "navigation mesh" listing (first 99 of 377, any holder) the
rest.

**Live, and near what #316 and #317 plan:**
- **US 8,979,652 B1** (2015-03-17) **and US 9,238,175 B2** (continuation, 2016-01-19), "Natural
  movement in a virtual environment", Techland. US; priority PL 2014-03-27; filed 2014-03-28.
  Active in Google's record (search-recorded). Expiry about 2034-03 (estimate). *Claim 1 (read,
  both alike):* movement hooks are determined as a player moves. One is chosen by a score based
  at least in part on its distance from the player. The player performs an action chosen from
  several, based at least in part on the hook's height relative to the player. **Close for a
  player controller** that takes the nearest passage by distance and picks vault, climb or mantle
  from its height relative to the body. NPCs crossing links their search chose are not "a
  player", and no distance score picks the link. *Way round:* the crossing is fixed by the
  passage's kind in data. The player crosses the passage its movement line meets, on a key
  press. Ask the owner before any automatic player vault.
- **US 11,679,331 B2**, "Object jump control method…", Tencent. US; priority CN 2019-05-23;
  filed 2021-06-24 from a PCT; granted 2023-06-20. Expiry about 2040 (estimate). *Claim 1
  (read):* an NPC checks periodically whether a walking path to a user's character exists and is
  marked within or out of reach. Out of reach, with a jump condition met, a landing point is
  chosen from the destination's position, a trajectory computed, and the NPC jumps. **Reads on a
  pursuer that leaps towards the player when no path exists.** *Way round:* jumps, vaults and
  climbs are links in the search. With no path, the pursuer goes to the reachable cell nearest
  the target and waits or searches.
- **US 11,406,898 B2** (waypoint regions) **and US 11,241,621 B2 / US 11,148,054 B2** (waypoint
  thinning), Square Enix. US; priority JP 2019-09-03/04; filed 2020-08-28; granted 2022-08-09,
  2022-02-08, 2021-10-19. Expiry about 2040-08 (estimate). KISTI gives '621 and '054 the same
  application number, unexplained. *Claim 1 of '898 (read):* waypoints placed on a navmesh by a
  rule and grouped into regions by a rule, the passable regions found from their arrangement,
  the route searched over their waypoints. *'621/'054 (read):* waypoints placed by a rule, a
  subset removed by a selection rule, the route searched over the rest. **Near a rooms-then-cells
  or tiles-then-cells search.** *Way round:* one-level A* over cells, smoothed after the search.
  Take-Two's US 11,071,916 / 11,684,855 (a coarse graph from the road graph, to 2039;
  `population-simulation.md`) bears on the same choice for long routes.
- **US 8,810,566 B2**, "Providing a navigation mesh by which objects of varying sizes can traverse
  a virtual space", Disney; **held by Adeia Media Holdings since 2026-05-08** (Adeia licenses
  patents; from memory). US; filed 2011-07-15. Active; adjusted expiry 2033-01-16 (Google, read).
  *Claim 1 (read):* one polygonal mesh for many sizes. From the object's radius, edge constraints
  (lines parallel to boundary edges, offset by the radius) and corner constraints (arcs at reflex
  corners) keep its centre inside the boundary and decide whether it passes from tile to tile.
  **Near a polygon mesh shared by several radii with run-time offsets.** *Way round:* a clearance
  per cell compared with each radius (no edge or corner constraints), and later one polygon mesh
  per radius class, Recast's way.
- **US 10,792,568 B1**, "Path management for virtual environments", Amazon. US; filed 2018-09-24;
  granted 2020-10-06. Expiry about 2038-09 (estimate). *Claim 1 (read):* a virtual sphere is swept
  along a found path to find bottlenecks where the width drops below its previous diameter by
  more than a threshold; path points go out with the bottleneck and the radius that passes, so
  the character is steered without the map. **Not #276's sweep** (a fixed capsule, a test, the
  server keeps the map). *Avoid* a variable-radius sweep that sends bottleneck radii to clients.
- **US 12,533,588 B2**, "Techniques for assisted gameplay using geometric features", EA. US; filed
  2023-03-31; granted 2026-01-27. Expiry about 2043 (estimate). *Claim 1 (read):* a scanning
  query cast as a plane of two or more dimensions; a geometric feature in its result; a scene
  feature recognised (an obstacle in the avatar's predicted path, or a change of ground level);
  the avatar controlled by its action. *Way round:* passages come from data, not from casts.
- **US 8,111,257 B2**, "System and method for the generation of navigation graphs in real-time",
  AiSeek. US; provisional 2007-03-06; filed 2008-01-22. "Active" in Google's family listing.
  Expiry about 2028-01 (estimate). *Claim 1 (read):* a dictionary of surface features to costs, a
  graphical module making navigation data from geometry and costs (dependent claims: rendered
  bitmaps), a graph built from it, a physics module updating the graph from collision tests.
  **Near only if Forge combined a material cost table, rendered maps and physics-edited edges.**
  *Way round:* CPU rasterisation of pieces; the sweep tests and never edits the graph.

**Live, not near the plan:**
- **US 12,515,132 B2**, Tencent (priority CN 2021-07-30; granted 2026-01-06; about 2042): climbing
  onto a wall on an input within a distance and an angle, then climbing it in the direction of a
  second input. Free wall climbing; #316's climb is one fixed crossing.
- **US 12,097,432 B2**, "Markup free ledge grab", Bungie (Sony) (filed 2022-03-10; about 2042):
  video streamed to a client, a ledge found by its lip and recess near the avatar, an anchor
  made at the lip. Forge streams no video and detects no ledges.
- **US 10,537,805 B2**, Square Enix (filed 2017-06-08; about 2037): on an animation request,
  locations are tried until one has a free area, and the animation plays there. *Avoid*
  searching nearby spots for room to play a crossing; it plays at its passage or not at all.
- **US 9,230,369 B2** and **US 10,109,097 B2**, Autodesk (filed 2012-07; about 2032 or later with
  adjustment): lane coordinates and projected collisions with neighbours in other lanes; speed
  changed from the union of others' collision speed intervals at an intersection. Not ORCA;
  they bear on #87's traffic (keep junctions to signals and right of way).
- **US 10,916,055 B2**, "Plane sweep for multi-level navigation meshes", Microsoft (US, EP; to
  2039-01-25, Google, read): a sweep line over overlapping floors with a status per interval.
  Forge keeps spans per column and runs no such sweep.
- Also met: Disney's US 8,257,173 B2 (an off-graph shortcut; filed 2008, about 2028); Apple's
  US 11,804,012 B1 and 11,532,139 B1 (XR agents); Cygames' US 10,828,568 B2 (routes from players'
  history); Square Enix's US 11,219,825 B2 (influence over regions; door, window and ladder ids
  and jump links only in its description); Nintendo's US 11,738,265 B2 (a jump on inked ground).

**Expired or not active:** Nintendo's US 7,667,705 B2 and US 8,976,184 B2, "controlling animation
by tagging objects within a game environment" (provisionals 2001; a nearby character turns to a
designer's tag; "Expired – Lifetime" in Google's records, search-recorded): free prior art for
data on pieces driving a character. Lucasfilm's US 8,659,598 B1, "Adjusting navigable areas of a
virtual scene" (2010; local re-polygonisation round an added object; "not active" in Google's
listing): free to use.

**Searched, none found:** Recast-style generation (spans, regions, contours, polygons); tiled
rebuilds round obstacles (DetourTileCache, Havok's silhouette cutting); Polyanya, Theta*, jump
point search and funnel string pulling; RVO and ORCA (UNC's RVO2 is Apache-2.0, whose terms
include a patent licence from contributors); flow fields (Emerson, Continuum Crowds); smart
objects. Chinese-only filings seen in listings, not read: NetEase's CN 112386911 B (navigation
grid generation), Beijing Pixel's CN 104548597 A, Tencent's CN 114177613 B (navmesh update; a US
member listed by Google was not found).

## 5. Rust crates in October 2026

Read today on crates.io's API (versions, dates, licences, dependencies) and GitHub READMEs.
"Deterministic" is what the project says; nothing was tested.

| Crate | Latest (date) | Licence | Bevy | Determinism | State, use for Forge | URL read |
|---|---|---|---|---|---|---|
| `rerecast` | 0.4.0 (2026-08-05) | MIT/Apache-2.0 | no (`bevy_reflect` optional) | claimed with the `libm` feature | Recast's generator only (watershed, monotone, layers; tiles on the roadmap); active; the reference to diff against | <https://crates.io/api/v1/crates/rerecast>, <https://github.com/janhohenheim/rerecast> |
| `bevy_rerecast` | 0.5.0 (2026-08-05) | MIT/Apache-2.0 | yes (0.19) | as above | its Bevy wrapper | <https://crates.io/api/v1/crates/bevy_rerecast> |
| `polyanya` | 0.17.1 (2026-08-25) | MIT/Apache-2.0 | no | not stated | any-angle search, layers with costs and toggles; uses `rerecast` 0.4; no links in its README | <https://crates.io/api/v1/crates/polyanya>, <https://github.com/vleue/polyanya> |
| `vleue_navigator` | 0.16.0 (2026-08-25) | MIT/Apache-2.0 | yes (0.19, required) | not stated | Bevy plugin over `polyanya` | <https://crates.io/api/v1/crates/vleue_navigator> |
| `landmass` | 0.9.3 (2026-10-11) | MIT/Apache-2.0 | no (`bevy_landmass` apart) | not stated | paths, simplification, steering, avoidance via `dodgy_2d`, animation links; the closest Rust whole | <https://crates.io/api/v1/crates/landmass>, its CHANGELOG |
| `dodgy_2d` / `dodgy_3d` | 0.5.5 (2025-04-24) / 0.5.2 (2024-12-15) | MIT/Apache-2.0 | no | not stated; depends on `rand` 0.9 | ORCA; quiet 18 months; read, port with a fixed neighbour order | <https://crates.io/api/v1/crates/dodgy_2d> |
| `recastnavigation-sys` | 1.0.3 (2024-01-11) | "non-standard" on crates.io | no | no | raw C++ bindings; quiet 21 months; a spike only | <https://crates.io/api/v1/crates/recastnavigation-sys> |
| `recastnavigation-rs` | 0.1.0 (2024-03-23) | MPL-2.0 | no | claimed, via a deterministic C++ fork | quiet; read the fork's diff | <https://crates.io/api/v1/crates?q=recastnavigation> |
| `landmark` / `waymark` (+ tile cache, crowd, dynamic) | 0.1.0 (2026-02-23) | MIT/Apache-2.0 | not mentioned | output matches the C++ on three test meshes | a new Rust port of Recast and Detour with off-mesh connections; one person's spare-time project, by its README | <https://github.com/wowemulation-dev/recast-rs> |
| `pathfinding` | 4.16.0 (2026-09-07) | MIT/Apache-2.0 | no | not stated (inferred: no randomness; ties by insertion order) | generic A*, Dijkstra and others; enough for a first grid search | <https://crates.io/api/v1/crates/pathfinding> |

`game-ai-ecosystems.md` §7's verdict stands: Forge writes the field, the search, the links and
ORCA, and diffs against `rerecast` and `waymark` in tests.

## 6. Recommendation for Forge

*Opinion. Costs are estimates until measured.*

### 6.1 A grid with links first, Recast's polygons later

| | 2.5D walk field with links (first) | Recast's polygons and Polyanya (later) |
|---|---|---|
| Builds on | `walk.rs`'s rasterisation and clearance, `sweep.rs`'s prisms | the same, then regions, contours, polygons, detail |
| Work to a first path | days | weeks |
| Agrees with the capsule check | by construction (same prisms, foot, top, radius) | approximately (simplified contours) |
| Several radii | one clearance field, compared per radius | one mesh per radius class |
| Determinism | integer cells and costs, fixed tie-breaks | care in the float stages (`libm`) |
| Scale | a house, a street, a garden block | cross-island paths, less memory |
| Path shape | 8-connected, then string-pulled | any-angle, optimal |

Links are the same records in both, so #316's work carries over.

### 6.2 #317: the first step

1. **The walk field.** Per tile (proposed: 25.6 m, 256 × 256 cells of 0.10 m), each column keeps a
   short list of walkable spans: floor height, head room to the next solid, sideways clearance.
   It is built from the pieces' prisms (walls, partitions, furniture, flights, garden walls,
   hedges, trunks, kerbs) and their floors and ground. A span is walkable where the head room
   passes the walker's top; neighbours join where the rise is within the step; flights join
   storeys as in `walk.rs`. Tiles are built on demand round agents (Unreal's invoker idea) and
   rebuilt when a piece changes. 0.10 m is a third of the radius (Recast's advice on the ratio
   is from memory).
2. **Links from passages** (§6.3): an edge from the cells before a passage on side A to those
   after it on side B, if the archetype may cross. Cost: the crossing's time. It carries the
   crossing kind and the passage id. Doors are open cells. Wheelchair users get doors and ramps.
3. **The search:** A* over spans and links, integer octile costs (10, 14), ties by cell index, a
   node budget per request. With no path: the reachable cell nearest the goal (§4, Tencent).
4. **Smoothing:** string pulling on the clearance field; link ends stay as fixed corners.
5. **The check (#276):** every path is swept with `sweep::sweep` against the same prisms. A hit
   is a bug in the field, counted in the anomaly report, not patched in the path.
6. **Following:** steering to the next corner with arrival; at a link, a hand-over to the
   crossing (§6.3), then on. Separation first for the pursuer test; ORCA after (a port of
   `dodgy`/RVO2 with a fixed neighbour order).

*Proof:* the #317 scene: pursuers round the house and through its door, climbers through the
window or over the low fence, the others by the gate, every path passing the sweep. Numbers:
field build per tile, A* expansions and µs per path (p50, p99), equal paths at 1 and 6 workers.

### 6.3 #316: passages as data, and the crossing

```rust
pub enum PassageKind { Door, Window, LowFence, TallFence, Gap }
pub struct Passage {
    pub kind: PassageKind,
    pub piece: PieceRef,        // the opening, garden wall or gap it crosses
    pub line: [DVec3; 2],       // its span on the wall's middle plane, at the sill or top
    pub across: DVec3,          // unit, side A to side B
    pub width: f32,             // clear width, m
    pub sill: f32,              // height climbed over, above side A's floor, m
    pub head: Option<f32>,      // a window's head, m
    pub depth: f32,             // the wall's thickness, m
    pub drop: f32,              // side B's floor minus side A's, m
    pub both_ways: bool,
    pub state: PassageState,    // Open | Shut | Broken | Barred, switched at run time
}
pub enum CrossingKind { Walk, Crouch, Vault, ClimbThrough, Climb, Mantle, Drop, Jump }
pub struct Crossing {           // one per (passage kind, archetype), in data
    pub kind: CrossingKind,
    pub clip: Option<ClipId>,   // root motion where it has it (#311)
    pub windows: Vec<WarpWindow>, // { t0, t1, target: Entry | Top | Exit }
    pub needs: Ability,         // climbs, vaults, wheelchair
    pub seconds: f32,           // also the link's cost
}
```

- **Sources.** Windows from `Opening` (width, sill, head); low and tall fences from `GardenWalls`
  (height by role); gaps from the kit's breaks. Made with the pieces and stored with them, never
  detected from geometry (§4, EA and Bungie).
- **Kind decides the crossing** (limits to check against the clips): a window sill up to 1.2 m,
  ClimbThrough; a wall up to 1.1 m, Vault; up to 2.2 m, Climb, for climbers; a gap up to 1.5 m,
  Jump. Fixed per kind and archetype, not chosen at run time from the body's height (§4,
  Techland).
- **The query:** passages within r of p, from a per-tile index, for the controller and the field.
  The player crosses the passage its movement line meets, on the context key, as §3 describes.

*Proof:* a capture strip of a walker climbing through a window and vaulting a garden wall; the
body never sinks into the sill or the wall; the exit check passes every time.

### 6.4 Rules the patents suggest

- Passages and crossings come from the pieces' data; nothing is detected by casting ahead.
- No run-time scoring of nearby hooks by distance with an action chosen by height.
- No "out of reach, so leap at the target": jumps are links, or the agent waits.
- A one-level search first; ask before rooms-then-cells or tiles-then-cells.
- Clearance per cell, or one mesh per radius class; no edge and corner offsets on a shared mesh.
- The capsule sweep stays a test and never sends bottleneck radii to clients.

### 6.5 Questions for the owner (defaults in brackets)

1. Cells and tiles: 0.10 m in 25.6 m tiles, or `walk.rs`'s 0.05 m? [0.10 m: a 0.9 m door keeps
   three free cells for a 0.3 m walker.]
2. One walker for field and controller: `WALKER` (step 0.35 m, top 1.95 m) or `CharacterDesc`
   (step 0.4 m, height 1.8 m)? [One record: the controller's step, the sweep's top.]
3. Who crosses what? [Climbers: windows and low fences; tall fences for athletic archetypes only;
   wheelchair users: doors and ramps.]
4. The player's crossing: on a context key, or automatic on walking into a low fence? [The key.
   Ask again before an automatic vault (Techland).]
5. Window states (shut, open, broken, barred) switching their links? [Yes; shut windows are links
   only for archetypes that break glass.]
6. Long routes across the village: one level with a node budget, or a hierarchy? [One level until
   a measured need, then ask (Square Enix '898, Take-Two '916).]
7. When do Recast's polygons and Polyanya take over outdoors? [With the island's terrain
   (Phase 3); the field stays inside buildings.]
8. A lawyer's reading of Techland's two claims before player traversal ships? [Before a public
   release, not before the prototype.]

## 7. Verification notes, and what was left out

- **Read:** the Detour, Unreal, Unity and Godot pages of §2–§3; MCV's article; `landmass`'s
  changelog; the crates.io records and READMEs of §5; Google Patents' pages for US 11,804,012,
  8,810,566 and 10,916,055 (their claims) and its "navigation mesh" listing; KISTI's claim 1 for
  the other US grants of §4 except Nintendo's, Lucasfilm's and Take-Two's; Patent Arcade for
  US 10,537,805, 11,241,621, 11,406,898 and 8,976,184.
- **Search-recorded:** the 4.26 `ANavLinkProxy` API; the Gears of War 4 talk; the statuses of
  Nintendo's tags and Techland's patents; Lucasfilm's abstract; Tencent's '331 family.
- **From memory:** Witkin and Popović 1995; Recast's cell-to-radius advice; Adeia's business.
- **Refused:** Google Patents (503 after four pages), Justia, FreePatentsOnline and uspto.report
  (403), patentguru (468), a game wiki (403), data.epo.org's GPI pages (404). One USPTO PDF had no
  text layer.
- **Left out:** Assassin's Creed's parkour data (GDC 2013's AI postmortem page names the work,
  not the method). A survival game's window and fence crossing (its wiki refused; forum posts
  disagree), though its shape (objects on the edges between tiles, crossed by an action) is
  §6.3's. EP and JP family members were not checked. Valve, Activision and Take-Two were not
  searched by name; nothing of theirs on these topics was in the listing's first 99.
