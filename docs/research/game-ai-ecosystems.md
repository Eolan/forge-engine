# Research — Game AI and ecosystems: decisions, navigation, crowds, animals, ships, life at planet scale

> Companion to `planet-environment.md` §3 and §5 (the ecosystem bake, fauna as densities per
> cell, the environment state), `procedural.md` §4 (ecosystems and settlements as rule systems),
> `animation.md` §5 (how a body meets the world; the crowd tiers), `netcode.md` §5–§6 (same-binary
> determinism, the server for a living world) and `task-system.md`'s verdict (`bevy_ecs` storage
> under a Forge executor). Written 2026-09-26 on the owner's brief — "game AI for the various
> NPCs, creatures, animals and even spaceship AI, and ecosystems" — for Phase 3's `forge-sim`
> (`docs/ROADMAP.md`: the fixed tick, simulation LOD, determinism digests), the island's village
> (#81), the living-city ideas (#84, #87, #90) and the space battle (#80), on a generated,
> planet-scale world, for one to sixteen players, deterministic on the server. Every citation was
> checked that day against a reachable page or, where the network proxy refused the host, against
> the search engine's record of it; the distinction is kept per entry ("(verified through search
> results)" on the URL line) and summarised under [Verification notes](#verification-notes) for
> issue #99; what was looked for and not found is under [Checked and left out](#checked-and-left-out).
> Entries the sibling files already carry (theHunter's need zones, Eco, Volterra, Hitman's crowd
> tiers, DReCon) are referenced by section and not repeated.

The question is which published architectures let a few hundred agents near the players decide,
move and interact convincingly, let thousands more exist cheaply in the same city, let a whole
planet's animal populations persist as numbers, and let ships fight in six degrees of freedom — all
inside a server tick that must produce the same bytes on any thread count (D-016), on a terrain
that is generated and can change under a crater. The short answer: the decision layer is settled
practice since 2005–2015 and the owner has chosen it (utility scoring and behaviour trees over
schedules and smart objects, a planner only for the reconstruction jobs); navigation is Recast's
voxel-to-polygon pipeline on tiles, rebuilt per tile where the terrain changes, with flow fields for
crowds, ORCA for local avoidance and a sparse voxel octree for whatever flies or swims; crowds and
cities are an AI level of detail (a few dozen full agents, a few hundred cheap ones, the rest
statistics), which is exactly the three-layer shape the planet needs; animals are needs plus habitat
plus a herd, with populations as damped per-cell densities that spawn individuals at need zones
near players; ships are steering behaviours with thrust limits, a threat table and a formation
slot, the fifty-year-old differential-game and guidance literature behind them; and in Rust the
navmesh generator, ORCA and the utility scorer exist as crates good enough to read and, in two
cases, to bind, while the decision layer, the schedules, the smart objects, the simulation LOD and
the digests are Forge's own by the house rule.

