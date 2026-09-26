# Research — The programme: from the island to a universe, what is known and what to build first

> Written 2026-09-26 on the owner's brief: procedural decor, objects, materials, textures and
> architecture of different styles (visitable buildings for different civilisations and
> cultures, by culture, weather, biome and planet, from primitive to medieval to current to
> futuristic); creatures with their animation; biomes, planets, solar systems, galaxies;
> spaceships, stations and ground bases; realistic clouds, atmospheres, oceans, beaches and
> water interactions, rivers and torrents; destructible architecture and terrain deformation
> (an explosion, a ship crashing on the ground, on a river, in the ocean); a crafting system
> that builds tools, buildings and cities and levels or repairs terrain (a ship crashed on a
> city, the rubble removed and the ground healed by players or NPCs); animation from nature
> reacting to the weather to complex animal movement; a sound system that follows the medium
> (none in space), with proximity chat for players and NPCs and group chat; game AI for NPCs,
> creatures, animals, ships and ecosystems; all in sync for 1 to 16 players, and the research
> on how many players one unified world can hold. "Start small using good foundations for the
> engine and build upon that step by step. The ideas in GitHub issues will help us prioritise."

This file is the map, not the research: it says, for every capability the brief names, what
the existing research files already establish (their verdicts are in `docs/RESEARCH.md`),
which new file fills the gap and what it recommends, which engine foundation it rests on,
where it sits in `docs/ROADMAP.md`'s phases and which issues carry it, and it ends with the
order of work, the consistency issues the brief raises and the questions the owner should
answer before the code. Everything below is CPU-side research written in the cloud without a
GPU; the sources' verification grades are in each file (#99).

## 1. The map

