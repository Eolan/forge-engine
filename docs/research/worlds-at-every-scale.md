# Research — Worlds at every scale: planets, galaxies and detailed maps

> Companion to `planet-terrain.md` (the cube sphere from orbit to the ground, D-056's source),
> `terrain-genesis.md` (uplift, stream-power erosion, hydrology; the Far Cry 5, Horizon and
> Tsushima entries), `large-worlds.md` (coordinates, partitioning, the voxel entries of §8),
> `procedural.md`, `vegetation-materials.md` (PCG, Nanite Foliage) and `planet-environment.md`
> (climate, biomes, clouds). Written 2026-10-09/10 for the owner's question of 2026-10-09, against
> D-014, D-037, D-053, D-055 and D-056 as they stand and `docs/ROADMAP.md` Phase 9's open item
> ("how to generate whole worlds, planets included, so that their chunks can be made or loaded
> alone"). Sources were checked with WebFetch and WebSearch only, no browser pane; each entry's
> grade (page read, or search record only) is under [Verification notes](#verification-notes).

The question is how today's games and the newest public techniques make many places, and whole
planets, with varied ground, flora, fauna and materials, deformable where the game needs it, and
small on disk; and what Forge should take for three scales: one planet and its moon now (closer to
Star Citizen), a galaxy later (No Man's Sky, Elite), and detailed maps without a galaxy (The Witcher
4's Unreal demo, Battlefield 6, Zelda, Enshrouded). The short answer: every shipped system splits
the world into a small coarse description and a deterministic run-time detail layer; they differ in
where the coarse description comes from (a seed, physical rules, or an artist's brushes). Forge's
D-014 already has that split, but the planet lacks the coarse level and the island stores its fine
level instead of regenerating it. The recommendation adds a per-face *planet atlas* of height,
climate and biome fields, stamped *places*, and tiles that are cache, never package.

> **Since this was written (2026-10-10):** the `planet` demo's Earth and Moon take their coarse
> height from real maps (NOAA's ETOPO 2022, NASA's CGI Moon Kit; `docs/demos/planet.md`), the
> owner's go of 2026-10-09: the height of §A's atlas, without its climate and biome fields yet.

> **State of the art in five sentences.** At galactic scale nothing below a body is stored: No
> Man's Sky addresses 2⁶⁴ planets from seeds in a 15 GB install, and Elite's Stellar Forge about
> 400 billion systems through 64-bit ids in an eight-layer sector octree, in 25 GB. Shipped planets
> pair coarse global data with run-time detail and differ in who makes the coarse data: artists
> with brushes "from a few meters in size to hundreds of kilometers in size" (Star Citizen; Space
> Engineers' 2048² faces), physical rules (Elite's Odyssey; Star Citizen's Planet Tech v5, still
> "Tentative" on the 2026-08-12 roadmap), or nobody (No Man's Sky's voxel noise around the player,
> meshed with dual marching cubes since 2024). Flora, rocks and points of interest are placed at
> run time by rules reading those maps, on the GPU in recent engines (Horizon, Tsushima, No Man's
> Sky 5.0, Unreal's PCG), and hand-made places are stamped in with a flattening or a blend band.
> Large detailed maps stay authored and baked offline, and the new levers are in rendering:
> Nanite Foliage's voxel far representation (the Witcher 4 demo's largest tree from 3.5 GB to
> about 29 MB) and fully editable voxel worlds lit by their own SDF rays (Enshrouded). Research
> goes beyond the games in geology — tectonic planets, erosion-consistent amplification, diffusion
> conditioned on a global river network on a quad sphere (Eurographics 2026) — while the games
> still pay in geology (no drainage, repeated shapes), and no learned generator is bit-exact
> across GPUs.

**Contents**

1. [No Man's Sky](#1-no-mans-sky)
2. [Elite Dangerous](#2-elite-dangerous)
3. [Star Citizen](#3-star-citizen)
4. [Other planets and voxel worlds](#4-other-planets-and-voxel-worlds)
5. [Large detailed maps without a galaxy](#5-large-detailed-maps-without-a-galaxy)
6. [Research and repositories](#6-research-and-repositories)
7. [Deformable terrain at scale](#7-deformable-terrain-at-scale)
8. [What professional engines do](#what-professional-engines-do)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [What to measure](#what-to-measure)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

---

## 1. No Man's Sky

**Hello Games. Sean Murray, PlayStation Blog (2014) on the 64-bit seed; Innes McKendrick,
"Continuous World Generation in 'No Man's Sky'", GDC 2017; Sean Murray, "Building Worlds Using
Math(s)", GDC 2017.** [talk] [web] [still-current]
<https://www.gdcvault.com/play/1024265/Continuous_World_Generation_in__No_Man_s_Sky_> ·
<https://gdcvault.com/play/1024514/Building-Worlds-Using> ·
<https://www.gematsu.com/2014/08/mans-sky-18-quintillion-planets>

A seed hierarchy: 2⁶⁴ planets because the generator is built on a 64-bit seed, each planet's seed
driving everything on it; guides nest galaxies (255 or 256), regions, systems, planets and moons.
McKendrick's abstract gives the pipeline "from voxel-based world generation, through
polygonization and texturing, to eventual population and simulation", run continuously in real
time; Murray's is about terrain made without artistic input and tested by a small team. The talks
themselves could not be read.
*Bearing:* the hierarchy is Forge's `Seed::derive` with a galaxy on top, and the pipeline is a
chain of producers per region, the shape of `planet-terrain.md`'s tile producers.

**Hello Games. "Worlds Part I" (update 5.0, July 2024) and "Worlds Part II" (update 5.50,
29 January 2025), patch notes and announcement.** [web] [recent]
<https://www.nomanssky.com/worlds-part-i-update/> ·
<https://www.nomanssky.com/2025/01/no-mans-sky-worlds-part-ii> ·
<https://www.nomanssky.com/worlds-part-ii-update/>

5.0's engine notes say terrain generation was rewritten to use "dual marching cubes voxel meshing",
with fewer vertices, faster generation and less memory; trees, rocks and grass moved to a GPU-based
renderer; shadows moved to screen-space techniques; water became mesh-based; "Underlying terrain
shapes have not been reset". 5.50 is a new terrain generator for new star systems only: "I've been
working on a new terrain system for a little while now" (Murray), the algorithm "evolved and
refined to generate more diverse planetary shapes" with reduced repeating patterns, "New tech
allows oceans that can be several kilometers deep", and gas giants "ten times bigger than our
biggest planet". Existing systems were not regenerated; the new terrain lives in a new star class
("purple" systems). The post ties the technology to Light No Fire, announced in December 2023 as
one procedurally generated planet of Earth's size.
*Bearing:* a shipped generator can change only by *adding* address space, because players' bases
sit on the old shapes, so Forge's generator version belongs in every key (D-053 does this). Dual
marching cubes is the near-field mesher to test first (§7). And the studio's next step is one
Earth-sized world, D-056's scale.

**No Man's Sky terrain edits as players report them (Steam discussions 2018–2023, a save-editor
guide; the Synthesis update notes).** [web] [still-current]
<https://steamcommunity.com/app/275850/discussions/0/1762482479185377839> ·
<https://gamepretty.com/no-mans-sky-how-to-clear-all-terrain-edits-using-the-save-editor/> ·
<https://pcgamer.com/no-mans-sky-synthesis-update>

Edits are stored in the save as per-save buffers with a global cap (reports range from 15 000 to
30 000 edits), the oldest evicted when full, so dug ground grows back; edits inside a base's
borders are "protected from regeneration" since the Synthesis update; other players' edits enter
one's own buffer, and terrain can differ between players. These are player reports, not Hello
Games documentation.
*Bearing:* the failure mode to design out. An edit must be addressable by cell, cheap to store,
replicated as an operation, and never silently evicted (§7 and the recommendation).

**Game Informer, "A Look At No Man's Sky's Creature Features" (December 2014).** [web]
<https://gameinformer.com/b/features/archive/2014/12/24/a-look-at-no-mans-skys-creature-features>

Art director Grant Duncan describes artist-made "blueprints" from real skeletons, limb lengths,
markings and gait varied by the generator and by climate, rigs shared between similar animals.
Pre-release coverage; the shipped system is not documented.
*Bearing:* life at galactic scale is *parts plus rules*; disk holds the parts (the game is 15 GB
on Steam), not the creatures.

*What NMS pays in geology:* no global pass, so no drainage networks, no basins, and repeated
landforms (the 5.50 notes name the repetition). Players on PC and consoles meet on the same planets,
so generation is deterministic across platforms in practice; how is not published.

---

## 2. Elite Dangerous

**Doc Ross (Lead Render Programmer, Frontier), interviewed by Kirill Tokarev, "Generating The
Universe in Elite: Dangerous", 80.lv, 5 April 2018.** [web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous/>

Stellar Forge runs top-down. Galaxy-scale distributions of mass and age shape the arms and the
bulge; the top-down map of the galaxy is "one of the only non-programmatic resources used in
StellarForge". Each system gets a primary of the right type and age, the leftover material forms a
disc, and the simulation steps through epochs (stellar wind, catastrophic events, tidal locking,
heating). Real stars from the Hipparcos and Gliese catalogues seed the generated galaxy. Sectors
form an eight-layer octree, and one 64-bit integer holds the sector coordinates, the layer, the
system and the body. Landable planets are cube-sphere quadtrees; patches have uniform vertex
spacing so they double as physics meshes; GPU compute shaders spherify the patches and evaluate
noise from the surface point, the planet's id and its astronomical data; inputs reach "tens of
billions of millimeters", so the team wrote 64-bit double and emulated dual-float libraries for
GPUs without fast doubles. Materials are artist textures blended by slope with Wang tiling and
tri-planar mapping, chosen from the planet's physical properties; far patches are flat geometry
with generated colour, normal and height textures.
*Bearing:* the addressing scheme to copy for a galaxy (one `u64` per body, parents implied), the
generation order (parents first, children read their parent's record only), and the one
non-programmatic input (a painted galaxy map). The GPU generation with emulated doubles is what
D-016 forbids for authoritative data; §9 B says how Forge could still use the GPU.

**Frontier, Odyssey planet technology (2021): the livestream with Dr Kay Ross as summarised by
MassivelyOP and forum notes; PC Gamer, "Here's how Frontier rebuilt a galaxy's worth of planets for
Elite Dangerous: Odyssey"; PCGamesN on Horizons' planets (14 August 2017).** [web] [recent]
<https://massivelyop.com/?p=324331> · <https://forums.frontier.co.uk/goto/post?id=8957363> ·
<https://www.pcgamer.com/heres-how-frontier-rebuilt-a-galaxys-worth-of-planets-for-elite-dangerous-odyssey/>
· <https://pcgamesn.com/elite-dangerous/elite-dangerous-shows-us-the-science-and-technology-behind-creating-realistic-planets>

Horizons (2015) had separate generators for rocky and icy surfaces; craters seen from orbit are
real dips once landed. Odyssey (2021) regenerated every landable planet: terrain types and
materials are selected and blended from Stellar Forge's values (crustal stress from gravity, the
crust–magma–core proportions, cratering, tidal locking), a large-scale mask sets terrain zones,
smaller sub-zones follow the flow of the layer above, and polar regions get their own "geomes".
Settlements get a radial flattening; Thargoid and Guardian sites are stamped from authored
resources; most sites are found procedurally on flat enough ground. The Odyssey sources were
reached only as search records and fan notes (the pages refused the fetch).
*Bearing:* the closest shipped analogue to "physical rules → zone masks → materials and shapes",
and the stamp-and-flatten pattern for authored sites. Its price: every planet regenerated once,
and no rivers, which airless bodies hide.

**Elite Dangerous on Steam: "25 GB available space".** [web]
<https://store.steampowered.com/app/359320/Elite_Dangerous/>

*Bearing:* hundreds of billions of systems in 25 GB, because only rules, parts and textures ship.

---

## 3. Star Citizen

**Chris Roberts, interviewed by GamersNexus, "Chris Roberts on Star Citizen's Procedural Planets,
Alpha 3.0, & CitizenCon", 24 September 2016.** [web] [still-current]
<https://gamersnexus.net/gg/2613-chris-roberts-star-citizen-procedural-planets-alpha3-citizencon>

The authored half, in Roberts's words: "There's an overall map for the planet, and there's both a
height map and a distribution map" (where trees and vegetation go); artists "push and pull it,
paint on top for the distribution map" with "brushes that go all the way from a few meters in size
to hundreds of kilometers in size"; biomes are artist-built and "will have all the rules for
whether there's creatures in that biome"; "They can also place specific art, like a mineshaft, or
ruins"; "An artist can crank out five or six moons in a week for you".
*Bearing:* the model the owner named for the `planet` demo: paintable global maps, biomes as
authored rule sets, procedural detail between, hand-made places dropped in. Forge can generate the
global maps from its genesis and keep the brushes as overrides.

**Star Citizen Wiki (fan-run): "Planet Tech v4" (CitizenCon 2019, Alpha 3.8), "Genesis (Star
Engine)" (edited 19 August 2026); RSI's Roadmap Roundup of 12 August 2026; patch notes for the
volumetric clouds (3.14, 3.15.1, 4.1).** [web] [recent]
<https://starcitizen.tools/Planet_Tech_v4> · <https://starcitizen.tools/Genesis_(Star_Engine)> ·
<https://api.star-citizen.wiki/comm-links/21284>

v4 (2019) adds "terrain texture blending, objects scattering and biome transitions", and
"Temperature and Humidity maps infer biome selection". Genesis adds geology, soil type and soil
depth maps that drive texturing and scattering; a physically accurate simulation is the stated goal,
not a result. The 2026-08-12 roadmap lists "Implementing Planet Tech V5, a complete rework of how
planets are built, populated, and rendered", including "moving generation and spawning to the GPU",
marked Tentative under Alpha 4.11, and "Genesis: Starchitect", which "automatically scatters
sectors, clusters, and locations across planetary bodies". Volumetric clouds shipped on Crusader
(3.14), then microTech (3.15.1) and more moons (4.1, March 2025).
*Bearing:* the trajectory is from painted maps toward simulated data maps driving materials and
placement on the GPU, where Forge's genesis already points; v5 is a direction, not a benchmark.

**Cloud Imperium, Letter from the Chairman, 19 December 2024 (Alpha 4.0, Pyro, server meshing);
Star Citizen Wiki, "Object Container Streaming".** [web] [recent]
<https://starcitizen.tools/Comm-Link:Letter_from_the_Chairman_-_2024-12-19> ·
<https://starcitizen.tools/Object_Container_Streaming>

Alpha 4.0 brought Pyro, the second system, with static server meshing: "Each planet, landing zone,
or major station is now covered by different game servers." Object containers are the unit of
streaming on client and server; entities far from players are serialised to a database
(persistent entity streaming). Installs need about 100 GB on an SSD (third-party guides citing
CIG; not read on RSI's site).
*Bearing:* the authoring unit (a hand-made location) is also the streaming and the server unit.
For Forge that is D-037's cells for ground and a *place* record for authored content, each with
its own key in the cache and later its own owner on a server (D-010).

*What Star Citizen pays:* each planet is hand-tuned, so a new system takes years (Pyro came five
years after the v4 planets), and the disk is the largest of the group because locations are
authored at full detail.

---

## 4. Other planets and voxel worlds

**Keen Software House. Space Engineers planets (2015): the modding wiki's "Creating a Planet"; Keen
support threads on voxel files.** [docs] [web] [still-current]
<https://spaceengineers.wiki.gg/wiki/Modding/Tutorials/Creating_a_Planet> ·
<https://support.keenswh.com/spaceengineers/pc/comments/57000/vote/toggle>

A planet is six authored height maps and six material maps (voxel material, foliage, ores): "Our
standard resolution for a big 120km planet is 2048x2048", "a 120 km diameter planet has a ratio of
1 pixel to 20 meters"; a `SurfaceDetail` texture adds small relief by slope. A Keen QA reply blames
client sync problems on a planet's `.vx2` voxel file of 34+ MB after heavy digging.
*Bearing:* the Star Citizen split in a voxel game, with numbers, and edits stored as whole voxel
data per object, growing until they hurt sync.

**Intercept Games. Eric DeFelice, "Developer Insights #12 – Planet Tech" (December 2021); the
Unity terrain team's SIGGRAPH 2021 Advances talk on CBT terrain (Xiaoling Yao).** [web] [talk]
<https://forum.kerbalspaceprogram.com/topic/205930-developer-insights-12-%E2%80%93-planet-tech> ·
<https://www.advances.realtimerendering.com/s2021/Siggraph21%20Terrain%20Tessellation.pdf>

The post (search record) says KSP2 uses "a very similar PQS (procedural quad sphere system)" to
KSP1, with vertex memory the main problem and a dithered cross-fade from the low-LOD planet. The
CBT terrain was Unity's own work in progress (a 2 km demo), not KSP2's.
*Bearing:* corrects `planet-terrain.md` §4: no shipped planet game is known to use a CBT, which
stays a measured spike, not a precedent.

**SpaceEngine (Vladimir Romanyuk), terrain blog posts (2010–2018).** [web]
<https://spaceengine.org/news/blog100824/> · <https://spaceengine.org/news/blog180323/>

Six quadtrees on a cube, nodes of 33×33 vertices and 256² textures; a shader writes elevation from
mixed noises, read back to the CPU for the mesh; the GPU move was reported at 100–200× the CPU.
*Bearing:* two orders of magnitude is what GPU generation buys a galaxy (§9 B.3), and the read-back
for collision is what D-016 then governs.

**Proland and Outerra** are in `planet-terrain.md` §4 and `terrain-genesis.md` (coarse real data
plus per-tile refinement, producers with a cache). **Dual Universe** (Novaquark) advertised voxel
planets with 25 cm precision and a single-shard server (Kickstarter material, 2016); no technical
account of edit storage was found.

**Bethesda, Starfield (2023), from interviews and a player's analysis.** [web] [secondary]
<https://starfieldportal.com/article/starfield-player-explains-tiles-in-world-map>
Cities and quest sites are hand-made; on landing, a bounded area is assembled from a finite set of
authored terrain tiles and generic points of interest chosen by biome.
*Bearing:* the cheap end of "controlled planets": no seamless planet, so no geology. D-056's
orbit-to-ground descent is the opposite choice.

**Keen Games, Enshrouded (2024): talks at the Graphics Programming Conference 2024 and 2025; Steam
page.** [talk] [web] [recent]
<https://graphicsprogrammingconference.com/archive/2024/> ·
<https://graphicsprogrammingconference.com/archive/2025/> ·
<https://store.steampowered.com/app/1203620/Enshrouded/>

An in-house engine (Holistic, Vulkan) and a hand-made voxel world: "Enshrouded has a voxel based
environment, where nearly everything can be destroyed or built from scratch", so lighting cannot be
baked; the GI talk explains "how we moved to our own SDF rays from Vulkan Raytracing to run on a
wide range of GPUs"; the fog talk uses "the voxel-representation of our world"; the 2025 water talk
simulates water "in a dynamic voxel based environment" for a "multiplayer open-world title". The map
was about 24 km² in early access, with about 64 km² announced for release (press figures). 60 GB on
Steam. The voxel size is not published (a fan blog says 50 cm).
*Bearing:* the reference for §9 C's voxel worlds: one representation serves the mesh, the edits,
the GI rays, the fog and the water. Forge's ground is a mesh with RT; its voxel layer is the edit
and cave layer, whose SDF could serve the same secondary uses later.

**Iron Gate, Valheim (Steam: "1 GB available space"; update 0.150.3, 2021).** [web]
<https://gamespot.com/articles/valheim-update-0-150-3-notes-new-terrain-system-for-faster-loading/1100-6490339/>
A seeded heightfield world with biomes by rule. Terrain edits were first one network object per
pickaxe or hoe stroke; 0.150.3 compacted them per area to cut network instances and loading time.
*Bearing:* an edit log must compact, or it dominates loading and replication.

**Minecraft (Java 1.18 and later), per the community wiki.** [docs] <https://minecraft.wiki/w/Terrain>
"More variation is created by three noise parameters: continentalness, erosion, and peaks and
valleys. These are also tied to biome placement."
*Bearing:* one set of low-frequency fields drives both shape and biome: §9 A's atlas, with noise
instead of genesis.

**Asobo, Microsoft Flight Simulator 2024.** [web] [talk]
<https://pcgamer.com/games/sim/more-details-on-ms-flight-simulator-2024-including-full-3d-landscapes-in-30-biomes>
The real Earth streamed (more than 2 PB of imagery in Asobo's 2022 deck, `planet-terrain.md` §4)
with procedural ground and vegetation in 30 biomes (the headline; the body did not load).
*Bearing:* the extreme of coarse data plus run-time detail, where the coarse data is the planet.

---

## 5. Large detailed maps without a galaxy

**CD Projekt Red and Epic Games, The Witcher 4 Unreal Engine 5 tech demo (State of Unreal, 3 June
2025); Epic's "Nanite Foliage" and "Using PCG Generation Modes" pages and the FastGeoStreaming plugin
index (Unreal Engine 5.8 documentation).** [web] [docs] [recent]
<https://press.cdprojektred.com/news/1778/cd-projekt-red-and-epic-games-present-the-witcher-4-unreal-engine-5-tech-demo-at-the-state-of-unreal-2025>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/using-pcg-generation-modes-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/API/PluginIndex/FastGeoStreaming>

The demo ran at 60 fps on a base PS5 with ray tracing. FastGeo (experimental) is "A system that
extracts and converts a partitioned world's geometry to optimize world streaming performance": a
World Partition cell transformer that turns allowed actors into lightweight geometry. Nanite
Foliage (experimental) is Assemblies (up to 65k instances of sub-meshes such as branches),
Skinning (wind by bones, so cluster bounds stay tight) and Voxels, "near pixel-sized voxels that
preserve detail, animation, and material properties"; on the demo, the biggest tree went from
3.5 GB to about 29 MB, one tree's streaming memory in a view from about 36 MB to 2.7 MB, and the
scene holds "500k instances of dozens of tree variants". PCG streams partitioned results with World
Partition, "Hierarchical Generation supports mesh generation using PCG at multiple scales", and
"Runtime Generation is a special generation mode for PCG components that generates and cleans up
dynamically" within radii of generation sources.
*Bearing:* the world is authored and baked; placement is procedural, by rules over cells of several
sizes, in the editor or at run time near the player. Forge places rocks this way already; missing
are per-rule cell sizes and a far representation for trees (`vegetation-materials.md`). FastGeo
solves an actor overhead Forge does not have.

**Electronic Arts. "How Battlefield 6 Redefined Destruction" (10 November 2025); "Frostbite
presents at GDC 2023" (Julien Keable, "From Battlegrounds to Fairways: Terrain Procedural Tools in
Frostbite").** [web] [recent]
<https://www.ea.com/en/news/how-battlefield-6-redefined-destruction> ·
<https://www.ea.com/pt-br/news/frostbite-presents-at-gdc-2023>

Destruction is part-based and persistent ("We wanted the result of destruction to stay"), with
terrain deformation among the systems revisited, and the network work was "prioritization and
culling of part destruction and debris spawning" for 64-player matches. The terrain tools are "an
advanced non-destructive GPU-based layer compositing system" with "data-driven behaviors that let
game entities affect terrain". How craters are stored was not found.
*Bearing:* authoring as non-destructive layers that entities can also write (a road, a base, a
crater) is D-053's world file plus place stamps plus an edit log: the same compositing, at
authoring time and at run time.

**Guerrilla (Horizon), Sucker Punch (Ghost of Tsushima), Ubisoft (Far Cry 5).** [talk]
`terrain-genesis.md` §4 holds them: van Muijden's GPU placement (GDC 2017), Pohlmann's GPU bytecode
rule language (GDC 2021) and Carrier's Houdini pipeline regenerating biomes, water networks and
cliffs nightly (GDC 2018). No Forbidden West placement or terrain talk was found in Guerrilla's
2022–2024 lists.
*Bearing:* the terrain is authored; everything derived from it (biomes, water, cliffs, flora) is
regenerated by graph when it changes and placed at run time by GPU rules. Forge's genesis and
cache are this with the authoring replaced by a seed, and should accept the authoring back as
overrides.

**Nintendo, The Legend of Zelda: Breath of the Wild (CEDEC 2017 talks as translated) and Tears of
the Kingdom (Fujibayashi to Famitsu, reported by Zelda Universe, 8 October 2023); install sizes.**
[talk] [web]
<https://zeldauniverse.net/2023/10/08/tears-of-the-kingdom-director-comments-on-how-fast-the-depths-were-made/>
· <https://www.thefamicast.com/2017/12/cedec-talks-translated-making-of-breath.html> ·
<https://automaton-media.com/en/news/20230508-18756/>

A hand-authored world whose level design rules are spatial ("the triangle rule": ridges that hide
what lies behind and pull the eye to the next landmark). Tears of the Kingdom adds the sky and the
Depths; Fujibayashi: "The Depths and surface have an inverted relationship;" low places and rivers
on the surface become tall walls below, a programmer "quickly finished a prototype" from those
conditions, and "the Depths were made in a surprisingly short period of time". 16.3 GB on Switch
(14.4 GB for Breath of the Wild).
*Bearing:* a stylised large map is a design problem more than a generation one, and its cheapest
trick is a *derived layer*, a second world computed from the first. Forge's cache makes such layers
cheap to try (an underground from the inverted genesis).

---

## 6. Research and repositories

**Yann Cortial, Adrien Peytavie, Éric Galin, Éric Guérin. "Procedural Tectonic Planets."
*Computer Graphics Forum* 38(2) (Eurographics 2019), DOI 10.1111/cgf.13614; and "Real-Time
Hyper-Amplification of Planets", *The Visual Computer* 36(10) (CGI 2020), DOI
10.1007/s00371-020-01923-4.** [paper] [recent]
<https://diglib.eg.org/handle/10.1111/cgf13614> · <https://hal.science/hal-02967067>

A procedural replacement for plate simulation: plates move and collide under user control
(including rifting events) and produce continents, ridges, ranges and island arcs, then the coarse
planet is amplified with procedural or real elevation data. The 2020 paper amplifies such planets in
real time (abstract not read).
*Bearing:* the generator for the *coarse* level of a galaxy's bodies, where an erosion bake per body
is too slow; for Forge's one planet, plates give the uplift field that the genesis erodes.

**Oliver Borg, James Gain, Éric Guérin, Adrien Peytavie, Marie-Paule Cani, Éric Galin, Guillaume
Cordonnier. "Authoring Terrestrial Planets with Diffusion Models." *Computer Graphics Forum* 45(2)
(Eurographics 2026), DOI 10.1111/cgf.70390.** [paper] [recent]
<https://diglib.eg.org/handle/10.1111/cgf70390>

Trained on satellite data of Earth and Mars: the user paints coarse elevation, land cover,
temperature and precipitation on a globe (a quad sphere); a global river network extracted from the
sketch conditions the diffusion so drainage stays continuous; the output holds for flybys a few
thousand kilometres up. Open access; the PDF refused the fetch.
*Bearing:* the research version of the owner's Star Citizen model, with the geology Star Citizen
lacks. Forge can take its *structure* — atlas, global drainage first, local detail conditioned on
it — without the learned model.

**Alexander Goslin. "InfiniteDiffusion: Bridging Learned Fidelity and Procedural Utility for
Open-World Terrain Generation" (first titled "Terrain Diffusion: A Diffusion-Based Successor to
Perlin Noise…"), arXiv:2512.08309, v1 9 December 2025 to v4 3 May 2026; repository
`xandergos/terrain-diffusion` (MIT).** [paper] [code] [recent]
<https://arxiv.org/abs/2512.08309> · <https://github.com/xandergos/terrain-diffusion>

Unbounded diffusion sampling with seed consistency and constant-time random access, a hierarchy of
models from planetary context to local detail, generating at nine times orbital velocity on a
consumer GPU (the author's figure); 30 m and 90 m models in the repository, which needs CUDA
PyTorch and names SIGGRAPH 2026; the weights' licence is not stated.
*Bearing:* the first learned generator with a noise-like interface. Not for run time: inference is
not bit-exact across GPUs (D-016) and PyTorch is out of scope. Usable offline to make an atlas that
is then stored.

**Lochner et al., "Interactive Authoring of Terrain using Diffusion Models" (*CGF* 42, PG 2023);
Borne-Pons et al., "MESA" (text-driven terrain from Copernicus data, CVPR Workshops 2025); Grenier,
Guérin, Galin, Sauvage, "Real-time Terrain Enhancement with Controlled Procedural Patterns" (*CGF*
43(1), 2024).** [paper] [recent]
<https://diglib.eg.org/handle/10.1111/cgf14941> · <https://arxiv.org/abs/2504.07210> ·
<https://diglib.eg.org/handle/10.1111/cgf14992>
The first two are learned authoring tools (sketch or text in, elevation out). Grenier adds
erosion-like gullies, nested across scales, to a coarse terrain with Phasor-noise patterns steered
by control maps, interactively on the GPU.
*Bearing:* Grenier's fits Forge's run time today: analytic, GPU-friendly, deterministic if written
as §9 B.3 says; a candidate for tile amplification where a short erosion pass is too slow.

**Schott et al. 2023 and 2024 (interactive erosion; multi-scale amplification), Cordonnier et al.
2016 (uplift and fluvial erosion), FastFlow 2024** — in `terrain-genesis.md` §1, read and
verified there. Schott 2024's "erode, then upsample ×2, erode again" is the bridge from a coarse
atlas to tiles.

**Guillaume Cordonnier et al. "Authoring Landscapes by Combining Ecosystem and Terrain Erosion
Simulation." *ACM TOG* 36(4), 134, 2017; Miłosz Makowski et al., "Synthetic Silviculture", *ACM
TOG* 38(4), 2019; Wojtek Pałubicki et al., "Ecoclimates: Climate-response Modeling of Vegetation",
*ACM TOG* 41(4), 155, 2022.** [paper] [still-current]
<https://doi.org/10.1145/3072959.3073667> ·
<https://research.google/pubs/synthetic-silviculture-multi-scale-modeling-of-plant-ecosystems/> ·
<https://history.siggraph.org/?p=117929>

Cordonnier 2017 couples erosion and vegetation both ways over layers of rock, sand, humus, grass,
shrubs and trees, over centuries. Synthetic Silviculture grows ecosystems of individual plants
(growth, tropisms, competition) in nine ecologies. Ecoclimates closes the loop between vegetation,
soil and air (forest edges, the Foehn effect).
*Bearing:* offline references for biomes as rules; Forge's biome rules should hold the same few
quantities (water, light, temperature, soil depth) so these methods can later replace hand-set
densities.

**Raistrick et al., "Infinite Photorealistic Worlds Using Procedural Generation" (Infinigen), CVPR
2023.** [paper] [code]
<https://openaccess.thecvf.com/content/CVPR2023/html/Raistrick_Infinite_Photorealistic_Worlds_Using_Procedural_Generation_CVPR_2023_paper.html>
Plants, animals, terrain and weather from randomised mathematical rules, no stored art; offline,
for training data. *Bearing:* procedures to read before writing flora and rock generators.

**Repositories.** `Zylann/godot_voxel` (MIT): "Smooth terrain with level of detail using
Transvoxel", paged, editable in game; multiplayer sync only on its roadmap. `SebLague/Solar-System`
(MIT): the cube-sphere planets with craters of his "Procedural Moons and Planets" video. Veloren
(Rust): noise, then stream-power erosion with uplift and hillslope diffusion; its developer warns
that "explicit numerical methods do not converge to the same solution if you make the time step
large enough" (devblog 43, 25 November 2019). `AnisB/large_cbt` (D3D12): Benyoub and Dupuy's 2024
CBT planet demo.
<https://github.com/Zylann/godot_voxel> · <https://github.com/SebLague/Solar-System> ·
<https://veloren.net/devblog-43> · <https://github.com/AnisB/large_cbt>
*Bearing:* Veloren is the nearest open-source analogue of the island's genesis in Rust, worth
reading for its time-stepping before the planet's; Lague's crater shapes are a starting point for
the moon's first layer.

---

## 7. Deformable terrain at scale

**Meshing.** Dual contouring (Ju et al. 2002) and Transvoxel (Lengyel 2010) are in
`large-worlds.md` §8. Transvoxel is "a method for seamlessly stitching together neighboring
triangle meshes generated from voxel data at differing resolutions", by transition cells at the
half-resolution boundary (transvoxel.org; patent-free per the page). Dual marching cubes (Schaefer
& Warren, Pacific Graphics 2004 / *CGF* 24(2) 2005) contours on the dual of an octree with one
vertex per cell placed at features: crack-free, adaptive, sharp features kept, with far fewer
triangles than marching cubes or dual contouring on the slides' example; it is what No Man's Sky
moved to in 2024. Surface nets (Gibson 1998; not re-checked today) are the simplest dual method,
one vertex per cell at the mean of its edge crossings, smooth but without sharp features.
Occupancy-Based Dual Contouring (Hwang & Sung, SIGGRAPH Asia 2024) is a learning-free GPU-parallel
mesher for occupancy fields, aimed at neural fields.
<https://transvoxel.org/> · <https://people.engr.tamu.edu/schaefer/research/dmc.pdf> ·
<https://arxiv.org/abs/2409.13418>

**Storage.** Sparse voxel octrees (Laine & Karras, I3D 2010) ray-cast competitively with triangles
but build slowly, so they suit static data; VDB (Museth, *ACM TOG* 32(3), 2013) gives a B+tree-like
sparse grid with O(1) average random insert and lookup and dynamic topology; NanoVDB (2021) is its
GPU-portable read-only form, 4–6× smaller with a fixed tree.
<https://research.nvidia.com/publication/2010-02_efficient-sparse-voxel-octrees> ·
<https://research.dreamworks.com/wp-content/uploads/2018/08/Museth_TOG13-Edited.pdf> ·
<https://research.nvidia.com/labs/prl/publication/nanovdb/>

**Edits and replication, from the games above.** No Man's Sky: bounded per-save buffers with
eviction, base edits protected. Space Engineers: whole voxel files per object that grow with
digging. Valheim: per-stroke objects compacted per area. Battlefield 6: part-based destruction
replicated with prioritisation and culling. Enshrouded: everything editable, water simulated and
synchronised in the voxel world. Teardown's voxel ray tracing was presented at GPC 2025.
*Bearing:* the pattern that avoids every failure listed: the generator is the base layer (never
stored), an edit is a small CSG operation on an SDF (shape, parameters, material, order) appended
to the log of the cells it touches, replicated as that operation in server order, replayed
deterministically, and compacted into SDF bricks per cell when the log of a cell passes a size.
Meshing runs per edited cell only, and the mesh joins the cluster-DAG path like any tile.

---

## What professional engines do

| Game | Generated at run time | Ahead of time | Authored | CPU / GPU | Disk | Deformable |
|---|---|---|---|---|---|---|
| No Man's Sky | everything below the seed: voxel terrain, flora, fauna, materials | nothing per planet | parts (skeletons, meshes, textures), rules, buildings | terrain not published; objects on GPU since 5.0 | 15 GB | yes, voxels; capped edit buffers in the save |
| Elite Dangerous | surfaces from body parameters, per patch | the galaxy's rules; a painted galaxy map; catalogue stars | materials, settlements, stamped sites | GPU noise with emulated doubles | 25 GB | no |
| Star Citizen | detail, scattering (GPU in the v5 plan), clouds | per-planet global maps (height, distribution, climate; v5 geology, soil) | biomes, brushes on the maps, every location | GPU for v5 (planned) | ~100 GB (third-party) | no (planets) |
| Space Engineers | voxel detail from maps | — | six 2048² height and material maps per planet | CPU | not checked | yes, voxel files per object |
| KSP2 | quad-sphere terrain (PQS+) | not published | planets' maps | not published | not checked | no |
| SpaceEngine | elevation and colour textures per node | real catalogues and maps where known | — | GPU, read back | not checked | no |
| Starfield | bounded landing tiles from a tile set | — | cities, quest sites, terrain tiles | not published | not checked | no |
| Enshrouded | — (world shipped) | the voxel world | the whole map | own engine, SDF GI rays | 60 GB | yes, everything |
| Valheim | heightfield and biomes from a seed | — | biome rules, assets | CPU | 1 GB | yes, compacted per area |
| Minecraft | 3D density and biomes from noise | — | rules, structures | CPU | not checked | yes |
| MSFS 2024 | ground detail, vegetation in 30 biomes | ML reconstruction of the Earth | photogrammetry cities | GPU | streamed (petabytes server-side) | no |
| UE5 Witcher 4 demo | PCG runtime generation near sources | World Partition, HLOD, FastGeo, PCG baked | the world | GPU (Nanite, PCG GPU) | n/a (demo) | no |
| Battlefield 6 | destruction and debris | terrain layers composited | maps, destructible parts | GPU layer compositing (tools) | not checked | parts; terrain deformation |
| Horizon / Tsushima | GPU placement by rules | terrain-derived maps | terrain, rules | GPU | not checked | no |
| Far Cry 5 | — | biomes, water, cliffs by Houdini, nightly | terrain | offline farm | not checked | no |
| Zelda TotK | — | — | everything; the Depths derived from the surface | — | 16.3 GB | building, not terrain |

The pattern: the larger the address space, the less is authored per place and the less geology
survives; the worlds that look right at walking height either author the ground (Zelda,
Enshrouded, the Witcher 4 demo) or are moving to simulated global data maps (Star Citizen v5,
Elite's Odyssey). Nobody ships a galaxy with drainage.

---

## Recommendation for Forge

The common structure, for all three scales: **coarse fields that are cheap to keep, a
deterministic producer that amplifies them per cell, rules that read them for materials and
placement, places stamped in, and edits as a log.** Only the coarse fields, the places, the rules
and the parts ship; tiles and placements are derived data (D-053) in a capped disk cache. This
answers Phase 9's open item: global work (drainage, plates, climate) is done only at the coarse
level, where a whole planet is a few tens of millions of samples, and regional or tile work runs
with halos under boundary conditions the coarse level fixed, so any chunk can be made alone.

### A. One planet and its moon, now (Star Citizen's model, with Forge's genesis)

What the `planet` demo should become after #220's step 1 draws:

1. **The planet atlas** (new, between D-056's steps 1 and 4). Per cube face, 1025² or 2049²
   samples (on Earth's radius about 9.8 or 4.9 km apart; 6.3 M or 25 M samples in all) of
   `height` (16-bit), `temperature`, `humidity`, `rock`, `soil_depth` and a derived `biome` id:
   one set of low-frequency fields for shape and biome, as in Star Citizen and Minecraft. Six
   bytes a sample is about 150 MB raw at 2049², 38 MB at 1025², before LZ4 (Space Engineers ships
   the same 2048² a face, at 20 m a pixel on a 120 km planet). Made from noise now, from plates
   (Cortial 2019) and D-056 step 4's genesis later, and in both cases with **painted overrides** in
   `assets/worlds/earth.toml`: regions with a target height, biome or climate offset, Star
   Citizen's brushes as data. Keyed by seed, overrides and code (D-053); the one large global
   product, shipped once final.
2. **Tiles read the atlas.** The tile producer samples the atlas (bicubic, with its halo) for the
   low frequencies and adds band-limited detail below the atlas's spacing, its amplitude and
   style chosen per biome and rock (Elite's material zones, Odyssey's sub-zones). The noise stops
   inventing continents. Later, the amplification becomes Schott 2024's erode-and-upsample or
   Grenier 2024's procedural gullies, conditioned on the atlas's drainage so valleys meet rivers.
3. **Biomes as rules, not as content.** A `[[biome]]` entry in the world file names its ground
   layers (D-028), its rock and flora sets, densities as functions of slope, soil depth and
   humidity, and its scatter cell sizes (UE PCG's hierarchical sizes: large cells for trees and
   boulders, small for grass). The GPU placement that drops rocks today evaluates these rules per
   tile; nothing placed is stored (Horizon, Tsushima).
4. **Places.** A `[[place]]` entry stamps authored or separately generated content onto the
   planet: a footprint on a cell, a height rule (flatten to a level, keep its own field, or blend
   over a band, as Elite flattens settlements and D-056 question 3 blends the island over 1 km),
   its own cooked meshes, and the cells it owns for streaming. The island is the first place;
   a port, a ruin, a crater field are the next. The tiles under a place read its rule; its content
   streams as its cells come in (Star Citizen's object containers).
5. **The moon** as the second planet description: no air, no water, a crater layer (a sum over
   seeded impacts by size class, as in Lague's video, newer craters cutting older ones)
   over regolith noise, and its own atlas (1025² a face is about 2.7 km a sample at the Moon's
   1 737 km radius). It tests that a planet is data, not code.
6. **Disk policy.** Tiles are never in the package. A tile is 0.1 s of noise and 0.16 s of cook
   today, so the cache is an LRU with a size cap and a first visit pays a quarter of a second per
   tile on a background core (`planet-terrain.md` §4 counted 3–4 tiles a second at 300 m/s). The
   package holds the atlas, the places and the rules. This is D-055's option 1 applied to the
   planet before the island needs it.

**Changes to the decisions.** D-056 gains a step "1b: the planet atlas and biomes as rules" before
step 4, and question 3's island becomes the first `[[place]]`. D-014's "genesis baked per region"
is made precise: genesis is baked at the atlas's resolution for the whole planet and amplified per
tile; regional genesis at finer spacing runs per region with the atlas's drainage as boundary
flows, so regions are independent.

### B. A galaxy, later (No Man's Sky's and Elite's model)

1. **Addressing first, not voxels.** A `BodyId` in 64 bits in Elite's shape (sector coordinates,
   sector layer of an octree, system, body), with `Seed::derive` from it; a body's record (mass,
   radius, age, composition, orbit, atmosphere, water) generated top-down from galaxy-scale
   distributions and one painted galaxy map, with catalogue overrides where the game wants real
   stars. D-037's `u64` cell ids continue below the body. This is cheap, testable without
   graphics, and makes every later step addressable.
2. **The atlas generated, not painted.** For a body the player approaches, the atlas of §A is made
   from the body record: plates (Cortial 2019) for uplift, erosion only for bodies with liquid,
   craters for airless ones, climate from insolation and atmosphere. It must take seconds, hidden
   behind travel as Elite and No Man's Sky hide theirs. At 1025² a face (6.3 M samples, 1.5 times
   the island's 4.2 M) a CPU erosion pass would take about half a minute by the island's 23 s, so a
   galaxy needs a coarser first atlas (513², refined later), the GPU, or no erosion on most bodies.
3. **The GPU under D-016.** The Vulkan specification makes 32-bit `OpFAdd`, `OpFSub` and `OpFMul`
   "Correctly rounded", but the rounding mode is implementation-defined unless the entry point
   declares `RoundingModeRTE`, denormals may be flushed unless a denorm mode is declared, a
   multiply and an add may be fused unless decorated `NoContraction`, and `OpFDiv` has a 2.5 ULP
   bound. Two ways keep the same bits on every vendor: **integer noise** (hashing and fixed-point
   interpolation in `uint`, exact by construction), or float code with RTE, a fixed denorm mode,
   `NoContraction` (the `precise` qualifier) and no division or transcendental. Measure both
   against the CPU's `dmath` digests on the RTX 5070 Ti and the AMD iGPU (`FORGE_GPU=amd`).
   SpaceEngine's 100–200× says what it buys.
4. **Voxels only if the game digs.** Elite has no voxels and lands on planets across 400 billion
   systems; No Man's Sky has voxels because everything is diggable and caves are everywhere.
   Forge's answer is D-014's hybrid: heightfield tiles everywhere, an SDF volume only in cells with
   caves, overhangs or edits, meshed per cell by dual marching cubes (No Man's Sky's choice) or
   surface nets into a cluster DAG, its border locked to the heightfield tile's border. The spike
   to run first is that border: one cell carved by a cave next to plain tiles, with a 0-crack
   check and the ꟻLIP of the swap.
5. **Parts and rules for life.** Flora and fauna from authored parts recombined per seed (No Man's
   Sky's blueprints; Synthetic Silviculture offline), placed by the biome rules of §A. The disk
   grows with parts and rules, not with bodies.

**Changes to the decisions.** D-014 keeps the volumetric near field and SDF bricks but states that
they are per cell and only where needed; D-016 gains the GPU rule above (integer or controlled
float, verified by digests on two vendors) before any GPU generator is authoritative.

### C. Detailed large maps without a galaxy (Witcher 4, Battlefield 6, Zelda, Enshrouded)

1. **Authored first, generated second.** The island pipeline stays (genesis baked ahead of time,
   D-053's world file), with the overrides of §A as the authoring layer: painted regions,
   splines (roads, rivers), stamps; derived layers (biomes, water networks, cliffs, flora) remade
   by the cache when an input changes (Far Cry 5's nightly regeneration, made incremental).
2. **Placement as a rule graph with cell sizes and radii,** evaluated on the GPU near the player and
   baked for far cells (UE PCG's hierarchical and runtime modes, Horizon, Tsushima).
3. **A far representation for dense trees.** Nanite Foliage's voxels are the current answer to
   forests at distance; `vegetation-materials.md` owns the choice, and this file only records the
   demo's numbers (3.5 GB → 29 MB, 500k instances) as the bar.
4. **Destruction and voxel worlds** use §B.4's SDF cells and §7's edit log; an Enshrouded-like
   game would make the SDF the ground everywhere and could also use it for fog and secondary rays.
5. **Styles** (Zelda to realistic) are the materials' and rules' business, not the generator's; the
   generator's job is to expose the fields (slope, curvature, wetness, distance to water) that a
   stylised material reads, and derived layers like the Depths.
6. **Disk.** D-055's option 1 (cook the 2 m tiles near the camera) when a second map exists; the
   eroded field at 8 m is the shipped product, the 2.65 GB of 2 m tiles are cache.

**Changes to the decisions.** D-053's world file gains `[[biome]]`, `[[place]]` and painted
overrides for flat maps as for planets; D-055's option 1 becomes the plan for any second map;
D-014's SDF bricks become the destruction layer as well as the edit layer.

---

## What to measure

- **Atlas:** build time and bytes (raw, LZ4) at 1025² and 2049² a face; digests at one and six
  workers; the same for the moon.
- **Tiles:** p50/p99 of produce and cook with the atlas (against 0.1 s and 0.16 s today); cache
  bytes per tile; latency from *wanted* to *resident*, first visit and cached; cores busy in the
  descent at 300 m/s and at re-entry speed.
- **The look:** the golden shots at 400 km, 10 km and the coast before and after the atlas, with
  ꟻLIP; repetition across tiles of one biome (`planet-terrain.md` §4's survey mode, with slope and
  biome histograms per face).
- **Places:** ꟻLIP across the island's blend band; 0 px of cracks at a flattened stamp's border.
- **Disk:** the package for planet and moon (atlas, places, rules) against the island's 2.65 GB of
  2 m tiles; the cache after a scripted orbit, capped and uncapped.
- **GPU generation:** bit equality with the CPU on the RTX 5070 Ti and the AMD iGPU, integer and
  float variants; ms per tile against the CPU.
- **Edits:** bytes per operation; replay ms per cell at 10, 100 and 1 000 operations; mesh ms per
  edited cell; the ꟻLIP of a cell turning volumetric.
- **Galaxy (later):** ms per body record; seconds from "system selected" to its atlas drawn.

---

## Checked and left out

- **No Man's Sky's terrain internals** (region size, octree depth, threads, the 5.50 method): the
  GDC Vault gives abstracts only; a forum guess of dual contouring is not a source.
- **KSP2 and concurrent binary trees:** not confirmed; the developer post says PQS+, the CBT claim
  is forum speculation plus Unity's own 2021 demo. `planet-terrain.md` §4 needs this correction.
- **Star Citizen's map resolutions, v5 internals, Pyro's tools:** not published as text; a forum
  account of Pyro's pipeline is unofficial.
- **Enshrouded's voxel size and storage:** a fan blog only (50 cm); the GPC 2025 slides not opened.
- **Battlefield 6's crater storage, a Forbidden West placement talk, MSFS 2024's GDC talk:** none
  found.
- **Astroneer, Dual Universe internals, Light No Fire's technology, Experilous' and "Undiscovered
  Worlds" planet generators:** no technical source found.
- **Hello Games' creatures first-hand:** 2014 press only; a 2019 thesis disputes a parts reading,
  hence "blueprints" here.
- **Learned generators at run time:** left out for D-016 (§6); kept as offline atlas producers.

---

## Verification notes

Checked on 2026-10-09 and 2026-10-10 with WebFetch and WebSearch only. "Read" means WebFetch
returned the page and the quotes are from it; "search record" means only the search engine's
extract or a summary of it was seen.

- **Read:** nomanssky.com Worlds Part I update (5.0; no date on the page, July 2024 by press),
  the Worlds Part II post (29 January 2025) and its update page; GDC Vault 1024514 (Murray's
  abstract, opening line); Steam pages of No Man's Sky (15 GB), Elite Dangerous (25 GB), Enshrouded
  (60 GB), Valheim (1 GB); 80.lv's Elite interview (5 April 2018); PCGamesN on Horizons (14 August
  2017); GamersNexus's Roberts interview (24 September 2016); starcitizen.tools Planet Tech v4
  (edited 14 April 2026), Genesis (edited 19 August 2026), Letter from the Chairman 2024-12-19;
  RSI's Roadmap Roundup of 12 August 2026 (via api.star-citizen.wiki); the Graphics Programming
  Conference archives 2024 and 2025; Epic's Nanite Foliage, PCG generation modes and
  FastGeoStreaming pages (5.8 documentation); EA's Battlefield 6 destruction article (10 November
  2025) and Frostbite GDC 2023 page; Zelda Universe on the Depths (8 October 2023); arXiv 2512.08309
  (versions 1–4); GitHub `xandergos/terrain-diffusion`, `AnisB/large_cbt`, `Zylann/godot_voxel`,
  `SebLague/Solar-System`; transvoxel.org; Veloren devblog 43 (25 November 2019); the Space
  Engineers wiki's "Creating a Planet"; minecraft.wiki "Terrain"; Éric Galin's publication list
  (titles and venues of Cortial 2019 and 2020, Borg 2026, Grenier 2024, Schott 2023 and 2024);
  the Vulkan specification's SPIR-V environment appendix (precision table).
- **Search record only:** McKendrick's GDC 2017 abstract (GDC Vault 1024265); the 2⁶⁴ figure
  (PlayStation Blog 2014 via Gematsu and others); No Man's Sky's edit caps (Steam threads,
  gamepretty) and the Synthesis note (PC Gamer); Game Informer's creatures (2014); Light No Fire
  (Push Square, PC Gamer, December 2023); the Odyssey material (MassivelyOP, Frontier forum notes,
  PC Gamer headline); Elite's 400 billion systems (Giant Bomb; Frontier's Newsletter #36 as quoted
  on its forum); Star Citizen's clouds (patch notes 3.14, 3.15.1, 4.1) and Object Container
  Streaming page; Star Citizen's ~100 GB (third-party guides); KSP2 Developer Insights #12 (the
  site returned 404) and the Unity SIGGRAPH 2021 talk; SpaceEngine's blog posts; Dual Universe
  (Kickstarter material, Wikipedia); Starfield (Den of Geek, Starfield Portal, Insider Gaming);
  Enshrouded's map size (Twinfinite, Steam discussion);
  the CD Projekt Red press release on the Witcher 4 demo (3 June 2025); Valheim 0.150.3
  (GameSpot); Space Engineers' `.vx2` sync reply (Keen support); MSFS 2024 (PC Gamer headline,
  Purexbox) and the Asobo deck; the Tears of the Kingdom and Breath of the Wild sizes (Automaton,
  Nintendo Life) and the CEDEC 2017 translations; Cortial 2019 and 2020 abstracts (Eurographics
  library and HAL refused the fetch); Borg et al. 2026 (Eurographics library record; the PDF
  refused); Lochner 2023, MESA 2025, Grenier 2024 abstracts; Cordonnier 2017, Synthetic Silviculture
  2019, Ecoclimates 2022; Infinigen 2023; dual marching cubes (2004/2005), Laine & Karras 2010,
  Museth 2013, NanoVDB 2021, Hwang & Sung 2024.
- **From earlier Forge research, not re-read today:** Horizon (GDC 2017), Ghost of Tsushima (GDC
  2021), Far Cry 5 (GDC 2018), Schott 2023/2024, Cordonnier 2016, FastFlow, Proland, Outerra,
  dual contouring and Transvoxel's dissertation, Benyoub & Dupuy 2024 — see `terrain-genesis.md`,
  `planet-terrain.md` and `large-worlds.md`.
- **Numbers to re-check before a spec:** the atlas sizes and bytes (computed here, not measured);
  the 23 s scaling to a 1025² face (a guess from the island's 2049² genesis, ignoring the sphere's
  neighbour rule); Space Engineers' 20 m a pixel (the wiki's figure for 120 km); Enshrouded's map
  sizes (press); Star Citizen's disk (third-party); Nanite Foliage's figures (Epic's page, demo
  content only). Forge's 0.1 s and 0.16 s per tile came from the task brief; `docs/demos/planet.md`
  has them measured since (2026-10-10).