> **State of the art in five sentences.** Behaviour selection in shipped games is one of four
> shapes — hierarchical state machines and behaviour trees (Halo 2, every engine's default),
> utility scoring over needs and considerations (The Sims, the Infinite Axis Utility System at
> ArenaNet), planners that search actions (F.E.A.R.'s GOAP, the HTNs of Killzone 2, Transformers
> and Horizon) and, for open worlds, schedules of packages evaluated by condition (Oblivion's
> Radiant AI, Watch Dogs: Legion's Census) — with smart objects carrying the animation and the
> advertisement so that the agent stays small. Navigation is Recast and Detour (voxelise, filter,
> region, contour, polygonise, on tiles, with a tile cache that rebuilds one tile in about 2 ms
> around a temporary obstacle), any-angle search on the polygons (Polyanya), flow fields when
> hundreds share a goal (Supreme Commander 2, Planet Coaster's 10 000 guests), reciprocal velocity
> obstacles for the last metre (RVO/ORCA) and a sparse voxel octree with Morton codes for flight
> (Warframe). Crowds are Reynolds' local rules or Helbing's forces for the motion, Treuille's
> potential field or Emerson's flow tiles for the routing, and an AI level of detail for the
> budget: Assassin's Creed Unity ran 40 real AIs and 120 high-resolution bodies inside a visible
> crowd of 10 000, Stalker's A-Life ran the rest of its world offline on a graph, and Stalker 2
> shipped the cautionary tale of what happens when the offline layer's radius and memory are cut.
> Animals are the same machinery with needs, senses, herds (a shared group agent, as Horizon's
> machines) and territories, and their populations are numbers per cell — Lotka–Volterra as the
> toy, NetLogo's Wolf Sheep Predation and Grimm's individual-based ecology as the practice —
> instantiated near players only. Ship AI has not moved since Elite's TACTICS routine and
> FreeSpace's AI profiles except in the mathematics behind it (Isaacs' differential games, the
> pursuit–evasion surveys, convex landing guidance, formation control) and in learned policies
> (GT Sophy, AlphaStar, the AlphaDogfight hierarchies), which win races and dogfights but are not a
> pure function of a seed and so stay off the server tick.

**Contents**

1. [Decision architectures](#1-decision-architectures)
2. [Navigation and movement](#2-navigation-and-movement)
3. [Crowds, cities and the simulation level of detail](#3-crowds-cities-and-the-simulation-level-of-detail)
4. [Animals, creatures and ecosystems](#4-animals-creatures-and-ecosystems)
5. [Spaceship and vehicle AI](#5-spaceship-and-vehicle-ai)
6. [Ecosystems at planet scale: layers, hand-offs, the tick and its digest](#6-ecosystems-at-planet-scale-layers-hand-offs-the-tick-and-its-digest)
7. [Rust and open source](#7-rust-and-open-source)
8. [What the shipped games do](#what-the-shipped-games-do)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [What the numbers say](#what-the-numbers-say)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

---

## 1. Decision architectures

Four shapes, each with one canonical talk. The owner's choice (programme.md §5, answer 8) is the
second and the first over schedules, with the third for the reconstruction jobs only; the entries
below are kept so the choice is auditable and so the boundaries between the shapes are drawn where
the shipped games drew them.

**Damian Isla (Bungie). "Handling Complexity in the Halo 2 AI." GDC 2005; proceedings article on
Gamasutra (now Game Developer), 11 March 2005.** [talk] [foundational]
<https://www.gamedeveloper.com/programming/gdc-2005-proceeding-handling-complexity-in-the-i-halo-2-i-ai>
(archived slides: <https://archive.org/details/GDC2005Isla>; verified through search results)

The behaviour-tree talk: Isla "describes some of the techniques Bungie used in the design of the
Halo 2 AI in pursuit of a beautiful, clean and ultimately scalable brain architecture" — a
prioritised tree (strictly a DAG, since a behaviour subtree can hang in several places) of on the
order of fifty behaviours, with stimulus-driven impulses, per-character behaviour masks and
"behaviour-level" memory, the pattern every later engine editor (Unreal's Behavior Trees, LimboAI,
Beehave, §7's Rust crates) copies.
*Bearing:* the shape of Forge's *reactive* layer: a small tree per archetype for the moment-to-moment
choices (flee, fight, use this object), selected by priority and conditions, ticked in a fixed order.
Its vocabulary (impulse, mask, stimulus) is what the perception layer of §3 feeds.

**Jeff Orkin (Monolith). "Three States and a Plan: The A.I. of F.E.A.R." GDC 2006.** [talk]
[foundational]
<https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf> (verified through search results)

Goal-oriented action planning as shipped: "the FSM for characters in F.E.A.R. has only three
states" — Goto, Animate, UseSmartObject — "and A* is used to plan sequences of actions as well as to
plan paths". The practical changes over academic STRIPS planning were a cost per action, dropping
add/delete lists for effects, and procedural preconditions and effects.
*Bearing:* the argument for the owner's restriction. A planner is the right tool when the action
set composes in ways nobody enumerates (clearing rubble, fetching materials, rebuilding a wall in
order); it is the wrong default for a villager's day, where the plan is the schedule. Forge's
`Planner` serves the reconstruction records of crafting-building-repair.md and nothing else at first.

**Dave Mark. *Behavioral Mathematics for Game AI*. Charles River Media / Course Technology, 2009.
— Dave Mark, Mike Lewis (ArenaNet). "Building a Better Centaur: AI at Massive Scale." GDC 2015 AI
Summit; and Intrinsic Algorithm, "IAUS" (the Infinite Axis Utility System page).** [book] [talk]
[still-current]
<https://openlibrary.org/books/OL25155853M/Behavioral_mathematics_for_game_AI> ·
<https://www.gdcvault.com/play/1021848/Building-a-Better-Centaur-AI> ·
<https://www.gameai.com/iaus.php> (verified through search results)

The utility school. The book's "topics include utility, the fallacy of rational behavior, and the
inconsistencies and contradictions that human behavior often exhibits", with response curves as
the tool. The talk showed "a new architecture that combines a modular, utility-based AI system and a
powerful influence map engine", the Infinite Axis Utility System, "designed to be a data-driven,
self-contained architecture that, once hooked up to the inputs and outputs of the game system, did
not require much programming support", deployed for Guild Wars 2: Heart of Thorns, with a time-lapse
of designers building "unique AI packages for their NPCs in as little as seven minutes". (The
co-presenter was Mike Lewis, not Kevin Dill; Dill co-authored the 2010 utility-theory talk, not
re-verified today.)
*Bearing:* the decision core the owner chose. A decision is a list of *considerations*, each an
input (a need, a distance, a time of day, an influence-map value) through a curve to [0, 1],
multiplied together with a compensation factor and a weight; the highest wins, with hysteresis. It
is data (P6), per archetype, and it is deterministic if the inputs are and ties break on entity
index.

**Kenneth D. Forbus (Northwestern), Will Wright (Maxis). "Some notes on programming objects in The
Sims" (course notes, 2001). — Richard Evans (Maxis). "Modeling Individual Personalities in The Sims
3." GDC 2010.** [web] [talk] [foundational]
<https://users.cs.northwestern.edu/~forbus/c95-gd/2001/homework/best-sims.html> ·
<https://www.gdcvault.com/play/1012450/Modeling-Individual-Personalities-in-The> (verified through
search results)

Smart objects and needs. In The Sims the objects are smart and the people are dumb: an object
*advertises* what it can satisfy (hunger, fun, comfort) and carries the interaction's animation and
state machine; a Sim's needs decay, and choice is a scored match of needs against advertisements
nearby. Evans's Sims 3 talk aimed to "make each Sim have her own individual personality which was
clearly manifest in autonomous behavior": the utility-weighted choice is drawn from a Boltzmann
distribution whose temperature rises as the Sim does worse, and personality is "80 traits, and 5 per
Sim", each trait a sparse row of modifiers over the actions.
*Bearing:* two of Forge's records come from here verbatim in spirit: `SmartObject { affordances:
[(Need, gain, duration, anim)] }` on benches, wells, counters, doors and beds (the interaction
lives with the object, so a new prop is data, D-035), and `Needs` that decay per tick and drive
the utility scores. The Boltzmann draw is fine on the server (seeded per entity and tick, D-016);
its temperature is the cheapest "personality" knob there is.

**Remco Straatman, Tim Verweij, Alex J. Champandard (Guerrilla). "Killzone 2 Multiplayer Bots."
Paris Game AI Conference, 2009; with Tim Verweij, "A hierarchically-layered multiplayer bot system
for a first-person shooter", master's thesis, 2007 (the PDF's "VUA07" prefix suggests VU
Amsterdam); and Guerrilla's "Hierarchical AI for Multiplayer Bots in Killzone 3", *Game AI Pro*,
ch. 29, 2013. — Troy Humphreys (High Moon).
"Exploring HTN Planners through Example." *Game AI Pro*, ch. 12, CRC Press, 2013.** [talk] [paper]
[book] [still-current]
<https://www.guerrilla-games.com/media/News/Files/VUA07_Verweij_Hierarchically-Layered-MP-Bot_System.pdf>
· <http://aigamedev.com/open/coverage/killzone2/> ·
<https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter12_Exploring_HTN_Planners_through_Example.pdf>
(verified through search results)

Hierarchical task networks in production. For Killzone 2 "the AI system was changed to make use of
a specialized implementation of a Hierarchical Task Network (HTN) planner" under a strategy layer
(squads, objectives) that hands the individual a task; Humphreys' chapter is the readable
description of "a total-order forward decomposition planner that was used on Transformers: Fall of
Cybertron", and the open-source Fluid HTN (§7) is built on it: "Partial planning", domain splicing,
"Replan only when plans complete/fail or when world state change".
*Bearing:* if the reconstruction planner outgrows GOAP's flat action list (it will, once "rebuild
the mill" is a dozen ordered sub-jobs with shared resources), the HTN is the next shape, and
Humphreys' total-order decomposition is small enough to write in a week. Guerrilla's layering —
strategy above, individual below, one interface between — is also the shape of §5's fleet AI.

**Julian Berteling (Guerrilla). "Beyond 'Killzone': Creating New AI Systems for 'Horizon Zero
Dawn'." GDC 2018 AI Summit; with Arjen Beij, "The AI of Horizon Zero Dawn", Game AI North 2017, and
Game Developer's two-part write-up.** [talk] [web] [recent]
<https://www.guerrilla-games.com/read/beyond-killzone-creating-new-ai-systems-for-horizon-zero-dawn>
· <https://www.gdcvault.com/play/1024912/Beyond-Killzone-Creating-New-AI> ·
<https://www.gamedeveloper.com/design/behind-the-ai-of-horizon-zero-dawn-part-1-> (verified through
search results; the talk is GDC 2018, not 2017 as the brief had it)

The move "from having to support a single human enemy in closed corridor spaces to a game with more
than 25 wildly different characters in a large open world", and "how they changed the navigation
and animation systems": HTN planning kept from Killzone, sensors calibrated per machine type,
navigation for walkers and fliers, and herds as a *group agent* — "herds rebalance their
composition when under threat, forming 'flee' and 'combat' groups, and a group agent enables data to
be shared between each machine, though herds don't have a hive intelligence".
*Bearing:* the group agent is Forge's `Herd` record (§4): shared blackboard, roles assigned per
threat, no omniscience. The per-species sensor calibration is the argument for `Senses` as data per
archetype rather than one sight cone for everything.

**Bethesda Game Studios. "Radiant AI" (Oblivion, 2006) and the Creation Kit's AI Packages
(Skyrim, 2011): packages, procedures, conditions.** [web] [docs] [still-current]
<https://en.wikipedia.org/wiki/Radiant_AI> · <https://ck.uesp.net/wiki/Category:Packages> ·
<https://ck.uesp.net/wiki/Category:Procedures> (verified through search results)

Schedules as the top of the stack. Radiant AI gave NPCs "general goals, such as 'Eat in this
location at 2pm'", left "to determine how to achieve them"; in the Creation Kit "AI Packages are an
actor's daily behavior", "Package Templates are composed of a structured Tree of Procedures", and
"the game will periodically reevaluate each Actor's Package Stack, with the topmost package whose
conditions are satisfied being run", a Travel procedure taking "data defining the location to
travel to". The wiki's own history is that Oblivion's autonomy was toned down after it produced
unwanted emergent behaviour, and later titles kept the packages and added random encounters.
*Bearing:* the `Schedule` record: an ordered stack of `(conditions, procedure tree)` entries,
re-evaluated at a low rate (once a game-minute), with the utility layer choosing *inside* the
current package (which bench, which stall). The Oblivion lesson is the reason the schedule is the
outer loop and utility the inner one, not the reverse: the day stays legible.

**Dmitriy Iassenev (GSC Game World). "A-Life, Emergent AI and S.T.A.L.K.E.R.": interview,
AiGameDev.com, 2008; with Game Developer's "A-Life: An Insight into Ambitious AI" (2017). — GSC
Game World's statements on A-Life 2.0 after the launch of S.T.A.L.K.E.R. 2 (November 2024), as
reported by PC Gamer, GameSpot and Windows Central.** [web] [foundational] [recent]
<http://aigamedev.com/open/interviews/stalker-alife/> ·
<https://www.gamedeveloper.com/design/a-life-an-insight-into-ambitious-ai> ·
<https://www.pcgamer.com/games/fps/stalker-2-devs-reassure-players-yes-a-life-2-0-is-in-the-game-no-its-not-working-right-and-yes-fixes-are-on-the-way/>
(verified through search results)

The offline/online split, stated by its author: "The gist of the A-life is that the characters in
the game live their own lives and exist all the time, not only when they are in the player's field
of view." Offline, characters move on a coarse graph between levels and fight by statistics; online,
within the player's radius, they are full agents. The 2024 sequel is the cautionary tale: the
studio said the system was in the game but broken, that "aggressive optimization of the game led to
major reductions in the system's sphere of influence", and that it "requires much larger area for
spawn NPCs, and it requires much more memory resources"; players saw spawns appear close by and
few groups travelling.
*Bearing:* both halves of §6 in one game: the layered simulation is a 2007 idea that worked, and its
failure mode is a radius too small and a budget too tight, which shows up as pop-in of *people*.
Forge's regional layer needs a radius measured in kilometres, a memory line item and a test that
watches spawns for visibility, before the first player walks the island.

**Michael Booth (Valve). "The AI Systems of Left 4 Dead." Keynote, AIIDE 2009 (Stanford), slides on
Valve's site. — Elan Ruskin (Valve). "AI-driven Dynamic Dialog through Fuzzy Pattern Matching." GDC
2012. — Pointers: Patrick Ewing, William Armstrong, "Do You Copy? Dialog System and Tools in
'Firewatch'", GDC 2017; Darren Korb, Greg Kasavin, the dialogue of Hades, GDC 2021.** [talk]
[still-current]
<https://steamcdn-a.akamaihd.net/apps/valve/2009/ai_systems_of_l4d_mike_booth.pdf> ·
<https://steamcdn-a.akamaihd.net/apps/valve/2012/GDC2012_Ruskin_Elan_DynamicDialog.pdf> ·
<https://www.gdcvault.com/play/1024415/Do-You-Copy-Dialog-System> ·
<https://gdconf.com/article/dive-into-the-dialogue-of-hades-at-gdc-2021/> (verified through search
results)

The director and the barks. Booth's keynote covers the Director that tailors each playthrough's
pacing and the navigation behind the hordes; Ruskin's talk is "a simple, uniform mechanism made for
the Left 4 Dead series for tracking thousands of facts and possibilities, allowing intelligent
characters to remember history, cascade from special to general cases, and select the optimal
dialog, script, behavior, or animation for every situation" — a query of facts against rules sorted
by specificity. Firewatch grew "from its beginnings as an interrupt heavy bark system" into
restartable conversations; Hades ships "300,000 words, 200,000 of which are dialogue, alongside
21,000 voice lines" selected by priority and conditions.
*Bearing:* Forge's bark system is Ruskin's rule matcher over a fact dictionary per agent (who, where,
what just happened, the material underfoot from D-007), run on the client from replicated events
(the server decides *that* a bark fires and its id, never the audio). The director is a server
system that scores pacing from the same facts; both are data.

**Peter R. Wurman et al. (Sony AI). "Outracing champion Gran Turismo drivers with deep reinforcement
learning." *Nature* 602, 2022, 223–228. — Oriol Vinyals et al. (DeepMind). "Grandmaster level in
StarCraft II using multi-agent reinforcement learning." *Nature* 575, 2019, 350–354. — SIMA Team
(DeepMind). "Scaling Instructable Agents Across Many Simulated Worlds." arXiv:2404.10179, 2024,
and SIMA 2, 2026.** [paper] [recent]
<https://www.nature.com/articles/s41586-021-04357-7> (DOI 10.1038/s41586-021-04357-7) ·
<https://www.nature.com/articles/s41586-019-1724-z> (DOI 10.1038/s41586-019-1724-z) ·
<https://arxiv.org/abs/2404.10179> (verified through search results)

Where learned policies stand. GT Sophy combined "model-free, deep reinforcement learning algorithms
with mixed-scenario training", "winning a head-to-head competition against four of the world's best
Gran Turismo drivers", with a reward shaped for "sportsmanship rules"; AlphaStar reached "the top
0.2% of human players"; SIMA agents take "image observations and language instructions" and output
"keyboard-and-mouse actions" across nine commercial games. All three run a network per decision on
accelerators, and none is a function the client and the server can both evaluate to the same bits.
*Bearing:* off the server tick, by P3 and D-016. The two legitimate places are offline (a learned
policy distilled into a utility table or a small tree, which is then data) and the client-side
locomotion controllers animation.md already routes through DReCon → SuperTrack. A racing line or a
dogfight policy for a *demo* opponent could run on the owner's GPU, but it would not replay.

**Epic Games. "State Tree", "Using Navigation Invokers", "City Sample Project" and the ZoneGraph
and Mass plugins. Unreal Engine 5.x documentation. — Godot Engine. "Navigation Server for Godot
4.0" and the navigation tutorials.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/state-tree-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/city-sample-project-unreal-engine-demonstration>
· <https://godotengine.org/article/navigation-server-godot-4-0/> (verified through search results;
the Godot tutorial on navigation meshes read on GitHub)

The engines' current defaults. Unreal keeps Behavior Trees and, since 5.0, adds StateTree, "a
general-purpose hierarchical state machine that combines the Selectors from behavior trees with
States and Transitions from state machines", Smart Objects that agents claim through a reservation,
Mass Entity for "thousands of entities", and ZoneGraph, "a lightweight navigation system that uses
metadata to represent specific navigation flows (traffic lanes, sidewalk, wood trail, etc.) as
opposed to a Navmesh"; the City Sample "uses multiple spawners, one each for crowds, intersections,
traffic, and parked vehicles" and ships "22 modular building kits, 13 vehicles". Godot 4 has no
behaviour tree of its own (LimboAI and Beehave, §7, fill the gap) but a NavigationServer whose
"navigation mesh generation is done using the Recast software", with
`bake_from_source_geometry_data_async()` for threaded runtime baking and RVO2 for avoidance;
"The NavigationRegion3D baking can also be used at runtime with scripts".
*Bearing:* the two things to copy are the *reservation* on a smart object (one claimant, a timeout)
and ZoneGraph's idea that lanes are a graph of their own, not a navmesh: Forge's roads from
city-generation.md already are polylines with lanes, so traffic (§2) walks them directly.

---

## 2. Navigation and movement

The ground truth is Recast; everything else in this section is either what to run on its output
or what to use where a navmesh does not apply (crowds, flight, lanes).

**Mikko Mononen et al. Recast & Detour (Recast, Detour, DetourTileCache, DetourCrowd). GitHub,
2009–2026, zlib.** [code] [foundational] [still-current]
<https://github.com/recastnavigation/recastnavigation>

Read on GitHub. The "industry-standard navigation-mesh toolset for games": Recast is "Navmesh
generation", Detour "Runtime loading of navmesh data, pathfinding, navmesh queries", DetourTileCache
"Navmesh streaming. Useful for large levels and open-world games", DetourCrowd "Agent movement,
collision avoidance, and crowd simulation"; "Recast & Detour is licensed under the ZLib license."
The pipeline rasterises triangles into a voxel heightfield, filters the walkable spans, partitions
them into regions, traces contours and polygonises them, per tile or as one mesh.
*Bearing:* the algorithm Forge's `forge-nav` implements over the terrain's heightfield and the
village's modules, per tile of the D-037 cells, with the same voxel size and agent parameters on
client and server. Not the C++ as a dependency: §7 weighs the ports and the bindings.

**Mikko Mononen. "Tiled Mesh Progress Pt. 2" (July 2009), "Improving Local Avoidance" (December
2009), "Handling Temporary Obstacles" (August 2010), "Temporary Obstacle Progress" (January 2011).
Digesting Duck (blog).** [web] [foundational] [still-current]
<http://digestingduck.blogspot.com/2010/08/handling-temporary-obstacles.html> ·
<http://digestingduck.blogspot.com/2011/01/temporary-obstacle-progress.html> ·
<http://digestingduck.blogspot.com/2009/12/improving-local-avoidance.html> (verified through search
results)

The design notes behind the tile cache: the preferred way to handle dynamic obstacles "is to
recreate the tiles where an obstacle has changed", with tiling limiting the work; the cache stores a
compressed layered heightfield per tile and rebuilds the tile's navmesh with the temporary obstacles
stamped in, "taking about 2 ms to update one tile"; and the avoidance post explains why DetourCrowd
samples velocities against velocity obstacles rather than solving ORCA's linear programme.
*Bearing:* the cost model for "a navmesh over the generated terrain rebuilt where the terrain
changes": keep the compact heightfield per tile, invalidate the tiles a crater or a placed module
touches, and rebuild them in `Low` jobs — a handful of milliseconds per tile on a 2011 CPU is well
under a tick even for a dozen tiles. Rebuilds are deterministic per tile if the input triangles are
sorted by id.

**Epic Games. "Using Navigation Invokers" and dynamic runtime generation. Unreal Engine 4.27 /
5.x documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/using-navigation-invokers-in-unreal-engine>
(verified through search results)

Unreal's answer to worlds too large to bake: invokers "generate the Navigation Mesh around the Agent
at runtime", which "removes the need to build the Navigation Mesh in the editor and can also limit
the number of tiles generated at runtime", with "Runtime Generation" set to "Dynamic"; off-mesh
links carry "specific actions like opening doors or jumping down from a ledge".
*Bearing:* the invoker is the shape for a planet: navmesh tiles exist only around agents that ask
(players, full agents), generated on demand like the terrain tiles, cached by content hash, and the
links (doors, ledges, ladders, the vault of animation.md §5) are data on the modules.

**Michael Cui, Daniel D. Harabor, Alban Grastien. "Compromise-free Pathfinding on a Navigation
Mesh." IJCAI 2017 (Polyanya); with the `polyanya` Rust crate.** [paper] [code] [still-current]
<https://www.ijcai.org/proceedings/2017/0070.pdf> (cited from the crate's README, read on GitHub) ·
<https://github.com/vleue/polyanya>

Any-angle search directly on the polygon mesh, optimal without a post-smoothing pass. The Rust
crate, read on GitHub, is "a any-angle path planning algorithm" with "Multi-layer Navigation Mesh
Support" (overlapping layers for 3D, one-way layers, layers enabled and disabled, layers with
different costs), meshes built "by specifying their outer edges and inner obstacles", dual-licensed
MIT/Apache-2.0.
*Bearing:* the query on top of Forge's tiles: Polyanya's search replaces Detour's corridor plus
string-pulling with one step and a shorter path; its layer flags are how a closed door or a burning
bridge is removed from routing without a rebuild.

**Elijah Emerson (Gas Powered Games). "Crowd Pathfinding and Steering Using Flow Field Tiles."
*Game AI Pro*, ch. 23, CRC Press, 2013. — Owen McCarthy (Frontier). "Simulating 10,000 Guests in
Planet Coaster" (talk, 2017) and Game Developer's "Game Design Deep Dive: Creating believable crowds
in Planet Coaster".** [book] [talk] [web] [still-current]
<https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter23_Crowd_Pathfinding_and_Steering_Using_Flow_Field_Tiles.pdf>
· <https://www.gamedeveloper.com/audio/game-design-deep-dive-creating-believable-crowds-in-i-planet-coaster-i->
(verified through search results)

Flow fields when many share a goal. Emerson's Supreme Commander 2 system moves "hundreds to
thousands of individual agents" by integrating a cost field into a per-tile flow field once per
goal and letting every unit read its cell's direction. Frontier's principal programmer: "10,000
guests was what we targeted, and simulating each seemed like a challenge. This was where using
flow/potential fields became very appealing"; the crowd is animated from "1,512 separate animations
totaling 92,740 frames of animation". (The brief's 20 000 was not found; 10 000 is the stated
target.)
*Bearing:* the crowd path for the market and the festival: a flow field per attractor (the well,
the stage, the gate) on a 1 m grid over the navmesh's walkable cells, integrated on the server in a
job when the attractor appears, read by the mid-tier agents as their velocity. Individuals keep
Polyanya paths; crowds read fields.

**Jur van den Berg, Ming Lin, Dinesh Manocha. "Reciprocal Velocity Obstacles for Real-Time
Multi-Agent Navigation." ICRA 2008, 1928–1935. — Jur van den Berg, Stephen J. Guy, Ming Lin,
Dinesh Manocha. "Reciprocal n-Body Collision Avoidance." ISRR 2009, published in *Robotics Research*
(Springer STAR 70), 2011, 3–19; with the RVO2 library (UNC, Apache-2.0).** [paper] [code]
[foundational] [still-current]
<https://gamma.cs.unc.edu/ORCA/> (verified through search results) · <https://github.com/snape/RVO2>

The local-avoidance standard. RVO lets "each agent take half of the responsibility of avoiding
pairwise collisions"; ORCA turns that into a linear programme over half-planes per neighbour. The
library, read on GitHub, presents "a formal approach to reciprocal collision avoidance, where
multiple independent mobile robots or agents need to avoid collisions with each other without
communication among agents while moving in a common workspace", as "an open-source C++98
implementation of our algorithm in two dimensions" under Apache-2.0 (a 3D variant, RVO2-3D, exists
in the same group).
*Bearing:* the last metre for every walking agent, on the server, in a fixed neighbour order so the
result is the same on every worker; `dodgy` (§7) is the Rust port to read against. ORCA also
answers the herd's spacing and the fish school's without a second system.

**Craig W. Reynolds. "Flocks, Herds, and Schools: A Distributed Behavioral Model." *Computer
Graphics* 21(4) (SIGGRAPH '87), 25–34. — "Steering Behaviors For Autonomous Characters." GDC 1999
proceedings, 763–782. — "Big Fast Crowds on PS3." ACM SIGGRAPH Symposium on Videogames (Sandbox)
2006, 113–121; with OpenSteer (MIT).** [paper] [talk] [code] [foundational] [still-current]
<https://www.red3d.com/cwr/papers/1987/boids.html> ·
<https://www.red3d.com/cwr/papers/1999/gdc99steer.html> ·
<https://www.red3d.com/cwr/papers/2006/PSCrowdSandbox2006.pdf> (DOI 10.1145/1183316.1183333;
verified through search results) · <https://github.com/meshula/OpenSteer>

The whole steering vocabulary. Boids: "each simulated bird is implemented as an independent actor
that navigates according to its local perception of the dynamic environment" (separation, alignment,
cohesion). The 1999 paper adds seek, flee, pursuit, evasion, arrival, wander, obstacle avoidance,
path and wall following, leader following and queuing, each a small force added to a point-mass
vehicle. The 2006 paper ran "15,000 individuals constrained to move on a 2D ground plane" at
60 fps on the PS3's SPUs, each SPU owning a spatial bucket. OpenSteer, read on GitHub, "is a C++
library for constructing steering behaviors for autonomous characters in games and animation",
begun "by Craig Reynolds beginning in 2002 at the Research and Development group of Sony Computer
Entertainment America", MIT.
*Bearing:* the movement layer under every decision layer in this file: walkers, herds, birds, fish
and ships all steer; only the constraints differ (a navmesh, a flow field, a 3D octree, a thrust
envelope). The PS3 number — a few thousand boids per core per frame — is the first calibration
point for §9's budget.

**Daniel Brewer (Digital Extremes). "3D Flight Navigation Using Sparse Voxel Octrees." *Game AI
Pro 3*, ch. 21, CRC Press, 2017; with Brewer, Sturtevant, "Benchmarks for Pathfinding in 3D Voxel
Space" (SoCS 2018), and the Nav3D plugin (MIT).** [book] [paper] [code] [still-current]
<https://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter21_3D_Flight_Navigation_Using_Sparse_Voxel_Octrees.pdf>
· <https://webdocs.cs.ualberta.ca/~nathanst/papers/voxels.pdf> (verified through search results) ·
<https://github.com/darbycostello/Nav3D>

Flight without a floor. Warframe "represents the data in a sparse voxel octree and uses a Morton
(or z-order) encoding", searches it with A* at mixed resolutions and smooths; the benchmark paper
compares the search variants on voxel maps. Nav3D, read on GitHub, implements "Daniel Brewer's '3D
Flight Navigation Using Sparse Voxel Octrees' from Game AI Pro 3" for Unreal, with "Real-time
modifications without full rebuilds", A* and "Theta-Star: Most accurate with line-of-sight
shortcuts".
*Bearing:* the navigation structure for birds over the village, fish in the lagoon and ships in the
belt: a sparse voxel octree per region, empty where nothing is, built from the same colliders as the
navmesh, searched by Theta*, with a 3D flow field per attractor for flocks. In open space it
degenerates to a few large empty leaves and the asteroids' occupied ones, which is what #80 needs.

**Dave Pottinger (Ensemble). "Coordinated Unit Movement" and "Implementing Coordinated Movement."
*Game Developer* (Gamasutra), January 1999. — Tucker Balch, Ronald C. Arkin. "Behavior-based
formation control for multirobot teams." *IEEE Transactions on Robotics and Automation* 14(6), 1998,
926–939.** [web] [paper] [foundational]
<https://www.gamedeveloper.com/programming/coordinated-unit-movement> ·
<https://www.gamedeveloper.com/programming/implementing-coordinated-movement> (verified through
search results) · DOI 10.1109/70.736776 (verified through search results)

Formations. Pottinger's two articles are the RTS treatment (group leaders, formation slots, path
sharing, collision resolution with priorities); Balch & Arkin give the robotics form — formation
keeping as one more behaviour summed with goal seeking and hazard avoidance, "on robots in the
laboratory and aboard DARPA's HMMWV-based Unmanned Ground Vehicles" — with line, column, diamond and
wedge and three reference schemes (leader, neighbour, unit centre).
*Bearing:* one `Formation` record for the patrol, the herd on the move and the fighter wing:
slots in a leader frame, each member steering to its slot with Balch & Arkin's weighting, the
leader alone planning the path. Pottinger's shared path is Factorio's group path (below).

**Colossal Order. Cities: Skylines' agent limits (65 536 citizens and 16 384 vehicles) and
"Development Diary #2: Traffic AI" for Cities: Skylines II (2023). — Rockstar's vehicle path nodes
(GTA IV/V `.ynd`, as documented by the modding wikis). — Roxanne Blouin-Payer (Ubisoft). "Helping
It All Emerge: Managing Crowd AI in 'Watch Dogs 2'." GDC 2017; and "Replicating Chaos: Vehicle
Replication in 'Watch Dogs 2'", GDC 2017.** [web] [talk] [still-current]
<https://colossalorder.fi/?p=1597> · <https://gtamods.com/wiki/.ynd> ·
<https://www.gdcvault.com/play/1024426/Helping-It-All-Emerge-Managing> ·
<https://gdcvault.com/play/1024597/Replicating-Chaos-Vehicle-Replication-in> (verified through
search results)

Traffic as agents on a lane graph. The first Cities: Skylines had "a hard limit of 65,536
individual citizen agents" plus 16 384 vehicles; in the sequel "pathfinding was proximity-based" no
longer — agents route on the network with costs for time, comfort and money, lane changes are part
of the path, and the game "doesn't feature hard limits for agents". GTA's traffic drives on
authored node files: "YND files (YND = Path Nodes) in GTA V define navigation paths for vehicles and
pedestrians, containing nodes and connections". Watch Dogs 2's crowd talk is Ubisoft's account of
emergent crowd AI at city scale, and its replication talk (projective velocity blending) is the
netcode for vehicles other players drive.
*Bearing:* #87's traffic: the road polylines of city-generation.md become a lane graph (`Lane {
polyline, width, direction, speed, next: [LaneId] }`, ZoneGraph's shape), vehicles are agents on
lanes with a car-following rule and a signal at each node, routed once by a cheap graph search and
re-routed on blockage; nothing needs the navmesh. Scripted traffic first (#79's moving geometry),
agents later.

**Wube Software. "Friday Facts #317 — New pathfinding algorithm." factorio.com, 18 October 2019.**
[web] [still-current]
<https://factorio.com/blog/post/fff-317> (verified through search results)

Pathfinding in the best-documented lockstep game: A* with a bidirectional search and a coarse
"chunk reduction" layer, and the group rule — "biters usually form groups so only one path needs to
be found for the entire group, and the paths get cached so they can be re-used later" — all of it
deterministic by construction because the game is lockstep (netcode.md §5).
*Bearing:* three rules for Forge's pathfinder: a coarse-to-fine search (cells, then tiles, then
polygons) so a cross-island path is cheap, one path per group with members steering to it, and a
path cache keyed by (from cell, to cell, archetype) that is part of the digested state.

---

## 3. Crowds, cities and the simulation level of detail

**Dirk Helbing, Péter Molnár. "Social force model for pedestrian dynamics." *Physical Review E*
51(5), 1995, 4282–4286. — Adrien Treuille, Seth Cooper, Zoran Popović. "Continuum Crowds." *ACM
Transactions on Graphics* 25(3) (SIGGRAPH 2006), 1160–1168.** [paper] [foundational]
<https://link.aps.org/doi/10.1103/PhysRevE.51.4282> (DOI 10.1103/PhysRevE.51.4282) ·
<https://dl.acm.org/doi/10.1145/1141911.1142008> (DOI 10.1145/1141911.1142008) (verified through
search results)

The two continuum views. Helbing's pedestrians move as if under "social forces": an acceleration
towards the desired velocity, repulsion that keeps "a certain distance from other pedestrians and
borders", and attraction. Treuille's crowd is a field: "a dynamic potential field simultaneously
integrates global navigation with moving obstacles such as other people, efficiently solving for the
motion of large crowds without the need for explicit collision avoidance", with lane formation and
other emergent phenomena for free.
*Bearing:* Forge does not need either as a system — ORCA and flow fields cover the same ground with
better control — but Helbing's force terms are the acceptance test for the crowd's *look* (do lanes
form in the market, does a doorway arch), and Treuille's density-weighted cost is the one addition
to Emerson's flow field that keeps a festival from funnelling everyone through one gap. UNC's Menge
(§7, read on GitHub), the modular crowd framework built around "goal selection, plan computation,
plan adaptation", names the decomposition Forge's schedule-and-utility, pathfinder and avoidance
layers follow, and its XML scenarios are ready-made tests for the avoidance step.

**François Cournoyer, Fernando Fonseca (Ubisoft Montréal). "Massive Crowd on Assassin's Creed
Unity: AI Recycling." GDC 2015.** [talk] [still-current]
<https://www.gdcvault.com/play/1022141/Massive-Crowd-on-Assassin-s> (archived:
<https://archive.org/details/GDC2015Cournoyer>; verified through search results)

The AI level of detail with numbers: "a pooling system that swapped from low-res NPCs to high-res
NPCs without the player noticing", and "with the limit of 40 real AIs and 120 high resolution
models, they could successfully create a scene where 10,000 crowd NPCs are on screen at the same
time". Hitman: Absolution's 1 200-character crowds (Fauerby, GDC Europe 2012) are the same idea a
console earlier; animation.md §5 carries that entry and the rendering tiers.
*Bearing:* the budget shape for the island's village and later the city: tens of full agents,
around a hundred with bodies and cheap brains, thousands as crowd particles on flow fields, and
promotion/demotion by distance, visibility and interest, with a pool so promotion never allocates.
The recycled agent keeps only what §6's hand-off says it keeps.

**Christopher Dragert (Ubisoft Toronto). "Census: The Systemic Backbone Behind Play As Anyone in
'Watch Dogs: Legion'." GDC 2021.** [talk] [recent]
<https://www.gdcvault.com/play/1027018/Census-The-Systemic-Backbone-Behind> (verified through search
results)

The population as a database: Census "generates characters, simulates their lives, and inserts them
into gameplay", with "detailed demographics and social profiles, which form the basis of
dynamically-generated schedules that span the entire open world, including meetings with friends,
relations, and adversaries"; the talk's own list of problems is "optimizing runtime performance of
the relational database", the generation algorithms and tagging across content teams.
*Bearing:* the persistent layer of a city's people is *records*, not agents: a `Person` row
(archetype, home, work, relations, schedule seed) from which a full agent is instantiated where the
player is and back into which it folds. It is D-035's data model doing AI, and the seed makes the
same person come back the next day.

**Cyril Brom, Ondřej Šerý, Tomáš Poch. "Simulation Level of Detail for Virtual Humans." *Intelligent
Virtual Agents* (IVA 2007), Springer LNCS. — Stephen Chenney, David Forsyth. "View-dependent
culling of dynamic systems in virtual environments." *Symposium on Interactive 3D Graphics* 1997.
— Stephen Chenney, Okan Arikan, David Forsyth. "Proxy Simulations for Efficient Dynamics."
Eurographics 2001, short presentations.** [paper] [foundational] [still-current]
<https://link.springer.com/chapter/10.1007/978-3-540-74997-4_1> (DOI 10.1007/978-3-540-74997-4_1)
· <https://dl.acm.org/doi/10.1145/253284.253307> ·
<http://www.okanarikan.com/assets/Papers/ProxySimulations/paper.pdf> (verified through search
results)

The academic name for §6. Brom et al. define "simulation LOD, which reduces quality of the
simulation at places unseen", for "a large virtual-storytelling game populated by tens of complex
virtual humans", simplifying both the space (rooms collapse to nodes) and the behaviour (plans run
at coarser steps). Chenney & Forsyth cull "moving objects by not solving the equations of motion
for objects that don't affect the view, while addressing problems of consistency and completeness";
the 2001 paper replaces culled dynamics with "proxy simulations, which reduce the cost of simulation
in large virtual worlds".
*Bearing:* the two words to keep are Chenney's: *consistency* (what the player finds when they come
back must be what the proxy predicted) and *completeness* (nothing important may be skipped). They
are the two tests in §9 for the regional and statistical layers.

**Tynan Sylvester (Ludeon). "'RimWorld': Contrarian, Ridiculous, and Impossible Game Design
Methods." GDC 2017. — Tarn Adams (Bay 12), Dwarf Fortress talks (GDC 2016 and after), as covered by
PC Gamer; Kenshi (Lo-Fi Games, 2018) and Mount & Blade (TaleWorlds) as titles.** [talk] [web]
[still-current]
<https://www.gdcvault.com/play/1024232/-RimWorld-Contrarian-Ridiculous-and> (slides:
<https://media.gdcvault.com/gdc2017/Presentations/Sylvester_Tynan_RimWorld_Contrarian_Ridiculous.pdf>)
· <https://www.pcgamer.com/dwarf-fortress-creator-on-how-hes-42-towards-simulating-existence/>
(verified through search results)

Off-screen worlds. RimWorld is framed "not as a game, but as a story generator": a few dozen fully
simulated pawns on one map, an AI storyteller choosing events, and the rest of the planet abstract
(factions, settlements, caravans as records). Dwarf Fortress aims at "an actual fantasy world
simulator, and storytelling engine" whose world-generation runs history and civilisations as
statistics and its fortress as agents; Kenshi keeps squads and factions moving across its map while
the player is elsewhere, and Mount & Blade's campaign map moves lords' parties by the same rules the
player follows — both known to the file by title only (§11).
*Bearing:* the tradition that makes the statistical layer respectable: the games with the deepest
"living worlds" simulate very few agents and a great many records, and their players do not notice
the seam because the records are consistent (Chenney's word). Forge's planet layer is a RimWorld
world map with climate.

**Tom Leonard (Looking Glass). "Building an AI Sensory System: Examining the Design of Thief: The
Dark Project." GDC 2003; article on Gamasutra (now Game Developer).** [talk] [web] [foundational]
<https://www.gamedeveloper.com/programming/building-an-ai-sensory-system-examining-the-design-of-i-thief-the-dark-project-i->
· <https://gdcvault.com/play/1022627/Building-AI-Sensory> (verified through search results)

Perception done properly: the article "lays out basic concepts of AI senses using Half-Life as a
motivating example, examines the more stringent sensory requirements of a stealth game design, and
describes the sensory system built for Thief" — vision as cones with light level and distance,
hearing from propagated sound events with a loudness, awareness as a graded state that rises and
decays, and memory of the last known position.
*Bearing:* `Senses` per archetype (cones, ranges, hearing threshold) and an `Awareness` per
(agent, target) that integrates evidence and decays; the sound events come from D-007's material
rows (a footstep on gravel is louder than on moss) through the same event bus the audio uses, so
stealth needs no second model of sound. The server evaluates senses for full agents only.

**Damián Isla (Bungie). "Building a Better Battle: The Halo 3 AI Objectives System." GDC 2008. —
Matthew Jack (Crytek). "Tactical Position Selection: An Architecture and Query Language." *Game AI
Pro*, ch. 26, CRC Press, 2013, 337–359.** [talk] [book] [still-current]
<https://gdcvault.com/play/497/Building-a-Better-Battle-HALO> (slides:
<https://web.cs.wpi.edu/~rich/courses/imgd4000-d09/lectures/halo3.pdf>) ·
<https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter26_Tactical_Position_Selection.pdf> (verified
through search results)

Groups and positions. Halo 3's designers script encounters as "tasks built into a task-tree
organized by priority" with capacities and conditions, and "squads trickle-down through the squad
tree" to the highest-priority open task; Jack's chapter is "a complete architecture for choosing
movement positions as part of sophisticated AI behavior" with a query language (generate candidates,
filter, weight, pick) used at Crytek.
*Bearing:* factions and groups in Forge are a task tree per faction (defend the mill, patrol the
road, flee to the keep) filled by squads, and every "where do I stand" question — cover, a vantage
point, a grazing spot, a firing position in the belt — is Jack's query over sampled points, which is
also how the smart-object choice and the animal's need zone are picked. One query engine, many
callers.

---

## 4. Animals, creatures and ecosystems

`planet-environment.md` §3 already decided the population model (densities per cell in Maxent's
habitat shape with damped Volterra coupling, individuals at need zones near players as theHunter
does, Eco as the persistence reference). This section adds the individual's brain, the herd, the
territory, the agent-based tradition the cell model must agree with, and the games that shipped
animals.

**Ubisoft Montréal. "Grounding Wildlife in the Mountains of Far Cry 4." GDC 2015. — Chris Seddon
(Ubisoft Toronto). "Animal House: Creating Systemic Animal Companions in Far Cry Primal." nucl.ai
2016; with Game Developer's "The Definition of [Artificial] Insanity: The Systemic AI of Far
Cry".** [talk] [web] [still-current]
<https://www.gdcvault.com/play/1022027/Grounding-Wildlife-in-the-Mountains> ·
<https://www.gamedeveloper.com/design/primal-instinct-companion-ai-in-far-cry-primal> ·
<https://www.gamedeveloper.com/programming/the-definition-of-artificial-insanity-the-systemic-ai-of-far-cry>
(verified through search results)

The food chain that made the series: since Far Cry 3 "predators will hunt prey" and the animals
"interact dynamically with the others on the island; for instance, if a deer is spotted by a tiger,
the tiger will go in for the kill", on "a systemic gameplay framework: where numerous systems and
mechanics interact with one another and enables emergent gameplay to arise"; Far Cry 4's talk is
about grounding those animals in mountain terrain, and Primal's about taming them into companions.
*Bearing:* the animal brain is the same utility layer as the villager's with different needs
(hunger, thirst, rest, safety, herd) and different smart objects (water, forage, cover, a carcass);
predator and prey are two archetypes whose senses and threat tables point at each other, and
"emergent" is what falls out when both run on the same tick. Taming is a faction change on one
record.

**Iain D. Couzin, Jens Krause, Richard James, Graeme D. Ruxton, Nigel R. Franks. "Collective memory
and spatial sorting in animal groups." *Journal of Theoretical Biology* 218(1), 2002, 1–11. — Paul
R. Moorcroft, Mark A. Lewis. *Mechanistic Home Range Analysis*. Princeton University Press
(Monographs in Population Biology 43), 2006.** [paper] [book] [foundational] [still-current]
<https://research-information.bris.ac.uk/en/publications/collective-memory-and-spatial-sorting-in-animal-groups/>
· <https://www.jstor.org/stable/j.ctt4cg9qf> (verified through search results)

The biology behind the herd and the territory. Couzin's zonal model (repulsion, alignment,
attraction shells) shows swarms, tori and polarised groups from the same rules and "the first
evidence for collective memory in animal groups"; Moorcroft & Lewis build home ranges from
"correlated random walk models for individual movement behavior" with scent marking and
conspecific avoidance, fitted to coyote territories in Yellowstone.
*Bearing:* the herd is boids with Couzin's three zones as species parameters (so a school and a deer
herd differ by numbers, not code), and a territory is a home-range centre with a scent field the
predator's utility reads; both are cheap enough for the mid tier and both give the visible
behaviour (a pack that patrols, a herd that turns as one) that a random walk never will.

**Uri Wilensky. "NetLogo Wolf Sheep Predation model." Center for Connected Learning and
Computer-Based Modeling, Northwestern University, 1997; with the NetLogo Models Library on GitHub.**
[web] [code] [foundational]
<http://ccl.northwestern.edu/netlogo/models/WolfSheepPredation> (verified through search results) ·
<https://github.com/NetLogo/models>

The reference agent-based ecosystem: wolves, sheep and regrowing grass on a grid, energy per
animal, reproduction by chance, with the well-known result that the two-species version collapses
and the version with grass persists. The models library, read on GitHub, "is bundled with NetLogo"
and its models are under mixed licences (this one CC BY-NC-SA 3.0), so it is a reference, not a
dependency.
*Bearing:* the calibration target for the cell model of planet-environment.md §3: run Wolf Sheep
Predation's rules as agents in one regional cell and the density equations for the same cell side
by side, and tune the damping until the densities track the agents' means over a hundred game-days.
The grass term is the coupling to the ecosystem bake's carrying capacity; without it the numbers
oscillate to zero, which players read as a bug.

**Volker Grimm et al. "A standard protocol for describing individual-based and agent-based
models." *Ecological Modelling* 198, 2006, 115–126; updated in *Ecological Modelling* 221, 2010,
2760–2768 and *JASSS* 23(2), 2020, 7. — Volker Grimm, Steven F. Railsback. *Individual-based
Modeling and Ecology*. Princeton University Press, 2005, 480 pp.** [paper] [book] [foundational]
[still-current]
<https://faculty.sites.iastate.edu/tesfatsi/archive/tesfatsi/ODDProtocolABM.GrimmEtAl.2006.pdf> ·
<https://www.jasss.org/23/2/7.html> ·
<https://press.princeton.edu/> (the book; verified through search results)

The discipline. ODD (Overview, Design concepts, Details) is the protocol ecologists use to describe
an agent-based model so that it can be reproduced — purpose, entities and state variables, process
overview and scheduling, design concepts (emergence, sensing, stochasticity), initialisation,
inputs, submodels; the book is "the first in-depth treatment of individual-based modeling and its
use to develop theoretical understanding of how ecological systems work".
*Bearing:* every Forge species and every ecosystem rule gets an ODD-shaped record in the data
(state variables, schedule, sensing, stochasticity source), which is also what the digest hashes.
The process-overview-and-scheduling item is D-016's "fixed order" written by ecologists thirty
years ago.

**"Using the Unity Game Engine to Develop a 3D Simulated Ecological System Based on a Predator–Prey
Model Extended by Gene Evolution." *Informatics* (MDPI) 9(1), 2022, article 9. — "A Study of AI
Population Dynamics with Million-agent Reinforcement Learning." arXiv:1709.04511, 2017. (Author
lists not shown in the search record; cited by title.)** [paper] [recent]
<https://www.mdpi.com/2227-9709/9/1/9> · <https://arxiv.org/abs/1709.04511> (verified through
search results)

Two ends of "ecosystem simulation in games" as papers: an engine-hosted predator–prey world where
"incorporating genetic evolution into simulations helps achieve system stabilization and long-term
operation, reducing the likelihood of extinction", and a million-agent learned population whose
dynamics reproduce Lotka–Volterra cycles.
*Bearing:* the first is the reminder that heritable variation (a `traits` vector per animal, drawn
per birth from the parents' seed) is cheap and stabilising; the second is evidence that agent
populations at scale *do* behave like the equations, which is what lets the planet layer be
equations.

**Red Dead Redemption 2's animals and routines (Rockstar, 2018), as described by press and community
write-ups; Horizon's machine herds (Guerrilla, §1); ARK: Survival Evolved's creatures (Studio
Wildcard, 2017) as a title.** [web] [recent]
<https://rockstarintel.com/a-day-in-the-life-of-a-red-dead-redemption-2-npc/> (verified through
search results; no primary Rockstar source exists, see §11)

What the bar looks like from outside: NPCs "have a fixed daily routine and fixed goals", farmers
waking before merchants, animals with diurnal patterns, predators that hunt, prey that flees, and
a memory of the player ("They will start looking over their shoulder if you follow them along their
routine"). None of it is published as technique.
*Bearing:* the acceptance criteria for the island's village and its animals are these observable
facts — a day that reads, animals at the water at dawn, a fox that hunts the hens — not any
particular architecture; the demo page should list them as checks.

---

## 5. Spaceship and vehicle AI

Ship AI is steering in six degrees of freedom under a thrust envelope, with pursuit and evasion as
the two primitive games, a threat table for target choice, a formation slot for the wing and a
planner for docking. The published sources are old games, robotics and guidance; the recent
learned work is in §1's verdict.

**Ian Bell, David Braben. Elite (Acornsoft, 1984): the fully documented BBC Micro source by Mark
Moxon, with the "TACTICS" routine and the deep dive "Aggression and hostility in ship tactics".**
[code] [web] [foundational]
<https://github.com/markmoxon/elite-source-code-bbc-micro-cassette> ·
<https://elite.bbcelite.com/deep_dives/> (verified through search results)

Read on GitHub: "Fully documented source code for the cassette version of Elite on the BBC Micro,
with every single line documented and (for the most part) explained"; "BBC Micro Elite was written
by Ian Bell and David Braben and is copyright © Acornsoft 1984", and "This repository is _not_
provided with a licence". The site's deep dives explain that "The TACTICS routine applies tactics to
ships with AI enabled" — a per-ship aggression byte, a few hundred bytes of rules: turn towards or
away from the target by dot products, fire when aligned, flee when damaged, launch a missile by
chance.
*Bearing:* the proof that a convincing space opponent is a page of rules over two dot products and
an aggression scalar; Forge's `ShipBrain` starts there (approach, attack pass, break, re-acquire),
as utility considerations, before any of the mathematics below.

**Volition / the FreeSpace 2 Source Code Project. FreeSpace Open (source released 25 April 2002;
`code/ai/aicode.cpp`, `ai.tbl`, `ai_profiles.tbl`).** [code] [docs] [still-current]
<https://github.com/scp-fs2open/fs2open.github.com> · <https://wiki.hard-light.net/index.php/Ai_profiles.tbl>
(verified through search results)

The most complete open space-combat AI: the repository (read on GitHub) is the "Origin Repository
for SCP FreeSpace 2 Open"; the wiki documents that "ai_profiles.tbl" "allows the creation and
management of different patterns of AI behavior called profiles, which consist of various statistics
and flags", and `ai.tbl` defines AI classes with abilities per difficulty. The AI code holds the
goal stack (chase, evade, guard, dock, waypoints, strafe), the pursuit geometry, the collision
avoidance against big ships and the wingman orders.
*Bearing:* the checklist of ship behaviours a battle needs (a dozen goals, an order system, a
per-class skill table) and the reminder that difficulty is a data column, not another brain. GPL,
so a reference to read.

**Rufus Isaacs. *Differential Games: A Mathematical Theory with Applications to Warfare and
Pursuit, Control and Optimization*. Wiley, 1965. — Timothy H. Chung, Geoffrey A. Hollinger, Volkan
Isler. "Search and pursuit-evasion in mobile robotics: A survey." *Autonomous Robots* 31(4), 2011,
299–316. — "Near-optimal interception strategy for orbital pursuit-evasion using deep reinforcement
learning." *Acta Astronautica*, 2022 (authors not shown in the search record).** [book] [paper]
[foundational] [recent]
<https://mathshistory.st-andrews.ac.uk/Extras/Isaacs_Differential_Games/> ·
<https://experts.umn.edu/en/publications/search-and-pursuit-evasion-in-mobile-robotics-a-survey/> (DOI
10.1007/s10514-011-9241-4) · <https://www.sciencedirect.com/science/article/abs/pii/S0094576522002764>
(verified through search results)

Pursuit and evasion as a game. Isaacs founded the field at RAND (the homicidal chauffeur, the game
of two cars) and was awarded the Lanchester Prize for the book; Chung et al. give "a taxonomy of
search problems" and the fundamental results for pursuers and evaders with different speeds, turn
rates and information; the 2022 paper formulates orbital pursuit–evasion "as a differential game"
with a closed-form barrier and learns a near-optimal interception law.
*Bearing:* the two facts a designer needs are in Isaacs: a faster pursuer with a worse turn rate is
beaten by turning inside it, and capture is decided by a barrier in state space, not by aim. The
ship's evasion consideration therefore scores *turn-in* when the pursuer is faster and *run* when it
is slower; the orbital work is for a later game with real orbits, not the belt.

**Robert L. Shaw. *Fighter Combat: Tactics and Maneuvering*. Naval Institute Press, 1985. — Adrian
P. Pope et al. (Lockheed Martin). "Hierarchical Reinforcement Learning for Air-to-Air Combat" (at
DARPA's AlphaDogfight Trials). arXiv:2105.00990, 2021.** [book] [paper] [foundational] [recent]
<https://www.usni.org/press/books/fighter-combat> · <https://arxiv.org/abs/2105.00990> (verified
through search results)

The manoeuvre vocabulary and its learned form. Shaw is the standard text on one-on-one and team
fighter tactics (lag and lead pursuit, the scissors, the yo-yos, energy management, the bracket and
the drag for pairs); Pope's agent took "2nd place finish in the final DARPA AlphaDogfight Trials
event" with "a high-level policy selector and a set of separately trained low-level policies
specialized for excelling in specific regions of the state space".
*Bearing:* the manoeuvres are the *actions* of the ship's utility layer (each a short scripted
control law: lag pursuit, lead pursuit, break turn, extend, bracket with a wingman), which is also
what the learned agent's low-level policies were; the selector is Forge's scorer instead of a
network, so it replays.

**Behçet Açıkmeşe, Scott R. Ploen (JPL). "Convex Programming Approach to Powered Descent Guidance
for Mars Landing." *Journal of Guidance, Control, and Dynamics* 30(5), 2007, 1353–1366. — Karl J.
Åström, Richard M. Murray. *Feedback Systems: An Introduction for Scientists and Engineers*.
Princeton University Press, 2nd ed. (free online). — Steven M. LaValle. *Planning Algorithms*.
Cambridge University Press, 2006 (free online). — James B. Rawlings, David Q. Mayne, Moritz M.
Diehl. *Model Predictive Control: Theory, Computation, and Design*. Nob Hill, 2nd ed.** [paper]
[book] [foundational]
[still-current]
<https://www.researchgate.net/publication/285667191_Convex_programming_approach_to_powered_descent_for_mars_landing>
· <https://fbswiki.org/wiki/index.php/Feedback_Systems:_An_Introduction_for_Scientists_and_Engineers>
· <https://lavalle.pl/planning/book.pdf> ·
<https://sites.engineering.ucsb.edu/~jbraw/mpc/MPC-book-2nd-edition-1st-printing.pdf> (verified
through search results)

Autopilot, docking and landing. Açıkmeşe & Ploen's "lossless convexification" turns the thrust-
limited soft-landing problem into a second-order cone programme solved to optimality, the basis of
the guidance behind propulsive landings since; Åström & Murray is the PID and state-feedback text
(free, with Princeton's agreement); LaValle covers kinodynamic planning (RRTs in state space, the
Dubins and Reeds–Shepp cars); Rawlings, Mayne & Diehl is the MPC reference.
*Bearing:* three tiers for Forge's ship control, all deterministic: a PID on attitude and a
proportional-derivative law on position for station keeping and docking approach (Åström & Murray),
a kinodynamic planner through the belt's octree for the approach path (LaValle), and, only if a
landing on a planet ever needs it, a convex descent solved offline per ship class into a lookup.
MPC on the tick is a pointer, not a plan.

**Keen Software House. Space Engineers, update 1.202 "Automatons" (2023): AI Flight, AI Basic, AI
Recorder, AI Offensive and AI Defensive blocks. — Frontier Developments. Elite Dangerous' NPC ship
AI, as reported by Game Developer (2016).** [web] [recent]
<https://www.spaceengineersgame.com/update-1-202-automatons/> ·
<https://www.gamedeveloper.com/production/frontier-inadvertently-drives-i-elite-dangerous-i-ai-to-create-superweapons>
(verified through search results)

Pointers only: Space Engineers exposes ship AI to players as blocks ("AI Offensive, AI Defensive,
AI Recorder, AI Basic, AI Flight"); Elite Dangerous' 2016 incident, in which a networking bug let
NPCs combine weapon modules into "a rail gun with the fire rate of a pulse laser", is the reminder
that NPC loadouts are data the server validates.
*Bearing:* a ship's brain is a component on the construct with a behaviour set chosen by data, so a
freighter, a fighter and an autopiloted shuttle differ by records, not by code.

---

## 6. Ecosystems at planet scale: layers, hand-offs, the tick and its digest

No single source describes this; the pieces are A-Life's offline/online split (§1), Assassin's
Creed Unity's AI LOD (§3), Census's population records (§3), Brom's and Chenney's simulation LOD
(§3), the density-per-cell model of planet-environment.md §3 and netcode.md's "parameters and
events, never poses". Put together:

- **Three layers, by distance and interest.** A *statistical* layer covers the planet: per
  regional climate cell (about 1.2 km, planet-environment.md's table), per species, a density, an
  age structure and a disturbance clock, and per settlement a `Population` record (Census's rows,
  RimWorld's factions), updated in game-day steps by the damped population equations, hunting and
  events. A *regional* layer surrounds each player out to a few kilometres (A-Life's radius, the
  Stalker 2 lesson): agents exist as records with a position on the coarse graph, a schedule
  cursor, needs integrated analytically, moving by straight lines on the cell graph, fighting by
  odds; no navmesh, no senses, a few hundred bytes each. A *full* layer within a few hundred metres
  (AC Unity's 40 and 120): navmesh, senses, utility, ORCA, smart objects, bodies through
  animation.md's tiers.
- **Hand-offs are pure functions.** Spawning from statistics draws individuals for a cell from its
  densities with `Seed::derive(cell_id, species, day)` at the species' need zones (planet-
  environment.md §3), so the same cell on the same day yields the same animals on the server and on
  a client asked to predict scenery; promotion from regional to full instantiates the record's state
  (needs, schedule cursor, health, relations) and nothing else; demotion writes those back and drops
  the rest; folding back into statistics adds the individuals' deaths and births to the cell's
  counts. Chenney's consistency test: a player leaving a herd of twelve and returning a day later
  finds a herd the density equations could have produced from twelve.
- **The tick.** The full layer runs every simulation tick (60 Hz, netcode.md) in a fixed system
  order: perception → decisions (utility, trees) → planning requests → movement (paths, fields,
  ORCA) → smart-object state → events out; the regional layer ticks a slice of its agents per tick
  (round-robin by entity index, so every agent is visited each second); the statistical layer runs
  once a game-day per cell in `Low` jobs, cells in parallel merged by index (D-016).
- **The digest.** Per tick, a hash over the full agents in entity-index order (position quantised
  to the replication's 1 cm, needs quantised, current action id, path cursor, awareness), then the
  regional records in id order, then the cells touched that tick; the 1 000-tick digest at one and
  six workers is the CI test task-system.md already requires, extended to this crate. Anything that
  enters a decision — noise, ties, neighbour order in ORCA, the queue order of the pathfinder — is
  pinned or seeded.
- **Memory per agent.** Full: on the order of a kilobyte (needs, senses' awareness table for the
  nearest few dozen targets, path, blackboard, ORCA neighbours); regional: a few hundred bytes;
  statistical: a few bytes per species per cell. For an island with a village of two hundred, a few
  hundred animals and the players' surroundings, the whole thing is megabytes.
- **What is replicated.** Netcode.md's rule: the record and the events (this agent exists with this
  archetype and this action; it chose this smart object; it fired this bark id; the herd fled), plus
  movement snapshots for the full layer's bodies; never poses, never scores, never paths. The client
  runs the same utility code only to *animate* between snapshots and for pure scenery (birds, fish,
  crowd particles) that the server does not simulate at all, spawned from the same seeds.

---

## 7. Rust and open source

Every repository below was read on GitHub on 2026-09-26 (READMEs; tree and blob pages were not
served). The house rule is that the decision layer, the schedules, the smart objects and the LOD
are Forge's; what is worth binding or porting is the geometry.

| Crate / project | What it is (from its README) | Licence | Verdict for Forge |
|---|---|---|---|
| `recastnavigation` (C++) | "Industry-standard navigation-mesh toolset for games": Recast, Detour, DetourTileCache, DetourCrowd | zlib | the algorithm to implement; the reference to test against |
| `rerecast` / `bevy_rerecast` | "Rust port of Recast, the industry-standard navigation mesh generator used by Unreal, Unity, Godot, and other game engines"; polygon and detail meshes, tiles, watershed/monotone/layer partitions; "bevy 0.19" with "bevy_rerecast 0.5" | MIT/Apache-2.0 | the port to read first and to diff Forge's generator against; usable as-is for a spike |
| `recastnavigation-rs-sys` | "Raw Rust bindings for `recastnavigation`, including Recast, Detour, DetourCrowd, and DetourTileCache"; "detour_large_nav_meshes - enables 64-bit dtPolyRefs" | MIT (bindings) | the fastest way to a working tile cache in a spike; C++ in the server build is the cost |
| `recastnavigation-rs` | "a rust wrapper for recastnavigation pathfinding library with cross-platform deterministic", via "a special fork of recastnavigation recastnavigation-deterministic" for "lock-step networking synchronize"; "recast/detour/detour_crowd are implemented" | MPL-2.0 | evidence that Recast's float paths needed a fork to be deterministic; read the fork's diff before writing Forge's generator |
| `recast-rs` | "Rust bindings for Recast from `recastnavigation`" (generation only) | MIT | superseded by the two above |
| `oxidized_navigation` | "Tiled **Runtime** Nav-mesh generation for 3D worlds in Bevy. Based on Recast's Nav-mesh generation but in Rust"; now "Depricated! See Rerecast" | MIT/Apache-2.0 | history only |
| `polyanya` / `vleue_navigator` | "a any-angle path planning algorithm" on navmeshes, "Multi-layer Navigation Mesh Support"; the navigator builds the mesh live from obstacle entities | MIT/Apache-2.0 | the query algorithm to adopt (port or depend); the live-obstacle mesh is the 2D fallback |
| `landmass` / `bevy_landmass` | "A Rust crate to provide a navigation system for video game characters to walk around levels": "Path finding (e.g., A-star), Path simplification (e.g., SSFA), Steering (e.g., boids), Local collision avoidance." | MIT/Apache-2.0 | the closest Rust analogue of DetourCrowd; read its agent loop |
| `dodgy` | "A Rust crate to compute local collision avoidance (specifically ORCA) for AI characters." (crates.io: "essentially a port of RVO2 to Rust") | MIT/Apache-2.0 | port into `forge-nav` with a pinned neighbour order, or depend and wrap |
| `RVO2` (C++), `Menge`, `OpenSteer` | ORCA's reference; the modular crowd framework; Reynolds' steering library | Apache-2.0 / Apache-2.0 / MIT | references and test scenarios |
| `big-brain` | "a Utility AI library for games, built for the Bevy Game Engine": Scorers "look at the world and evaluate into `Score` values", Actions are "the actual things your entities will _do_", Thinkers and Pickers; "compatible with `bevy@0.16.0`" | Apache-2.0 | the right shape (scorers as entities, actions as state machines) but three Bevy versions behind `bevy_ecs` 0.19 and not deterministic by contract; write Forge's scorer, keep its API in mind |
| `bonsai-bt` | "Rust implementation of Behavior Trees" with Sequence, Select, If, While, WhenAll, WhenAny, Race, After, Invert, Timeout; "the behavior tree is always responsive" | MIT | small enough to depend on for the reactive trees, or to copy; no Bevy coupling |
| `bevy_behave` | "A behaviour tree plugin for bevy with dynamic spawning": an action spawns "a new entity ... along with a `BehaveCtx` component", one global observer; "100k enemies in the chase demo" | MIT/Apache-2.0 | the ECS-native design to study; its entity-per-task model conflicts with a digest-friendly fixed order |
| `pathfinding` | "several pathfinding, flow, and graph algorithms in Rust" (A*, Dijkstra, IDA*, BFS, DFS, Fringe, Edmonds–Karp, Kuhn–Munkres) | MIT/Apache-2.0 | the graph searches for lanes, the cell graph and task assignment (Kuhn–Munkres for slots) |
| Fluid HTN (C#) | "a total-order forward decomposition planner, as described by Troy Humphreys in his GameAIPro article"; "Partial planning", "Replan only when plans complete/fail or when world state change" | MIT | the design to port when the reconstruction planner needs HTN |
| LimboAI, Beehave (Godot) | "an open-source C++ plugin for Godot Engine 4 providing a combination of Behavior Trees and State Machines"; "a powerful addon for Godot Engine that enables you to create robust AI systems using behavior trees" | MIT | editor-side references for the tree authoring tool, later |
| Nav3D (Unreal) | Brewer's SVO for UE5, "Real-time modifications without full rebuilds", Theta* | MIT | the octree layout and its incremental update to port for flight |

What to write and what to use, in one paragraph: write the navmesh generator (Recast's algorithm,
over Forge's own heightfield and modules, deterministic per tile, checked against `rerecast`),
write Polyanya (the crate is a good port to read; owning the query keeps the layer flags and the
determinism), write ORCA (from `dodgy`/RVO2, small), write the scorer, the schedules, the smart
objects, the herd, the senses, the LOD and the digests; depend on `pathfinding` for graph searches
and on `bonsai-bt` (or copy it) for the reactive trees; bind `recastnavigation-rs-sys` only in a
spike that measures the tile cache before Forge's generator exists.

---

## What the shipped games do

| Game | Decision | Movement | Scale trick | What to take |
|---|---|---|---|---|
| Halo 2/3 (Bungie) | behaviour DAG; task tree per encounter | authored navmesh | squads fill prioritised tasks | the reactive tree; the faction task tree |
| F.E.A.R. (Monolith) | GOAP over three states | navmesh, smart objects | small squads | planner scope; UseSmartObject as a state |
| The Sims 1–3 (Maxis) | needs × advertisements; Boltzmann pick; traits | lot-local | one lot simulated, the rest abstract | smart objects; needs; temperature as personality |
| Oblivion/Skyrim (Bethesda) | package stacks by condition | navmesh | packages reevaluated slowly | schedules as the outer loop |
| Killzone 2/3, Horizon (Guerrilla) | HTN under a strategy layer; group agent for herds | navmesh, 3D for fliers | sensors per species | the herd blackboard; per-species senses |
| S.T.A.L.K.E.R. 1/2 (GSC) | A-Life: online agents, offline graph | level graph offline | radius and budget (and their failure in 2024) | the layered simulation and its radius test |
| Left 4 Dead (Valve) | director; fact-matched barks | navmesh | pacing from facts | the bark matcher; the director as data |
| Assassin's Creed Unity (Ubisoft) | full AI ×40, hi-res ×120, crowd ×10 000 | crowd flow | pooled promotion | the LOD budget shape |
| Watch Dogs 2 / Legion (Ubisoft) | emergent crowd AI; Census population database | lanes, navmesh | records for the whole city | the person record; schedule seeds |
| Planet Coaster (Frontier) | guest needs | flow/potential fields | 10 000 guests | flow fields for attractors |
| Supreme Commander 2 (GPG) | RTS orders | flow field tiles | thousands of units | the tile integration |
| Cities: Skylines I/II (Colossal Order) | agents with purposes | lane graph routing | 80 k agent cap → none | lanes as a graph; costs in routing |
| GTA (Rockstar) | scripted ambient | authored path nodes | density by zone | node files as the lane format |
| RimWorld, Dwarf Fortress | utility jobs; storyteller | grid | tens of agents, a world of records | few agents, many records |
| Far Cry 3–5, Primal (Ubisoft) | systemic animal needs and threat | navmesh | food chain from two archetypes | predator/prey as archetypes |
| Red Dead Redemption 2 (Rockstar) | routines, memory of the player | navmesh, roads | unpublished | the acceptance list |
| Elite (1984), FreeSpace 2 | aggression byte and TACTICS; goal stack and AI profiles | 6-DoF steering | a page of rules; a table per class | the ship's action list; difficulty as data |
| Space Engineers | AI blocks on the grid | autopilot | player-authored | the brain as a construct component |
| Gran Turismo 7 (GT Sophy), StarCraft II (AlphaStar) | learned policies | — | accelerators | offline distillation only |

---

## Recommendation for Forge

*Opinion, shaped by the owner's answers of 2026-09-26 (utility and behaviour trees with schedules,
planners for reconstruction only; one culture, one humanoid, one generated quadruped and a few
animals; two players on a listen server first; the space battle as the second demo) and by the
constraints already decided: deterministic on the server (D-016), records in packages (D-035),
cells of 1 km (D-037), the material row's events (D-007), the animation contract of
animation.md §5. Costs are estimates to be replaced by the F1 overlay's and Tracy's numbers.*

**Build order, starting small on the island.**

1. **`forge-sim`'s tick and the digest (Phase 3 item 1, before any AI).** The fixed 60 Hz tick over
   `bevy_ecs` with the Forge executor, systems in a declared order, per-entity seeds
   (`Seed::derive(world, tick, entity)`), the per-tick digest in entity-index order, the CI test at
   one and six workers, and the three LOD tiers as components (`Full`, `Regional`, `Statistical`)
   with promotion and demotion systems that do nothing yet. A day. Everything below is a system in
   this schedule.
2. **The navmesh over the generated terrain, rebuilt where it changes.** Recast's pipeline in
   `forge-nav` over the 2 m heightfield and the village's modules, per 256 m tile (16 tiles per
   D-037 cell), voxel 0.25 m, agent radius per archetype class; the compact heightfield kept per
   tile; tiles invalidated by an SDF edit or a placed module and rebuilt in `Low` jobs; a digest per
   tile; Polyanya for queries; off-mesh links from the modules' data (doors, ledges, the vault).
   Proof: a walkable-area image over the island's hillshade, a crater dug at run time and the tiles
   around it rebuilt in under 5 ms each, the same tile bytes at one and six workers.
3. **The village's NPCs: schedules and smart objects.** Twenty villagers from `Person` records
   (archetype, home, work, relations, seed) with a `Schedule` stack (sleep, work, market, tavern,
   church, wander) evaluated once a game-minute, `Needs` decaying per tick, utility scoring inside
   the package over the `SmartObject`s in reach (beds, benches, the well, stalls, doors, the forge),
   a reservation per object, a reactive tree for interrupts (rain → shelter, a threat → flee), ORCA
   in the lanes; barks as fact-matched ids on the client. Proof: a day watched at 8× with the
   acceptance list of §4's last entry, and zero stuck agents in an hour of soak.
4. **Animals with needs, and a herd.** Two archetypes first (a grazer in a herd, a predator with a
   territory) plus birds and fish as scenery: `Needs` (hunger, thirst, rest, safety), `Senses` per
   species, a `Herd` group agent with Couzin's zones and Horizon's role split, a home range for the
   predator, need zones from the ecosystem bake (water, forage, cover), diurnal schedules; the
   generated quadruped's body plan (creatures.md) decides the gait the movement layer drives. Proof:
   the herd at the river at dawn, the predator's hunt, the herd's flight and regrouping, and a
   population that holds for a hundred game-days in one cell against Wolf Sheep Predation's rules.
5. **The simulation LOD and the regional layer.** Demotion at 300 m (full → regional) and 3 km
   (regional → statistical), promotion on approach and on interest (a quest, a fight), pools so
   promotion never allocates; the regional agents on the cell graph with analytic needs; the
   statistical cells in game-day steps; Chenney's consistency and completeness as tests (leave,
   return, compare with the equations; nothing scheduled is skipped). Proof: the whole island
   populated (a village of 200, a thousand animals) with the full layer under 2 ms of one core's
   tick and the regional layer under 0.5 ms.
6. **The crowd and the traffic (#87, #90).** Flow fields per attractor for the market and the
   festival with Treuille's density term; `Lane` graph from the road polylines; carts (later
   vehicles) as lane agents with car-following and signals; scripted traffic on #79's moving
   geometry first, agents after. Proof: a thousand crowd particles and a hundred carts in the town
   under 1 ms, no visible collisions in the captures.
7. **The reconstruction planner (crafting-building-repair.md).** A GOAP over the repair records
   (clear rubble → fetch material → rebuild module → heal terrain), A* with costs, procedural
   preconditions, run by the villagers' work package; HTN (Humphreys, Fluid HTN's design) only when
   the action list stops composing. Proof: the crash demo's village rebuilt by NPCs over a game-week
   without a script.
8. **The ship AI for #80.** `ShipBrain` as utility over the manoeuvre actions (approach, lag and
   lead pursuit, break, extend, bracket, evade, dock), a `ThreatTable` (distance, closing rate,
   facing, damage dealt, wing orders), thrust-limited steering in 6 DoF through the belt's sparse
   voxel octree with Theta*, a PID attitude controller, `Formation` slots for the wing, a fleet
   layer handing objectives to wings (Guerrilla's layering, Halo 3's task tree), difficulty as a
   data column (FreeSpace). Proof: the ballad's short scenario, the hero wins, the same replay from
   the same inputs bit for bit.
9. **The planet-scale layers (Phase 10).** The statistical layer over the regional climate cells
   of D-034's atlas, the hand-offs of §6 across cells and workers, the populations' persistence as
   records per session (the owner's answer 1), the spawn-visibility test (no animal or person may
   appear inside a player's view frustum within 150 m; the Stalker 2 lesson).

**The data model, as Rust records** (RON in D-035 packages; the names are proposals):

```rust
pub struct AgentArchetype {
    pub id: RecordId,                 // "forge:villager", "forge:deer", "forge:fighter"
    pub body: BodyPlanId,             // creatures.md; decides the locomotion the AI drives
    pub needs: Vec<NeedDef>,          // { need: NeedId, decay_per_hour: f32, curve: CurveId }
    pub senses: Senses,               // { sight: Cone { fov_deg, range_m, light_weight }, hearing_db: f32, smell_m: f32 }
    pub decisions: Vec<Decision>,     // utility: { action: ActionId, considerations: Vec<Consideration>, weight: f32 }
    pub reactive: BehaviourTreeId,    // interrupts (threat, weather, damage)
    pub schedule: Option<ScheduleId>, // villagers; animals use diurnal packages
    pub group: Option<GroupRules>,    // herd/flock zones (repulsion, alignment, attraction), roles
    pub movement: MovementClass,      // Walker { radius, height, step, slope } | Flier | Swimmer | Ship { thrust, torque, mass }
    pub lod: LodBudget,               // full_radius_m, regional_radius_m, memory_bytes
}
pub struct Consideration { pub input: InputId, pub curve: Curve, pub weight: f32 }  // input: need, distance, hour, influence, threat
pub struct Needs { pub values: [f32; N_NEEDS] }                       // 0..1, quantised in the digest
pub struct SmartObject {
    pub id: RecordId, pub cell: CellId, pub pose: Pose,
    pub affordances: Vec<Affordance>,   // { need: NeedId, gain: f32, duration_s: f32, anim: InteractionId, slots: u8 }
    pub reservation: Option<(Entity, Tick)>,
}
pub struct Schedule { pub entries: Vec<ScheduleEntry> }               // ordered stack, topmost satisfied wins
pub struct ScheduleEntry { pub conditions: Vec<Condition>, pub procedure: Procedure } // Travel, Sandbox(area), Use(kind), Sleep, Work(site)
pub struct Person { pub archetype: RecordId, pub home: SmartObjectId, pub work: Option<SmartObjectId>,
                    pub relations: Vec<(PersonId, Relation)>, pub seed: Seed }       // Census's row
pub struct Herd { pub members: Vec<Entity>, pub centre: DVec3, pub heading: Vec3, pub alarm: f32,
                  pub roles: Vec<(Entity, Role)>, pub home_range: Option<HomeRange> }
pub struct NavTile { pub cell: CellId, pub index: (u8, u8), pub polys: Vec<NavPoly>, pub links: Vec<OffMeshLink>,
                     pub compact: CompactHeightfield, pub digest: u64, pub built_from: ContentHash }
pub struct Lane { pub polyline: Vec<DVec3>, pub width: f32, pub speed: f32, pub next: Vec<LaneId>, pub signal: Option<SignalId> }
pub struct EcoCell { pub cell: CellId, pub species: Vec<SpeciesState>, pub disturbance_day: u32 }
pub struct SpeciesState { pub species: SpeciesId, pub density_per_km2: f32, pub age_classes: [f32; 4], pub suitability: f32 }
pub struct ShipBrain { pub archetype: RecordId, pub threats: Vec<Threat>, pub wing: Option<(FormationId, u8)>,
                       pub objective: ObjectiveId, pub skill: SkillRow }
```

**What runs where.** The server runs everything above: the tick, the layers, the navmesh rebuilds,
the digests. The client predicts nothing but movement (its own avatar and, as netcode.md says,
what it touches); for NPCs it interpolates the replicated movement and animates from the replicated
action ids and events (the smart-object interaction, the bark, the herd's alarm); for pure scenery
(birds, fish, crowd particles, distant traffic) it runs the same seeded steering locally from the
same records, with no authority. A listen server for two players runs the same code in the same
process.

**Costs, from the published numbers and to be measured.** Reynolds' PS3 crowd (15 000 boids at
60 fps over a handful of SPUs) puts pure steering at a few thousand agents per core per frame, so
around 100–200 boids per millisecond per core; Planet Coaster's 10 000 guests on flow fields and
City Sample's 35 000 pedestrians are the same order on a desktop with cheaper brains. A *full*
Forge agent (senses over a few dozen candidates, a utility pass of a few dozen considerations, an
ORCA solve over ten neighbours, a path cursor) is estimated at 20–50 µs per tick, so 20–50 agents
per millisecond per core and a few hundred in a 2 ms slice; a *regional* agent at 1–2 µs when
visited, so thousands per millisecond; an *ecosystem cell* at well under a microsecond per species
per game-day. A navmesh tile rebuild is Mononen's 2 ms on a 2011 CPU, a few milliseconds with Forge's
halo on today's. A cross-island path is a three-level search of a few hundred nodes. These are the
numbers the demo pages must replace.

**What to measure, and the digests.** Per tick in the F1 overlay and Tracy: the AI group's zones
(perception, decisions, planning, movement, smart objects, LOD, navmesh rebuilds), agents per tier,
promotions and demotions per second, path requests and cache hits, ORCA neighbours per agent (p50,
p99), tiles rebuilt and their milliseconds, and the stuck detector's count (an agent whose position
has not changed in 10 s while its action says move). Digests: the tick digest (agents, records,
cells), the navmesh tile digest, the ecosystem cell digest per game-day, all compared at one and six
workers over 1 000 ticks in CI, and a replay of the island's day that must reproduce the RL stream
byte for byte (netcode.md). Look tests: the acceptance list of §4's RDR2 entry on the village and
the animals, Helbing's lanes in the market, no visible spawn within 150 m, no herd that oscillates.

**Questions for the owner** (defaults in brackets):

1. The first full-layer budget on the island: how many full agents at once, and at what radius?
   [64 full within 300 m, 512 regional within 3 km, the rest statistical.]
2. Should villagers persist across sessions as records (their relations, their memory of the
   player), or restart from the seed each session? [Persist the `Person` rows and the ecosystem
   cells in the session's save; everything else from the seed.]
3. Do animals need hunting and taming in the first slice, or only presence? [Presence and the
   food chain first; hunting as a faction change on the record when crafting arrives.]
4. Is the reactive tree authored as data from day one (a RON tree per archetype) or as Rust closures
   until an editor exists? [RON from day one; the editor is later and the trees are small.]
5. For #80: does the battle need a fleet layer (objectives, wings) in the first scenario, or a dozen
   independent fighters with a threat table? [A dozen fighters and one wing formation; the fleet
   layer when the scenario grows.]
6. Which learned work, if any, is worth a spike on the owner's GPU: a distilled racing/dogfight
   policy as demo opposition (off the tick), or none? [None until the utility ships are boring.]

---

## What the numbers say

Crowd and agent counts from the sources: Reynolds' PS3 demos ran "15,000 individuals" on a ground
plane and "10,000 to 20,000 agents" in general at 60 fps in 2006; Planet Coaster targeted 10 000
guests on flow fields with 1 512 animations and 92 740 frames; Assassin's Creed Unity ran 40 real
AIs and 120 high-resolution models inside 10 000 visible crowd NPCs on 2014 consoles; Hitman:
Absolution 1 200 characters at 30 fps a generation earlier (animation.md); Cities: Skylines capped
at 65 536 citizens and 16 384 vehicles and its sequel removed the cap; Epic's City Sample simulates
35 000 pedestrians with Mass and ships 22 building kits and 13 vehicles; `bevy_behave`'s author
runs 100 000 tree-driven entities in a demo. Navigation: Mononen's tile cache rebuilds one tile with
temporary obstacles in about 2 ms (2011); Factorio finds one path per group and caches it; Cities:
Skylines II routes on the network with costs where its predecessor used straight-line proximity.
Decision layers: Halo 2's DAG had on the order of fifty behaviours; F.E.A.R.'s state machine three
states; The Sims 3 chose from 80 traits, five per Sim, over a sparse trait × action matrix;
ArenaNet's designers built an NPC's utility package in about seven minutes. Learned agents: GT
Sophy beat four of the best Gran Turismo drivers, AlphaStar reached the top 0.2 % of StarCraft II
players, Lockheed's hierarchy took second in the AlphaDogfight Trials; none of them replays from a
seed. Dialogue: Hades ships 300 000 words and 21 000 voice lines through a priority system. Forge's
own numbers to compare against: the tick at 60 Hz with 30 Hz snapshots and 24 KB/s per client
(netcode.md), the six frame workers and the `Low` background arena (task-system.md), 1 km cells and
the 1.2 km regional climate cells (D-037, D-034).

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A primary Rockstar source on Red Dead Redemption 2's AI** — none exists; Rockstar has published
  only the atmosphere talk (lighting-gi.md). The entry rests on community write-ups and is used only
  as an acceptance list.
- **"Building a Better Centaur" with Kevin Dill** — the brief's attribution; the talk was Dave Mark
  and Mike Lewis (ArenaNet). Dill and Mark's "Improving AI Decision Modeling Through Utility Theory"
  (GDC 2010) was not re-verified today and is not cited.
- **"Beyond Killzone" as GDC 2017** — it is GDC 2018 (Berteling); Beij's "The AI of Horizon Zero
  Dawn" was Game AI North 2017. Corrected in the entry.
- **Planet Coaster's "20 000 guests"** — the sources say a 10 000-guest target; 10 000 is used.
- **A GDC talk on spawning and despawning around players** — searched, not found as a talk of its
  own; the topic is covered by A-Life, AC Unity's recycling and Census, and the spawn-visibility test
  in the recommendation is Forge's own.
- **The Sims' lot-based simulation as a documented technique** — only folklore and the Forbus–Wright
  notes; folded into the Sims entry without a claim.
- **Kenshi and Mount & Blade's world simulation** — store pages, wikis and a Bannerlord dev blog
  only; kept as titles.
- **ARK's creature AI and theHunter's need zones** — the first has no technical source; the second
  is in planet-environment.md §3 and not repeated.
- **Proportional navigation and missile guidance (Zarchan, *Tactical and Strategic Missile
  Guidance*)** — the search tool refused the query; the missile's guidance law is therefore not
  cited and the ship entry uses pursuit behaviours and Isaacs instead. A machine with a full network
  should add it (#99).
- **Unreal's Behavior Trees documentation page** — its URL was not confirmed by search today; the
  StateTree page, the Navigation Invokers page and the City Sample page were, and stand for Epic.
- **Recast's own documentation on off-mesh connections and DetourCrowd** — the repository's
  `Docs/_1_Introduction.md` was readable on GitHub but covers only the build pipeline; the tile-cache
  and avoidance facts come from Mononen's blog posts.
- **`landmass`'s islands, off-mesh links and 3D support** — not in the README extract read; only
  the four features it lists are quoted.
- **`dodgy`'s dimensionality and its RVO2 origin** — the GitHub README extract did not state them;
  the RVO2 origin is crates.io's description as the search engine returned it.
- **Lotka 1920 (PNAS)** — not searched; Volterra 1926 is in planet-environment.md §3 and stands for
  the equations.
- **Learned locomotion (DReCon, SuperTrack)** — animation.md §4; not repeated.
- **The CEDEC 2022 Mass Framework talk (Epic Games Japan)** — seen in the search record as a slide
  deck in Japanese; not read, not cited.
- **The "AI and Games" documentaries** (Horizon, Watch Dogs: Legion, RDR2) — videos, not citation
  grade; the underlying talks are cited where they exist.
- **Hades' and Firewatch's dialogue systems in detail** — pointers only, as the brief asked.

---

## Verification notes

Checked on 2026-09-26 with WebSearch and WebFetch only; no browser pane and no video pages. The
session's egress proxy served `github.com` to WebFetch (repository front pages and, this time, two
blob pages: Recast's `Docs/_1_Introduction.md` and Godot's navigation-mesh tutorial) and refused
every other host tried (gameaipro.com and aiandgames.com to WebFetch; the proxy's log shows
dl.acm.org, arxiv.org, en.wikipedia.org and gdcvault.com answered 403 on CONNECT). One search query
(missile guidance) was refused by the search tool itself. Verification therefore has two grades;
every entry not in the first carries "(verified through search results)" on its URL line.

- **Fetched and read (GitHub READMEs):** recastnavigation/recastnavigation (zlib; the module
  descriptions are verbatim); janhohenheim/rerecast (the first sentence and the Bevy matrix
  verbatim); SlimeYummy/recastnavigation-rs (MPL-2.0; the determinism sentences verbatim);
  andriyDev/recastnavigation-rs-sys and andriyDev/recast-rs (MIT); TheGrimsey/oxidized_navigation
  (the deprecation notice verbatim); vleue/polyanya and vleue/vleue_navigator; andriyDev/landmass
  and andriyDev/dodgy; zkat/big-brain (Apache-2.0; the Bevy 0.16 sentence verbatim);
  Sollimann/bonsai; RJ/bevy_behave (the 100k sentence verbatim); evenfurther/pathfinding;
  ptrefall/fluid-hierarchical-task-network; snape/RVO2 (the abstract sentence and the C++98
  sentence verbatim); MengeCrowdSim/Menge; meshula/OpenSteer; darbycostello/Nav3D;
  limbonaut/limboai; bitbrain/beehave; NetLogo/models; scp-fs2open/fs2open.github.com;
  markmoxon/elite-source-code-bbc-micro-cassette (the three sentences verbatim); and the two blob
  pages named above.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  volume, pages, DOI or ISBN, and the sentences quoted, which are the search engine's extracts of
  the page named): the GDC talks through GDC Vault listings 1021848, 1012450, 1024912, 1015317,
  1024415, 1022141, 1027018, 1024426, 1024597, 1024232, 1022627, 497 and 1022027, with the Internet
  Archive copies (Isla 2005 and 2008, Mark 2015, Cournoyer 2015, Reynolds 2006), Game Developer's
  previews and write-ups, Armstrong's notes and, for Booth and Ruskin, Valve's own PDFs; the Game AI
  Pro chapters (Humphreys, Emerson, Brewer, Jack, the Killzone 3 chapter) through gameaipro.com's
  PDF listings, Taylor & Francis and Semantic Scholar; the Guerrilla material through
  guerrilla-games.com (Berteling's page, Verweij's thesis PDF) and aigamedev.com's coverage and
  interview (Killzone 2, Iassenev); the journal papers through their publishers' pages (APS for
  Helbing & Molnár, ACM DL for Treuille and Chenney & Forsyth, Springer for Brom and Chung, Nature
  for Wurman and Vinyals, MDPI, ScienceDirect for the orbital paper, arXiv for Pope, SIMA and the
  million-agent paper) and an author's or institution's listing where one exists (red3d.com for
  Reynolds, the UNC ORCA page, Arikan's and Chenney's pages, the Alberta PDF for Brewer & Sturtevant,
  the Iowa State and Bergen PDFs and JASSS for ODD, the Bristol and Bath portals for Couzin, the
  Scholar record for Balch & Arkin, ResearchGate for Açıkmeşe & Ploen); the books through Open
  Library and booksellers (Mark 2009, Shaw 1985), JSTOR and De Gruyter (Moorcroft & Lewis), MacTutor,
  INFORMS and Cambridge Core (Isaacs), Princeton's pages and reviews (Grimm & Railsback), fbswiki,
  lavalle.pl and the UCSB PDF (the control texts); the engine documentation through
  dev.epicgames.com's and docs.godotengine.org's listings and godotengine.org's article; Mononen's
  posts through digestingduck's listings; Cui, Harabor & Grastien through the IJCAI PDF the crate
  README links; the games through the pages named in their entries (Wikipedia and ck.uesp.net for
  Bethesda; PC Gamer, GameSpot and Windows Central for Stalker 2; the Steam guide and GamesRadar
  for Cities: Skylines' limits and colossalorder.fi for its sequel; gtamods.com; factorio.com;
  gdconf.com for Hades; the Northwestern course page for Forbus & Wright; the CCL page's record,
  Modeling Commons and the EduTech wiki for Wilensky; elite.bbcelite.com; the Hard Light wiki;
  spaceengineersgame.com; Game Developer for Far Cry, Elite Dangerous and Watch Dogs: Legion;
  RockstarINTEL and Medium for RDR2; Steam, Lo-Fi's FAQ and GameSpace for Kenshi and Bannerlord).
- **Weaker confirmations, stated plainly.** The Halo 2 "order of fifty behaviours" and the Sims 3
  Boltzmann and trait-matrix details are the search engine's summary sentences, whose page of origin
  was not shown; they are paraphrased, not quoted. The Wright "ant model" remark was seen only in a
  secondary essay's extract and is not quoted. The Elite TACTICS description beyond the one quoted
  sentence is from memory of the disassembly's structure, marked as paraphrase. The FreeSpace goal
  list is from memory of `aicode.cpp`, not re-read. Brom 2007's page range and LNCS volume are from
  the Springer listing. Balch & Arkin's page range and DOI are from a Google Scholar record. The
  Açıkmeşe & Ploen entry links a ResearchGate listing because the AIAA page did not appear in the
  record. The RDR2 entry is press and fan material by construction. The orbital pursuit–evasion
  paper, the MDPI ecosystem paper and the million-agent arXiv paper are cited by title because the
  record showed no author lists; Verweij's university is inferred from the PDF's file name; the
  Åström & Murray and Rawlings editions are named without years for the same reason. The "Improving Local
  Avoidance" post's content (sampling versus ORCA) is remembered, not extracted. The Godot
  NavigationServer's use of RVO2 is the search engine's summary of the docs, not a sentence of them.
- **Forge's own numbers** (60 Hz, 30 Hz snapshots, 24 KB/s, six workers, the `Low` arena, 1 km
  cells, 1.2 km regional climate cells, the crate list) are from `docs/research/netcode.md`,
  `docs/research/task-system.md`, `docs/DECISIONS.md` (D-034, D-037) and `docs/ARCHITECTURE.md` as
  of 2026-09-26.
- **Numbers to re-check before they enter a spec:** every figure in the recommendation's cost
  paragraph (microseconds per agent per tier, milliseconds per tile, the 64/512 budget) is an
  estimate, marked as such, to be replaced by the overlay's numbers; Reynolds' 2006 figures are on
  the PS3's SPUs; Mononen's 2 ms is 2011 hardware; AC Unity's 40/120/10 000 are PS4-era; the City
  Sample's 35 000 comes from press coverage of the demo, not from Epic's page; Cities: Skylines'
  limits are community-documented engine constants.