| The brief asks for | Already researched (file, section) | The gap file (this programme) | Rests on | Phase | Issues |
|---|---|---|---|---|---|
| Buildings, decor, materials and textures per civilisation, culture, climate, biome, planet, era | city-generation.md (roads, lots, grammar-driven kit assemblies, D-039 🟡); vegetation-materials.md §4–§5 (trim sheets, materials without an art team); procedural.md §2, §4, §5 (grammars, settlements, texturing by maths); planet-environment.md (the climate atlas a civilisation answers to) | **civilisations-styles.md** | D-035 packages, D-039's style sets and module grid, the cluster DAG cook, the unified material row (D-026) | 8, 10 | #83, #85, #86, #87, #88, #91 |
| Creatures, humans, animals: generated bodies and their animation | animation.md §3–§5 and its verdict (generated creatures after Spore, IK, powered ragdolls, foot events) | **creatures.md** | `forge-anim` (Phase 7), Jolt's ragdolls (D-009), the material layer's contacts | 7 | none yet |
| Biomes, planets, solar systems, galaxies | planet-environment.md (climate, biomes, ecosystems, weather); planet-terrain.md (the planet from orbit to the ground); large-worlds.md and D-037 (frames, sectors, cells) | **universe-generation.md** | `forge-world`'s frame tree (sectors of 2⁴⁰ m, systems, bodies), D-016 determinism | 2 | #93 |
| Spaceships, stations, ground bases | gpu-geometry.md (the cluster DAG any hull goes through); city-generation.md (kits and grammars); physics-fluids.md §3 (vehicles) | **spacecraft-structures.md** | D-039's kits and grammar, one physics space per construct (D-009), the instance cells (#38) | 3, 5–7 of the ballad | #80, #24, #12 |
| Clouds, atmospheres | lighting-gi.md §6 (sky, atmosphere, clouds, fog, night); planet-environment.md §4 (weather rendering, Nubis weather maps); D-023 (Hillaire's tables, the march from space) | none needed now: the research is done, the build is Phase 4 item 3 | the froxel volume (D-032), the weather state (D-034) | 4 | none yet |
| Oceans, beaches, water interactions, rivers, torrents | water.md (spectra, cascades, shores, rivers, lakes, the genesis hand-off; D-038 🟡); physics-fluids.md §5 (the far/mid/near water tiers, buoyancy, shallow water) | none needed now; the interaction tier (wakes, splashes, whitewater, buoyancy) is physics-fluids.md's near tier and Phase 3 item 4 | the coast distance, the river polylines and the lake levels the genesis bakes | 2–3 | #96, #97 |
| Destructible architecture, terrain deformation, crashes on ground, river or sea | physics-fluids.md §4 (destruction: pre-fracture, Chaos, Teardown), §5 (fluids); large-worlds.md (SDF bricks for edits) | **destruction-deformation.md** | Jolt (D-009), the SDF edit layer, the drainage recompute (`forge_procgen::flow::drain`), the replication of world edits | 3 | #89, #12, #24 |
| Crafting, building, levelling and repairing terrain, removing rubble, NPC reconstruction | data-driven.md (records, packages, load order); procedural.md §6 | **crafting-building-repair.md** | world edits as a package layer over the deterministic generator (D-035 + D-016), the module grid (D-039), the SDF edit layer | 3, 10 | #82, #84 |
| Nature reacting to the weather; complex animal movement | vegetation-materials.md §1–§2 (wind, deformation); animation.md §3–§4; planet-environment.md §5 (the weather fields) | creatures.md covers the animals; the wind field is Phase 8 item 1 | the shared wind field, the foot events | 7, 8 | none yet |
| Sound that follows the medium, none in space; proximity chat for NPCs and players; group chat | audio.md §2 (spatialisation, propagation, the acoustic LOD), §6 (multiplayer voice chat); netcode.md §8 (voice) | **voice-chat-media.md** (short: the media conventions, NPC proximity speech, channels) | `forge-audio` (Phase 6), `forge-net`'s datagrams (D-010) | 5, 6 | none yet |
| Game AI for NPCs, creatures, animals, ships; ecosystems | planet-environment.md §3 (ecosystems: flora, succession, fauna); procedural.md §4 (ecosystems as rule systems); animation.md §5 | **game-ai-ecosystems.md** | `forge-sim` (Phase 3 item 1: the fixed tick, simulation LOD, digests), navigation over generated and deformable terrain | 3, 10 | #90, #84, #87 |
| 1 to 16 players in sync; how many players one unified world can hold | netcode.md (transport, replication, prediction, determinism, §6 server architecture for a living world, the replication layer, Star Citizen and SpatialOS) | **many-players.md** (short: the two tiers and the 2026 state of single-shard scaling) | D-010, D-016, the frame tree, cell interest | 5 | none yet |

The gap files are written one at a time, in the order of what the two demos need (the
owner's answer of 2026-09-26: the demos drive the priorities): civilisations and styles (done),
game AI and ecosystems, spacecraft and structures (the battle), creatures (the island's
animals and its fantasy), destruction, deformation and fluids (both demos), crafting and
repair, the universe, then the short primer on voice and media; the question of many players
is folded into §5 below rather than a file, since the hardware and the player count (two
now, four to sixteen later) put it years out. Each file follows the house format and ends
with a "Recommendation for Forge" and a build order that starts small.

**The two demos that set the priorities.** (a) *The island* (#81): a fantasy world at a
medieval level of technology, a tropical island first with architecture that fits the place,
other islands with other climates visitable later, still medieval; so the first style set is
a **tropical medieval** one and the first axis to prove is **climate at a fixed era**, not the
era axis (D-039's "Mediterranean village first" becomes "tropical medieval village first";
the downtown set waits for city-blocks). (b) *The battle* (#80): spaceships fighting in an
asteroid belt, perhaps with the Earth in the background; so ship generation and damage,
asteroid destruction, ship AI, the planet seen from space and the sound convention in vacuum
come before the universe's systems and galaxies.

## 2. What is already settled, and what each new file adds

**Civilisations, styles, decor, materials.** Settled: streets and lots are solved (Parish &
Müller, tensor fields, Vanegas 2012); buildings are kit assemblies decided by a split grammar
(D-039 🟡); materials are trim sheets and tileables per region with one unified row; the
climate atlas exists to answer to. *civilisations-styles.md adds:* the climate response is an algorithm, not a mood: the Mahoney tables
(Koenigsberger 1974) and Givoni's chart take the monthly temperature and humidity D-034's
atlas already stores per cell and give the opening share, the wall mass, the roof and the
layout, about a hundred lines that the `Civilisation` record overrides culture-first
(Rapoport); every published style grammar (Palladio, Wright's Prairie houses, Flemming's Queen
Anne, Duarte's Malagueira, Li's Yingzao Fashi, Knight's tatami grid, Kaplan's star patterns,
Havemann's Gothic tracery, CGA on Pompeii and the Puuc Maya) is a small rule set plus
attributes, and each contributes one column of D-039's control grammar (plan rule, grid,
bands, openings, roof, articulation, ornament, detail, era), so a culture and an era are data
over one split grammar, parametricism being the one deliberate grid-breaker handled as
landmark overrides and the 1.8 m tatami bay the test that a style set may declare its own
bay; props and rooms come from Infinigen Indoors (BSD-3: 79 procedural object generators and
a constraint solver for arrangement) with ShapeAssembly's cuboid programs as the
representation and a culture parameter block per generator, so the decor layer needs no new
research; materials per culture need two operators beyond the noise canon (a recursive stamp
for every masonry bond, phasor noise for weave, thatch and corrugation) and ageing as a
process (Dorsey's flow and patina at module cook, γ-ton exposure on D-029's TLAS per
instance, a wear scalar per instance row); a style set is 100–150 MiB of textures and as much
of cluster pages, 10–20 s to cook 300 modules; no shipped game generates the set (Age of
Empires II hand-made eleven, Civilization VI culture groups by era, No Man's Sky six
archetypes), so sameness is the risk and asymmetry rules, landmarks per culture, authored
overrides, wear and a repetition metric are the antidote; the build order proves the era
axis (a primitive and a futuristic set) before widening the culture axis.

**Creatures and their animation.** Settled: motion matching needs capture, so the honest
humanoid is a blend space with inertialization and IK; contact is kinematic first, powered
ragdolls track the pose; generated creatures follow Spore (author against chain roles, derive
the gait from the body plan, IK onto the ground); parameters and events are replicated, never
poses; the IK layer emits foot events for the deformation and the sound. *creatures.md adds:*
CREATURES_ADDS

**Biomes, planets, systems, galaxies.** Settled: a game climate is twelve months of a few
fields baked from latitude, altitude and wind-carried moisture, biomes by plant tolerances,
ecotones soft or sharp by feedback (D-034); the planet is D-037's cube sphere with tiles as
cluster-DAG props streamed through the page pool (planet-terrain.md); the frame tree holds
sectors of 2⁴⁰ m, systems, bodies and constructs (`forge-world`). *universe-generation.md
adds:* UNIVERSE_ADDS

**Spacecraft, stations, bases.** Settled: any hull is clusters in pages through the same cook;
a construct has its own physics space (D-009) and its own frame; kits and grammars build
buildings, and D-039's module grid was chosen to hold from a hut to a station corridor.
*spacecraft-structures.md adds:* SPACECRAFT_ADDS

**Clouds and atmospheres.** Settled and not re-researched: Hillaire 2020's tables and the
per-pixel march from space are built (D-023); Nubis-style clouds with weather maps, froxel fog
(the first volume is built, D-032) and the weather director are Phase 4 item 3; the numbers to
beat are the belt's dust volume's. Other planets' atmospheres are Hillaire's model with other
compositions and the climate atlas's pressure and humidity as inputs.

**Oceans, beaches, rivers, torrents.** Settled: water.md's plan (D-038 🟡): FFT cascades on
the compute queue and a forward surface pass, the shore from the coast distance the genesis
already bakes, rivers as ribbons over the traced polylines with flow maps, lakes at their
levels; the near tier (buoyancy, splashes, wakes, whitewater on the torrents) is
physics-fluids.md §5's shallow-water field and particles, Phase 3 item 4. Torrents are the
steep reaches of the river network (a waterfall split where the slope breaks, Emilien 2015)
with the same ribbon and a foam rule.

**Destruction, deformation, crashes.** Settled: pre-fractured kit modules with Jolt's
constraints, Chaos and Teardown as the two published shapes (physics-fluids.md §4); terrain
edits as SDF bricks near the camera (large-worlds.md); the drainage is now a linear re-run
(`flow::drain`, 0.27 s for the whole 4 m island, milliseconds for a tile).
*destruction-deformation.md adds:* DESTRUCTION_ADDS

**Crafting, building, repair.** Settled: content is records in packages with a load order
and a conflict rule (D-035), the module grid is the building unit (D-039), the generator is
deterministic (D-016) so the baseline is always known. *crafting-building-repair.md adds:*
CRAFTING_ADDS

**Sound, media, chat.** Settled: the data layer (events, buses, RTPCs, states, HDR loudness,
virtual voices) is Forge's to write; Steam Audio is the spatialiser; a three-tier acoustic
LOD; impacts from modal banks per material; rain and wind from the weather fields; voice over
the netcode's datagrams with Opus (audio.md §6, netcode.md §8). *voice-chat-media.md adds:*
VOICE_ADDS

**Game AI and ecosystems.** Settled: ecosystems as rule systems over the climate atlas
(succession, fauna by tolerance, planet-environment.md §3); the simulation runs on a fixed
tick with simulation LOD and digests (`forge-sim`); animation reacts to the world through
events. *game-ai-ecosystems.md adds:* the shape the owner chose (utility over needs inside schedule
packages, a small reactive tree, smart objects that advertise, reserve and carry the
animation, a planner for reconstruction only) is what The Sims, ArenaNet (Mark & Lewis, GDC
2015), Bethesda's Creation Kit and F.E.A.R. published, with Evans' Sims 3 Boltzmann pick and
its temperature as the personality knob; navigation on generated and deformable terrain is
Recast's voxel pipeline per tile with the tile cache (about 2 ms a tile rebuild) plus
Polyanya's any-angle query, and the lesson that `recastnavigation-rs` needed a deterministic
fork of Recast for lock-step play, so Forge's generator pins its float paths from day one
(`rerecast` is the port to diff against, `big-brain` is not deterministic by contract, so
the scorer is Forge's); every living world that works is an AI level of detail with numbers
(Assassin's Creed Unity's 40 real AIs and 120 high-resolution bodies in a crowd of 10 000,
Stalker's offline graph, Watch Dogs: Legion's population as database rows) and Stalker 2 is
the documented failure (a cut offline radius made spawns visible), which gives the planet
three layers (statistics per climate cell in game-day steps, regional records on the cell
graph to a few kilometres, full agents within a few hundred metres), hand-offs as pure
functions of the seed and a "no spawn within 150 m of a view frustum" test; animals are the
same machinery with per-species senses, a herd as a group agent (Horizon's roles, Couzin's
zones as species data), home ranges for predators, populations as damped densities
calibrated against NetLogo's Wolf Sheep model in Grimm's ODD form (D-016's fixed order,
written by ecologists); ship AI has not changed since Elite's TACTICS routine and FreeSpace's
goal stack, its mathematics (Isaacs' pursuit barrier, Balch & Arkin's formations, Açıkmeşe's
convex descent) is deterministic while the learned agents (GT Sophy, AlphaDogfight) stay off
the server tick; budgets from the sources (Reynolds' 15 000 boids at 60 fps on a PS3, the
City Sample's 35 000 pedestrians, Cities: Skylines' 65 536 citizens) give an estimate of
20–50 full agents per millisecond per core, to be replaced by the overlay's numbers; the
build order: `forge-sim`'s tick and digest, the navmesh rebuilt around a crater, twenty
villagers, two animal archetypes and a herd, the LOD, the crowd and carts, the
reconstruction planner, the ballad's ships, the planet layers last.

**Players in sync, and how many.** Settled: QUIC datagrams and streams, acked-baseline
deltas, cell interest, 60 Hz simulation and 30 Hz snapshots, inputs redundant, determinism
same-binary only, clients talk to a replication layer, workers stateless with a write-behind
database (netcode.md). *many-players.md adds:* PLAYERS_ADDS

## 3. The foundations everything rests on (already decided or proposed)

- **Determinism by construction (D-016).** Every generator is a pure function of a seed and
  a parameter record, with digests; the island's field is the same bytes on any machine and
  with any thread count (the 4 m island's digest is `9eacfe0f827fa7dd`). A universe that
  clients and the server generate independently needs this at every scale: galaxy, system,
  planet, tile, building, creature. Nothing in the brief is possible without it.
- **Content as packages with a load order (D-035).** Civilisations, style sets, kits,
  recipes, creature archetypes and world edits are records in packages merged by add,
  replace and patch. A "repair" is a record that cancels a "damage" record; both are layers
  over the generator's baseline, never edits of it.
- **The frame tree and the cells (D-004, D-037, `forge-world`).** Sectors of 2⁴⁰ m, systems,
  bodies, constructs; `f64` frames and camera-relative `f32` rendering; 1 km cells on the
  GPU record. Galaxies and ships live in the same tree.
- **The cluster DAG and the instance table (D-025, #38).** Every mesh, from a hut's wall
  module to a station's hull to a creature's body, is clusters in pages, instanced by the
  million; kits, not unique geometry (D-039).
- **The unified material row (D-026, D-028).** Render layers, physics, sound, tags, weather
  overrides and a deform block in one row: a civilisation's palette and a crash's debris
  are rows, and the sound and the footprints follow.
- **The environment state (D-034, D-019).** A baked climate atlas, weather as a function of
  seed and time, wetness and snow in a clipmap. Cultures answer to the atlas; the wind field
  drives the trees, the sound and the rain.
- **The job system and the simulation tick (`forge-task`, Phase 3's `forge-sim`).** Fixed
  tick, simulation LOD, digests; the AI, the ecosystems and the physics run there.
- **The server as the authority (netcode.md, D-010).** Clients predict, the server decides;
  world edits, damage and repairs are replicated as records, not as geometry.

## 4. The order of work: the engine's foundations first, in any case

The owner's rule (2026-09-26): **the game engine's foundations come first, in any case.** The
roadmap's phases are that order (`forge-sim`, `forge-physics`, `forge-net`, `forge-audio`,
`forge-anim`, the vegetation ladder, memory and streaming); the brief adds content axes
(cultures, eras, planets, species, ships) that multiply whatever the foundations can carry.
So the research below is not a list of work items: it is what each foundation must be shaped
to carry when its phase comes (records for world edits in the data model, a medium per
listener in the audio, cells and events in the netcode, a body plan in the animation, a tick
with simulation LOD and digests in the simulation), so that the content axes fit later without
rebuilding. The second rule follows from the first: **no axis grows before one vertical slice
touches every foundation once**, on the island, small.

1. **Phase 2, now.** The island's genesis, its water (D-038 🟡) and its planet variant
   (planet-terrain.md) as planned; then the first culture: a tropical medieval village on the
   island's coast from city-generation.md's layout and one style set (D-039 🟡, its climate
   response from the atlas: stilts, verandas, steep thatched roofs, shade), and the first
   `Civilisation` record that selects it (civilisations-styles.md). One culture, one era, one
   island: the look judged on the GPU before a second of anything; the second island, with
   another climate at the same era, is the first axis to prove.
2. **Phase 3.** `forge-sim`'s tick, Jolt and the material layer as planned; on them the first
   forms of three things the brief asks for: destruction as pre-fractured kit modules and
   debris instances (destruction-deformation.md), terrain deformation as the SDF edit layer
   with a local re-run of the drainage (a crater fills, a river finds its way), and crafting
   as records that place or remove modules and terrain edits (crafting-building-repair.md).
   The crash demo: a hull, one construct with its own physics space, dropped on the village,
   on the river and in the sea, with the debris, the splash and the repair afterwards.
3. **Phase 4.** Clouds and weather rendering as planned (the research is done); nothing new
   from the brief.
4. **Phase 5.** `forge-net` for the co-op tier first (1–16 players, a dedicated or listen
   server, full replication of a cell around each player), with the protocol shaped for the
   second tier from the start (records for every world edit, cell interest, authority
   handoff between workers); proximity voice over the same datagrams (voice-chat-media.md);
   many-players.md's numbers as the ceiling to design against, not to build yet.
5. **Phase 6.** `forge-audio` with the media conventions (air, water, vacuum, the suit),
   the NPC proximity speech and the channels.
6. **Phase 7.** The first generated creature (a quadruped from a body plan with a
   synthesised gait, creatures.md) and the humanoid rig with a parametric body; the animals'
   locomotion driven by game-ai-ecosystems.md's utility layer.
7. **Phase 8.** The wind field the trees, the sound and the rain share; the second and third
   style sets (a primitive and a futuristic one) to prove the era axis on the same grammar
   and module grid, with their trim sheets and procedural materials.
8. **Phase 10 and after.** The universe around the island (universe-generation.md: the
   system, then the galaxy, as frames of the tree with deterministic ids), spacecraft and
   stations as constructs from the same kits and grammars (spacecraft-structures.md), the
   ecosystems' simulation LOD across a planet, the second player tier.

**The vertical slice to aim at**, as one demo that grows (`island`): a village of one culture
on the island, one creature and a few animals living there, a ship that crashes on it (once on
the ground, once in the river, once in the sea), an NPC and a player who clear the rubble and
heal the ground, two players in the same instance talking by proximity, the sound following
the medium, at 120 fps with TAA. Every system appears once and small; the numbers of every
step go in its demo page. Only then: a second culture, a second era, a second planet, a
second species, sixteen players.

## 5. Consistency issues the brief raises, and the questions for the owner

**Issues to settle before the code.**

- **One shard or shared seeds.** "A single unified game world / universe" for as many players
  as possible is a persistent shard with a database, a replication layer and server meshing
  (netcode.md §6: Star Citizen took five years, SpatialOS is the warning); "1 to 16 players in
  sync" is a co-op session on one server. The engine can be built for the first while shipping
  the second, if world edits are records and interest is per cell from day one, but the
  persistence model (who owns a crater on a planet nobody is visiting) is a design decision,
  not an engine one. No Man's Sky's answer is a shared deterministic universe with per-session
  state and a thin persistence of player edits; that is the cheapest consistent choice.
- **Determinism against the GPU.** Everything that decides is CPU and deterministic (the
  generators, the physics on the server, the AI); everything on the GPU is visual (the sea's
  FFT, the foam, the clouds). A crash that reroutes a river is a server decision replayed as
  records, never a GPU simulation trusted by two machines.
- **Eras multiply content.** From huts to habitats is one grammar with era parameters
  (materials, module kits, motifs, floor heights) or it is six engines. The proposed D-039
  module grid holds from a stone hut to a station corridor; it breaks for tents, organic
  forms and megastructures, which need their own generators (civilisations-styles.md says
  which). The kits are generated at cook time; there is no art team, so "same generator,
  different palette" sameness is the risk and wear, landmarks and authored overrides are the
  antidote.
- **Destruction must be reversible and cheap to persist.** A destroyed module is a record; a
  crater is an SDF edit record; the generator knows the baseline, so a repair is the removal
  of records plus a local re-run of the drainage, and "an NPC repairs the terrain" is a
  planner emitting those records over time. Physical debris is visual after it settles.
- **How physical a crash is.** A hull hitting the sea at 200 m/s is a splash and a buoyant
  wreck in the near water tier, not a fluid simulation; a hull hitting a river is debris in
  the SDF layer and the drainage re-run; the visual part (particles, the plume) is separate
  from the decision part (where the debris lands, what breaks). The brief's "good physics"
  is Jolt's rigid bodies plus these two hand-offs, and the research files say so; a full
  coupled simulation is out of reach in real time and in this team.
- **Creatures without capture.** There is no motion capture: humans get a blend space and IK
  first (animation.md), creatures get synthesised gaits from body plans; the humanoid's
  clothing per culture is a content axis as large as the buildings' and should follow the
  same kit-and-grammar route.
- **Sound in space is a convention.** Silence, the suit's own sounds, muffled hull sounds,
  or the cinematic lie: the owner picks the feel; the engine needs only the medium per
  listener (air, water, vacuum, inside a hull) and a propagation rule per medium.
- **Voice chat is a product decision as well as a feature.** Own transport over QUIC with
  Opus (netcode.md §8, audio.md §6) is cheap; moderation, recording and privacy are not
  engine questions but they decide whether proximity chat ships.

**The questions, and the owner's answers of 2026-09-26** (everything below stays research
and implementation propositions, no code yet):

1. *One shard or shared seeds?* Both are possible; starting small it is per-session state
   when players start a new game; EVE Online's and No Man's Sky's approaches are interesting;
   the hardware is very limited for now. → The engine keeps the records-and-cells shape that
   allows the second tier; nothing of the shard is built.
2. *Which eras first?* The island game is a fantasy world at a medieval level of technology, a
   tropical island first with architecture fitting the place, other islands with other
   climates later, still medieval; the other demo is the space battle in the belt, perhaps
   with the Earth behind. → The first style set is tropical medieval; the first axis is
   climate at a fixed era; the battle's ships and damage come before the universe.
3. *How many cultures and species?* One culture, one humanoid, one generated quadruped and a
   few animals. → As proposed.
4. *Players?* Two at most for now; four to sixteen depending on the demo later. → The co-op
   tier only; a listen server is enough for two.
5. *Voice?* The owner will research the technologies; the game may not use Steam for a while.
   → voice-chat-media.md is written as a primer on the options (own transport with Opus over
   the netcode's datagrams, WebRTC, Mumble, the providers), not a recommendation.
6. *Sound in space?* The suit and the hull, muffled, no exterior sound. → Settled.
7. *How physical the fluids?* Something that resembles reality depending on gravity and the
   environment, real time within a budget, never all of the hardware; games that do not need
   it use static decor. → destruction-deformation.md covers the tiers (static, analytic,
   shallow water, particles) with gravity as a parameter (waves disperse as `ω² = g k`, the
   splash and the run-up scale with `g`), and the records-and-re-run answer for rivers.
8. *NPC AI?* Utility and behaviour trees with schedules, planners for reconstruction only. →
   Settled.
9. *Breadth before depth?* Not for now. → The vertical slice first.
