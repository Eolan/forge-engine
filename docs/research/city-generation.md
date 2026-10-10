# Research — City generation: roads, blocks and lots, districts, buildings from grammars and kits

> Companion to `procedural.md` §4 ("Settlements, roads and buildings": Parish & Müller, CGA, Emilien's
> villages, Galin's road hierarchies, the GDMC rubric) and to `vegetation-materials.md` §4 (trim
> sheets and the regional material set). Written 2026-09-26 for two of the living city's ideas,
> #85 (layout: road hierarchy, blocks, lots, districts, landmarks) and #86 (modular buildings from
> style sets and shape grammars, to decide early), with #84, #88 (interiors) and #91 (a
> south-of-France village or a Mediterranean town) in view. Every citation was checked that day
> against a reachable page or, where the network proxy refused the host, against the search
> engine's record of it; the distinction is kept per entry ("verified through search results", for
> issue #99) and summarised under [Verification notes](#verification-notes). What was looked for
> and not found is under [Checked and left out](#checked-and-left-out). Entries `procedural.md`
> already carries are re-verified and placed where the city pipeline needs them, not repeated.
>
> **Extended on 2026-10-10 with §7–§19** for the owner's question of that day: a city sandbox with
> release-level consistency, a European city of today a few years ahead and much greener, by the sea
> with a river through it, every building enterable with an interior that fits its use, and a city of
> public transport (trams, public automated taxis that may share the tram tracks, electric river
> taxi-boats) without private cars; with the owner's notes on integrated technology and well-being and
> the ideas the owner accepted. The extension builds on `procedural-content.md`,
> `population-simulation.md` and `assets-and-licences.md` of the same day, `rivers.md`, `water.md`,
> `vegetation-materials.md` and D-046. Its sources were checked with WebFetch, WebSearch and `gh api`
> only, no browser pane, nothing downloaded; each entry says whether it was read or only seen as a
> search record. Its own recommendation, measures, patents, left-outs and verification notes follow the
> original ones, marked "(extension)". The plan for one city stays in the private sandbox.

The question is what the published work and the shipped cities say about generating a town from a
seed so that Forge's pipeline can be fixed in order before any code: how a road network with a
hierarchy is grown (L-systems with goals and constraints, tensor fields, example-based growth, cost
paths over terrain), how blocks become lots (oriented-box and straight-skeleton subdivision), how
districts and landmarks are decided (land value and zoning fields, Lynch's five elements, authored
overrides), how buildings are produced (split grammars against artist kits against hand placement,
and the hybrid every large studio has converged on), what building generation must leave room for
(floor plans, portals, room streaming), and what it costs to draw a city of module instances
through the cluster DAG Forge already has. The short answer: streets are a solved problem twice
over — Parish & Müller's propose-validate-commit loop for the hierarchy and Chen et al.'s tensor
fields for the pattern — and both are CPU work of seconds for a 4 km city; lots are Vanegas et al.
2012, a page of geometry; and the building decision is not grammar *or* kits, because the studios
that ship cities (Bethesda, Massive, Ubisoft Toronto, Epic's City Sample) all assemble buildings from
kits of a few hundred modules, and the research grammars (Wonka 2003, Müller 2006, CGA++) are the
best way to *decide* which module goes where. For a procedural-first engine without an art team, a
CGA-style split grammar whose terminals are kit modules, instanced through the existing DAG path,
with a style set per district and a merged proxy per far building, is the architecture that makes
interiors, destruction, variety and memory all possible at once; the modules themselves can be
generated at cook time from the same profile-and-trim tools the material research already asked for.

> **State of the art in five sentences.** Road networks are grown, not drawn: either as an L-system
> whose every proposed segment is checked against terrain, water and existing roads (Parish &
> Müller 2001, the CityEngine lineage), or as streamlines of a designed tensor field whose major and
> minor directions give grids, radial patterns and coast-following streets in one operator (Chen
> et al. 2008), with example-based (Aliaga 2008, Nishida 2016) and simulation-based (Weber 2009,
> Vanegas 2012) control layered on top; villages use interest maps and cost paths over the slope
> instead (Emilien 2012, Galin 2011). Blocks are the faces of the road graph and lots are cut from
> them by recursive oriented-bounding-box splits or by offsetting the block's straight skeleton so
> every lot keeps a street frontage (Vanegas et al. 2012), while districts come from a land-use
> field (agents, or distance and slope rules) and landmarks from authored overrides at the nodes
> Lynch named. Buildings in research are split grammars (Instant Architecture 2003, CGA 2006,
> CGA++ 2015) that produce unique geometry; buildings in shipped cities are kits — Skyrim's
> footprint-snapped pieces, The Division's footprint-plus-kit building tool, Watch Dogs: Legion's
> 250-module industrial kit, the City Sample's 24 kits and 2 000 meshes for 7 000 buildings of
> "hundreds of instances" each — because instancing is what fits in memory and what breaks, opens
> and varies. Interiors are floor plans (Merrell 2010, Lopes 2010, Marson & Musse 2010) behind
> portals (Teller & Séquin 1991, Luebke & Georges 1995) or a shader illusion (interior mapping,
> 2008), and every one of them needs the exterior to be built on a grid the interior can share.
> The engines confirm the split: Unreal's City Sample and PCG framework place kit pieces from
> Houdini-generated points and rules, CityEngine's grammars reach engines as instanced meshes or
> through a plugin that runs CGA at run time, Unity has nothing built in, and Houdini Engine cooks
> assets in the editor but not in the shipped game.

**The extension of 2026-10-10, in short.** The city the owner describes is buildable from rules that real European cities
already write down, and most of them are checkable numbers. Heights follow street widths (Paris
since 1784); cornices and balconies align (Haussmann); blocks are 113 m with cut corners (Cerdà) or
perimeter rings round courtyards; three-by-three blocks become a superblock with 10 km/h inside
(Barcelona); every home is within 300 m of a park and sees three trees (the 3-30-300 rule); a plot's
"ecologically effective" share is scored (Berlin's biotope area factor, 0.6 for housing); street space
in car cities goes 55–58 % to cars, so a transit-first city reverses that split. Transit-first cities
keep the vehicles a city needs and filter them by permit, time window and lane: Ghent lets emergency
services, public transport, refuse trucks, taxis and permitted care providers through its filters;
Pontevedra admits service vans at 10 km/h on set routes and hours; tram lanes are shared, where
paved, with buses, taxis and emergency vehicles. A tram line carries some 3 000–11 000 people an hour
each way, so a dense city of a million needs either many parallel lines or a trunk underground. For
interiors, the shipped answer at game rates is a greedy, scored room placement on a tile grid
(Shadows of Doubt), with rule-based dressing per room in milliseconds when a cell opens; constraint
solvers in the Infinigen Indoors style stay offline (minutes a room), and the learned floor-plan
generators (House-GAN, HouseDiffusion, the RPLAN dataset) are research-only by licence. Interiors line up
with facades if partition walls may only fall on the piers between D-039's window modules, and if the
interior-mapping shader reads the same bay widths and floor heights. A million people need about half a
million dwellings and 20–30 thousand shopfronts, so interiors are derived from seeds on demand, never
stored. The style bible becomes one data file every generator reads and every generator's output is
checked against.


**Contents**

1. [Road networks and layout](#1-road-networks-and-layout)
2. [Blocks, lots and parcels; districts and landmarks](#2-blocks-lots-and-parcels-districts-and-landmarks)
3. [Buildings: grammars, kits, constraint solvers, interiors](#3-buildings-grammars-kits-constraint-solvers-interiors)
4. [Rendering a city of modules through the cluster DAG](#4-rendering-a-city-of-modules-through-the-cluster-dag)
5. [What professional engines do](#5-what-professional-engines-do)
6. [The decision #86 asks for: grammar, kits or hybrid](#6-the-decision-86-asks-for-grammar-kits-or-hybrid)
7. [The real coast and river: from elevation and hydrography data to a flat city and back](#7-the-real-coast-and-river-from-elevation-and-hydrography-data-to-a-flat-city-and-back)
8. [Waterfronts and harbours](#8-waterfronts-and-harbours)
9. [A river through the centre](#9-a-river-through-the-centre)
10. [Parks, green infrastructure, rooftops and visible nature](#10-parks-green-infrastructure-rooftops-and-visible-nature)
11. [Public space: squares, pedestrian and play streets, markets and events](#11-public-space-squares-pedestrian-and-play-streets-markets-and-events)
12. [Streets of transit-first, pedestrian-first cities](#12-streets-of-transit-first-pedestrian-first-cities)
13. [The vehicles a car-free city still needs](#13-the-vehicles-a-car-free-city-still-needs)
14. [Block types and mixed use](#14-block-types-and-mixed-use)
15. [Consistency: the style bible as data, and landmarks](#15-consistency-the-style-bible-as-data-and-landmarks)
16. [The near-future city: integrated technology, well-being, no fossil fuels](#16-the-near-future-city-integrated-technology-well-being-no-fossil-fuels)
17. [Interiors by building use](#17-interiors-by-building-use)
18. [The city in numbers](#18-the-city-in-numbers)
19. [Reference cities and games: what makes a place read as one](#19-reference-cities-and-games-what-makes-a-place-read-as-one)
- [Recommendation for Forge](#recommendation-for-forge) and [its extension](#recommendation-for-forge-extension)
- [What to measure (extension)](#what-to-measure-extension)
- [What the numbers say](#what-the-numbers-say)
- [Patents](#patents)
- [Checked and left out](#checked-and-left-out) and [its extension](#checked-and-left-out-extension)
- [Verification notes](#verification-notes) and [their extension](#verification-notes-extension)

---

## 1. Road networks and layout

Two ideas carry this section. Parish & Müller's is procedural control: rules propose a street
segment, global goals (population, a pattern template) and local constraints (terrain, water, the
roads already there) adjust or reject it. Chen et al.'s is geometric: a smooth tensor field over the
map, its streamlines the streets, its singularities the places where a grid turns. Everything after
them is a way to steer one of the two from examples, from simulation or from a designer's sketch.

**Yoav I. H. Parish, Pascal Müller. "Procedural Modeling of Cities." *SIGGRAPH 2001*, 301–308.**
[paper] [foundational] [still-current]
<https://dl.acm.org/doi/10.1145/383259.383292> (DOI 10.1145/383259.383292; PDF
<https://cgl.ethz.ch/Downloads/Publications/Papers/2001/p_Par01.pdf>; verified through search
results)

The CityEngine origin paper, already in `procedural.md` §4. From "image maps given as input, such as
land-water boundaries and population density, the system generates a system of highways and
streets, divides the land into lots, and creates the appropriate geometry for the buildings on the
respective allotments"; the extended L-system separates the *global goals* (population density,
the street pattern: rectangular raster, radial, following the elevation) from the *local
constraints* that snap a proposed segment to a nearby crossing, shorten it at water or reject it on
a slope. Highways are grown first towards population peaks, streets fill between them.
*Bearing:* the hierarchy. Whatever traces the streets, the propose → validate → commit loop over a
graph with two classes first (highways and arterials) and the rest after is the structure #85's
"highways to alleys" needs, and the local constraints are where the island's fields (slope, water
distance, coast) enter.

**Jing Sun, Xiaobo Yu, George Baciu, Mark Green. "Template-based generation of road networks for
virtual city modeling." *ACM VRST 2002*, 33–40.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/585740.585747> (DOI 10.1145/585740.585747; verified through
search results)

Road networks from "image-derived templates" and "a rule-based generating system": a population
image and a pattern template (grid, radial, mixed) drive the growth, and the network "adjusts
itself intelligently to avoid restricted geographical areas or urban developments".
*Bearing:* the pattern-as-image idea Chen et al. generalise into a field. Worth knowing as the
cheapest form of the idea: a district's pattern is a template, and mixing templates per district is
how one city has a downtown grid and a radial old town.

**George Kelly, Hugh McCabe. "A Survey of Procedural Techniques for City Generation." *The ITB
Journal* 7(2), article 5, 2006; with "Citygen: An Interactive System for Procedural City
Generation", *Fifth International Conference on Game Design and Technology*, 2007.** [paper]
[still-current]
<https://arrow.tudublin.ie/itbj/vol7/iss2/5/> (verified through search results)

The survey covers "fractals, L-systems, Perlin noise, tiling systems and cellular basis" as applied
to cities, and the follow-up system grows primary roads between user-placed nodes and fills the
cells with secondary streets interactively.
*Bearing:* the reading list for the road step, and the origin of the two-level scheme (primary
roads by the designer or the field, secondary by growth) that every later tool keeps.

**Guoning Chen, Gregory Esch, Peter Wonka, Pascal Müller, Eugene Zhang. "Interactive Procedural
Street Modeling." *ACM Transactions on Graphics* 27(3) (SIGGRAPH 2008).** [paper]
[foundational] [still-current]
<https://dl.acm.org/doi/10.1145/1399504.1360702> (DOI 10.1145/1360612.1360702; project page
<https://www.sci.utah.edu/~chengu/street_sig08/street_project.htm>; verified through search
results)

The tensor-field method: "interactively modeling large street networks" through "designing an
underlying tensor field and editing the graph representing the street network", so that "a user can
create a street network from scratch or modify an existing street network". The field is a sum of
basis fields (grid elements with an orientation, radial elements around a centre, elements that
follow a boundary polyline such as a coast or a river, and a heightfield's gradient), smoothed;
major roads are streamlines of the major eigenvector traced with a large separation, minor roads
streamlines of the minor eigenvector with a small one, and the graph's faces are the blocks.
*Bearing:* the recommended tracer. It gives Santa Cruz's grid meeting its curved coast and a
Mediterranean town's streets bending with the contour lines from the same operator, with the
hierarchy from the separation distances; the field's basis elements are the designer's handles,
and a field is a pure function of the seed and the terrain fields (P3). The open implementation
below shows it is a few hundred lines.

**Daniel G. Aliaga, Carlos A. Vanegas, Bedřich Beneš. "Interactive Example-Based Urban Layout
Synthesis." *ACM Transactions on Graphics* 27(5) (SIGGRAPH Asia 2008), article 160.** [paper]
[still-current]
<https://dl.acm.org/doi/10.1145/1409060.1409113> (DOI 10.1145/1409060.1409113; verified through
search results)

Layouts by example rather than by rule: the system "simultaneously performs both structure-based
synthesis and image-based synthesis to generate complete urban layouts with plausible street
networks and aerial-view imagery, using data from real-world urban areas", with "join, expand, and
blend" as the user's operations.
*Bearing:* the way to get a real place's street statistics (block sizes, intersection angles,
dead-end ratios) into a seeded generator: measure a district of Santa Cruz or a Provençal village
from OpenStreetMap, then fit the field's parameters to it, rather than copying the map.

**Basil Weber, Pascal Müller, Peter Wonka, Markus Gross. "Interactive Geometric Simulation of 4D
Cities." *Computer Graphics Forum* 28(2) (Eurographics 2009), 481–492.** [paper] [still-current]
<https://diglib.eg.org/items/e0848b54-e157-4213-9a75-e16483be5213> (DOI
10.1111/j.1467-8659.2009.01387.x; author PDF
<http://www.peterwonka.net/Publications/pdfs/2009.EG.Weber.UrbanSimulation.FinalVersion.pdf>;
verified through search results)

A city grown over time: the approach "does not rely on land-use simulation on a regular grid, but
instead builds a complete and inherently geometric simulation that includes exact parcel
boundaries, streets of arbitrary orientation, street widths, 3D street geometry, building
footprints, and 3D building envelopes", at "about 1 second per time step". Growth follows land
value, traffic and a street-pattern hierarchy; old cores stay irregular while new quarters are
gridded.
*Bearing:* the argument for an *old town* that differs from the rest by history, not by a style
tag: run the growth from a seed settlement (the port, the church square) and let the later
districts be gridded around it. A cheap version is two fields (an organic one near the core, a
grid outside) blended by a "founding date" map.

**Carlos A. Vanegas, Daniel G. Aliaga, Peter Wonka, Pascal Müller, Paul Waddell, Benjamin Watson.
"Modelling the Appearance and Behaviour of Urban Spaces." *Computer Graphics Forum* 29(1), 2010,
25–42; with Vanegas, Aliaga, Beneš, Waddell, "Interactive Design of Urban Spaces using Geometrical
and Behavioral Modeling", *ACM Transactions on Graphics* 28(5) (SIGGRAPH Asia 2009).** [paper]
[still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/j.1467-8659.2009.01535.x> (DOI
10.1111/j.1467-8659.2009.01535.x; EG diglib
<https://diglib.eg.org/handle/10.2312/egst.20091059.001-016>) ·
<https://dl.acm.org/doi/10.1145/1618452.1618457> (verified through search results)

The state-of-the-art report of the field's founders, and the paper that couples geometry with
behaviour (an urban simulation deciding land use and density, the geometry re-derived from it).
*Bearing:* the map of the field for anyone joining the city work; the behavioural coupling is
where #90's simulation and the layout would meet, later.

**Carlos A. Vanegas, Ignacio Garcia-Dorado, Daniel G. Aliaga, Bedřich Beneš, Paul Waddell. "Inverse
Design of Urban Procedural Models." *ACM Transactions on Graphics* 31(6) (SIGGRAPH Asia 2012).**
[paper] [still-current]
<https://dl.acm.org/doi/10.1145/2366145.2366187> (DOI 10.1145/2366145.2366187; project
<https://www.cs.purdue.edu/cgvlab/www/publications/Vanegas12ToG/>; verified through search results)

Control by target rather than by parameter: a framework for "adding intuitive high-level control to
existing urban procedural models", to "interactively edit urban models" by specifying indicators
(sunlight on a square, a skyline, a density) and solving for the generator's parameters.
*Bearing:* the pattern for the owner's overrides at city scale ("the towers stay below this line
from the harbour", "this square gets the afternoon sun"): keep the generator's parameters
solvable, do not hand-edit its output.

**Gen Nishida, Ignacio Garcia-Dorado, Daniel G. Aliaga. "Example-Driven Procedural Urban Roads."
*Computer Graphics Forum* 35(6), 2016, 5–17.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.12728> (DOI 10.1111/cgf.12728; project
<https://www.cs.purdue.edu/cgvlab/www/publications/nishida2016example/>; verified through search
results)

"An interactive tool that allows untrained users to design roads with complex realistic details and
styles", the roads "growing a geometric graph" whose local patches are taken from example networks
(real maps), warped to the terrain and blended, so that a sketched arterial gets the side streets of
the example around it.
*Bearing:* the third option for the tracer, and the best for "a city like Santa Cruz": grow from
example patches of the real place. It is more code than the tensor field (patch matching and
warping); recommended as the second step once the field version renders.

**Arnaud Emilien, Adrien Bernhardt, Adrien Peytavie, Marie-Paule Cani, Éric Galin. "Procedural
Generation of Villages on Arbitrary Terrains." *The Visual Computer* 28(6–8), 2012, 809–818; with
Éric Galin, Adrien Peytavie, Éric Guérin, Bedřich Beneš, "Authoring Hierarchical Road Networks",
*Computer Graphics Forum* 30(7) (Pacific Graphics 2011), 2021–2030.** [paper] [foundational for
villages] [still-current]
<https://link.springer.com/article/10.1007/s00371-012-0699-7> (DOI 10.1007/s00371-012-0699-7) ·
DOI 10.1111/j.1467-8659.2011.02055.x (both in `procedural.md` §4; verified through search
results)

The village pair, re-verified rather than repeated: settlement seeds and roads co-evolving from
*interest maps* (slope, sun, water, defensibility, access), parcels by anisotropic region growing
along the slope and the road, houses from an open grammar adapted to the slope; and roads between
settlements by shortest path under a terrain cost with junction merging, cuttings and bridges.
*Bearing:* #91's generator. A south-of-France village is not a small city: it has no tensor grid,
its lots follow the contour and the lane, and its roads are least-cost paths. The recommendation
keeps both paths under one `RoadGraph` and one `Lot` type, and picks the tracer per district.

**Introversion Software. Subversion's city generator (Chris Delay, 2006–2010; the Imperial College
Games and Media demonstration; development blog and forum posts).** [web] [historical]
<https://en.wikipedia.org/wiki/Subversion_(video_game)> ·
<https://www.engadget.com/2007-01-04-subverison-to-deploy-procedural-city-generator.html>
(verified through search results; no conference talk was found, see below)

The independent studio's unreleased game, remembered for its generator "capable of building a
model city, complete with buildings, roads and highways", demonstrated at an Imperial College event
and in blog videos; Delay called procedural generation "utterly crucial to companies like
Introversion and basically ignored by the bigger boys in favour of banks of expensive artists".
*Bearing:* the precedent for Forge's situation (no art team, so the city is generated) and its
lesson: Subversion's city was convincing from the air and empty at the street, because nothing
below the block was designed. The plan below puts the lot, the facade module and the room in the
data model from the start.

**Ubisoft Toronto. "Watch Dogs: Legion – The Tools That Built London" (Ubisoft News, 2020); the
GDC 2021 sessions on Census and the procedural systems; Daniel Luka, "Watch Dogs Legion –
Industrial Building Kits" (ArtStation).** [web] [talk] [recent]
<https://news.ubisoft.com/en-us/article/4po3S9Pwp1YcgBmGPmQxAh/watch-dogs-legion-the-tools-that-built-london>
· <https://toronto.ubisoft.com/ubisoft-toronto-programmers-2021-game-developers-conference/> ·
<https://www.artstation.com/artwork/48kakl> (verified through search results)

The studio's own account names two systems, Census ("generates the city's inhabitants") and the
Player Attention System, and says procedural generation "required massive tuning to align output
with game-design goals". The city itself is hand-built from kits: the industrial kits "consist of
250+ modules that world level artists used to assemble many different variations of industrial
style buildings throughout London".
*Bearing:* a data point for the kit size (a few hundred modules per architectural family) and a
warning: Legion's procedural effort went into people, not streets. London was authored over the
real map, which is the OSM-derived route Forge can take for a real place and cannot for a seeded
one.

**Massive Entertainment. "Trade Secrets of Game City Building in The Division" (80.lv, with the
Snowdrop team, 2016); the Snowdrop GDC 2014 tech showcase.** [web] [talk] [still-current]
<https://80.lv/articles/division-pt-1> ·
<https://www.engadget.com/2014-03-20-gdc-2014-ubisoft-shows-off-its-divison-powering-snowdrop-engine.html>
(verified through search results)

Manhattan in Snowdrop: the tools team's building tool lets an artist "define a footprint for a
building, then specify which building kits (collections of models) it should use and then to select
from those for each of the wall tiles", "a rather quick and flexible way of creating the kinds of
buildings needed to recreate Manhattan"; the 2014 showcase's line was that "with a limited amount
of blocks, we can create huge and detailed worlds".
*Bearing:* the hybrid in production form eight years before the City Sample: a footprint, a kit
per building, a choice per wall tile. Forge's grammar automates exactly the choice the artist made
by hand, and the footprint-plus-kit data model is the one to copy.

**CD Projekt Red. "Art Direction Summit: Building Night City" (GDC 2022; Jakub Knapik, Kacper
Niepokólczycki) and "Building Night City: The Technology of 'Cyberpunk 2077'" (GDC); 80.lv's
interview on the world of Cyberpunk 2077.** [talk] [web] [recent]
<https://www.gdcvault.com/play/1027571/Art-Direction-Summit-Building-Night> ·
<https://www.gdcvault.com/play/1028734/Building-Night-City-The-Technology> ·
<https://80.lv/articles/interview-how-cyberpunk-2077-s-night-city-was-built-almost-entirely-by-hand>
(verified through search results)

The counter-example: Night City was "built almost entirely by hand", and "the closest the studio
came to procedural generation was a road tool that used splines to auto-place road, bridge, and
overpass meshes"; the art direction talk explains the "rule of contrast" that keeps its districts
distinct and the four chronological visual styles of its architecture.
*Bearing:* what hand authoring buys (districts that read at a glance, controlled chaos) is what a
generator must reproduce by rule: a district is a style set *and* a contrast rule against its
neighbours, and the spline road tool is the same road-profile-from-a-polyline step the plan below
has.

**Epic Games. "City Sample Project Unreal Engine Demonstration"; "City Sample Quick Start for
Generating a City and Freeway using Houdini"; the State of Unreal 2022 talks "The Matrix Awakens:
Generating a World / Creating a World"; 80.lv, "Breakdown: Creating The Matrix Awakens in Houdini &
Unreal Engine 5".** [docs] [talk] [web] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/city-sample-project-unreal-engine-demonstration>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/city-sample-quick-start-for-generating-a-city-and-freeway-using-houdini>
· <https://80.lv/articles/breakdown-creating-the-matrix-awakens-in-houdini-unreal-engine-5>
(verified through search results)

The most complete public city pipeline. The sample is "a technical demonstration of how you can use
procedurally generated data from SideFX's Houdini Engine in Unreal Engine 5 to create a working
simulated world"; "Houdini is used to handle all the upfront work of generating the city shape, the
road networks, connecting freeway system, building placement", and "the Rules Processor maps the
generated point cloud data from Houdini Engine to rules that tell Unreal Engine how to use that data
to populate the world". The breakdown gives the size: a city of 16 km² with "260 km of roads, 512 km
of sidewalks, 7000 unique buildings and 1248 intersections", "8 000 000 instances", built over a
year and a half by a team.
*Bearing:* the numbers the plan below is measured against, and the architecture to keep: the
generator emits *points with attributes* (a road centreline, a lot, a building footprint with a
style), and rules turn points into instances. Forge's `CityPlan` is that point cloud as typed
records, and its placement pass is the rules processor.

**Open implementations: ProbableTrain, "Map Generator" (TypeScript, LGPL-3.0/GPL-3.0, a tensor-field
city-map generator); Tobias Knerr et al., OSM2World (Java, MIT); Oleg Dolya (watabou), "Medieval
Fantasy City Generator" (web, 2017–2026); phiresky, "procedural-cities" (an overview and demo).**
[code] [web] [still-current]
<https://github.com/ProbableTrain/MapGenerator> · <https://github.com/tordanik/OSM2World> ·
<https://watabou.itch.io/medieval-fantasy-city-generator> ·
<https://github.com/phiresky/procedural-cities>

Read on GitHub: the map generator "procedurally generates images of city maps. The process can be
automated, or controlled at each stage give you finer control over the output", traces major and
minor roads from a field the user composes (grids, radial elements), cuts blocks and lots, and
exports "3D models of generated cities" as STL; OSM2World is "an Open Source converter that creates
three-dimensional models of the world in various formats from OpenStreetMap data" (glTF, OBJ, 3D
Tiles), the reference for an OSM-derived layout of a real place; watabou's generator partitions a
Voronoi-based plan into wards and fills them with an organic street pattern, the look #91 wants on
a map; phiresky's repository holds "a 15-page overview over various approaches to city modeling"
and a browser demo.
*Bearing:* the tensor-field tracer is small enough that a TypeScript hobby project ships it with
lots and buildings; its GPL licence makes it a reference to read, not a dependency. OSM2World is
how a Santa Cruz-shaped test city would be imported for the A/B against the generated one.

---

## 2. Blocks, lots and parcels; districts and landmarks

A block is a face of the planar road graph, inset by the street's half-width and the sidewalk. A
lot is a piece of a block with a frontage. Everything about a city's texture at walking speed —
how wide the houses are, whether corners are built, where the gardens are — is decided here, and
the literature is short because the answer is short.

**Carlos A. Vanegas, Tom Kelly, Basil Weber, Jan Halatsch, Daniel G. Aliaga, Pascal Müller.
"Procedural Generation of Parcels in Urban Modeling." *Computer Graphics Forum* 31(2)
(Eurographics 2012), 681–690.** [paper] [foundational] [still-current]
<https://onlinelibrary.wiley.com/doi/abs/10.1111/j.1467-8659.2012.03047.x> (DOI
10.1111/j.1467-8659.2012.03047.x; EG diglib
<https://diglib.eg.org/items/ce1eac67-658d-41b2-b937-15396114c1b2>; project
<https://twak.org/project/parcels/>; verified through search results)

The lot paper: "interactive procedural generation of parcels within the urban modeling pipeline,
performing a partitioning of the interior of city blocks using user-specified subdivision
attributes and style parameters", "both robust and persistent, able to map individual parcels from
before an edit operation to after an edit operation". Two subdivision schemes: recursive splits of
the block's *oriented bounding box* along its longer axis until the target area is reached (the
gridded, American look, with a rule that keeps every lot touching a street), and a *straight
skeleton* scheme that offsets the block boundary by the lot depth and cuts the strip perpendicular
to the frontage (the European look: lots of equal depth along the street, a back garden or a
courtyard in the middle). Parcels keep ids across edits.
*Bearing:* the lot step, in two modes selected by district: OBB for the grid, skeleton strips for
the old town and the village. Persistence matters for Forge for a different reason than editing:
a lot id must be stable across seeds' small changes so that a hand-authored landmark (D-035) stays
where the owner put it.

**Oswin Aichholzer, David Alberts, Franz Aurenhammer, Bernd Gärtner. "A Novel Type of Skeleton for
Polygons." *Journal of Universal Computer Science* 1(12), 1995, 752–761; with Tom Kelly's
campskeleton (Java, Apache-2.0) and CGAL's 2D straight skeleton package.** [paper] [code]
[foundational]
<https://www.jucs.org/jucs_1_12/a_novel_type_of> (verified through search results) ·
<https://github.com/twak/campskeleton>

The straight skeleton, "composed of pieces of angular bisectors that partition the interior of a
given n-gon in a tree-like fashion into n monotone polygons"; the same structure gives a block's
lot strips, a roof's ridges and hips from a footprint, and an offset polygon at any distance.
campskeleton, read on GitHub, is a "weighted straight skeleton implementation in java" that "allows
negative weights for offsetting in either direction", the code behind the procedural-extrusions
paper below.
*Bearing:* one geometry routine serves three steps (lot strips, roofs, block insets), so it is
worth writing carefully once in `forge-procgen` with a pinned event order (D-016); campskeleton is
the reference to test against, CGAL the second.

**Markus Lipp, Daniel Scherzer, Peter Wonka, Michael Wimmer. "Interactive Modeling of City Layouts
using Layers of Procedural Content." *Computer Graphics Forum* 30(2) (Eurographics 2011),
345–354.** [paper] [still-current]
<https://www.cg.tuwien.ac.at/research/publications/2011/lipp2011a/> (DOI
10.1111/j.1467-8659.2011.01865.x; author PDF
<http://peterwonka.net/Publications/pdfs/2011.EG.Lipp.CityControl.final.pdf>; verified through
search results)

Layouts that "combine the power of procedural modeling with the flexibility of manual modeling":
"transformation and merging operators for both topology preserving and topology changing
transformations based on graph cuts, which in combination with a layering system allows intuitive
manipulation of urban layouts using operations such as drag and drop, translation, and rotation".
*Bearing:* the model for overrides. A landmark, a park or a hand-drawn boulevard is a *layer* over
the generated layout, merged by a graph cut that re-seals the roads around it; that is how #85's
"procedural base with hand-authored landmarks and overrides" works without editing the generator's
output, and it maps onto D-035's layered records.

**Thomas Lechner, Pin Ren, Benjamin Watson, Craig Brozefski, Uri Wilensky. "Procedural modeling of
urban land use." *ACM SIGGRAPH 2006 Research Posters* (and Northwestern TR-2007-33); with Saskia
Groenewegen, Ruben Smelik, Klaas Jan de Kraker, Rafael Bidarra, "Procedural City Layout Generation
Based on Urban Land Use Models", *Eurographics 2009* short papers, 45–48.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/1179622.1179778> · <https://ccl.northwestern.edu/2006/TR-2007-33.pdf>
· <https://graphics.tudelft.nl/procedural-city-layout-generation-based-on-urban-land-use-models/>
(verified through search results)

Districts from behaviour: Lechner et al. generate "typical patterns of urban land use using
agent-based simulation" (developers, residents, industry competing for land by value, access and
noise), with "a painting interface to establish global developmental behavior, guide local
development trends, or directly set desired land use". Groenewegen et al. get "structurally
plausible cities from high-level, intuitive user input such as city size, location and historic
background" and report it "significantly faster than comparable agent-based software" by replacing
the agents with land-use rules.
*Bearing:* the district field. Agents are the faithful version; the rule version (a land value
from distance to the centre, the coast and the arterials, industry downwind and by the rail, the
old town at the founding point, parks where the slope or the stream forbids building) is what a
seeded generator wants, with the agent version as a later authoring mode. Either way the output is
a field the lot step samples, not a hand-drawn zoning map.

**Kevin Lynch. *The Image of the City*. MIT Press, 1960.** [book] [foundational]
<https://mitpress.mit.edu/9780262620017/the-image-of-the-city/> (verified through search results)

The five elements from which people build a mental map of a city — paths, edges, districts, nodes
and landmarks — from a study of Boston, Jersey City and Los Angeles; often called the most
influential twentieth-century text on city design.
*Bearing:* the acceptance vocabulary for #85, as the GDMC rubric is for settlements: a generated
plan is right when each element is present and legible — arterials that read as paths, the coast
and the rail as edges, districts that differ, squares and crossings as nodes, and landmarks placed
where paths end or bend (GTA V's "spatial compression" for sightlines is the same idea from the
studio side). The skyline is the landmarks' silhouette: a height field per district with a few
authored peaks.

---

## 3. Buildings: grammars, kits, constraint solvers, interiors

Three families produce buildings. Grammars refine a mass into facades, floors, bays and elements by
rules, and can make every building unique. Kits are finite sets of pieces on a grid that artists (or
rules) snap together, and make every building an assembly of instances. Constraint solvers (model
synthesis, WFC) fill a grid with tiles under adjacency rules and make the assembly itself the
search. Shipped cities use kits; research uses grammars; the recommendation uses grammars to drive
kits, with the solver kept for the places grammars handle badly.

**Peter Wonka, Michael Wimmer, François Sillion, William Ribarsky. "Instant Architecture." *ACM
Transactions on Graphics* 22(3) (SIGGRAPH 2003), 669–677.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/882262.882324> (DOI 10.1145/882262.882324; in `procedural.md`
§2; verified through search results)

Split grammars (a facade into floors, a floor into bays, a bay into wall and window) plus a
separate *control grammar* that distributes attributes over the building so that rules fire
coherently, and attribute matching to choose among applicable rules.
*Bearing:* the two-grammar separation is the design of the style set: the split grammar is shared
by every style; the control grammar (the attributes: floor heights, bay widths, window ratios,
roof type, materials) *is* the style, and a district hands its buildings a control grammar.

**Pascal Müller, Peter Wonka, Simon Haegler, Andreas Ulmer, Luc Van Gool. "Procedural Modeling of
Buildings." *ACM Transactions on Graphics* 25(3) (SIGGRAPH 2006), 614–623.** [paper]
[foundational] [still-current]
<https://dl.acm.org/doi/10.1145/1141911.1141931> (DOI 10.1145/1141911.1141931; in
`procedural.md` §4; verified through search results)

CGA shape: a grammar over oriented *scopes* with split, repeat and component-split operations,
snapping and occlusion queries so facades line up across mass-model parts, and terminal symbols
that insert assets. Demonstrated on Pompeii; the language inside CityEngine.
*Bearing:* the operation set of Forge's grammar, almost exactly: `split` along an axis with
absolute and relative sizes, `repeat` to fill a length with bays, `comp` to get the faces of a
mass, `insert` to place a module, plus occlusion and snap queries against the neighbours. CGA's
`insert` of an asset is already the terminal-as-module the recommendation relies on; the
difference is only that Forge makes *every* terminal a module and never emits raw geometry.

**Pascal Müller, Gang Zeng, Peter Wonka, Luc Van Gool. "Image-based Procedural Modeling of
Facades." *ACM Transactions on Graphics* 26(3) (SIGGRAPH 2007), article 85.** [paper]
[still-current]
<https://dl.acm.org/doi/10.1145/1276377.1276484> (DOI 10.1145/1276377.1276484; PDF
<https://homes.esat.kuleuven.be/~konijn/publications/2007/eth_biwi_00530.pdf>; verified through
search results)

Grammars from photographs: the method "automatically derive[s] 3D models of high visual quality
from single facade images", "combining the procedural modeling pipeline of shape grammars with
image analysis to derive a meaningful hierarchical facade subdivision", with as one application
"the automatic derivation of shape grammar rules from facade images to build a rule base for
procedural modeling technology".
*Bearing:* the facade grammar's shape — a subdivision into floors and tiles with symmetry and
repetition detected — is the data a style set stores per facade type; the paper is also how a
regional style (the Provençal three-storey house with its shutters and génoise cornice) would be
measured from a few photographs rather than guessed.

**Jerry O. Talton, Yu Lou, Steve Lesser, Jared Duke, Radomír Měch, Vladlen Koltun. "Metropolis
Procedural Modeling." *ACM Transactions on Graphics* 30(2), 2011.** [paper] [code]
[still-current]
<https://dl.acm.org/doi/10.1145/1944846.1944851> (DOI 10.1145/1944846.1944851; code, exported
from Google Code: <https://github.com/meshula/metropolis-procedural-modeling>; verified through
search results)

Grammars under constraints: an algorithm "for controlling grammar-based procedural models" that
"computes a production from the grammar that conforms to a high-level specification of the desired
production" by Markov-chain Monte Carlo over derivations, shown on "trees, cities, buildings, and
Mondrian paintings".
*Bearing:* the answer to "the grammar must hit this footprint, this height and this door position":
a derivation search, not a hand-tuned rule. For Forge the cheap form is enough at first (rules
parameterised by the lot; a few retries with a different seed when a constraint fails), with
Metropolis as the fallback for landmarks and awkward lots.

**Tom Kelly, Peter Wonka. "Interactive Architectural Modeling with Procedural Extrusions." *ACM
Transactions on Graphics* 30(2), 2011, 14:1–14:15.** [paper] [code] [still-current]
<https://dl.acm.org/doi/10.1145/1944846.1944854> (DOI 10.1145/1944846.1944854; tech-report PDF
<https://peterwonka.net/Publications/pdfs/2011.TOG.Kelly.ProceduralExtrusions.TechreportVersion.final.pdf>;
code in <https://github.com/twak/chordatlas> (Apache-2.0); verified through search results)

Buildings from footprints by "procedural extrusions": a plan-angle profile per footprint edge swept
by a weighted straight skeleton, giving "curved roofs, overhanging roofs, dormer windows, interior
dormer windows, roof constructions with vertical walls, buttresses, chimneys, bay windows, columns,
pilasters, and alcoves" through "a sweep plane algorithm to compute a two-manifold architectural
surface". chordatlas (read on GitHub) is "an urban procedural modeling and data fusion research
platform" holding this and the later BigSUR and FrankenGAN work.
*Bearing:* roofs. A kit cannot hold every roof over every lot shape; the straight skeleton gives
the ridge lines of any footprint, and the roof *modules* (tile field, eave, ridge, hip, gable end)
are placed along them. That is the one place where the grammar emits a generated mesh besides
modules, and it is the same skeleton routine as §2's lots.

**Michael Schwarz, Pascal Müller. "Advanced Procedural Modeling of Architecture." *ACM Transactions
on Graphics* 34(4) (SIGGRAPH 2015), 107:1–107:12; with Schwarz, Wonka, "Practical Grammar-based
Procedural Modeling of Architecture", *SIGGRAPH Asia 2015 Courses*.** [paper] [course]
[still-current]
<https://dl.acm.org/doi/10.1145/2766956> (DOI 10.1145/2766956) ·
<https://dl.acm.org/doi/10.1145/2818143.2818152> (verified through search results)

CGA++, which "grants first-class citizenship to shapes, enabling, within a grammar, directly
accessing shapes and shape trees, operations on multiple shapes, rewriting shape (sub)trees, and
spawning new trees", with "events" for "coordination across multiple shapes"; the motivation is
that in CGA "many context-sensitive tasks are precluded, not least because within the rules
specifying how one shape is refined, the necessary knowledge about other shapes is not available".
The course is the practitioners' summary of a decade of CGA.
*Bearing:* what the first grammar will lack and need within a year: rules that look at siblings
(align the windows of two wings, run a cornice around a corner, decide a stair's position from the
whole floor). Design the derivation as a tree of scopes with queries over it from day one, so the
CGA++ features are additions, not a rewrite.

**Joel Burgess, Nathan Purkeypile (Bethesda Game Studios). "Skyrim's Modular Level Design." GDC
2013, Level Design in a Day; the transcript on Burgess's blog and on Game Developer; the GDC 2016
follow-up "Modular Level Design of Fallout 4"; 80.lv, "Building Huge Open Worlds: Modularity, Kits
& Art Fatigue".** [talk] [web] [foundational] [still-current]
<http://blog.joelburgess.com/2013/04/skyrims-modular-level-design-gdc-2013.html> ·
<https://www.gamedeveloper.com/design/skyrim-s-modular-approach-to-level-design> ·
<https://www.slideshare.net/slideshow/gdc-2016-modular-level-design-of-fallout-4/59770460> ·
<https://80.lv/articles/building-huge-open-worlds-modularity-kits-art-fatigue> (verified through
search results)

The kit doctrine, from the studio that shipped Oblivion, Fallout 3 and Skyrim on it: a kit is a set
of pieces built to a shared *footprint* (a bounding box on the grid that every piece fits), designers
snap at half the footprint, the grid is kept as large as the design allows, pieces are made to
combine in more ways than the artist foresaw ("more than the sum of its parts"), and each
architectural family (Nordic ruin, dwarven, cave, farmhouse) is one kit with its own proportions.
The Fallout 4 talk extends it to exteriors, and the 80.lv article records the cost: art fatigue
when the same pieces are recognised.
*Bearing:* the grid rule and the fatigue warning. Forge's modules need a footprint grid the grammar
splits along (the recommendation sets it), and the fatigue is the owner's "repeating textures at
once" applied to geometry: it is fought with variety *within* the module (parameters the cook
varies: a sill's depth, a shutter's angle, dirt) and with the grammar's mixing, not with more
hand-made pieces.

**Epic Games. "City Sample Buildings" (Fab; 24 modular kits, 44 sample buildings) and the City
Sample's building construction; CG Channel's coverage of the release.** [docs] [web] [recent]
<https://www.fab.com/listings/4898e707-7855-404b-af0e-a505ee690e68> ·
<https://www.cgchannel.com/2022/04/download-epic-games-free-city-sample-assets-for-ue5/>
(verified through search results)

The kits behind the 7 000 buildings: "a set of 24 modular kits for creating custom 3D buildings,
plus 44 sample buildings", "over 2,000 individual meshes" with textures "between 2,048px and
8,192px"; in the city "each building in the city is made up of hundreds of instances" of those
meshes, assembled by Houdini's rules per footprint and style and rendered through Nanite.
*Bearing:* the ratio to plan for: about 85 meshes per kit, hundreds of instances per building,
7 000 buildings from two dozen kits. A Forge style set of 150–400 modules is in the same range,
and 44 sample buildings per 24 kits says how few *whole* buildings a kit needs authored: the
grammar makes the rest.

**Digital Extremes. Daniel Brewer, "Handling AI in procedural levels" (GDC 2013, AI Summit) and
"Managing Pacing in Procedural Levels in Warframe", *Game AI Pro* (online edition, 2021).** [talk]
[web] [still-current]
<https://www.gameaipro.com/GameAIProOnlineEdition2021/GameAIProOnlineEdition2021_Chapter07_Managing_Pacing_in_Procedural_Levels_in_Warframe.pdf>
(verified through search results)

Levels assembled from *tiles* (rooms with typed connectors) chosen by a template per mission type,
with the pacing and the navigation data solved per tile so that an assembled level plays like an
authored one.
*Bearing:* the interior precedent for #88 and #90: a room is a tile with connectors and its own
navigation data, and a building's navmesh is the union along the connectors. Modules that carry
their nav tags make the same true for Forge's buildings.

**Maxim Gumin. WaveFunctionCollapse (GitHub, 2016–, MIT); Isaac Karth, Adam M. Smith,
"WaveFunctionCollapse is Constraint Solving in the Wild", *FDG 2017*; Oskar Stålberg, "Wave
Function Collapse in Bad North" (Everything Procedural Conference 2018) and "Organic Towns from
Square Tiles" (IndieCade Europe 2019); Marian Kleineberg, "Infinite procedurally generated city
with the Wave Function Collapse algorithm" (GitHub, MIT); gridbugs, wfc (Rust crates, MIT).**
[code] [paper] [talk] [still-current]
<https://github.com/mxgmn/WaveFunctionCollapse> · <https://dl.acm.org/doi/10.1145/3102071.3110566>
(DOI 10.1145/3102071.3110566; verified through search results) ·
<https://www.gamedeveloper.com/game-platforms/how-townscaper-works-a-story-four-games-in-the-making>
(verified through search results) · <https://github.com/marian42/wavefunctioncollapse> ·
<https://github.com/gridbugs/wfc>

Read on GitHub: Gumin's program "generates bitmaps that are locally similar to the input bitmap",
builds on Merrell's model synthesis (the tiled form and the comparison are in `procedural.md` §2)
with a lowest-entropy heuristic and tile symmetries, and lists Bad North, Townscaper and Caves of
Qud among the games that ship it. Karth & Smith read it as "an example-driven image generation
algorithm where new images are generated in the style of given examples by ensuring every local
window of the output occurs somewhere in the input", operationally "a non-backtracking, greedy
search method", and place it among constraint solvers. Stålberg's Bad North talk shows islands
assembled "from handcrafted tilesets"; Townscaper adds the irregular grid — a hexagon of triangles,
pairs merged at random, everything subdivided into quads, the mesh relaxed — so that square tiles,
deformed into the irregular quads, make organic towns. Kleineberg's project is "an infinite,
procedurally generated city, assembled out of blocks using the Wave Function Collapse algorithm
with backtracking"; the Rust crates implement "wfc on arbitrary grids".
*Bearing:* not the primary building generator (a solver has no hierarchy: it cannot say "three
floors, a shop below, a cornice, then the roof" without the rules being smuggled into the tiles),
but two real uses: interiors, where filling a floor plan's grid with room-and-corridor tiles under
adjacency rules is exactly the problem; and #91's organic village, where Stålberg's relaxed
irregular grid is the best published answer to "modules on a grid, but no grid visible".

**Paul Merrell, Eric Schkufza, Vladlen Koltun. "Computer-Generated Residential Building Layouts."
*ACM Transactions on Graphics* 29(6) (SIGGRAPH Asia 2010).** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/1866158.1866203> (DOI 10.1145/1866158.1866203; verified through
search results)

Floor plans and the house around them: "automated generation of building layouts for computer
graphics applications", "motivated by the layout design process developed in architecture" — a
Bayesian network "trained on real-world data" proposes the rooms and their adjacencies, a
stochastic optimisation places them, and the exterior (walls, roof, windows) is derived from the
plan.
*Bearing:* the plan-first order for houses. For #88 the useful lesson is the data flow: rooms and
adjacencies first, walls from rooms, windows where rooms want light. Forge's grammar runs the
exterior first for towers and offices (the facade decides) and the plan first for houses, and both
must agree on the same grid.

**Ricardo Lopes, Tim Tutenel, Ruben M. Smelik, Klaas Jan de Kraker, Rafael Bidarra. "A Constrained
Growth Method for Procedural Floor Plan Generation." *GAME-ON 2010*.** [paper] [still-current]
<https://publications.tno.nl/publication/104066/Yar9HQ/lopes-2010-constrained.pdf> (verified
through search results)

Motivated by the fact that "building interiors in games are typically represented only by their
facade, because of the excessive costs it would entail to model all building interiors of a city
by hand": rooms grow on a grid from seeds inside the footprint under user constraints (areas,
adjacencies, connectivity), for several building classes and across connected floors.
*Bearing:* the cheapest floor-plan generator that respects a given footprint and a grid, which is
what a grammar-derived exterior hands it. It is the first interior method to implement: a grid
growth with a pinned order is deterministic and a few hundred lines.

**Fernando P. Marson, Soraia R. Musse. "Automatic Real-Time Generation of Floor Plans Based on
Squarified Treemaps Algorithm." *International Journal of Computer Games Technology*, 2010,
article 624817.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1155/2010/624817> (DOI 10.1155/2010/624817; verified
through search results)

Floor plans as treemaps: the squarified treemap algorithm splits the footprint into rooms of given
areas with good aspect ratios, in real time, with corridors added so that "all rooms can be
accessible from outside the environment", and semantic room data for agents.
*Bearing:* the office and apartment-block variant (rectangular footprints, repeated floors), where
a treemap per floor plus a core (stairs, lift, corridor) is enough and runs in microseconds per
floor when a room streams in (#88).

**Joost van Dongen. "Interior Mapping: A new technique for rendering realistic buildings." *Computer
Graphics International (CGI) 2008*.** [paper] [still-current]
<https://www.proun-game.com/Oogst3D/CODING/InteriorMapping/InteriorMapping.pdf> (verified through
search results)

Rooms without geometry: the shader "renders the interior of a building when looking at it from the
outside, without the need to actually model or store this interior"; "raycasting in the pixel
shader is used to calculate the positions of floors and walls behind the windows", and "the number
of rooms rendered does not influence the framerate or memory usage".
*Bearing:* #84 and #88 already name it as the first step: a shading class on the glass rows (D-026,
#41, #56). What building generation must add is the *room grid* the shader needs — floor height
and bay width per facade — which the module grid gives for free.

**David Luebke, Chris Georges. "Portals and Mirrors: Simple, Fast Evaluation of Potentially Visible
Sets." *ACM Symposium on Interactive 3D Graphics* 1995; with Seth Teller, Carlo Séquin,
"Visibility Preprocessing for Interactive Walkthroughs", *SIGGRAPH 1991*.** [paper] [foundational]
<https://luebke.us/publications/pdf/portals.pdf> (verified through search results)

Cells and portals: the model is divided "into cells and portals" (rooms and their openings), and
visibility is found at run time by clipping the view through the portals, with no precomputed PVS;
Teller & Séquin's is the precomputed form.
*Bearing:* what #88 needs the modules to carry: a door or window module *is* a portal (an opening
polygon in the module's frame), and a room a cell. With those tags the building's portal graph is
a by-product of assembly, and the probe cascades (D-036) get the room boundaries they need to stop
light leaking.

**Ubisoft Montréal. Assassin's Creed Unity's Paris: the interviews on scale and interiors
(TechRadar, 2014) and the architecture team's kit pages (ArtStation).** [web] [weak]
<https://www.techradar.com/news/gaming/inside-assassin-s-creed-unity-ubisoft-s-leap-of-faith-1268435>
(verified through search results; no technical talk on the buildings was found, see below)

The precedent for seamless interiors at city scale: "about a quarter of the buildings" were
enterable, with the character moving "seamlessly from outdoor to indoor scenes"; the buildings
were built by an architecture team from exterior and interior kits (the "Generic Landmark
buildings" kits on the artists' portfolios). The GDC talks on Unity that exist are about crowds
and networking.
*Bearing:* the ratio to aim for (a quarter enterable) and the confirmation that interiors were
kits too, with interior and exterior kits sharing one grid; nothing here is citable as method.

**SideFX. "Labs Building Generator" and "Building from Patterns" (SideFX Labs, free and
open-source; the Houdini tutorials "Building Generator", "Procedural building from modules").**
[docs] [code] [still-current]
<https://www.sidefx.com/tutorials/building-generator/> ·
<https://www.sidefx.com/tutorials/procedural-building-from-modules-in-houdini/> (verified through
search results) · <https://github.com/sideeffects/SideFXLabs>

The hybrid as a tool: the building generator, "given a basic 'block out' shape, and a named set of
building components, can generate detailed buildings", i.e. a mass model plus a kit of named
modules assembled by pattern rules per facade; SideFX Labs (read on GitHub) is "a free,
open-source, and artist-friendly toolset developed by SideFX" of "hundreds of Houdini Digital
Assets".
*Bearing:* the closest existing tool to the recommendation, and the reference for its
*interface*: a mass, a kit, facade patterns. Forge writes the same three things as data (P6) and
evaluates them itself, since Houdini cannot run in the shipped world.

---

## 4. Rendering a city of modules through the cluster DAG

This section has few new sources because Forge already measured most of it. The question is what
changes when a building stops being one 0.5–3 M-triangle mesh and becomes hundreds of module
instances: the instance count, the memory, the LOD at distance and the texture budget.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry."
SIGGRAPH 2021 *Advances in Real-Time Rendering in Games*; with the City Sample's "hundreds of
instances" per building and Forge's own record (`docs/demos/city-blocks.md`,
`docs/demos/meshlets.md`).** [talk] [recent]
<https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf>
(verified through search results; in `terrain-genesis.md` §5 and `gpu-geometry.md`)

Nanite's answer to a city is instancing of cluster-DAG meshes with per-cluster LOD and streaming;
the City Sample builds every building from kit instances and lets the DAG take each instance down
to a cluster or two at distance. Forge's record on the same idea: a million instances of twenty
props (25.8 M triangles in 612 k clusters, 983 MiB of 128 KiB pages, the instance 80 bytes since
#93) drawn in 1.06 ms of culls at 1440p with #38's cells of 64 instances, 3.38 ms for the whole
frame with everything on.
*Bearing:* a kit is *less* geometry than today's props, not more: 300 modules at 5–50 k triangles
are 1.5–15 M triangles, the size of five to twenty of today's props, so the pages of a whole style
set fit the 512 MiB pool with room to spare, and a village's kit at 2–5 k triangles per module is
a few tens of megabytes, resident. What grows is the instance table: a building of 300 modules is
300 rows; 5 000 buildings are 1.5 M instances (120 MB) and the city's props on top. That is the
scale the culls were built for, and #38's cells are where a building's proxy goes (below).

**Epic Games. "World Partition – Hierarchical Level of Detail (HLOD)", Unreal Engine 5
documentation.** [docs] [still-current] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---hierarchical-level-of-detail-in-unreal-engine>
(verified through search results)

HLOD "uses custom HLOD Layers to organize large amounts of Static Mesh Actors and generate a single
proxy mesh and Material", "to visualize unloaded World Partition grid cells, to reduce the number
of draw calls per frame, and to increase performance"; a layer's type is "Instancing, Merged Mesh,
Simplified Mesh, or Approximated Mesh", and layers are chained for a sequence of proxies.
*Bearing:* the far-building answer #86 asks about. A module's DAG bottoms out at one cluster per
module, so a far building is still hundreds of roots; the fix is a *merged proxy per building*
cooked from the assembled modules through the same DAG cook (a 50 k-triangle building cooks in
about 0.1 s at the city's 12 s per 8 M triangles), swapped in when the building's projected size
falls below a threshold, and a second proxy per block (D-037's cells) beyond that. In Forge the
building is a #38 cell: the instance cull already has "hidden whole" and "opened again" states per
cell; "drawn as its proxy" is the third, and the swap distance is the number to measure (the
ꟻLIP at the swap, `tools/compare.sh`).

**Materials and texture budget** (a synthesis; the sources are `vegetation-materials.md` §4 and #83,
with the City Sample's texture sizes above). A kit is the case trim sheets were made for: modules
share the regional material set (2–4 trim sheets, 6–10 tileables, a decal atlas, vertex-paint masks
per that research), and the grammar's semantic tags on faces — cornice, sill, lintel, jamb, quoin —
become the module's authored UVs onto the trim rows once, at module cook, instead of a UV pass per
building. The City Sample spends 2 k–8 k textures on 2 000 meshes; Forge's set for a district is
those two to four sheets plus tileables, a few hundred megabytes in BC7 at most, and the same for
every building of the district. Coherence comes from the set, variety from the module parameters
the cook varies and from the decals; the owner's repeating-texture eye is answered by hex-tiling on
the tileables and bombing on the decals, as that research recommends, and by never placing the
same module variant twice in a row along a street (a grammar rule, free).

**Memory of unique against modular** (a synthesis; the numbers are Forge's). Today's twelve
buildings average 1.4 M triangles and 44 MB of pages each; a unique CGA building at even 100 k
triangles is about 4 MB of pages, so 5 000 unique buildings are 20 GB — streamable through D-025's
pool, but every page is read from disk on first sight and nothing is shared. The same 5 000
buildings from a 300-module kit are the kit's pages (tens to hundreds of megabytes, resident after
the first street) plus 120 MB of instances. The proxies add one small mesh per building (5 000 ×
50 k triangles = 250 M triangles, about 10 GB at today's 40 bytes per full-detail triangle — too
much; proxies are cooked at 5–10 k triangles, so 1–2 GB, streamed by cell, or per block rather than
per building where blocks are dense). These estimates are marked as such and are the first thing
the demo measures.

---

## 5. What professional engines do

**Epic Games. "Procedural Content Generation Overview" and "Procedural Content Generation
Framework in Unreal Engine" (PCG); "City Sample PCG for Unreal Engine"; "City Sample gets a major
update with PCG and Unreal MCP workflows".** [docs] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/procedural-content-generation-overview>
· <https://dev.epicgames.com/documentation/unreal-engine/city-sample-pcg-for-unreal-engine> ·
<https://www.unrealengine.com/learning/city-sample-gets-a-major-update-with-pcg-and-unreal-mcp-workflows>
(verified through search results)

PCG is "a toolset for creating your own procedural content and tools inside Unreal Engine", for
content "ranging from Asset utilities, such as buildings or biome generation, up to entire worlds",
a node graph in which "spatial data flows into the graph from a PCG Component in your Level and is
used to generate points" that carry "transforms, bounds, color, density, steepness, and seed"; it
"shipped as an experimental feature in 5.2 and became production-ready in 5.3" with "both
in-editor tools and a runtime component". The City Sample's later update rebuilds its city with
PCG in place of the Houdini step.
*Bearing:* Unreal moved the City Sample's generation from an external DCC into the engine within
two years, as a graph over points with attributes evaluated in the editor and at run time. That is
the direction Forge takes from the start: `CityPlan` as typed points, rules as data (P6),
evaluated in `forge-procgen` on both the client and the server (P8).

**Unity Technologies. No built-in city or building generator: ProBuilder for in-editor geometry;
third-party generators on the Asset Store (BuildR 3, ProGen, Procedural Building Builder).**
[docs] [web] [still-current]
<https://docs.unity3d.com/Packages/com.unity.probuilder@3.0/manual/index.html> ·
<https://discussions.unity.com/t/released-buildr-3-procedural-building-and-city-generator/791614>
(verified through search results)

Unity has nothing native for cities. ProBuilder is a modelling tool; BuildR is a paid asset that
"uses multi-core procedural mesh generation" and generates "endless buildings with complex
explorable interiors" in the editor or at run time, one of several such assets.
*Bearing:* confirms that the layer Forge is about to write does not exist off the shelf in any
engine's core; the Asset Store tools are single-building generators without a layout, which is why
the recommendation starts from the plan and treats the building generator as one consumer of it.

**Esri. ArcGIS CityEngine: "Export DATASMITH (Unreal and Twinmotion)", "Export models overview"
(FBX to Unity); "ArcGIS CityEngine for Unreal Engine" (formerly Vitruvio; GitHub, Apache-2.0
source, licence terms in the README).** [docs] [code] [still-current]
<https://doc.arcgis.com/en/cityengine/2025.0/help/help-export-unreal.htm> (verified through search
results) · <https://github.com/Esri/cityengine_for_unreal>

The commercial grammar pipeline's exits: in the Datasmith export "each exported mesh is represented
as an actor in Unreal, with each unique mesh exported as a static mesh and reused if instancing is
enabled"; to Unity by FBX, where "an FBX instancing feature allows for small file sizes and high
frame rates". Read on GitHub, the plugin "enables the use of ArcGIS CityEngine shape grammar rules
in UE5 for the generation of procedural buildings" from rule packages, in the editor or at run time,
"free for personal, educational, and non-commercial use" while "commercial use requires at least
one commercial license of the latest CityEngine version installed in the organization".
*Bearing:* CityEngine is the proof that CGA runs in an engine at run time, and its licence is the
reason Forge writes its own: a grammar interpreter over scopes is a small crate, and the rule
packages' format (rules + assets + attributes) is the shape of a Forge style set.

**SideFX. Houdini Engine for Unreal (GitHub README) and the Houdini Engine plug-ins; SideFX Labs.**
[docs] [code] [still-current]
<https://github.com/sideeffects/HoudiniEngineForUnreal> · <https://github.com/sideeffects/SideFXLabs>
· <https://www.sidefx.com/products/houdini-engine/> (the product page: verified through search
results)

Read on GitHub: the plug-in "brings Houdini's powerful and flexible procedural workflow into Unreal
Engine through Houdini Digital Assets"; "artists can interactively adjust asset parameters inside
the editor and use Unreal assets as inputs. Houdini's procedural engine will then 'cook' the asset
and the results will be available in the editor without the need for baking", with binaries "for
UE5.8, UE5.7". It is an editor-time pipeline (the Far Cry 5 and City Sample model): the game ships
the cooked result.
*Bearing:* Houdini is the right tool when a team has it and the world is baked; Forge's world is
generated from a seed on the client and the server, so the generator must be the engine's own.
What is worth copying is the contract: a generator is an asset with parameters and typed outputs
(points, curves, meshes with attributes), cooked on demand and cached by its inputs, which is what
`terrain-genesis.md`'s stages already are.

---

## 6. The decision #86 asks for: grammar, kits or hybrid

**The three options, honestly.**

- *Grammar-generated unique geometry* (CGA as CityEngine uses it): every building is its own mesh,
  windows and cornices cut into it by rules. Strengths: unlimited variety, exact fit to any lot,
  one system. Costs: no instancing, so memory scales with the city (§4's 20 GB for 5 000 buildings
  at modest detail); no natural unit for destruction (#89), for rooms (#88) or for navigation
  (#90); every building must be cooked through the DAG (0.1–5 s each at today's cook rate, hours for
  a city, cached on disk); and the look comes entirely from the rules, which the studios' experience
  says converges on a recognisable sameness unless assets are inserted anyway.
- *Artist kits, hand-placed or rule-placed* (Bethesda, Massive, Ubisoft Toronto, the City Sample):
  a few hundred modules per family on a footprint grid. Strengths: instancing (memory, streaming,
  the DAG per module), a unit for interiors, breakage and navigation, coherence by construction
  (every piece shares the style's proportions and trims), and proven at the scale #84 aims at.
  Costs: an art team to model the kits and to assemble the buildings; art fatigue when a piece is
  recognised; and lots of odd shapes that the kit cannot fill.
- *Hybrid: a grammar decides, modules are the terminals* (Snowdrop's building tool with the
  choice automated, SideFX Labs' building generator, the City Sample's Houdini rules): the split
  grammar refines the mass into floors and bays and *inserts* a module per bay, the roof from the
  skeleton, the corners and cornices from the context queries; the modules are instances. It keeps
  the kit's strengths and replaces the artist's assembly with the grammar; what it needs from an
  art team is the kit, not the buildings.

**The argument for Forge.** Forge has no art team and a procedural-first charter (P6: rules are
data), an owner who sees repetition at once and wants coherence, and near targets that are a
Provençal village and a Mediterranean town before any Manhattan. The hybrid is the only option
that fits all four: rules make the buildings (no assembly labour), modules make them instanced,
breakable and enterable (#88, #89, #90), and the missing art team is answered by *generating the
kit* at cook time — Forge's `building()` already cuts window recesses, ledges and cornices into a
displaced box from parameters; the same code cut into one-bay modules, with the profile-extruded
mouldings the materials research asked for and a few variants per module, is a style set. A
Provençal set (rubble stone and ochre plaster, a génoise under the eave, canal tiles, shutters,
narrow bays) and a downtown set (curtain wall, spandrel, mullion, a plinth of stone) are two
parameter files over the same module generators. Coherence is then a property of the style set,
and repetition is fought where it is cheapest: in the cook's variants, in the grammar's rules, in
the trims and decals.

**Where the hybrid is weak, and what to do about it.** Odd lots: the grammar's mass model is a
union of boxes fitted to the lot (Kelly & Wonka's extrusions for the roof), and the corner and
infill modules absorb the remainder; when a facade length is not a whole number of bays, a
*filler* module family of 0.5 m steps takes the rest, as kits do. Curved facades: the module grid
bends along the frontage's arc (Townscaper's deformed quads) at village scale, and downtown curves
are rare enough to be landmarks. Unique landmarks: hand-modelled or grammar-generated as single
meshes, exactly today's props, placed by a D-035 override. Interiors: the grid rule — exterior wall
modules occupy the outer half-metre of the footprint grid, interior modules start inside it, floor
heights are per style and shared by both — is the one constraint that must be fixed before the
first module is generated, because it cannot be retrofitted.

---

## 7. The real coast and river: from elevation and hydrography data to a flat city and back

A city generated on a flat plane can still take its coastline and its river from a real place, so that
moving it onto Forge's planet later is a placement, not a redesign. Forge already holds the elevation;
what the city adds is the hydrography and a frame.

**Copernicus DEM GLO-90, as Forge already uses it (D-058, "The tour's region at 90 m").** [data]
(read in the repository)
`docs/DECISIONS.md`, D-058; `CREDITS.md`

With the owner's go of 2026-10-10, Forge mosaicked 25 tiles of GLO-90 over 41°–47° N and 5°–10° E, read
over ETOPO as a world-file region; the sea takes ETOPO's depths. The credit line required by the data
is in `CREDITS.md`.
*Bearing:* the coast of any city site on that window is already a contour of a grid Forge loads. At
90 m a beach, a quay or a river mouth is three to five samples wide, so the coast is taken as a
smoothed polyline and the city's detail is generated, not sampled.

**Copernicus Land Monitoring Service, "EU-Hydro River Network Database 2006–2012 (vector), Europe".**
[data] (read)
<https://land.copernicus.eu/api/en/products/eu-hydro/eu-hydro-river-network-database>

A photo-interpreted river network with a drainage model from EU-DEM: catchments, drainage lines,
nodes, lakes and wide rivers as polygons, a coastal polygon layer; delineated at 1:30 000 with a 1 ha
minimum mapping unit; version 1.3 files. Access is free and open under Regulation (EU) 1159/2013,
with conditions: name the source, mark adaptations as such, imply no EU endorsement, acknowledge EU
funding; the data remain EU property.
*Bearing:* the river's centreline and its wide-river polygon (the bankfull width where the river is
wide) for a European site, under terms a shipped game can meet with a credit line.

**HydroSHEDS, "HydroRIVERS".** [data] (read)
<https://hydrosheds.org/page/hydrorivers>

A global vector river network derived from the HydroSHEDS grids at 15 arc-seconds, keeping rivers with
a catchment of at least 10 km² or a mean flow of at least 0.1 m³/s, with reach lengths, distances to
the outlet, a river order and an estimated long-term mean discharge. The page says the data are free
for scientific, educational and commercial use under the HydroSHEDS licence agreement (whose full
text was not read).
*Bearing:* the discharge per reach, which the regional curves of `rivers.md` (D-041) turn into a
channel width and depth anywhere on the planet, not only in Europe.

**OpenStreetMap Foundation, "Copyright and License".** [web] (read)
<https://www.openstreetmap.org/copyright>

OSM's data are under the Open Database License: credit OpenStreetMap, say the data are under the ODbL,
and distribute anything altered or built upon the data only under the same licence.
*Bearing:* OSM is the best map of a real city's quays and streets, and the worst source for a shipped
world's coastline because of the share-alike clause (how it applies to a game's "produced work" was not
checked here). Use it as a reference to look at; take the shapes from GLO-90 and EU-Hydro.

**From data to a flat city and back** (a synthesis; the steps are this note's):
1. **Pick the window** on the GLO-90 grid: an origin in latitude and longitude, a rotation, a size
   (8–16 km). Store them in the `CityPlan` as its `GeoFrame` (D-004's f64 frame).
2. **Coast:** the sea-level contour of the DEM, cleaned of islands smaller than a block, smoothed and
   resampled at 5–10 m, then corrected against EU-Hydro's coastal polygon. Bathymetry from ETOPO, as
   the planet does.
3. **River:** EU-Hydro's centreline (or HydroRIVERS outside Europe), its discharge, and from it the
   bankfull width and the floodplain by `rivers.md`'s curves and D-041's exaggeration; the river's
   urban form (quays, terraces, a wide park bed) is then a design choice per reach (§9).
4. **Flatten for the first city:** keep the coast and river polylines in local metres, replace the
   land's heights by a city plane plus a gentle rim (a few metres of rise inland so streets drain
   toward the water), and record the replaced heights. Generation runs on the plane.
5. **Place on the planet later:** project the same polylines onto the planet's tiles through the
   `GeoFrame`, blend the city plane into the real heights over a margin, and let the lots on slopes take
   terraces. Because roads, blocks and lots are generated from the coast, the river and fields, a city
   regenerated on the real heights differs only where slope rules fire; that difference is the number
   to measure.

---

## 8. Waterfronts and harbours

A waterfront is a section profile swept along the coast (water, edge, promenade, buildings), plus
structures placed at rule spacing: stairs to the water, pontoons, bollards, ladders, bathing decks. The
sources below give the profiles and the numbers.

**Wikipedia, "HafenCity" (Hamburg).** [web] (read)
<https://en.wikipedia.org/wiki/HafenCity>

About 2.2 km² of former port, planned for some 12 000 residents and 40 000 workplaces. Dikes were judged
too costly and too intrusive; instead the new roads and public spaces sit on sand terraces more than
7.6 m above normal high tide, and shore-side buildings are waterproofed up to the raised road level.
The Elbphilharmonie, a concert hall on top of an old warehouse, opened on 11 January 2017.
*Bearing:* the **flood terrace** profile: the street and the ground floors on a raised plinth, the
promenade and the quay at water level, allowed to flood. It reads as a city of stairs and levels down
to the water, and it needs no wall between the city and the sea.

**Wikipedia, "Copenhagen Harbour Baths".** [web] (read)
<https://en.wikipedia.org/wiki/Copenhagen_Harbour_Baths>

Four harbour baths; Islands Brygge, the first (2002 season, a permanent bath in 2003), has five pools
for 600 people, up to 7 m deep, with two children's pools, two 50 m pools and a diving pool with 3 m and
5 m boards. Water quality is monitored and published; unusually heavy rain can spill sewage into the
harbour and close the baths for a couple of days until the tides exchange the water.
*Bearing:* a harbour bath is a module family (floating deck, pool frames in the water, diving tower,
ladders, a lifeguard post) placed on a quay; and its open or closed state is a visible output of the
city's simulation (rain, then a red flag), which is the owner's "live data" idea in its simplest form.

**Ville de Paris, "Baignade dans la Seine : comment est mesurée la qualité de l'eau ?" (updated
13 July 2026); press coverage of the 2025 season.** [web] (read; the season's dates and the 10 mm rule
from search records)
<https://www.paris.fr/pages/baignade-dans-la-seine-comment-sera-mesuree-la-qualite-de-l-eau-31118>

Three Seine bathing sites (Bras Marie, Bercy, Grenelle) opened on 5 July 2025; the checks use
*E. coli* and enterococci, green or red flags, and a rain event may close the sites preventively for
24–36 hours; the Austerlitz basin stores 50 000 m³ of storm water. Press reports give the opening rule
as no bathing after more than 10 mm of rain in 12 hours until results arrive (search records).
*Bearing:* the river bath needs a storm-water story: a visible retention basin or sponge park
upstream, and a flag that follows the rain. The EU Bathing Water Directive (2006/7/EC) sets the
thresholds by percentile for both indicators (the annex's numbers were not read, see Checked and left
out).

**Basel, swimming in the Rhine (Basel tourism and Swiss Lifesaving Society advice, as reported).**
[web] (search record; basel.com refused the fetch)
<https://www.basel.com/en/activities-excursions/swimming-rhine>

Swimmers float about 1.8–3 km through the city with their clothes in a waterproof "Wickelfisch" bag,
entering below the Tinguely museum and leaving by stair exits on the Kleinbasel bank before the
Johanniterbrücke; the city advises swimming only above 18 °C water and below set river levels; the
Rheinbad bathhouses offer showers and changing rooms.
*Bearing:* river swimming needs no pool, only **stairs and exits at rule spacing** along a quay, a
bathhouse every kilometre or two, and a current the player can feel; the river's flow field
(`water.md` §3) is already the gameplay.

**Wikipedia, "Old Port of Marseille".** [web] (read)
<https://en.wikipedia.org/wiki/Old_Port_of_Marseille>

About 200 000 m² of harbour with about 3 500 berths, largely pedestrianised in 2013 after a competition
won by Michel Desvigne with Foster and Partners.
*Bearing:* the **marina quay** profile: a wide paved quay with almost nothing on it, finger pontoons
and masts as the texture, and the city's main square being the water itself.

**Wikipedia, "Promenade des Anglais"; French Wikipedia, "Promenade du Paillon".** [web] (read)
<https://en.wikipedia.org/wiki/Promenade_des_Anglais> ·
<https://fr.wikipedia.org/wiki/Promenade_du_Paillon>

The Promenade des Anglais runs about 7 km along the sea from the airport to the Quai des États-Unis,
used by cyclists, skaters and families. The Promenade du Paillon is a park over the covered Paillon
river: 1.2 km opened on 26 October 2013, extended by 1 km in October 2025 to 20 ha, with a 3 000 m²
water mirror of 128 jets, 1 600 trees, 6 000 shrubs and 50 000 perennials.
*Bearing:* two profiles: the **seafront promenade** (a long straight, a beach below the sea wall, one
row of trees or palms, seating facing the sea) and the **covered-river park** (a linear park whose
water feature recalls the river beneath). A water mirror is a cheap, spectacular shader surface.

**De Urbanisten, "Watersquare Benthemplein" (Rotterdam, 2013).** [web] (read; the volume from search
records)
<https://www.urbanisten.nl/work/benthemplein>

Three sunken basins collect rain: two shallow ones for the surroundings, one deep one that is a sports
pit and a stage when dry; opened on 4 December 2013. Other pages give its storage as about 1 700 m³
(search records).
*Bearing:* a square that is also a basin is a waterfront idea moved inland, and the same "rain fills
it, players see it" state as the bath flags.

**Waterfront profiles for the generator** (a synthesis, the dimensions *(estimate)* to tune on the map):

| Profile | Section, water to buildings | Structures at rule spacing | Floods? |
|---|---|---|---|
| Quay wall | water, 2–4 m wall, 8–20 m paved quay, trees, street | stairs every 100–200 m, bollards 10–15 m, ladders 30–50 m, lamps | no |
| Flood terrace (HafenCity) | water, low promenade at +1–2 m, steps or ramps, terrace at +6–8 m with the street | stair landings, pontoons, kiosks on the low level | the low level |
| Seafront promenade (Nice) | beach 20–50 m, sea wall 2–4 m, promenade 10–20 m, one tree row, buildings | beach stairs every 100–150 m, showers, kiosks | storms only |
| Marina quay (Marseille) | pontoons and moorings, 15–30 m open quay | finger pontoons per boat length, service bollards | no |
| Harbour or river bath | a deck on the quay or floating, pools in the water | diving tower, ladders, changing cabins, a flag | closes after rain |
| Water square (Rotterdam) | sunken basins in a square | gutters, steps, a sports pit | by design |

---

## 9. A river through the centre

**City of Paris pages on the Seine (as reported) and Wikipedia, "List of crossings of the Seine".**
[web] (bridge count read; the 13 km from search records)
<https://en.wikipedia.org/wiki/List_of_crossings_of_the_Seine> ·
<https://www.paris.fr/dossiers/la-seine-coeur-battant-de-paris-216>

Paris has 37 bridges over the Seine, three of them pedestrian-only and two rail bridges, along about
13 km of river inside the city. That is one crossing every 350 m or so *(estimate)*.
*Bearing:* the **bridge spacing rule** of a river city's centre: 300–500 m between crossings in the
core, wider outside, with footbridges filling the gaps. A river with crossings farther apart than a
ten-minute walk splits a city in two.

**Parc Rives de Seine (Paris): the closure of the right-bank expressway, 2016.** [web] (search record)
<https://www.weka.fr/actualite/developpement-durable/article/paris-la-pietonnisation-des-berges-rive-droite-officialisee-par-un-arrete-municipal-43476/>

The Georges-Pompidou riverside road was closed to cars over 3.3 km of the low quay in October 2016,
after left-bank stretches in 2012–2013; the order kept emergency and police vehicles allowed on it.
*Bearing:* the low quay is a park that doubles as an emergency route: a 4 m clear strip is part of the
profile, not an exception to it.

**French Wikipedia, "Berges du Rhône" (Lyon).** [web] (read)
<https://fr.wikipedia.org/wiki/Berges_du_Rh%C3%B4ne>

About 5 km of riverside promenade from the Parc de la Tête d'Or to the Parc Henry-Chabert, about 10 ha,
finished in 2007 on quays that had been car parks (cars left in 2005).
*Bearing:* a river park is long and narrow: 10 ha over 5 km is 20 m of average width *(estimate)*,
enough for a walk, a cycle path, lawns and moored boats.

**Room for the River Waal (Nijmegen), EEA Climate-ADAPT case study.** [web] (read)
<https://climate-adapt.eea.europa.eu/en/metadata/case-studies/room-for-the-river-waal-2013-protecting-the-city-of-nijmegen>

The Lent dike was moved about 350 m inland, a side channel 150–200 m wide and 3 km long was dug, and
the island between became an urban river park reached by an extended bridge and new ones; flood levels
fell by 35 cm (27 cm targeted); finished in 2016; the channel is used for rowing and sailing.
*Bearing:* the **flood park** profile for a river with a large flood: the wide bed is the park, and it
floods on purpose. It is the honest form for a Mediterranean river whose flood is fifty times its
mean flow.

**French Wikipedia, "Île de Nantes"; "Navibus".** [web] (read)
<https://fr.wikipedia.org/wiki/%C3%8Ele_de_Nantes> · <https://fr.wikipedia.org/wiki/Navibus>

The Île de Nantes is 4.9 km long and up to 1 km wide, tied to the banks by 13 bridges; its former
shipyards hold the Machines de l'île and the Titan crane. Navibus is Nantes' river shuttle: three lines,
six stations, 2.36 km in all. N1 crosses the Loire in 6 minutes over about 1.4 km, every 10 minutes at
weekday peaks and 20 otherwise, with 95-passenger boats carrying 10 bikes (693 200 trips in 2019); N3
is a 160 m Erdre crossing on demand with a 12-passenger electric catamaran on hydrogen fuel cells and
batteries.
*Bearing:* a river taxi-boat is a **crossing**, not a line: a pontoon each side, a boat sized for 12–100
people and some bikes, a headway of 10–20 minutes, placed where the bridges are far apart or the
crossing links two attractions.

**Wikipedia, "Copenhagen Harbour Buses"; "Amsterdam IJ Ferries"; "Batobus"; "Vaporetto".** [web] (read)
<https://en.wikipedia.org/wiki/Copenhagen_Harbour_Buses> · <https://en.wikipedia.org/wiki/Amsterdam_IJ_Ferries> ·
<https://en.wikipedia.org/wiki/Batobus> · <https://en.wikipedia.org/wiki/Vaporetto>

Copenhagen's harbour buses run three routes and 11 stops with seven electric boats delivered in 2020.
Amsterdam's GVB ferries F1–F9 are free for pedestrians and cyclists; the Buiksloterweg ferry runs two
boats in tandem every 6 minutes by day, one every 12 minutes after 21:00. Paris's Batobus is a
tourist service of nine stops with boats of 150–200 people; Voguéo, a commuter river service, was
trialled from June 2008 to December 2010. Venice's ACTV water buses carry 210–220 people (vaporetti) or
154 (motoscafi); a Series 90 vaporetto is 23.9 m by 4.2 m.
*Bearing:* three river services, three shapes: **crossings** (Navibus, IJ ferries) that run all day
and carry bikes; **a river line** along the water with stops (harbour buses, vaporetti); **a leisure
loop** (Batobus). Commuter lines along a river struggle (Voguéo) unless the river is the fastest route;
crossings succeed. Boat sizes for the module kit: 12, 100 and 200 people.

**River sections for the generator** (a synthesis, *(estimate)*): a **quay river** in the old centre
(walls, a low promenade that floods, bridges every 300–500 m); a **park river** outside it (the bed
widened into lawns and gravel bars that flood, as in Nijmegen, crossings every 600–1 000 m with boat
crossings between); a **harbour mouth** (the river meeting the harbour basins and the sea, with the
river line and the crossings converging on it). The channel's width and depth come from `rivers.md`'s
regional curves with D-041's exaggeration, the reach type from its slope; in a city, weirs hold the
water level in pools between them, which is also what makes boats possible on a shallow river.

---

## 10. Parks, green infrastructure, rooftops and visible nature

**European Environment Agency, "Who benefits from nature in cities?" (Briefing 15/2021, 1 February
2022).** [web] (read)
<https://www.eea.europa.eu/en/analysis/publications/who-benefits-from-nature-in-cities-social-inequalities-in-access-to-urban-green-and-blue-spaces-across-europe>

Green infrastructure covers on average 42 % of city area across 38 EEA countries (from 6.8 % to
95.8 %), but publicly accessible green space only about 3 % of the city area on average; mean urban tree
cover is 30 %. The WHO asks for at least 0.5–1 ha of public green space within 300 m of home; less than
half of Europe's urban population has it (over 80 % in Stockholm, under 20 % in Heraklion). Data from
2012.
*Bearing:* "green" and "a park I can use" are different numbers. A generated city reports both: total
green (gardens, courtyards, roofs, street trees, parks) and publicly accessible park area within 300 m
of each home.

**Cecil C. Konijnendijk, "Evidence-based guidelines for greener, healthier, more resilient
neighbourhoods: Introducing the 3–30–300 rule", *Journal of Forestry Research* 34(3), 821–830 (online
2022, DOI 10.1007/s11676-022-01523-z).** [paper] (read)
<https://pmc.ncbi.nlm.nih.gov/articles/PMC9415244>

Three well-established trees in view from every home, school and workplace; at least 30 % tree canopy
in every neighbourhood; no more than 300 m to the nearest public green space from every residence (the
paper cites the WHO's suggestion of at least 1 ha).
*Bearing:* three checks a generator can run on its own output: trees visible from each dwelling's
windows (a ray test from the facade module's window), canopy share per district, and distance to the
nearest park of 1 ha or more.

**City of Vienna, "Statistics – urban area" (2020).** [web] (read)
<https://www.wien.gv.at/en/administration/statistics-urban-area>

Of 41 487 ha: green space 45.0 %, built-up 35.9 %, roads and streets 14.5 %, water 4.6 %.
*Bearing:* a real land budget of a green European city of two million, to compare the generated map
against (Vienna's area includes forest and farmland at its edge; an 8 km dense city will be less green
and more built).

**EEA Climate-ADAPT, "Berlin Biotope Area Factor".** [web] (read)
<https://climate-adapt.eea.europa.eu/en/metadata/case-studies/berlin-biotope-area-factor-2013-implementation-of-guidelines-helping-to-control-temperature-and-runoff/>

BAF = ecologically effective surface area ÷ total land area, each surface weighted: sealed asphalt 0,
vegetation on the natural soil 1, extensive green roof 0.5, semi-intensive 0.7, intensive 0.8, vertical
greenery not connected to the ground 0.7 per m² (factors refined in December 2019). Targets: 0.6 for
residential and public areas, 0.3 for commercial and administrative ones. In use since 1994.
*Bearing:* the best single green check for a lot: the generator sums its surfaces by type and must
reach the target for the lot's use, and it can choose how (a courtyard garden, a green roof, climbers).

**EEA Climate-ADAPT, "Four pillars to Hamburg's green roof strategy"; Copenhagen's green roof rule (as
reported).** [web] (Hamburg read; Copenhagen search record, stateofgreen.com refused)
<https://climate-adapt.eea.europa.eu/en/metadata/case-studies/four-pillars-to-hamburg2019s-green-roof-strategy-financial-incentive-dialogue-regulation-and-science>

Hamburg's strategy (2014) aims to green at least 70 % of new buildings and suitable renovated flat or
gently pitched roofs, with about 20 % of the new green roofs open for recreation, subsidising 30–60 %
of costs; by 2020 the metropolitan total was about 168 ha. Copenhagen has required green roofs on new
buildings with roofs under 30° since about 2010, with exceptions for aesthetics and structure (search
records).
*Bearing:* the roof rule for a style set: flat or nearly flat roofs are green by default (extensive),
a fifth of them usable; pitched roofs keep their material.

**Urban Redevelopment Authority (Singapore), circular DC14-12, "LUSH 2.0: Landscape Replacement Policy
for Strategic Areas" (12 June 2014; superseded 2017).** [docs] (read)
<https://www.ura.gov.sg/guidelines/archived-circulars/dc14-12/>

New developments must provide replacement landscaped areas at least equal to the site area, on the
ground, on roofs or on mid-level sky terraces, and at least 40 % of that area must be greenery.
*Bearing:* the contrast case and a useful accounting: green measured as a multiple of the site,
counted at any height. Bosco Verticale's balcony trees (`procedural-content.md` §4.3: balconies about
3.3 m deep) are the European form of the same idea; both read as "architecture covered in plants",
which the owner's brief wants as accents, not as the city's norm.

**Rotterdam's roofs: MVRDV, "Rooftop Catalogue" launch (31 May 2021); De Urbanisten's roofscape
colours (as reported); rotterdam.info on multifunctional roofs.** [web] (MVRDV and rotterdam.info read;
the colour scheme from search records)
<https://mvrdv.com/news/3875/rooftop-catalogue-launch> ·
<https://www.rotterdam.info/en/multifunctional-roofs-the-new-horizon-of-rotterdam>

Rotterdam has 18.5 km² of mostly empty flat roofs; the catalogue lists 130 uses, from offices to sports
fields, with a possible "Rooftop Code". The city's roof types are colours: green (vegetation), blue
(water storage), yellow (energy), red (people: meeting places, leisure); Rooftop Days opens roofs to the
public each year; the Dakpark is a park on a car park's roof (rotterdam.info).
*Bearing:* "rooftops as a second city" is a roof **use** per building, from a short list, chosen by
rule: green by default, yellow where the roof faces south, blue in the cloudburst catchment, red on a
share of buildings with public access.

**Guard heights for usable roofs (France's NF P01-012 and England's Approved Document K, as reported).**
[web] (search records; the standards themselves not read)
<https://www.construires.fr/garde-corps-balustrade-declaration/>

Secondary sources give a minimum guard of 1.0 m in France (0.8 m for a parapet over 0.5 m thick) and
1.1 m for roof terraces and external balconies in England.
*Bearing:* what a usable roof adds to the grammar (below).

**What a usable roof asks of D-039's grammar** (a synthesis): a **stair core that reaches the roof**
(a stair head and lift overrun as modules, at the core's grid position), a **parapet or guard module**
of 1.0–1.1 m on every edge where people go, a **roof deck slab** that carries the load of an intensive
roof (the structure class decides extensive or intensive), **roof modules** (planters, decking,
pergola, solar rows, plant room) on the same 0.5 m grid, and **access rules** (public roofs open from a
ground-floor entrance or a bridge between roofs). A rooftop bar is then a building use on the top floor
(§17), not a special case.

**Visible nature.** [web]
- **Ville de Paris, "Plongée dans la biodiversité de la Seine" (updated 4 February 2025)** (read):
  about thirty fish species in the Seine at Paris today, three (carp, gudgeon, bream) at the worst of
  the pollution in the 1970s; three rare mussel species found near the islands in January 2025;
  concrete banks replaced by earth banks for spawning
  (<https://www.paris.fr/pages/plongee-dans-la-biodiversite-de-la-seine-29148>).
- **Scott et al., "A citizen science based survey method for estimating the density of urban
  carnivores", *PLoS One* 13(5), 2018** (read): an average of 3.72 fox groups per km² across 14
  suburban areas in eight English towns, about double the 1980s estimates
  (<https://pmc.ncbi.nlm.nih.gov/articles/PMC5963764>).
- **Brighton & Hove's swift-brick condition** (search record): since April 2020, at least three swift
  bricks per dwelling in new developments over 5 m tall; England proposed swift bricks for all new
  homes in December 2025
  (<https://www.dezeen.com/2025/12/18/swift-bricks-england-homes-planning-policy/amp/>, refused).
- **Paris's shared gardens (Main Verte charter)** (search record): 154 shared gardens in April 2025, no
  pesticides, open to the public at least two half-days a week (<https://paris.fr/jardinspartages>).

*Bearing:* nature a game shows cheaply: **fish** as a few instanced schools in clear water near the
banks (the river's water type decides visibility); **birds** as flocks with a nesting rule (swifts
under eaves and swift bricks, gulls on the quays and roofs, pigeons in squares), drawn as GPU
particles with a few hero birds; **a fox at night** as one rare agent on a park path; **bees** as sound
and a few particles over flowering roofs; **community gardens** as a lot use with plots, sheds and
residents who tend them (a population appointment, people topic d). Everything else is planting
(`vegetation-materials.md`).

---

## 11. Public space: squares, pedestrian and play streets, markets and events

**Reid Ewing and Keith Bartholomew, *Pedestrian- & Transit-Oriented Design*, excerpt citing Jan Gehl,
*Cities for People* (2010), reposted on Viewpoint Vancouver (12 March 2013).** [web] (read)
<https://viewpointvancouver.ca/2013/03/12/eight-qualities-of-pedestrian-and-transit-oriented-design/>

People are recognised as people at 300–500 m, individual features and body language read at 100–25 m,
and inside 25 m the "social" field of vision begins; the most noted public squares in Europe are
almost all under 10 000 m², most under 8 000 m².
*Bearing:* square sizes for the generator: neighbourhood squares 1 000–3 000 m², district squares
3 000–8 000 m², at most one civic square near 10 000 m²; no facade more than ~100 m from the centre of
the space it faces.

**Wikipedia, "Strøget".** [web] (read)
<https://en.wikipedia.org/wiki/Str%C3%B8get>

About 1.1 km long, pedestrian since 17 November 1962 (trial, permanent from 1964); about 80 000 people a
day in summer, 48 000 on a winter day, up to 120 000 before Christmas. Jan Gehl studied it from 1962.
*Bearing:* a pedestrian high street is about a kilometre between two anchors (a square, a station),
and its crowd is a population output to calibrate against (tens of thousands a day).

**Barcelona's superblocks: the city's Superilles page; reports on the programme.** [web] (city page read;
size, speed and access rules from search records)
<https://ajuntament.barcelona.cat/superilles/en>

The city's plan names green hubs (Consell de Cent, Rocafort, Comte Borrell, Girona) and four squares of
about 2 000 m² at the junctions between them. Reports give the original superblock as three by three
blocks (about 400 × 400 m), 10 km/h inside, only residents, services and emergency vehicles admitted,
interior streets one-way so no vehicle can cross straight through; the 2016 Poblenou pilot first; the
original plan of 503 superblocks was later scaled back to linear "green axes" (search records).
*Bearing:* the superblock as a **lot-and-street rule**: in a gridded district, every third street in
each direction carries the through traffic and the trams, and the inner streets become shared surfaces
with a square at every inner crossing.

**Puig-Ribera et al., "Use of the Superblock model for promoting physical activity in Barcelona: a
one-year observational comparative study", *Archives of Public Health* 80:257 (2022).** [paper] (read)
<https://pmc.ncbi.nlm.nih.gov/articles/PMC9793503/>

At the Sant Antoni superblock about 2 340 people an hour were observed at baseline, 93 % walking, about
3 % vigorously active and 3 % sitting; weekly visitors fell from 37 432 to 30 838 over the year,
seniors by about 60 %; the authors conclude that the built changes alone did little to increase
activity.
*Bearing:* removing cars does not fill a street; **programme** does (benches, play objects, terraces,
events). The generator places activities, not only space; the population's schedules (§ weekly rhythm
below) bring the people.

**Ville de Paris, "Plus de 300 « rues aux écoles » dans Paris" (updated 24 September 2026).** [web] (read)
<https://www.paris.fr/pages/57-nouvelles-rues-aux-ecoles-dans-paris-8197>

By September 2025 more than 300 streets in front of schools had been calmed (about half of the nursery
and primary schools), 100 of them rebuilt and planted; closed to motor vehicles, mostly with removable
barriers, the freed space given to play.
*Bearing:* a **school street** is a frontage rule: the street segment in front of a school entrance is
a shared or pedestrian surface with play objects and planting, closed at the ends by removable
bollards (emergency access kept).

**Markets, beaches and crowds.** [web]
- **Paris's open-air markets** (search records): about 70 open-air food markets around 2013 for 2.1 M
  people, mornings until early afternoon; nine covered markets (paris.fr, Réussir)
  (<https://www.paris.fr/pages/les-marches-parisiens-2428>).
- **Wikipedia, "Paris-Plages"** (read): since 2002 the riverside roads close in July and August for
  sandy beaches, palm trees and activities; over four million visitors in 2007
  (<https://en.wikipedia.org/wiki/Paris-Plages>).
- **G. Keith Still, "Static crowd density"** (read): event safety limits are typically assessed at
  2 people per m²; 5 per m² should be the upper limit for standing and viewing; the UK Green Guide uses
  4.7 per m² for standing terraces (<https://gkstill.com/Support/crowd-density/CrowdDensity-1.html>).

*Bearing:* event space is sized by people: a concert of 10 000 needs about 5 000 m² of standing area at
2 per m² plus 20 % circulation *(estimate)*, which is a district square plus its streets or a quay; a
market of 60–100 stalls on 3 × 3 m pitches with 3–4 m aisles fills 2 000–4 000 m² *(estimate)*; a
river festival uses the quays and the low promenade (§9). At 1 game hour = 15 real minutes a game week
lasts 42 real hours (`population-simulation.md` §1.1), so a market twice a week and a festival every
summer weekend come round often enough to be seen.

**Streets as play spaces** (a synthesis of the above): what car-free streets actually host, in the
sources: walking and through-movement first (Sant Antoni's 93 %), then sitting, children's play,
chess tables and benches (Sant Antoni's furniture), school-gate play (Paris), skating and cycling
(Strøget, the Promenade des Anglais), beaches in summer (Paris-Plages). Outdoor cinema and street
sport are common practice but were not found in a checked source (Checked and left out). The generator
places a **programme** per public space (play, terraces, sport court, market pitch, stage, screen) and
the population fills it.

---

## 12. Streets of transit-first, pedestrian-first cities

**Nantes' tramway: French and English Wikipedia, "Tramway de Nantes", "Nantes tramway".** [web] (read)
<https://fr.wikipedia.org/wiki/Tramway_de_Nantes> · <https://en.wikipedia.org/wiki/Nantes_tramway>

Reopened as a modern tramway on 7 January 1985: three lines, 84 stations over 45.6 km of
commercial lines, mostly on reserved track; in a few narrow streets the tram shares its track with
cars ("site banal"), because the street is too narrow for a traffic lane beside the rails. Trams every
3–8 minutes in the day; an average speed of about 20 km/h. The Busway (line 4, 2006) is a 7 km bus rapid
transit line with 15 stops, BRT-certified in 2013, with electric bi-articulated buses ordered in 2019.
84 stations over 45.6 km is one every 540 m *(estimate; shared trunks counted once)*.
*Bearing:* the owner's model, with numbers: **shared running is the exception for narrow streets, not
the rule**; stops about 500 m apart; 20 km/h average. A tram street in the generator is a profile with
two tracks (paved or grassed), platforms at stops, and, in narrow streets, a single shared lane.

**Wikipedia, "Strasbourg tramway".** [web] (read)
<https://en.wikipedia.org/wiki/Strasbourg_tramway>

Opened in November 1994; six lines, 86 stations, a route length of about 49 km; car access to the
centre closed in 1992, the centre pedestrianised with trams sharing it with pedestrians and cyclists;
fully low-floor Eurotrams of 33 m (210 people) and 43 m (270); Citadis 403 of 43 m (288 people);
Homme de Fer is the hub where five of six lines meet.
*Bearing:* the **tram-in-a-pedestrian-zone** profile (no kerbs, tracks in paving, people walking
across) and the hub square where most lines meet: a node in Lynch's sense, and the place for the
city's central stop.

**Île-de-France Mobilités, "Pourquoi le tramway plutôt qu'un autre mode de transport ?" (T7 extension);
studies on catchment.** [web] (IDFM read; the 400 m catchment from search records)
<https://tzen4.iledefrance-mobilites.fr/en/le-reseau/projets/tram-t7-prolongement/questions/pourquoi-le-tramway-plutot-quun-autre-mode-de-transport>

The tram gives a fine service with a 500 m inter-station, a dedicated lane and signal priority, trams
every 4 minutes at peak and 8 off-peak, and is fully accessible; studies for Paris tram extensions take
about 400 m of walk as a stop's catchment (search records).
*Bearing:* stops every 400–600 m, a 400 m walking catchment: lines about 800 m–1 km apart cover a
district *(estimate)*.

**Cambridge Connect, "Light Rail, Metro or Tram? Some definitions from UITP".** [web] (read)
<https://cambridge-connect.uk/lightrail/systems>

The UITP considers light rail suited to 3 000–11 000 passengers per hour per direction.
*Bearing:* the capacity check (with the vehicle sizes above): a 45 m tram of about 290 people every
3 minutes carries about 5 800 an hour each way, every 2 minutes about 8 700 *(estimate)*. A street
tram is a medium-capacity mode; a dense city's busiest corridors need parallel lines, coupled trams
on a segregated trunk, or a metro.

**Wikipedia, "Low-floor tram".** [web] (read)
<https://en.wikipedia.org/wiki/Low-floor_tram>

Typical low floors are 300–350 mm (about 180 mm on the Ultra Low Floor tram); the GT6N of 1990 was the
first 100 % low-floor tram; low-floor trams serve kerb-level stops with level boarding.
*Bearing:* stops are a kerb at about 300 mm, not a platform: the tram stop module is a raised strip of
pavement 45–50 m long, a shelter, a display, and a ramp at each end.

**German Wikipedia, "Rasengleis" (green track).** [web] (read)
<https://de.wikipedia.org/wiki/Rasengleis>

Track with grass or other vegetation between and beside the rails: high-lying (vegetation just below
the rail head, best for noise and looks, harder to maintain) or low-lying (to the rail foot); sedum
tracks build 4–8 cm, grass needs at least 15 cm of soil. Germany had under 50 km of green track in 1993
and about 600 km in 2017.
*Bearing:* green track is the default between stops on reserved track; it is **not drivable**, so
wherever a tram lane must also carry taxis or emergency vehicles, the track is paved (the article does
not address road vehicles; this is the engineering reading of "grass on soil").

**Ground-level power supply (APS): English and French Wikipedia.** [web] (read)
<https://en.wikipedia.org/wiki/Ground-level_power_supply> · <https://fr.wikipedia.org/wiki/Alimentation_par_le_sol>

Developed by Innorail for Bordeaux (first commercial use 2003–2004; poor availability until late 2005,
then over 99 %): a third rail between the running rails, in segments switched on by radio only under a
tram. The extra cost was about €2.5 M per km over overhead lines. Used since in Reims, Angers, Orléans,
Tours, Dubai, Cuenca, Rio, Sydney, Istanbul and Lusail; Nice uses Alstom's SRS (charging at stops)
instead; alternatives are on-board batteries and supercapacitors (CAF, Siemens), induction (Primove)
and segmented plates (TramWave).
*Bearing:* wire-free trams are the near-future look the owner wants, and APS or battery-and-charge
both explain it. A game only draws it: a centre rail in segments, a charging contact at stops, no
poles. The patent status is under [Patents](#patents).

**Wikipedia, "Reserved track".** [web] (read)
<https://en.wikipedia.org/wiki/Reserved_track>

A semi-reserved tram lane is normally for trams only, or for trams, buses, taxis and emergency
vehicles; it may be separated by a 10 cm raised edge, studs or paint, and is sometimes paved with
cobbles to discourage cars.
*Bearing:* the owner's "taxis on the tram tracks" is ordinary practice where the lane is paved:
public taxis, emergency vehicles and service vehicles share the tram lane, never the grass track.

**Wikipedia, "Woonerf".** [web] (read)
<https://en.wikipedia.org/wiki/Woonerf>

A living street with shared space and low speeds: 15 km/h in the Netherlands, 20 km/h in Belgium, where
pedestrians may use the whole width and play is allowed and parking only in marked places; over 6 000
woonerven in the Netherlands in 1999, some 2 million residents on them by 2011.
*Bearing:* the **shared surface** profile: one level, no kerbs, planting and benches as obstacles, a
speed limit the vehicles obey because the street tells them to.

**Wikipedia, "Vauban, Freiburg".** [web] (read)
<https://en.wikipedia.org/wiki/Vauban,_Freiburg>

Built from 1998 on a former barracks; 5 267 residents in 2020; most streets are "stellplatzfrei" (no
parking): vehicles may enter at walking pace to pick up and deliver, not to park; car owners buy a
space in a peripheral garage (€17 500 plus a monthly fee in 2006); a tram serves the district, all homes
within easy walk of a stop; filtered permeability, with crescents and cul-de-sacs joined by paths.
*Bearing:* the residential street rule of a car-free district: access at walking pace for drop-offs and
deliveries, nothing parked, the garage (here: the taxi stand and the delivery hub) at the edge.

**Wikipedia, "Pontevedra".** [web] (read)
<https://en.wikipedia.org/wiki/Pontevedra>

The old town pedestrianised from 1999, the centre and more streets later; motorised access limited to
residents and services; the speed limit cut to 30 km/h in 2010 and 10 km/h in 2019, with service vans
and couriers entering only at set times on designated routes; 65 % of trips in the centre on foot;
fossil CO₂ emissions down 67 % since 1999.
*Bearing:* the service rule in its plainest form: **10 km/h, set hours, set routes**.

**City of Ghent, "The Circulation Plan".** [web] (read)
<https://stad.gent/en/mobility-ghent/circulation-plan>

Since 2017 the centre is split into six sections; cars change section only via the R40 ring; 14 points
are closed to most motor traffic; pedestrian streets bar vehicles from 11:00 to 18:00, enforced by
cameras. Allowed through the filters: emergency services, public transport, refuse trucks, taxis and
permitted care providers, plus bikes, cargo bikes and some mopeds.
*Bearing:* the **filter** as a road-graph element: a node type that passes vehicle classes by a list
and by time. It is the cleanest way to make "no private cars" and "the city still works" both true.

**TØI, "BYTRANS: Changes in the City Centre of Oslo 2017–2019" (report 1765/2020); POLIS, "Open for
business? Making car-free city centres work for everyone" (22 September 2020).** [web] (read)
<https://www.toi.no/publications/bytrans-changes-in-the-city-centre-of-oslo-2017-2019-effects-and-consequences-for-commuters-city-centre-users-city-centre-attractiveness-and-for-delivery-drivers-article36324-29.html> ·
<https://www.polisnetwork.eu/article/open-for-business-making-car-free-city-centres-work-for-everyone/>

Oslo removed about 700 public parking spaces and added dedicated spaces for disabled people,
tradespeople and deliveries; walking improved, the centre stayed as attractive and as visited, but
truck drivers found deliveries harder.
*Bearing:* delivery is where car-free plans hurt: the generator must place delivery bays and loading
times explicitly (§13).

**Wikipedia, "Cycling in Copenhagen".** [web] (read)
<https://en.wikipedia.org/wiki/Cycling_in_Copenhagen>

62 % of inhabitants commute to work, school or university by bike; about 350 km of kerb-separated
cycle tracks, 23 km of cycle lanes and 43 km of green routes; Cykelslangen is a cycle viaduct over the
harbour; green waves timed for cyclists.
*Bearing:* the owner's list has trams, taxis and boats but no bikes; a transit-first European city
without a cycle network is not consistent. Bikes (and cargo bikes, §13) belong in every street profile.

**Creutzig et al., "Fair street space allocation: ethical principles and empirical insights",
*Transport Reviews* (2020), via PIK's press release of 18 May 2020.** [web] (read)
<https://www.pik-potsdam.de/@@multilingual-selector/517dca71b2db425fad042a22ad290ced/en>

In 18 streets inside Berlin's S-Bahn ring, 58 % of street space serves cars while only one trip in six
is by car; parking Berlin's 1.2 M cars would need a 7 200 km lane. A Dutch study of 20 municipalities
gives 55 % of street space to cars, about a third to pedestrians and 12 % to cyclists (search record).
*Bearing:* the street-space budget to beat. A transit-first profile set should give at least 60 % of
the street to walking, sitting and planting, 15 % to cycling and at most 25 % to tram, taxi and service
lanes *(estimate)*, and the map check reports it per district.

**Hamburger Hochbahn, "Autonomous on-demand shuttles" (ALIKE); Toyota, Woven City announcement
(7 January 2020).** [web] (read)
<https://www.hochbahn.de/en/projects/autonomous-bus-transport/autonomous-on-demand-shuttles> ·
<https://global.toyota/en/newsroom/corporate/31221914.html>

ALIKE tests automated ridepooling as part of Hamburg's public transport: up to 20 vehicles (HOLON's
electric shuttle with an automated ramp, VW's ID. Buzz AD for four), about 37 km² for 320 000
residents, booked in the public transport app; driverless test drives since autumn 2025, test users
from 2026 with a safety driver. Woven City's masterplan (BIG) separates three street types: a street for
fast automated zero-emission vehicles (separated from people by trees), a promenade for pedestrians
and slow personal mobility, and a linear park path for pedestrians only; goods move in an underground
network.
*Bearing:* the owner's **public automated taxi** exists as a public-transport service booked in the
transit app. Woven City's three street types are the clean version of the profile set below; its
underground logistics is the futuristic step the owner's brief does not want.

**Street profiles for the generator** (a synthesis of the sources above; widths *(estimate)*):

| Profile | Width | Carries | Section |
|---|---|---|---|
| Tram avenue | 26–32 m | two tracks (grass between stops, paved where shared), taxi and service lane each way or one shared, cycle tracks, wide pavements, two tree rows | pavement 5–7 · cycle 2 · lane 3.25 · tracks 6.5 · lane 3.25 · cycle 2 · pavement 5–7 |
| Tram street (narrow) | 14–18 m | two paved tracks shared with taxis and service vehicles (Nantes' site banal), no separate lanes | pavement 4–5 · tracks 6.5 · pavement 4–5 |
| Service street | 12–16 m | one lane (taxis, deliveries, emergency) at 20–30 km/h, cycle lane, terraces, trees | |
| Shared street (woonerf, superblock interior) | 8–20 m | everyone at walking pace; 4 m clear strip for fire engines | one level, planting and benches as obstacles |
| Pedestrian street | 6–15 m | people; deliveries at set hours; emergency through | one level |
| Promenade, park path, quay | 4–20 m | people and bikes; emergency on the quay strip | |
| Green track corridor | 7–10 m | trams only | grass or sedum, poles absent if wire-free |

---

## 13. The vehicles a car-free city still needs

The owner asked where a city of trams, public taxis and boats needs adjusting. Real car-free and
car-light centres keep five classes of vehicle and filter them by **permit, time window and lane**.

**Deliveries.**
- **Off-peak and night deliveries:** Fraunhofer IML, "Avoiding urban traffic jams with low-noise
  logistics" [web] (read) <https://iml.fraunhofer.de/en/press-and-media/discover-logistics/discover-logistics-25/avoiding-urban-traffic-jams-with-low-noise.html>:
  the Dutch PIEK certificate allows vehicles and equipment under 60 dB(A) at 7.5 m to deliver at night,
  with drivers trained to unload quietly; electric trucks can meet the limits.
- **Cargo bikes:** CycleLogistics baseline study (2013), as reported by Resilience.org [web] (read)
  <https://www.resilience.org/stories/2013-10-17/the-huge-potential-of-bikes-headlines/>: about 51 %
  of motorised goods-related trips in European cities could be made by bike or cargo bike (a modelled
  potential; light loads, short distances).
- **Time windows and routes:** Ghent bars vehicles from pedestrian streets 11:00–18:00; Pontevedra
  admits vans at 10 km/h on set routes and hours; Oslo added delivery bays (§12).
- **Consolidation:** an urban consolidation centre at the edge splits loads onto cargo bikes and small
  electric vans (the Wikipedia article on such centres returned 404; the practice is described in
  Checked and left out).

*Bearing:* in the generator, deliveries are **hubs** (one per district edge, near the tram depot or
the port), **delivery bays** on service streets every 100–150 m *(estimate)*, **windows** (vans
06:00–11:00, cargo bikes all day, quiet trucks to supermarkets at night) and **lockers** at tram stops.
Vehicle classes: cargo bike, small electric van, quiet electric truck.

**Emergency services.**
- **Légifrance, Arrêté du 31 janvier 1986, article 4 (fire safety of residential buildings)** [docs]
  (read) <https://www.legifrance.gouv.fr/loda/article_lc/LEGIARTI000006828433>: a fire-engine road is
  3 m wide clear of parking, 3.5 m high clear (3.30 m vehicle plus 0.20 m margin), carries 130 kN, has an
  11 m inner turning radius and a slope under 15 %; a ladder road is 4 m wide, at least 10 m long, under
  10 % slope. A regional guidance sheet adds that engines must reach within 60 m of a building's entrance
  (search record).
- Ghent, Paris's riverside park and Barcelona's superblocks all let emergency vehicles through (§11–§12).

*Bearing:* every street profile keeps a **4 m clear strip** (ladder road) along facades over a set height,
free of trees, terraces and fixed furniture, with **retractable bollards** at filters; paved tram lanes
count as emergency routes, grass track does not; every building entrance within 60 m of such a strip.
That is a check the map can run before any building exists.

**Works and refuse.**
- **Wikipedia, "Automated vacuum collection"** [web] (read) <https://en.wikipedia.org/wiki/Automated_vacuum_collection>:
  waste drops into inlets (separate ones for waste, recycling, food) and is sucked through underground
  pipes to a central station; Envac's first system was at Sollefteå hospital in 1961, the first
  housing in 1965; used in Copenhagen, Barcelona, London, Stockholm, Helsinki's Jätkäsaari and others;
  transport emissions down by up to 90 %; costly and hard to retrofit, best in dense new districts.
- Ghent's filters pass refuse trucks; Oslo kept spaces for tradespeople (§12).

*Bearing:* new districts get **waste inlets** (a street-furniture module) and one collection station per
district; old districts keep small electric refuse trucks at dawn. Works vans enter by permit; a
utility trench under the service street is where works happen.

**Residents with reduced mobility.**
- Low-floor trams with level boarding (§12); ALIKE's shuttle with an automated ramp (§12).
- **Ljubljana's car-free centre** [web] (search records; the city page returned 404)
  <https://citychangers.org/ljubljana-pedestrianisation/>: about 10 ha pedestrian since 2007, car
  access only by permit and for deliveries; free electric "Kavalir" carts (six to eight vehicles, up to
  five passengers) aimed especially at people with reduced mobility.
- Oslo kept on-street spaces for disabled people (§12).

*Bearing:* door-to-door service in a car-free core is a **public taxi allowed at walking pace** in
shared and pedestrian streets for reduced-mobility trips (Vauban's rule), plus a free electric cart line
in the old centre. Without it the car-free core excludes people, which a release-quality city must not.

**Night service.**
- **Wikipedia, "Vienna U-Bahn"** [web] (read) <https://en.wikipedia.org/wiki/Vienna_U-Bahn>: since
  4 September 2010 the U-Bahn runs all night every 15 minutes on Friday and Saturday nights and before
  holidays, with NightLine buses.
- **Le Bonbon Nantes, "Nantes buses drop you off at home starting at 10:30 pm" (23 September 2024)**
  [web] (read) <https://en.lebonbon.fr/nantes/news/bnantes-buses-drop-you-off-at-home-starting-at-1030>:
  from 22:30 Nantes buses let passengers off between stops as close to home as possible; the Luciole
  night lines run from the Hangar à Bananes (the nightlife quay) until 4:00–4:45 on Thursdays and
  Saturdays.

*Bearing:* the night timetable is data: trams every 15 minutes through Friday and Saturday nights,
night lines from the nightlife quarter, and public taxis dropping people at their door after 22:30. A
"good night out" ends with a ride home.

**Service vehicles by class** (a synthesis; the vehicle sizes and counts are topic e's to decide):

| Class | Vehicles | Where | When |
|---|---|---|---|
| Public transport | trams, public automated taxis and shuttles, river boats | tram lanes (paved where shared), service streets, shared streets at walking pace for drop-offs | always; night timetable |
| Emergency | fire engines, ambulances, police | everywhere through filters, on paved tram lanes and the 4 m strips | always |
| Deliveries | cargo bikes; small electric vans; quiet trucks | bays on service streets; pedestrian streets in windows | vans 06:00–11:00; trucks at night under 60 dB(A) |
| Refuse and works | small electric refuse trucks, works vans | service streets; permits for others | dawn; by permit |
| Reduced mobility | taxis with ramps, free electric carts | door to door at walking pace | always |

---

## 14. Block types and mixed use

**Wikipedia, "Haussmann's renovation of Paris" (§"The Haussmann building").** [web] (read)
<https://en.wikipedia.org/wiki/Haussmann%27s_renovation_of_Paris>

Facades on the new boulevards were regulated to the same height, colour, material and general design;
built or faced with cut stone; balconies and cornices aligned without projections; a second-floor
piano nobile with a balcony, a continuous plain balcony on the fifth; a mansard roof at 45°; shops or
offices on the ground floor. Before Haussmann, heights were capped at 22.41 m.
*Bearing:* the canonical **perimeter block** facade and its alignment rules (§15).

**French Wikipedia, "Règlements d'urbanisme de Paris".** [web] (read)
<https://fr.wikipedia.org/wiki/R%C3%A8glements_d%27urbanisme_de_Paris>

Height tied to street width since 1783–1784 (facades of 11.70 m in streets under 7.80 m, 14.62 m to
9.75 m, 17.55 m beyond) with roofs under a 45° line from the eaves; the 1859 decree allowed 20 m on
streets over 20 m wide and set a 2.60 m minimum floor height; 1884: 12, 15, 18 and 20 m by street-width
band, roofs within an arc whose radius depends on the street; 1902: an eighth of a circle then 45°;
1967: caps of 31 m in the centre and 37 m outside; 2006 PLU: facade height P + 2 to P + 3 m for a street
width P, a roof envelope of 6–8 m above, overall caps of about 25 m in central arrondissements, 31 m in
outer ones and 37 m in some districts.
*Bearing:* the height rule as a function: `H_facade = min(W_street + k, H_cap(district))`, with a roof
envelope above the eaves. It is the single most powerful consistency rule a European city has.

**Barcelona's Eixample: Wikipedia, "Eixample"; F. A. Shaikh, "Why (nearly) all blocks in Barcelona look
the same" (Celog, 9 January 2022).** [web] (read; the 113.3 m and the original depth and height also
in search records)
<https://en.wikipedia.org/wiki/Eixample> · <https://celog.substack.com/p/why-nearly-all-blocks-in-barcelona>

Cerdà's 1859 plan: square blocks of 113.3 × 113.3 m with corners cut at 45°, streets planned 20 m wide
and avenues 50 m; buildings on two or three sides around a garden, under half the block, height limited
to about 20 m (16 m and 20 m deep in another account). In practice blocks were built on all four sides,
built area grew from a planned 67 000 m² to about 300 000 m² by the 1950s, and courtyards filled with
workshops and car parks. 7.48 km² in all.
*Bearing:* the **chamfered grid** block (one parameter set: block size, chamfer, street widths, depth,
height) and a warning: rules decay into infill unless the courtyard is part of the rule (a BAF target
per lot, §10).

**Wikipedia, "Karl-Marx-Hof" (Vienna).** [web] (read)
<https://en.wikipedia.org/wiki/Karl-Marx-Hof>

1 100 m long, 1 382 flats of 30–60 m² for about 5 000 people, built 1927–1930; only 18.5 % of its
156 000 m² site built up, the rest gardens and play areas, with laundries, baths, kindergartens, a
library and doctors' offices inside the block.
*Bearing:* the **courtyard block with services** (the 1920s Viennese model): homes on the perimeter,
shared facilities in the ground floor and the courtyard. It is the easiest block to make "a city where
everything is near".

**Mixed use against zoning.**
- **German Wikipedia, "Urbanes Gebiet"** [web] (read) <https://de.wikipedia.org/wiki/Urbanes_Gebiet>:
  since May 2017 Germany's land-use ordinance has an "urban area" category for denser mixed use of
  housing, business, offices, social and cultural uses, without requiring a balanced mix; a daytime
  noise guide value of 63 dB(A) (60 in mixed areas); and a plan may exclude housing on the street-side
  ground floor.
- **Paris's PLU: protection of commercial ground floors** [web] (search records)
  <https://www.banquedesterritoires.fr/commerce-de-proximite-le-tribunal-administratif-annule-les-mesures-de-protection-du-plu-de-paris>:
  along listed streets, shops and craft workshops on the ground floor may not change use (simple
  protection), or new buildings must provide them (reinforced protection); the city announced more
  than 35 km of protected frontage in 2020; a 2007 court ruling had annulled an earlier, blanket version.
- **Wikipedia, "Form-based code"** [web] (read) <https://en.wikipedia.org/wiki/Form-based_code>:
  rules tied to a regulating plan that set how facades meet the public realm, the form and mass of
  buildings and the types of streets and blocks, instead of separating uses; SmartCode is a
  transect-based model code (Duany Plater-Zyberk).
- **Wikipedia, "15-minute city"** [web] (read) <https://en.wikipedia.org/wiki/15-minute_city>: daily
  needs within a 15-minute walk, ride or transit trip; Carlos Moreno (2016); taken up in Anne Hidalgo's
  2020 campaign in Paris.

*Bearing:* uses are assigned per **floor and frontage**, not per lot: an "active frontage" class on
listed streets forces shops, cafés or workshops on the ground floor (Paris's reinforced protection),
housing above, offices in a share of upper floors near tram hubs, services (school, clinic, crèche) in
courtyard ground floors (Karl-Marx-Hof). A district is a mix by share, checked by walking distances
(§16).

**Block types for the generator** (a synthesis):

| Type | Size | Rule set | Ground floor |
|---|---|---|---|
| Haussmann perimeter | irregular, 60–150 m sides | H from street width, aligned cornices, cut stone, 45° roof | active on boulevards |
| Chamfered grid (Eixample) | 113 m square, 45° chamfer | two to four sides, courtyard kept by BAF | active on all sides |
| Courtyard block (Vienna, Copenhagen) | 80–150 m | perimeter 5–8 floors, services in the courtyard | mixed |
| Superblock | 3 × 3 blocks, ~400 m | through traffic on its edges, shared interior, squares at inner crossings | active on edges and squares |
| Old-town organic | lanes 3–8 m | skeleton-strip lots (`city-generation.md` §2), heights 3–5 floors | shops on lanes |
| Waterfront slab or point blocks (HafenCity, Confluence) | 40–80 m | plinth on the flood terrace, varied signature architecture | cafés on the quay |

---

## 15. Consistency: the style bible as data, and landmarks

**City colour plans: Turin (as described by Gardenvisit; Giovanni Brino's plan, as reported).** [web]
(Gardenvisit read; Brino's 1978–1983 plan from search records)
<https://www.gardenvisit.com/landscape_architecture/landscape_plans_planning/landscape_architectural_environmental_policies/colour_planning_policy>

In the first half of the nineteenth century Turin coordinated more than eighty colours across the city
to emphasise its main processional routes; the restoration of the scheme has run for about two
decades. Brino drew the first modern colour plan for Turin in 1978–1983 (search records).
*Bearing:* a **palette per route and district**, not per building: a city reads as one when colour is a
function of where a facade stands.

**Cities: Skylines II's regional themes (as reported).** [web] (search records)
<https://www.paradoxinteractive.com/games/cities-skylines-ii/modding/dev-diary-netherlands-region-pack>

Eight free region packs (Japan, UK, US East and West, China, France, Eastern Europe, Germany) with over
2 500 assets join the generic European and American themes; reports say themes can be set district by
district within one city.
*Bearing:* the game that ships the most "European city" generation applies a style **per district**,
the same unit as D-039, which is also where it risks looking like a catalogue; the contrast rule and a
shared palette are what hold the districts together.

**The style bible as data** (a synthesis of §14–§15 and `city-generation.md` §1–§3). One file per city
(and one per district as an override layer, D-035) that every generator reads and every generator's
output is checked against. What it contains:

```toml
[palette]                       # materials with shares, per district; one accent colour each
stone   = { material = "limestone_ashlar", share = 0.35 }
render  = { tints = ["#efe6d6", "#e8dcc4", "#d9cbb2"], share = 0.40 }
brick   = { share = 0.10 }
timber  = { share = 0.10 }
metal   = { share = 0.05 }
accent  = "#2f5d50"             # shutters, doors, railings in this district

[height]                        # Paris-style function of street width, with caps
facade  = "min(street_width + 2.0, cap)"      # metres
cap     = 25.0                  # centre; edge districts 18, landmark zones override
roof_envelope = { angle_deg = 45, above_eaves_max = 7.0 }
floors  = { ground = 4.2, typical = 3.0, attic = 2.8 }   # D-039's set

[alignment]
cornice_tolerance = 0.5         # adjacent cornices on one frontage within 0.5 m, or a stepped return
balcony_rows      = [2, 5]      # continuous rows on these floors where the block is "boulevard"
setback           = { build_to = 0.0, upper_from_floor = 7, upper_depth = 2.0 }

[openings]
bay_widths        = [2.5, 3.0]  # metres, on the 0.5 m grid
window_to_wall    = { ground = [0.6, 0.85], upper = [0.25, 0.40] }
vertical_stacking = true        # windows of a bay line up floor to floor
piano_nobile      = 2           # the floor with the tallest windows and a balcony

[ground_floor]
active_frontage   = "streets.class in ['tram_avenue','pedestrian','square']"
min_active_share  = 0.7

[green]
baf_target        = { residential = 0.6, commercial = 0.3 }
flat_roof_default = "extensive"  # under 30 degrees
usable_roof_share = 0.2
street_tree_spacing = [8.0, 10.0] # where the pavement is 3 m or more
rule_3_30_300     = true

[lighting]
cct_max_k      = 3000
ulr_max        = 0.01
dim_when_empty = 0.2
```

**How a generator verifies itself** (each is a function of the generated records, run in the cook
step and in the map preview; a failure names the lot, the frontage or the building):
- **Height:** every building's eaves height ≤ `facade(street_width)`; roof inside the envelope.
- **Cornice continuity:** along each frontage, |cornice(i) − cornice(i+1)| ≤ tolerance or a return
  module is present; reports the longest run.
- **Opening rhythm:** bays on the grid; window columns stacked (zero misalignment in grid steps);
  window-to-wall ratio per floor within range; no two identical module variants adjacent along a
  frontage (`city-generation.md` §4).
- **Palette:** per district, the area share of each material within ±5 % of target; one accent.
- **Ground floor:** active share on listed frontages ≥ target; every school frontage a school street.
- **Green:** BAF per lot ≥ target; canopy share per district; 3-30-300 per dwelling (trees seen from a
  window, park of ≥1 ha within 300 m).
- **Access:** every entrance within 60 m of a 4 m emergency strip; every dwelling within 400–500 m of a
  stop; every dwelling within a 15-minute walk of a school, a food shop, a GP and a park (§16).
- **Landmarks:** sightline corridors free of taller buildings.
- **Contrast:** neighbouring districts differ in at least two of palette, height cap, block type
  (Cyberpunk's contrast rule, `city-generation.md` §1).

**Landmarks** (Lynch, `city-generation.md` §2). A landmark anchors a district when it stands where a
path ends or bends, at a node (the tram hub, a bridgehead, a square), or on an edge seen from far (the
river, the seafront); HafenCity's Elbphilharmonie (a concert hall on a warehouse at the tip of a quay)
and Nantes' Titan crane (a kept shipyard crane at the island's western tip) are both edge landmarks on
water. A city needs few: one per district, three to six in all, each with a protected sightline.

---

## 16. The near-future city: integrated technology, well-being, no fossil fuels

The owner's note: the smart-city idea is a bit outdated, so make a city a few years ahead where
technology is integrated with the city and nature, people are well, and nothing burns fossil fuel.
The projects say what not to do; the cities that lasted say what to show.

**Cautions.**
- **Wikipedia, "Sidewalk Toronto"** [web] (read) <https://en.wikipedia.org/wiki/Sidewalk_Toronto>:
  Alphabet's Sidewalk Labs won Toronto's 4.9 ha Quayside in 2017 with a digital layer over the
  infrastructure (adaptive roads, automated waste collection, a self-driving shuttle); privacy and
  data governance drew criticism; cancelled on 7 May 2020.
- **Wikipedia, "Songdo International Business District"** [web] (read)
  <https://en.wikipedia.org/wiki/Songdo_International_Business_District>: 600 ha of reclaimed land,
  about 167 000 residents in 2020, pneumatic waste (no bins, no refuse trucks), computers built into
  homes, streets and offices; criticised as car-centric with poor walkability and some greenwashing.
- **Wikipedia, "Masdar City"** [web] (read) <https://en.wikipedia.org/wiki/Masdar_City>: planned from
  2006 for 45 000–50 000 residents, car-free with an underground personal rapid transit; the PRT was
  cut to a pilot in 2010; about 15 000 people living and working there in 2023, 5 000 of them residents;
  wind towers and short narrow streets for climate.
- **Toyota's Woven City** [web] (read, §12; Wikipedia "Woven City" read): first residents (about 360)
  from 25 September 2025, 2 000 targeted; hydrogen fuel cells; three street types.

**What lasted.**
- **Barcelona:** the superblocks and green axes (§11); Decidim, a participatory democracy platform
  under AGPL-3.0 used for participatory budgets in some 300–400 institutions (Wikipedia, "Decidim",
  read, <https://en.wikipedia.org/wiki/Decidim>).
- **Copenhagen:** district heating covers about 98 % of heat demand over some 1 500 km of pipes (search
  records); the cloudburst plan (§ below); harbour baths and electric harbour buses (§8–§9); cycling
  (§12).
- **Helsinki:** Helen's Eiranranta plant recovers heat and cold from treated wastewater for district
  heating and cooling (about 90 MW and 60 MW, completed August 2026) [web] (read)
  <https://www.helen.fi/en/about-us/energy/eiranranta-heat-pump-plant>; Katri Vala before it lies in a
  rock cavern under a park (search records); Jätkäsaari has vacuum waste collection (§13).
- **Amsterdam and the Netherlands:** no gas connection for new buildings since 1 July 2018; 11.2 % of
  Dutch homes fully gas-free in 2024, heated by heat pumps or district heating (CBS, 9 December 2025)
  [web] (read) <https://www.cbs.nl/en-gb/news/2025/50/ever-more-gas-free-homes>; Amsterdam aims to be
  gas-free by 2040 (search records).
- **Paris:** Fraîcheur de Paris, Europe's largest district cooling network, uses Seine water for free
  cooling in winter, serves over 700 customers and is to grow by 158 km by 2042 (ENGIE) [web] (read)
  <https://www.engie.com/en/explore/projects/engie-x-fraicheur-de-paris/>; schoolyards turned into
  "oasis" cool islands (203 by January 2025, search records).

*Bearing:* the projects that failed sold technology as the city; the ones that lasted bought a quieter
street, warm homes, clean water and a say. In a game, "integrated" means the technology **disappears
into things that were there anyway** (a roof, a quay, a lamp, a stop sign) and shows only as better
behaviour and true numbers.

**Water: the sponge city and the cloudburst plan.**
- **Wikipedia, "Sponge city"** [web] (read) <https://en.wikipedia.org/wiki/Sponge_city>: proposed by
  Kongjian Yu, national policy in China from 2014 with pilot cities from 2015, aiming to absorb and use
  70 % of rainwater with green spaces, wetlands, rain gardens, green roofs, recessed green spaces and
  permeable paving; criticised for flooding in over half the pilots and thin monitoring.
- **WUSF/NPR, "Copenhagen is adapting to a warmer world with rain tunnels and sponge parks" (13 June
  2025)** [web] (read) <https://www.wusf.org/2025-06-13/copenhagen-is-adapting-to-a-warmer-world-with-rain-tunnels-and-sponge-parks>:
  after the July 2011 flood, a $1.3 bn plan of 300 projects over two decades; Enghaveparken holds
  25 000 m³ when full behind a low wall with gates that rise from the ground, with a 2 000 m³ tank under
  a rose garden for irrigation and a football pitch sunk into the ground; seven rain tunnels (the
  Kalvebod Brygge tunnel holds 10 000 m³); about 20 green areas turned into sponge parks.

**Energy and heat.** District heating and cooling (above); rooftop and facade solar (the yellow roofs of
§10); river and sea heat pumps (Helsinki's wastewater plants above; Stockholm's Värtan sea-water heat
pumps, 180 MW in six units, search records); storage in batteries and heat tanks (not sourced here).
Visible parts: a small heat-pump plant on the quay with a water intake, solar rows on roofs, a
substation in a ground floor, district heating manholes, chargers at taxi stands, no chimneys.

**Light at night.**
- **The French decree on light nuisance (arrêté of 27 December 2018), as summarised by COST LONNE**
  [web] (read) <https://cost-lonne.eu/solutions/france-2018-lighting-decree/>: new outdoor luminaires at
  or below 3 000 K, an upward light ratio strictly under 1 % (4 % in service), signs and advertising off
  from 1:00 to 6:00 or an hour after closing, non-residential facades off by 1:00, interior lighting seen
  from outside off an hour after the last occupant leaves.
- **DarkSky International and IES, "Five Principles for Responsible Outdoor Lighting"** [web] (search
  record; darksky.org refused) <https://darksky.org/lighting-principles>: light that is useful,
  targeted, low, controlled (timers, motion sensors, dimmers) and warm-coloured.
- **Tvilight, "Case study: residential areas"** [web] (read) <https://tvilight.com/case-study-residential-areas/>:
  in Eeneind (Nuenen) lamps dim to 20 % with no activity and rise to 100 % when a pedestrian, cyclist or
  car is detected, neighbours brightening too. Toulouse's Kara sensors cover 24 km of streets (search
  record; edf.fr refused).

*Bearing:* **darker nights** are rules a renderer can follow: warm 2 700–3 000 K street lights, fully
shielded (no light above the horizontal), dimmed to 20 % where no one is near and raised around the
player and the people simulated near them, shop signs off after 1:00. The sky above a dim city shows its
stars (D-046's night sky), from rooftops and the beach especially; the light budget is also a frame-time
budget, since fewer bright lights cost less.

**Sound: a quiet city.**
- **WHO, *Environmental Noise Guidelines for the European Region* (2018)** [docs] (search records)
  <https://iris.who.int/handle/10665/279952>: road traffic below 53 dB Lden and 45 dB Lnight, railway
  below 54 and 44, aircraft below 45 and 40.
- **Bruitparif, Paris car-free days 2015–2019** [web] (search records)
  <https://www.bruitparif.fr/pages/Bandeau%20haut/800%20Presse/2022-09-19%20CP%20Impact%20Paris%20Respire%202022.pdf>:
  drops of about 10 dB(A) on the Champs-Élysées in 2015, averages of 1–3 dB(A) inside the perimeter
  other years; 2.2 dB(A) for "Paris Respire" in 2022 (about 40 % less sound energy); sirens and diverted
  traffic raised levels elsewhere. Barcelona's Sant Antoni superblock: 4 dB less noise and 25 % less NO₂
  (search records).

*Bearing:* removing engines takes a street from traffic levels toward a quiet one; the measured drops
are a few dB averaged, about 10 dB where traffic is fully removed. For audio (later): the soundscape is
people, footsteps, terraces, bikes, tram bells and wheel squeal in curves, boats, water, birds and
wind; distant traffic is absent, so the far field is the sea, the river and the city's murmur. The
owner's "quiet city" is mostly a **mixing rule** (no engine layer) plus emitters placed by the street
programme.

**Well-being: proximity, shade, cool islands.** The 15-minute city (§14) and the 3-30-300 rule (§10)
are checkable; shade is a rule on street trees and awnings (canopy over pavements and squares,
measured as the share of walking surface shaded at 15:00 in July); cool islands are parks, water
mirrors (Nice), oasis schoolyards and fountains every few hundred metres *(estimate)*. Mueller et al.
(*Environment International* 134, 2020, search records) estimated that Barcelona's 503 superblocks
would avoid 667 premature deaths a year, mostly through lower NO₂, noise and heat: the health argument
behind the street rules.

**No fossil fuels: what changes on the street.** No petrol stations, exhausts, gas mains, gas meters or
boiler flues; instead chargers at taxi stands and depots, heat-pump units in courtyards and on roofs
(with their hum), district heating substations, solar on roofs and some facades, quiet electric service
vehicles, and wire-free trams (APS or battery). Cooking is electric (no gas hobs in interiors, §17).

**Live data on the city's screens** (the owner's accepted idea).
- **GTFS Realtime reference** [docs] (read) <https://gtfs.org/documentation/realtime/reference/>:
  a realtime feed is defined against a static GTFS feed and carries trip updates (predicted arrivals and
  delays per stop), vehicle positions (with occupancy) and alerts.

*Bearing:* stop displays, river-level boards, harbour-bath flags, energy dashboards and the smart pad
read the **simulation's own state** through one feed shaped like GTFS Realtime (trip updates, vehicle
positions, alerts) plus city channels (river level, bath water, energy flows, air and noise). The same
feed is the debug view: a display that disagrees with what the player sees is a bug in the simulation,
found by looking. Transport's side of it is topic e's.

**For a game: visible or behavioural** (a synthesis):

| Technology | Visible, worth modelling | Behaviour or numbers only |
|---|---|---|
| Energy | solar roofs, a heat-pump plant on the quay, substations, chargers, no chimneys | the energy balance on dashboards |
| Water | sponge parks that fill, water squares, rain gardens, bath flags, a retention basin | storage volumes, rain events |
| Waste | inlets (vacuum collection) in new districts; small electric trucks at dawn elsewhere | collection schedules |
| Transport | wire-free trams, public taxis, boats, chargers, stop displays | dispatch, headways (topic e) |
| Light | warm shielded lamps that dim and brighten | energy saved |
| Sensors | almost nothing: a discreet box on a pole | the data on the screens |
| Information | stop displays, information points, the smart pad | the feed |
| Well-being | trees, shade, water, benches, play, quiet | 3-30-300, noise, heat checks |

---

## 17. Interiors by building use

Every building enterable, its interior consistent with its use, staffed, and fed by the population's
places and jobs (`population-simulation.md`, `Place { kind, capacity, hours, doors }`). Below: the
numbers per use, then the generators and their speed tiers, then how interiors share D-039's grid.

### 17.1 Sizes, capacities and staff: the sources

**UK Government, "Technical housing standards – nationally described space standard" (27 March 2015,
amended 19 May 2016).** [docs] (read)
<https://www.gov.uk/government/publications/technical-housing-standards-nationally-described-space-standard/technical-housing-standards-nationally-described-space-standard>

Minimum gross internal areas in m² (one storey): 1 bedroom 1 person 39 (37 with a shower room); 1b2p
50; 2b3p 61; 2b4p 70; 3b4p 74; 3b5p 86; 3b6p 95; 4b5p 90; 4b6p 99; 4b8p 117; with built-in storage of
1–4 m². A single bedroom at least 7.5 m² and 2.15 m wide; a double at least 11.5 m², one at least
2.75 m wide and others 2.55 m; ceilings at least 2.3 m over 75 % of the area.
*Bearing:* the home programme by household, as minimums the generator may exceed.

**Approved Document B, Volume 2, floor space factors (as reproduced by Northamptonshire Fire and
Rescue Service under the Open Government Licence).** [docs] (read)
<https://www.northantsfire.gov.uk/floor-space-factors/>

m² per person: standing areas and bars within 2 m of a counter 0.3; clubs, dance floors, bars without
seating 0.5; concourses and queues 0.7; restaurants, dining rooms, lounges, meeting rooms 1.0;
exhibition halls 1.5; shop sales areas (supermarkets, department stores) 2.0; museums, galleries,
workshops 5.0; offices 6.0; bulky-goods shops, kitchens, libraries 7.0; bedrooms 8.0; storage 30.
*Bearing:* **maximum** occupancies per room type, the cap a venue's crowd may not exceed in the
simulation (fire-code realism), distinct from the design occupancy used for staffing.

**British Council for Offices, report of 10 July 2025; BCO density figures as reported.** [web] (read;
the 2018 figure from a search record)
<https://www.bco.org.uk/bco-report-calls-for-new-approach-to-space-planning-as-office-use-reaches-critical-shift>

Office utilisation has settled at about 66 % (down from the 80 % benchmark), giving an effective
15 m² per occupant against 12.5 m² before; the 2018 BCO study found 9.6 m² per desk (CIBSE Journal,
search record).
*Bearing:* offices at 10 m² per desk and 15 m² per employee; two-thirds of desks in use on a weekday,
which the population's office template should reproduce.

**Designing Buildings Wiki, "Structuring a restaurant floor plan".** [web] (read)
<https://www.designingbuildings.co.uk/wiki/Structuring_a_restaurant_floor_plan>

A 60/40 split between the dining room and the kitchen, preparation and storage; area per seat about
11–14 sq ft in full service and 18–20 in high-end dining (about 1.0–1.3 and 1.7–1.9 m²); 15 sq ft
(1.4 m²) as a bistro average including circulation; aisles at least 18 in (0.46 m); bar standing room
500–550 mm per person, stools 600 mm apart.
*Bearing:* the restaurant and bar programmes.

**Schools and hotels.** [web] (search records)
- The Académie de Paris counted 733 écoles, 178 collèges and 159 lycées at the 2024 school year, with
  average classes of 20.4 (nursery), 19.7 (primary), 25.9 (collège) and 31.4 (general lycée) pupils
  (<https://www.ac-paris.fr/chiffres-cles-de-l-academie-134536>, refused; search records).
- London's application of the UK's BB103 lists primary classrooms of 55 m² and reception classrooms of
  62 m² (search record).
- Hotel staffing varies from about 0.3 staff per room (economy) to 0.7 (luxury) in a 2002 study of Greek
  hotels and higher in vendor benchmarks (search records): an assumption to tune, not a figure.

**Shadows of Doubt (ColePowered Games): "DevBlog #13: Creating Procedural Interiors" (28 February
2019).** [web] (read)
<https://colepowered.com/shadows-of-doubt-devblog-13-creating-procedural-interiors/>

A detective game on sale (its release date was not checked here) in a procedural city where, in the
studio's words, a mix of procedural generation and hand-crafted design lets every room of every
building be explored.
The world is a grid of 1.8 m tiles; a building floor is 15 × 15 tiles; locations nest as city,
district, block, building, floor, **address**, room, tile. A designer hand-makes floor plans that cut a
floor into addresses; inside an address, room types are placed in priority order (for a flat: living
room first, then bathroom, then bedrooms, kitchen, and a study or dining room if space allows), each
room tried in every free position and scored on floor area, a compact shape (few corners) and windows,
the best position kept; hallways are drawn when the entrance is far from the space's corners so that no
room is reached only through another; some rooms may not hold the entrance (a bedroom), some take one
door only (a bathroom); a later room may take space from an earlier one that has spare. Furniture is the
next devblog's subject.
*Bearing:* the **game-rate floor-plan method** with a shipped city behind it: greedy placement by a
score, on a grid, with adjacency rules and hallways. It is what Forge's first interior generator should
copy, on D-039's 0.5 m grid with bays instead of 1.8 m tiles.

### 17.2 The catalogue of uses

The table is the building-use catalogue's first draft. Sizes are net areas; "design" occupancy staffs
and fills the place in the simulation, "max" is the fire-code cap from Approved Document B. Figures not
from a source above are *(estimate)*.

| Use | Rooms and sizes | Adjacencies | Furnishing rules | Staff | Visitors (design / max) |
|---|---|---|---|---|---|
| Studio (1 person) | 37–39 m²: main room 20–25, kitchenette, shower room 3–4, hall | entrance → main room; bath off hall | bed against wall away from the door; table by the window | — | 1 / — |
| Flat 1b2p | 50 m²: living-kitchen 22–28, double bedroom ≥ 11.5, bath 4–5, hall, storage 1.5 | hall serves all; bedroom not off the kitchen | sofa faces window or TV; dining by window; bed head on a solid wall | — | 2 / — |
| Family flat 3b5p | 86 m²: living 25–30, kitchen 10–12, double ≥ 11.5, two singles ≥ 7.5, bath, WC, storage 2.5 | living by the entrance; bedrooms on a corridor | children's rooms with desks; table seats the household | — | 5 / — |
| Loft | 80–150 m² open plan, ceilings 3.6–4.2 m, mezzanine, kitchen island *(estimate)* | open; bath enclosed | zones by rugs and lights; party capacity | — | 2–4; party 15–30 / 0.5 m² each |
| Shared flat (students) | rooms 9–12 m² each around a kitchen-living 20–25 *(estimate)* | rooms off a hall | desk and bed per room | — | 3–5 / — |
| Small shop | 30–80 m² sales, 10–20 back room, WC *(estimate)* | street door → sales → stock | shelving on walls, counter by the door, window display | 1–3 | 5–20 / 2 m² each |
| Supermarket | 1 000–2 500 m² sales, stock and cold rooms, staff room *(estimate)* | delivery bay at the back | aisles on a grid, tills by the exit | 20–50 | 100–400 / 2 m² each |
| Shopping centre | 20 000–60 000 m² lettable over 2–4 floors, mall streets, food court *(estimate)* | anchors at the ends, escalators at nodes | unit fit-outs by shop type | 500–2 000 | thousands / 2 m² each in shops |
| Café | 40–80 m², 20–40 seats, counter, small kitchen, WC; terrace outside *(estimate)* | counter near the door | bistro tables 0.6–0.7 m, chairs 0.75 m clearance | 2–4 | 30 + terrace / 1 m² each |
| Restaurant | 100–250 m²: dining 60 %, kitchen and stores 40 %, 1.0–1.9 m² a seat, WCs | kitchen → pass → dining; WCs off dining | tables by type (2, 4, 6), aisles ≥ 0.46 m | 1 per 10–15 seats *(estimate)* | 50–120 seats / 1 m² each |
| Bar | 60–200 m²: bar counter, standing, seating, stock, WCs *(estimate)* | counter on the long wall | stools 0.6 m apart, standing 0.5 m each | 2–6 | 50–200 / 0.3–0.5 m² each |
| Club | 300–1 500 m²: dance floor, bars, DJ booth, cloakroom, WCs, smoking terrace *(estimate)* | entrance → cloakroom → floor | lights and speakers on the ceiling grid | 10–30 | 300–1 500 / 0.5 m² each |
| Spa / urban bath | 500–3 000 m²: reception, changing, showers, pools, saunas, rest rooms, treatment rooms 10–12 m² *(estimate)* | dry → changing → wet | loungers, lockers, benches | 10–40 | 100–400 / — |
| Museum | galleries 100–1 000 m² each, foyer, shop, café, stores, workshops *(estimate)* | circuit through galleries | exhibits along walls and on plinths, benches | 10–100 | hundreds / 5 m² each |
| Office floor | open plan at 10 m² a desk, meeting rooms, kitchen, WCs around a core | core central or on a party wall | desks in pairs facing, meeting tables | 15 m² per employee | 66 % of desks in use / 6 m² each |
| School | classrooms 55–62 m² for 20–30, hall, canteen, gym, yard, staff room | classrooms on corridors; yard on the ground | desks in rows or groups, teacher's desk at the board | 1 adult per 10–15 pupils *(estimate)* | 200–800 pupils |
| Clinic / GP practice | waiting, reception, consulting rooms 12–16 m², treatment, WCs *(estimate)* | waiting → consulting | desk, couch, sink per consulting room | 5–20 | 10–40 waiting / 1 m² each |
| Hotel | rooms 18–30 m² with bath, lobby, breakfast room, back of house *(estimate)* | lift core; rooms on corridors | bed head on the corridor wall, desk by the window | 0.3–0.7 per room | 1.6–2 per room / bedroom 8 m² each |
| Tram stop / station | platform 45–50 m, shelter, display, ticket machine | — | benches, bins, display | 0 | 50–300 waiting |
| Tram depot | sheds, tracks, workshop, washing, offices: about 0.1–0.2 ha per tram *(estimate; search record: Luxembourg's depot, 6.5 ha for 32 trams)* | yard fan → sheds | — | 1–2 per tram *(estimate)* | — |

Cross-cutting rules: kitchens are electric (§16); every habitable room has a window bay (§17.4);
public places have step-free entrances and an accessible WC; the population places one `Place` per
address with `capacity` = design visitors and jobs = staff, opening hours from the use.

### 17.3 How a generator makes them, and how fast

**What exists** (the canon is in `city-generation.md` §3, `civilisations-styles.md` §3 and
`procedural-content.md` §3.3; read there, re-weighed here for speed and licence):

| Method | Kind | Speed | Licence / use | Use in Forge |
|---|---|---|---|---|
| Shadows of Doubt's greedy scored placement (above) | rules on a grid | game rates (in a released game) | technique | **plans, tier 1** |
| Marson & Musse 2010, squarified treemaps | rules | real time | technique | offices and hotels |
| Lopes et al. 2010, constrained growth | rules on a grid | fast | technique | odd footprints |
| Merrell et al. 2010, Bayesian network + optimisation | learned + search | seconds | technique | reference for houses |
| WFC (D-039) | constraint solver | ms per floor | MIT code | tiled interiors (corridors, malls) |
| Wu et al. 2019, room-then-wall networks on RPLAN | learned | fast, needs a GPU | RPLAN research-only | not used |
| House-GAN 2020, House-GAN++ 2021 | learned (GAN on bubble graphs) | fast | code GPLv3 "research purposes only"; LIFULL data | not used |
| HouseDiffusion 2023 | learned (diffusion) | slow | code and weights not for commercial use | not used |
| Graph2Plan 2020 | learned + user graph | fast | original repository without a licence file | not used |
| Infinigen Indoors 2024 | procedural + annealing solver | 8–13 min a room, 34 s a floor layout (CPU) | BSD-3 | offline reference, hero rooms |
| Germer & Schwarz 2009; Tutenel et al. 2009 | rules, agents | ms per room, streamed | technique | **dressing, tier 2** |
| Merrell 2011; Yu et al. 2011 (Make it Home) | density / annealing | seconds | technique | offline polish |
| LayoutGPT 2023 | LLM to layout | seconds, a hosted LLM | MIT code; 3D-FRONT data terms not confirmed | briefs only |
| Holodeck 2024 | LLM to constraints + solver | minutes | Apache-2.0 code, Objaverse assets | briefs only |

Sources for the rows not read in the sibling notes:
- **Wu, Fu, Tang, Wang, Qi, Liu, "Data-driven interior plan generation for residential buildings",
  *ACM TOG* 38(6), 2019, DOI 10.1145/3355089.3356556** [paper] (search record; the dataset's request form
  read in `unaisaralegui/rplanpy`'s README via `gh api`): a living room placed first, then rooms, then
  walls by an encoder-decoder, on RPLAN, more than 80 000 annotated plans obtained through a request
  form. Its licence is described in arXiv 2605.14117's appendix (read): restricted-access, research-only,
  non-commercial, no redistribution in any form.
- **House-GAN** (`ennauata/housegan`, read via `gh api`): the licence file opens with "this code can only
  be used for research purposes" above GPLv3; trained on 117 587 plans from the LIFULL HOME's database.
- **HouseDiffusion** (`aminshabani/house_diffusion`, read via `gh api`): code and weights not allowed for
  commercial use; GPLv3 for research.
- **LayoutGPT** (`weixi-feng/LayoutGPT`, read via `gh api`): MIT, NeurIPS 2023; its 3D layouts use the
  3D-FRONT and 3D-FUTURE datasets, whose agreements were not read to the end (Checked and left out).

**The tiers** (Forge's design; the times are targets to measure):

| Tier | What | When | Budget | Output |
|---|---|---|---|---|
| 0. Facade illusion | interior mapping on every window from the bay grid and the use (lit or dark, curtains, room type colours) | always | free per room (van Dongen 2008) | shader parameters |
| 1. Plan | addresses per floor from the building grammar, then rooms per address by scored greedy placement on the bay grid, doors, hallways | with the building plan, or on demand from the seed | 10–100 µs a room *(estimate)*, ~1 ms a building | room and portal records (D-039), never stored |
| 2. Dressing | furniture and fittings by semantic rules (against wall, facing, clearance, on surface, count) with a fixed number of improvement steps and a pinned seed | when a building's cell opens within ~30–60 m, or a door within ~15 m | 5–20 ms a room on a low-priority job (`procedural-content.md` §4.1) | instances in the cell |
| 3. Clutter and life | small objects by surface (Fisher et al. 2012), occupant traces by the population (who lives here, wealth, tidiness) | when the room is in view | 1–5 ms a room *(estimate)* | instances, decals |
| 4. Hero rooms | landmarks, signature bars and shops: hand-made or solved offline with Infinigen-like annealing, cooked into packages | cook time | minutes a room, offline | package records |

What runs at game rates: tiers 0–3. Only offline: tier 4, learned layout models, LLM briefs.

### 17.4 Interiors on D-039's grid, lined up with the facades

The rules (a synthesis of D-039, `city-generation.md` §3 and the sources above):
1. **The facade decides first** for city blocks: the grammar fixes bays (2.5–3.0 m on the 0.5 m grid),
   piers between window modules, floor heights and the core (stairs and lift) position; the plan
   generator receives a **bay map** per floor: which grid steps are window, pier, door, party wall.
2. **Partitions only on piers:** a partition wall is centred on a grid line that falls on a pier
   module, never across a window module; every habitable room spans at least one window bay; kitchens
   and bathrooms may be interior where ventilated.
3. **Addresses before rooms:** entrance modules on the street and on the core landing define the
   addresses per floor (Shadows of Doubt's floor → address → room), so the flats' front doors match the
   facade's doors and the stair.
4. **Heights:** clear height = floor height − slab (0.3 m) − finishes, at least 2.3 m (2.7 m from a
   3.0 m floor); ground floors of 4.2 m allow a mezzanine for shops and lofts.
5. **Same numbers for the illusion and the real room:** tier 0's interior mapping reads the same bay
   widths, floor heights and room types as tier 1's plan, so walking closer swaps an illusion for the
   room behind it without a pop (the swap distance and its ꟻLIP are the measure).
6. **Portals from modules:** every door and window module is a portal (D-039); a room is a cell; light
   probes stop at room walls.

---

## 18. The city in numbers

**Sources.**
- **Ville de Paris, "Avec 28 commerces pour 1 000 habitants…" (Apur 2023 survey; updated 16 January
  2024)** [web] (read) <https://www.paris.fr/pages/avec-28-commerces-pour-1000-habitants-paris-maintient-sa-densite-commerciale-a-un-haut-niveau-26008>:
  60 846 ground-floor shops, bars, restaurants and commercial services among 83 154 premises, 28 per
  1 000 residents, 10.9 % of premises vacant (9 059) in April 2023. Restaurants and cafés rose from 6 to
  7.3 per 1 000 over ten years (search record).
- **INSEE, "Les conditions de logement en Île-de-France en 2013"** [web] (read)
  <https://www.insee.fr/fr/statistiques/1285809>: Paris dwellings average 58.7 m² and 2.7 rooms, with
  1.9 people per household: 31.0 m² per person (32.1 in Île-de-France).
- **Destatis, press release of July 2026 (dwelling stock at the end of 2025)** [web] (read)
  <https://www.destatis.de/DE/Presse/Pressemitteilungen/2026/07/PD26_250_31231.html>: 44.0 M dwellings
  in Germany, 94.0 m² on average, 49.5 m² of living space per person.
- **Paris housing stock** (search records): about 1.4 M dwellings, 19 % unoccupied, occasional or
  second homes (Apur 2020); about 49 000 private residential buildings (Cerema).
- **Paris schools** (search records, §17.1): 733 écoles, 178 collèges, 159 lycées for about 2.05 M
  people at the 2024 school year.

**Per 1 000 residents** (sources above; design values for a green European city of a million *(estimate)*):

| Item | Paris (source) | Design range per 1 000 | For 1 M |
|---|---|---|---|
| Dwellings | ~680 incl. 19 % unoccupied | 470–520 (2.2 people a household + 5 % vacancy) | 470 000–520 000 |
| Living area per person | 31 m² (Paris), 49.5 m² (Germany) | 35–40 m² | 35–40 M m² |
| Residential buildings | ~24 (private only) | 22–26 (about 20 flats each) | 22 000–26 000 |
| Shops, cafés, services (ground floor) | 28 | 18–25 | 18 000–25 000 |
| of which cafés, restaurants, bars | 7.3 | 5–7 | 5 000–7 000 |
| Primary schools | 0.36 | 0.3–0.4 | 300–400 |
| Lower / upper secondary | 0.087 / 0.078 | 0.08 / 0.07 | 80 / 70 |
| Open-air markets | ~0.033 (70 for 2.1 M) | 0.03 | ~30, each 2–3 days a week |
| Shared gardens | ~0.075 (154) | 0.1 | ~100 |
| Office floor | — | 200 000 jobs × 15 m² (BCO) | ~3 M m² net |

**What it means for interiors** *(estimate)*: about half a million dwellings and 20–25 thousand
commercial units are about 520 000 **addresses**; at five rooms a dwelling and a dozen a commercial
unit, about 2.8 M rooms. Stored as records at 48 bytes each they would be about 130 MB; derived from
seeds they cost nothing until needed. Around a player in a dense street, the facades within 50 m show
about 200 window bays (two sides, 6 floors, 3 m bays), about 100 rooms: tier 2 for all of them is
0.5–2 s of one low-priority core, streamed over the seconds it takes to walk there; entering a building
dresses its current floor (20–40 rooms, 0.1–0.8 s) while the player is at the door. Distinct
**programmes** (use × size class × district style) number in the hundreds; variety comes from the seed
within each. The floor area to dress is never the city's 40 M m², only the few hundred to few thousand
m² the player can see.

---

## 19. Reference cities and games: what makes a place read as one

| Reference | What makes it coherent | Rules a generator can extract |
|---|---|---|
| Paris (Haussmann; planted streets; Seine banks) | one stone, heights from street widths, aligned cornices and balconies (§14) | height function, cornice tolerance, balcony rows; school streets; river park with an emergency strip |
| Barcelona (Eixample; superblocks) | one block size, chamfers, the grid (§14, §11) | block parameters; superblock street hierarchy; squares at inner crossings |
| Copenhagen (harbour, baths, bikes) | the harbour as the city's square; bikes everywhere; cloudburst parks | waterfront profiles with baths; cycle tracks in every profile; sponge parks |
| Nantes and the Île de Nantes | industrial heritage kept and reused (cranes, halls), trams, boats | heritage objects as landmarks; tram profiles; crossings by boat |
| Lyon Confluence | 150 ha of signature buildings round a nautical basin, tram T1, a river park on the Rhône (Berges, 5 km) | waterfront district with varied architecture held by a shared plinth and palette |
| Hamburg HafenCity | flood terraces, promenades at water level, a landmark on a warehouse (§8) | flood-terrace profile; landmark on an edge |
| Rotterdam | multifunctional roofs, water squares (§10, §8) | roof uses by colour; squares as basins |
| Milan, Bosco Verticale | trees on deep balconies (`procedural-content.md` §4.3) | planted balconies only where 3 m deep or more; an accent, not the norm |
| Nice, Monaco, Marseille seafronts | a 7 km promenade; a park over a covered river; a pedestrian old port (§8) | seafront promenade and marina quay profiles |
| Vienna | 45 % green; courtyard blocks with services; Seestadt Aspern (240 ha, half public open space, 255 cars per 1 000 residents, search records) | land budget; courtyard block with services |
| Ljubljana, Pontevedra, Vauban | car-free centres that still serve deliveries and people with reduced mobility (§12–§13) | filters, windows, walking-pace access, free electric carts |
| Singapore (contrast) | greenery counted as a multiple of the site, at any height (§10) | replacement-area accounting; not the look |

| Game or demo | What it shows | Lesson |
|---|---|---|
| Watch Dogs: Legion's London (`city-generation.md` §1) | a real city from 250-module kits; procedural people (Census) | kits plus a real plan; people are where the procedure went |
| Assassin's Creed Unity's Paris (§3 there) | about a quarter of buildings enterable; interior and exterior kits on one grid | the shared grid; a quarter is already a lot |
| Hitman (2016), Paris (search records) | one palace with 60 rooms and over 300 NPCs | a hand-made landmark interior is worth its cost; do few |
| Cities: Skylines II (§15) | European themes per district | a style per district, held by a palette |
| Townscaper (§3 there) | WFC on an irregular grid; charm from rules | the village and paving, not the city |
| Shadows of Doubt (§17) | every room of a procedural city enterable | addresses and scored rooms at game rates |
| The Matrix Awakens / City Sample (§1, §4 there) | 7 000 buildings from 24 kits, US downtown | the architecture to keep (points with attributes, rules to instances); the look to avoid (wide car streets, towers) |

---

## Recommendation for Forge

**The pipeline, in order.** Every stage is a pure function of a `Seed::derive`d seed, the terrain
fields (`Field2` height, slope, water distance, coast distance from `terrain-genesis.md`), a
parameter record and the loaded packages (D-035), writes typed records, and is cached by the
content hash of its inputs, so a style change re-runs stage 6 only and a road edit stages 2–7.

1. **District field.** From the fields and a few seeded anchors (a centre, a harbour, a station,
   the founding point of the old town): land value by distance to the anchors, the coast and the
   arterials; industry downwind and by the rail; parks where the slope, the stream or the coast
   forbid building; the old town's radius. Output: a `Field2<DistrictSample>` (kind weights,
   density, founding age, style set id) at 8 m, plus a road *pattern field* per district (grid
   with an orientation, radial, contour-following, organic). Cost: one pass, milliseconds.
2. **Roads.** Chen et al.'s tensor field summed from the district patterns (grid elements
   oriented per district, radial elements at squares, boundary elements along the coast and the
   rivers, the height gradient where the slope is steep), smoothed; highways and arterials traced
   first as major streamlines with a 400–800 m separation under Parish & Müller's constraints
   (snap to crossings within 30 m, stop at water, bridge below a width, reject above a slope),
   collectors at 150–250 m, streets and alleys at 40–100 m along the minor direction, all with
   pinned seed order (D-016). The village district uses Emilien's interest maps and Galin's cost
   paths instead of the field, under the same `RoadGraph`. Output: `RoadGraph` (nodes with a kind,
   edges with a class, a width, a profile id and a polyline). Cost: a city of 4 km traces a few
   hundred kilometres of streets in well under a second; the graph's planar faces are the blocks.
3. **Blocks and lots.** Blocks from the faces inset by half the street and the sidewalk; lots by
   Vanegas et al.'s OBB recursion (target area per district, every lot touching a street) in
   gridded districts and by straight-skeleton strips (lot depth per district, the courtyard or
   garden left inside) in the old town and the village; Emilien's anisotropic growth for the
   village's slope lots. Output: `Block` and `Lot` records with stable ids (hash of the block's
   centroid cell and the lot's frontage index), a frontage edge, an area, a use. Cost: ~2 000
   blocks and 10–20 k lots for the 4 km city, milliseconds.
4. **Landmarks and overrides.** Package records (D-035) that name a lot id or a footprint polygon
   and a mesh or a grammar override; merged into the plan as Lipp et al.'s layers (a park removes
   its lots and re-seals the roads; a landmark reserves its lot and a sightline corridor).
   Nodes for landmarks are proposed by Lynch's rule (ends and bends of arterials, squares) and
   accepted by the owner. Output: `Landmark` records; a skyline envelope per district.
5. **`CityPlan` and its preview.** The records serialised as RON (D-035), hashed for D-016's
   digest, and drawn as a PNG map (roads by class, blocks by district, lots, landmarks) by the
   `genesis`-style tool, so the plan is judged on the map before any building exists.
6. **Buildings by grammar.** Per lot, a derivation seeded by the lot id: the mass model (boxes
   fitted to the lot under the district's height and setback rules; the roof by the straight
   skeleton), `comp` into facades, `split` into floors by the style's floor heights, `repeat`
   into bays on the module grid, context queries for corners, party walls and the street side,
   `insert` a module per bay from the style set; the ground floor from the lot's use (shop,
   entrance, garage); the plan per floor (Lopes's growth for houses, Marson & Musse's treemap for
   repeated floors) as room records, not geometry. Output: `BuildingPlan` (mass, floors, module
   instances on the grid, rooms, portals, nav tags, the proxy's id). Cost: 10 k buildings × a
   few hundred rule applications, seconds on the job system.
7. **Assembly and hand-off.** Module instances written to the instance table (a compute pass
   expanding `BuildingPlan`s a thread per module, as `place_main` does for props today, or the CPU
   for a village), each building a #38 cell with its proxy; props (lamps, benches, trees, signs)
   placed by the existing GPU pass reading the roads and lots; the ground's layer map (#42) drawn
   from the roads' profiles and the lots (streets, sidewalks, plazas, gardens); the proxies cooked
   through the DAG cook and cached; rooms and portals kept for #88.

**The module grid (the part that cannot be retrofitted).** Horizontal step 0.5 m, module widths
in whole steps (bays of 2.0–4.0 m; fillers of 0.5 and 1.0 m); floor heights per style from a small
set (2.8, 3.2, 3.6, 4.2 m; a style uses two or three); exterior wall modules 0.5 m thick occupying
the footprint's outer step, interior modules starting inside it; a module's origin at its bottom
left on the exterior face, +Y up, facing −Z outwards (the props' convention); rotation in quarter
turns only, a `u8`; a module's footprint is its `(steps_x, floors, steps_z)` box, checked at cook.
Every module carries: a `ModuleId` in its style set, its variant count, its material rows, its
semantic tags (portal polygon for doors and windows; walkable slab polygon; stair link; structural
role and `breaks_into` for #89), and its DAG in the cache like any prop.

**The style set.** A package record: the module kit (generators and parameters, cooked to the
cache), the control grammar's attributes (floor heights, bay widths, window ratios, roof types,
storefront rules, the cornice and plinth choices), the regional material set (the trim sheets and
tileables of `vegetation-materials.md` §4), the decal atlas and a contrast rule against
neighbouring districts. First two: *Mediterranean village* and *downtown*, because #91 and
city-blocks need them; then *residential*, *commercial*, *industrial* and *old town* for #85's
list.

**The data model, as Rust records** (names to be argued in the code, not here):

```
CityPlan { seed, frame: f64 origin, districts, roads: RoadGraph, blocks, lots, landmarks, buildings }
District { id, kind, style: StyleSetId, anchors, density, founded }
RoadGraph { nodes: [RoadNode { pos, kind: Crossing|End|Bridge|Square }],
            edges: [RoadEdge { a, b, class: Highway|Arterial|Collector|Street|Alley|Path,
                               width, profile: RoadProfileId, polyline }] }
Block { id, face: [edge ids], district, inset polygon }
Lot { id, block, polygon, frontage: edge id, area, use: Residential|Shop|Office|Civic|Park|…, seed }
Landmark { lot or footprint, source: Mesh(id) | Grammar(override), from: PackageId }
BuildingPlan { lot, style, seed, mass: [Box], floors: [f32], modules: [ModuleInstance],
               rooms: [Room], portals: [Portal], proxy: MeshId }
ModuleInstance { module: ModuleId, cell: IVec3 (grid steps, floor, steps), turns: u8, variant: u8 }
Room { floor, polygon, kind, portals: [index] }    Portal { module instance, polygon, rooms: [2] }
```

**What to measure** (the demo pages, `docs/PROFILE.md`, and the F1 overlay for every new pass):

- generation time per stage for the 4 km city and a 1 km village, cold and from the cache, one and
  six workers with equal digests (D-016);
- the plan's counts against the City Sample's: kilometres of road by class, blocks, lots,
  buildings, module instances, unique modules per style set, and the largest bay-count sequence
  repeated along any street (the repetition metric the owner's eye will apply);
- the GPU frame at the south view and on the flight, module buildings against today's single
  meshes: the instance cull's cost against the instance count, clusters drawn, the page pool's
  residency (target: within city-blocks' 3.38 ms at 1440p with everything on, and the flight
  with no holes in the 512 MiB pool);
- the proxy swap: the distance at which a building's cell closes to its proxy, the ꟻLIP between
  the modules and the proxy at that distance (`tools/compare.sh`), and the number of cells open
  per view; the A/B harness and the mesh path against the fallback still at 0 pixels;
- memory: the style set's pages, the proxies' pages, the instance table, against §4's estimates;
- the look, in captures the owner judges: a street at eye height in each style set, the skyline
  from the harbour, the village from the hill, all with TAA on.

**What to build first, without a GPU (best cost/benefit).**
1. `forge-procgen::city`: the district field, the tensor field and the tracer, the block faces,
   the map PNG. Two or three days; the map decides whether the layout reads before anything else.
2. Lots by OBB and by skeleton strips, with the skeleton routine tested against campskeleton.
3. `CityPlan` records, digests, the landmark layer merge.
4. `forge-procgen::grammar`: scopes, split/repeat/comp/insert, context queries, a text form
   for the rules (RON, P6), a derivation test that reproduces a hand-built module list.
5. `forge-geom::modules`: the module generators from today's `building()` pieces, the two first
   style sets, the footprint checks, the cache.
6. Assembly into the instance table and the proxies; the demo `city-blocks --plan SEED`
   beside today's grid, then the village on the island (`island --village`), with the numbers.

**Proposed decision entry for #86** (for `docs/DECISIONS.md`, 🟡 until the owner accepts it):

> ## D-039 — Buildings are grammar-derived assemblies of kit modules; a style set per district;
> a proxy per far building 🟡 (proposed 2026-09-26)
>
> A building is a `BuildingPlan`: a mass model fitted to its lot, refined by a CGA-style split
> grammar (split, repeat, component split, insert, context queries) whose terminals are modules of
> a style set, never raw geometry, except the roof surface from the lot's straight skeleton and
> authored landmarks. Modules sit on a grid of 0.5 m steps and per-style floor heights, exterior
> walls in the footprint's outer step so interiors share the grid, rotated in quarter turns; each
> module is a cooked cluster-DAG prop carrying its material rows, its variants and its tags
> (portals, walkable slabs, stair links, structural role, break-up). A style set is a package
> record: module generators and parameters, the control grammar's attributes, a regional material
> set and a contrast rule; districts assign style sets. Module instances are ordinary instances;
> a building is a cell of the instance hierarchy with a merged proxy cooked from its assembly,
> drawn when the building's projected size falls below a threshold, and blocks get a proxy beyond.
> Rooms and portals are records of the plan, instantiated when #88 streams them. Everything is a
> pure function of the seed, the terrain fields and the packages (D-016, D-035).
> *Not chosen:* unique geometry per building from the grammar (no instancing, no unit for
> interiors, destruction or navigation, hours of cooking per city); hand-assembled kits (no art
> team, and the assembly is what the grammar automates); Wave Function Collapse as the primary
> generator (no hierarchy; kept for interiors and the village's irregular grid); CityEngine or
> Houdini in the loop (commercial, editor-time, not a pure function on the client and the server).
> *(research: city-generation.md §3–§6; procedural.md §2, §4; vegetation-materials.md §4; issues
> #84, #86, #88, #89, #91)*

---

## Recommendation for Forge (extension)

**Verdict.** Nothing in the owner's city needs a new kind of generator; it needs `city-generation.md`'s
pipeline to carry more **records** (water, green, transit, uses), one **rule file** read by every
stage, **checks** that every stage's output passes, and a **tiered interior generator** that copies a
shipped game's method rather than a research solver. Everything stays a pure function of the seed,
the fields and the packages (D-016, D-035, D-053).

**1. The `CityPlan`, extended** (names to be argued in the code):

```
CityPlan { …, geo: GeoFrame, water: Water, green: Green, transit: Transit, uses: UseMap, rules: StyleBibleId }
GeoFrame { origin_lat_lon: (f64, f64), rotation: f64, size_m: f64, dem: SourceRef, flattened: bool }
Water   { coast: Polyline, river: [Reach { centreline, width, depth, type: Quay|Park|Mouth, weirs }],
          basins: [Polygon], edges: [WaterEdge { polyline, profile: QuayWall|FloodTerrace|Seafront|Marina|Bath|Beach }] }
Green   { parks: [Park { polygon, kind, programme }], corridors: [Polyline], street_trees: rule,
          roofs: per-building RoofUse (Green|Blue|Yellow|Red), baf_per_lot }
Transit { lines: [Line { kind: Tram|Boat|Shuttle, stops: [StopId], path, headways }],
          stops: [Stop { pos, kind, platform }], crossings: [BoatCrossing { a, b }], depots, hubs }
RoadEdge { …, profile: TramAvenue|TramStreet|Service|Shared|Pedestrian|Promenade|GreenTrack,
           filters: [Filter { classes, hours }], emergency_strip: bool }
UseMap  { per lot: frontage class (active/residential/service), per floor: Use, per address: PlaceKind }
Landmark { …, sightline: Polygon, anchors: Node|Edge|Bend }
```

**2. The building-use catalogue as data** (a D-035 package record per use): rooms with sizes and
adjacencies, furnishing rules, design and maximum occupancy, staff, hours, the frontage and floor
rules where the use may go, the population's `PlaceKind`. §17.2 is its first draft.

**3. The interior generator, in tiers** (§17.3): interior mapping always; plans by scored greedy
placement on the bay grid (Shadows of Doubt's method) with treemaps for offices and hotels and WFC for
tiled interiors; dressing by semantic rules in a fixed step budget when a cell opens; clutter by surface;
hero rooms cooked offline. No learned floor-plan model (licences, determinism), no runtime annealing.

**4. The consistency rules as checks** (§15): the style bible file, read by the district field, the
road tracer (profiles), the lot step, the grammar, the plan generator and the dresser; a checker crate
that computes every metric of §15 from the records and fails the cook with the lot, frontage or building
named. The checks run on the map before any building exists (heights, access, green, stops, sightlines)
and on the buildings after.

**5. The first style set.** D-039 named a tropical medieval village first and the downtown second; the
owner's direction of 2026-10-10 makes a **contemporary European set** the one that every current demo
and the sandbox need. Recommended order: the contemporary European set first (renovated and new
perimeter blocks, green roofs, active ground floors), the village after. This is a change to D-039's
order, so it needs the owner's decision (a 🟡 entry).

**6. The real site** (§7): `GeoFrame` and the coast and river from GLO-90 and EU-Hydro, so the flat
city can be placed on the planet later.

**For a decision later (D-0xx):** the style bible's format (TOML, as D-053) and who may override what;
the street profile set and the filter model; the interior tiers' budgets; whether the checks fail the
cook or only report; whether D-039's first style set changes.

---

## What to measure (extension)

**First, the map image, judged before any building exists** (`city-generation.md`'s step 5, extended):
roads by profile, tram lines and stops, boat crossings, filters, blocks by type, lots by use and
frontage class, parks by kind, water edges by profile, landmarks with their sightlines; with the
check overlays: dwellings beyond 300 m of a 1 ha park (red), beyond 500 m of a stop, entrances beyond
60 m of an emergency strip, lots under their BAF target, street space by class per district. *Measure:*
generation time per stage, the counts (km of each profile, stops, bridges, crossings, parks, lots per
use), the shares (land: built, streets, parks, water; street space: walking, cycling, lanes), every
check's pass rate, the digest at one and six workers.

**Then, measured steps:**
1. **Water edges and bridges** on the map; then one waterfront and one river section built in 3D.
   *Measure:* edge km per profile; stairs, bridges and pontoons per km against the spacing rules.
2. **One block per type** with the style bible's checks. *Measure:* every check's value; cornice runs;
   opening misalignment (zero); palette shares.
3. **Interiors, tier by tier** on one block: µs per room for plans, ms per room for dressing (p50/p99),
   rooms per building, the swap distance from interior mapping to rooms and its ꟻLIP, zero rooms
   without a window bay, zero partitions across a window.
4. **One district with its uses and places** handed to the population (`population-simulation.md`
   step 3): capacities and jobs per place against §17.2; occupancy curves by hour.
5. **The night:** lamps by rule, dimming around people, stars visible from a roof; frame time with and
   without dimming.

**The "day in the city" flight** (the owner's accepted idea): a scripted camera path through every
district at fixed game times — morning commute (07:45), lunch terraces (12:30), the evening on the
quays (19:30), the night (23:30) — and on a market morning and a festival evening, recording per
frame the GPU time by pass (F1 zones), the draw counts (instances, people, lights) and captures at
fixed points compared with the accepted set (`tools/compare.sh`, ꟻLIP), with the culling A/B at 0
pixels. It is the city's regression check and the owner's first look at every change.

---

## What the numbers say

Layouts: the City Sample's 16 km² hold 260 km of roads, 512 km of sidewalks, 1 248 intersections
and 7 000 buildings of hundreds of instances each, 8 M instances in all, from 24 kits of about 85
meshes; Watch Dogs: Legion's industrial family alone is 250+ modules; The Division built Manhattan
from a footprint and a kit per building with a choice per wall tile; Weber et al.'s geometric
growth simulation runs at about a second per time step; Vanegas et al.'s parcels are interactive
and persistent across edits. Forge today: a 2.4 km grid of 100 m blocks and 20 m streets, four
buildings per block from twelve single meshes of 0.5–3 M triangles, a million instances (the props
25.8 M triangles in 612 k clusters, 983 MiB of pages), the frame at 1440p 3.38 ms with everything
on, the culls 0.72 ms, the instance 80 bytes. The plan's estimates, to be replaced by the demo's
timings: a 4 km city of ~2 000 blocks, 10–20 k lots and 5–10 k buildings traced and lotted in
under a second and derived in seconds; 1.5–3 M module instances (120–240 MB); a style set's pages
in the tens to hundreds of megabytes against 4 MB per unique building at 100 k triangles (20 GB
for 5 000); proxies at 5–10 k triangles each, 1–2 GB streamed by cell. Interiors: a quarter of
Assassin's Creed Unity's buildings were enterable; interior mapping costs nothing per room.

---

## Patents

Found with WebSearch and Google Patents pages on 2026-10-10. Statuses and expiry dates are Google
Patents' estimates, which it labels assumptions, not legal conclusions; claims were read only where
said. This is a flag list for the owner, not a freedom-to-operate review. A patent on physical
infrastructure (a power rail, a guided vehicle, a vacuum pipe, a lamp) is practised by building that
thing, not by drawing it in a game; only patents on software methods bear on the engine.

| Patent | Holder | Jurisdiction | Status (as listed) | Expiry (estimate) | Technique | Alternative |
|---|---|---|---|---|---|---|
| US 12,220,640 B2, "Generative interior design in video games" | Electronic Arts | US | granted 2025-02-11, active | 2043-10-29 | claim 1 (read): a user's boundary and constraints (with an in-game budget) → a floor-plan generator model → a layout generator of graph neural networks adding object nodes by sampled types and edges, rendered in play | rule-based scored placement (Shadows of Doubt, 2019, public) and semantic dressing rules; no GNN layout model; check the claims before any learned or player-driven furnishing |
| US 10,427,046 B2, "System and method for game object and environment generation" | Take-Two Interactive | US | granted 2019-10-01, active | 2037-12-22 | claim 1 (read): an asset store with metadata records (identifier, property tag) loaded by a rendering engine on requests from environment systems; procedural interiors from modular parts, an ordered graph and parameter blocks in the description and dependent claims 3–5 | D-039's modules and grammar are published techniques (CGA 2006, Skyrim's kits 2013); read the claims before shipping a tag-matched interior assembler |
| US 10,380,790 B2, "System and methods for generating procedural window lighting effects" | Robert Bosch GmbH | US | granted 2019-08-13, active | 2035-12-22 | claim 1 (read): floor boundaries as texture-space ranges per floor, a per-pixel pseudo-random lit/unlit test, a window lighting layer; interior mapping in texture space in dependent claim 3 | van Dongen's interior mapping (2008, world-space rooms, cited as prior art by the patent); Forge's rooms come from the module grid in world space |
| US 12,293,452 B2, "Room shaders for animated movies" | Sony Group, Sony Pictures | US | granted 2025-05-06, active | 2039-12-05 | stacked painted planar room layers with parallax offsets across windows (summary read) | ray-cast interior mapping (2008) |
| US 9,785,728 B2, "Computer system and method for generating a 3D geometric model" | Esri (from ETH Zurich and Procedural AG) | US | granted 2017-10-10, active | 2029-04-26 | claim 1 (read): rectify a facade photograph, split it into floors and tiles by symmetry, match regions to a 3D library, build the model (Müller et al. 2007, cited in `city-generation.md` §3) | author style grammars by hand (D-039's control grammar) instead of deriving them from photographs; or wait for 2029 |
| US 9,129,432 B2, "Image-based procedural remodeling of buildings" | Hong Kong University of Science and Technology | US | granted 2015-09-08, active | 2031-10-07 | aerial and ground photographs fused, grammar rules extracted to remodel facades (summary read) | no reconstruction from photographs in Forge |
| EP 0 979 176 B1 (APS) | Innorail | EP | **expired** (lifetime) | 2018-03-16 | ground-level power supply: segments live only under the vehicle, others earthed (summary read) | free to use |
| US 7,932,625 B2, "Ground-level power supply circuit, especially for a tramway" | Alstom Transport Technologies | US | **expired** (fee-related, as listed) | — | APS circuit (search record) | free to use |
| US 8,469,166 B2, "Ground-based power supply system for a transportation vehicle" | Alstom (Holdings) | US | granted 2013-06-25, active | 2031-12-16 | contactless inductive supply from buried emitters under the vehicle (summary read) | irrelevant to a game that draws trams; battery-and-charge trams need no ground system |
| EP 4 739 531 A1, "Autonomous vehicle for road and rail" | Mordechai Teicher | EP | published application (May 2026) | — | an autonomous road vehicle carried by a railcar (search record) | the owner's public taxis drive on paved tram lanes as road vehicles (Wikipedia, "Reserved track"); nothing to practise |
| EP 2 158 142 B1 and US 9,073,706 (vacuum waste collection) | Envac AB | EP, US | granted (search records) | not checked | collection stations and emptying sequences | depiction only |

**Looked for and not found** (searches only): a patent by Procedural Inc. or Esri on the CGA grammar
itself (only US 9,785,728 on facades from images); a patent on Shadows of Doubt's method; a patent on a
public automated taxi sharing tram track in mixed traffic; patents on the 3-30-300 or BAF checks
(regulations, not inventions); a European counterpart of US 9,785,728 (not searched). The navigation
patents of Take-Two (US 11,071,916 B2 and its continuation) remain flagged in `population-simulation.md`.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **"Building the World of The Division" as a GDC 2016 talk** — not found under that title. The
  GDC 2014 Snowdrop showcase and the 80.lv "Trade Secrets of Game City Building" article stand for
  Massive; a figure of "70 million procedurally placed instances" appears in secondary pages
  without a primary source and is not used.
- **A Watch Dogs: Legion talk on the city's layout** — the GDC 2021 sessions are about Census, the
  procedural population and mission systems; the city was authored over the real map. The Ubisoft
  article and the kit pages are cited instead.
- **GTA V's layout method** — no primary source; Rockstar's development pages mention field trips
  and "Google Maps projections of Los Angeles" for the road network, and press pieces the "spatial
  compression" of sightlines. Cited only in passing under Lynch.
- **An Assassin's Creed talk on building generation** (Unity's Paris, Origins' Houdini placement,
  Shadows) — the GDC talks found are on crowds, networking and dialogue; Origins used Houdini
  Engine for placement (80.lv). The Unity entry is graded weak and rests on interviews and
  portfolio pages.
- **"Unity's procedural buildings"** — there is no such Unity feature; the Asset Store tools are
  listed under §5 instead.
- **Introversion's Subversion at GDC** — no conference talk was found; the Imperial College demo,
  the blog and the 2007 press are what exist.
- **Battlefield's destruction and the Frostbite talks** — #89's subject, out of this file's scope;
  the module's structural tags are the hook left for it.
- **Navigation and traffic (Mass, ZoneGraph, navmesh generation)** — #87 and #90, out of scope;
  the nav tags on modules and the road classes are the hooks.
- **Machine-learned layouts** (GlobalMapper 2023, CityGen 2023, DRoLaS 2025, Neural Turtle
  Graphics 2019, the diffusion floor-plan papers) — seen in search results; not adopted for the
  same reason as in `terrain-genesis.md`: not a pure function of a seed across machines (P3/P8),
  and no guarantee of a connected road graph. Listed so nobody re-derives the omission.
- **BigSUR and FrankenGAN (Kelly et al. 2017, 2018)** — reconstruction from data, not generation;
  only chordatlas is cited as the code host of the extrusions paper.
- **Straight-skeleton crates in Rust** — not surveyed; CGAL and campskeleton are the references and
  the routine is small enough to own.
- **Stefan Huber's straight-skeleton work and "On Implementing Straight Skeletons" (SoCG 2020)** —
  seen; the robustness notes are worth reading when the routine is written, not cited as method.
- **The 80.lv Matrix Awakens breakdown's author and date, the City Sample's Fab page text** — the
  hosts are blocked; the numbers come from the search engine's extracts of those pages.

---

## Checked and left out (extension)

- **The EU Bathing Water Directive's thresholds** (Annex I of 2006/7/EC): the EUR-Lex page returned no
  readable text and Wikipedia omits the numbers; a search tool's recollection of them was not used.
- **Basel's own swimming page and the Ljubljana city page**: refused (403) or gone (404); their facts
  are search records, marked as such.
- **The chamfer length of Cerdà's blocks**: no source read gave it (only the 45° angle); left out.
- **Urban consolidation centres**: the Wikipedia article returned 404 and no primary evaluation was
  read; the practice (a depot at the edge splitting loads onto cargo bikes and small vans) is described
  without figures.
- **Outdoor cinema and street sport on car-free streets**: common practice, but no checked source was
  found; the generator's programme list includes them as design choices.
- **School area guidelines (BB103)**: the gov.uk notes page gave no figures and the PDF was not
  fetched; the 55–62 m² classrooms are a search record from a London planning document.
- **Hotel staffing ratios**: only vendor blogs and a 2002 Greek study (search records); marked as an
  assumption.
- **The 3D-FRONT and 3D-FUTURE data agreements**: the retrieved texts stopped before their purpose
  clauses; whether they forbid commercial use was not confirmed.
- **Graph2Plan's licence**: the original repository has no licence file; a fork is GPL-3.0; not used.
- **Monaco's Mareterra extension, Rotterdam's Markthal, Copenhagen's Nordhavn**: not fetched; the
  waterfront and roof references above cover their lessons.
- **DarkSky's principles page, the WHO noise guidelines PDF, Bruitparif's PDFs, the OECD superblock
  chapter, Bloomberg/CityLab on superblocks, the ISGlobal press release**: refused or PDF; used as
  search records where cited.
- **Learned city and floor-plan generators beyond those listed** (MaskPLAN, diffusion layout papers of
  2025–2026): seen in search results; not adopted for the reasons of `city-generation.md` (not a pure
  function of the seed across machines) and the licences above.
- **Underground logistics (Woven City)**: a real proposal, left out because the owner's brief excludes
  the futuristic.

---

## Verification notes

Checked on 2026-09-26 with WebSearch, WebFetch and the GitHub API only; no browser pane and no
video pages. The session's egress proxy served `github.com` and refused every other host tried:
sci.utah.edu, twak.org, history.siggraph.org, dev.epicgames.com, sidefx.com, blog.joelburgess.com,
level-design.org, gamedeveloper.com and 80.lv all returned "blocked by the network egress proxy",
and the hosts `terrain-genesis.md` listed as blocked (doi.org, dl.acm.org, onlinelibrary.wiley.com,
arxiv.org, gdcvault.com, en.wikipedia.org, docs.unity3d.com) were not retried. Verification has two
grades; every entry not in the first carries "(verified through search results)" on its URL line.

- **Fetched and read (GitHub):** mxgmn/WaveFunctionCollapse (README and LICENSE, MIT);
  ProbableTrain/MapGenerator (README; LGPL-3.0/GPL-3.0); tordanik/OSM2World (MIT);
  marian42/wavefunctioncollapse (MIT); twak/chordatlas and twak/campskeleton (Apache-2.0);
  Esri/cityengine_for_unreal (README with the licence terms; Apache-2.0 source);
  sideeffects/HoudiniEngineForUnreal (README); sideeffects/SideFXLabs (README);
  gridbugs/wfc (MIT); phiresky/procedural-cities (README); a third-party description of
  Townscaper's dual grid (kai-denrei/oskar-procedure), used only to check the grid steps the
  Game Developer article describes. Quotes from these are verbatim from their READMEs. The
  issues #84, #85, #86, #88 and #91 were read through the GitHub API.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  volume, pages, DOI, and the sentences quoted, which are the search engine's extracts of the page
  named): Parish & Müller 2001 (ACM DL, SIGGRAPH history, the ETH PDF listing); Sun et al. 2002
  (ACM DL); Kelly & McCabe 2006 and Citygen 2007 (TU Dublin Arrow, Semantic Scholar); Chen et al.
  2008 (ACM DL, the Utah project page listing); Aliaga et al. 2008 (ACM DL); Weber et al. 2009 (EG
  diglib, KAUST, the Wonka PDF listing); Vanegas et al. 2010 and 2009 (Wiley, ACM DL); Vanegas
  et al. 2012 inverse design (ACM DL, Purdue, SIGGRAPH history); Nishida et al. 2016 (Wiley,
  Purdue); Emilien et al. 2012 (Springer) and Galin et al. 2011 (as in `procedural.md`);
  Subversion (Wikipedia, Engadget 2007, the PCG wiki); Watch Dogs: Legion (Ubisoft News, Ubisoft
  Toronto's GDC 2021 page, ArtStation); The Division (80.lv, Engadget's GDC 2014 report, MCV);
  Cyberpunk 2077 (GDC Vault 1027571 and 1028734, 80.lv); the City Sample (dev.epicgames.com
  listings, 80.lv, CG Channel, Fab); watabou's generator (itch.io devlogs); Vanegas et al. 2012
  parcels (Wiley, ACM DL, EG diglib, the White Rose PDF listing); Aichholzer et al. 1995 (JUCS,
  Springer); Lipp et al. 2011 (TU Wien, ACM DL, Kesen's EG 2011 list); Lechner et al. 2006 (ACM DL,
  the Northwestern TR PDF listing) and Groenewegen et al. 2009 (TU Delft); Lynch 1960 (MIT Press,
  Wikipedia); Wonka et al. 2003 and Müller et al. 2006 (ACM DL, as in `procedural.md`); Müller
  et al. 2007 (ASU, KAUST, the KU Leuven PDF listing); Talton et al. 2011 (ACM DL, SIGGRAPH
  history, the GitHub export); Kelly & Wonka 2011 (ACM DL, SIGGRAPH history, the Wonka PDF
  listing); Schwarz & Müller 2015 and the SIGGRAPH Asia 2015 course (ACM DL, SIGGRAPH history);
  Burgess & Purkeypile 2013 and the 2016 follow-up (Game Developer, the blog listing, SlideShare,
  level-design.org); Warframe (MCV's GDC 2013 report, the Game AI Pro PDF listing); Karth & Smith
  2017 (ACM DL, dblp, eScholarship); Stålberg 2018 and 2019 (the EPC and IndieCade listings, Game
  Developer); Merrell et al. 2010 (ACM DL, dblp); Lopes et al. 2010 (the TNO PDF listing, Bidarra's
  list); Marson & Musse 2010 (Wiley/Hindawi, Semantic Scholar); van Dongen 2008 (the Proun PDF
  listing, Papers We Love); Luebke & Georges 1995 (Semantic Scholar, the luebke.us PDF listing);
  Assassin's Creed Unity (TechRadar, ArtStation); SideFX Labs' building tools (sidefx.com tutorial
  listings); Karis 2021 (the Advances 2021 PDF listing); Epic's HLOD, PCG and City Sample pages
  (dev.epicgames.com listings, unrealengine.com); Unity's ProBuilder page and BuildR's forum
  thread; CityEngine's Datasmith and export pages (doc.arcgis.com); Houdini Engine's product page.
- **Weaker confirmations, stated plainly.** The Bethesda kit rules (footprint, half-footprint snap,
  large grids) are paraphrased from summaries of the transcript, not quoted, because the transcript
  hosts are blocked. The City Sample's "hundreds of instances" per building and the 24-kit/2 000-mesh
  figures are from Epic's and CG Channel's pages as extracted by the search engine; the 16 km²,
  260 km, 512 km, 7 000 and 1 248 figures from 80.lv's breakdown. The Division's building-tool
  sentence is 80.lv's rendering of the Snowdrop team's words. Lipp et al.'s and Müller et al. 2007's
  DOIs are from the search record, not the publisher's page. Warframe's "at least 30 tiles" per
  tile set is from a fan wiki and is not used as a number. Assassin's Creed Unity's "a quarter"
  is an interview figure. Vanegas et al. 2012's two schemes (OBB and straight skeleton) are
  described from the abstract and from citing pages, not from the paper's body. Lipp et al.'s DOI
  is from the search engine's extract of the Eurographics PDF, not the publisher's page. Article
  numbers the search record did not show (Chen 2008, Vanegas 2012 inverse design, Merrell 2010,
  Talton 2011) are left out rather than remembered. The Matrix Awakens team size and duration are
  80.lv's.
- **Forge's own numbers** (the 2.4 km grid, 100 m blocks, 20 m streets, twelve buildings of
  0.5–3 M triangles, 25.8 M triangles in 612 k clusters, 983 MiB of pages, 80-byte instances,
  3.38 ms at 1440p, 0.72 ms of culls, the 12 s cook of 8 M triangles) are from
  `docs/demos/city-blocks.md` and `crates/forge-render/src/placement.rs` as of 2026-09-26.
- **Numbers to re-check before they enter a spec:** every figure in §4's memory paragraphs and in
  the recommendation's cost lines is an estimate, marked as such, to be replaced by the demo's
  printed timings; the module grid (0.5 m, the floor-height set) is a proposal for the owner, not
  a finding; the separation distances of the road classes are starting values for the map preview.

---

## Verification notes (extension)

Checked on 2026-10-10 with WebFetch, WebSearch and `gh api` (for GitHub files) only; no browser pane;
nothing downloaded into the repository or on purpose. One fetch of ISGlobal's press release returned a
PDF that the fetch tool cached itself in the session's tool-results folder; it was not opened further
and is not cited as read. "Read" means the page or file was returned and the facts are from it;
"search record" means only a search engine's extract was seen.

- **Read:** Copernicus EU-Hydro product page; hydrosheds.org HydroRIVERS; openstreetmap.org/copyright;
  Wikipedia (en): HafenCity, Copenhagen Harbour Baths, Old Port of Marseille, Promenade des Anglais,
  List of crossings of the Seine, Copenhagen Harbour Buses, Amsterdam IJ Ferries, Batobus, Vaporetto,
  Nantes tramway, Strasbourg tramway, Low-floor tram, Ground-level power supply, Reserved track,
  Woonerf, Vauban (Freiburg), Pontevedra, Cycling in Copenhagen, Strøget, Paris-Plages, Haussmann's
  renovation of Paris (both halves), Eixample, Karl-Marx-Hof, Form-based code, 15-minute city,
  Sidewalk Toronto, Songdo International Business District, Masdar City, Woven City, Decidim, Sponge
  city, Automated vacuum collection, Vienna U-Bahn, Var (river); Wikipedia (fr): Tramway de Nantes,
  Navibus, Alimentation par le sol, Promenade du Paillon, Berges du Rhône, Île de Nantes, Règlements
  d'urbanisme de Paris, Lyon Confluence; Wikipedia (de): Rasengleis (Grüngleis), Urbanes Gebiet;
  paris.fr (Seine bathing quality, Seine biodiversity, rues aux écoles, Apur 2023 commerce); INSEE
  1285809; Destatis July 2026 release; CBS 9 December 2025; EEA briefing 15/2021; Climate-ADAPT (Berlin
  BAF, Hamburg roofs, Nijmegen); URA circular DC14-12; MVRDV Rooftop Catalogue; rotterdam.info; De
  Urbanisten Benthemplein; PMC (Konijnendijk; Scott et al.; Puig-Ribera et al.); wien.gv.at urban area
  statistics; Ewing and Bartholomew's excerpt (Viewpoint Vancouver); G. K. Still's crowd density page;
  Barcelona's Superilles page; IDFM's T7 page; Cambridge Connect's UITP page; Ghent's circulation plan
  page; TØI report 1765/2020 page; POLIS article; PIK press release (second half); Fraunhofer IML;
  Resilience.org on CycleLogistics; Légifrance (arrêté 31 January 1986, art. 4); Le Bonbon Nantes;
  Hochbahn ALIKE; Toyota newsroom 31221914; Helen Eiranranta; ENGIE Fraîcheur de Paris; WUSF/NPR
  Copenhagen; COST LONNE French decree summary; Tvilight case study; gtfs.org realtime reference;
  gov.uk NDSS (both pages); Northamptonshire FRS floor space factors; BCO July 2025 news;
  Designing Buildings Wiki restaurant page; ColePowered DevBlog #13; arXiv 2605.14117 (HTML appendix)
  and 2602.22507 (abstract and HTML); Google Patents US 12,220,640 (claims), US 10,427,046 (claims),
  US 10,380,790, US 12,293,452, US 9,785,728, US 9,129,432, US 8,469,166, EP 0 979 176 B1; celog
  substack (Eixample); Gardenvisit colour planning. Via `gh api`: `unaisaralegui/rplanpy` README;
  `ennauata/housegan` licence and README; `ennauata/houseganpp` and `aminshabani/house_diffusion`
  metadata and licence; `weixi-feng/LayoutGPT` README and metadata; `allenai/Holodeck` metadata.
- **Search record only:** the Seine's 13 km in Paris; Parc Rives de Seine's 3.3 km and emergency
  access; Paris markets; Barcelona superblocks' size, speed, access and 503-plan; Mueller et al. 2020;
  Sant Antoni NO₂ and noise; Copenhagen's green roof rule and district heating share; Stockholm's
  Värtan; Katri Vala; Paris oasis schoolyards; Amsterdam gas-free 2040; WHO 2018 noise values;
  Bruitparif car-free-day results; DarkSky principles; Toulouse's Kara; the 400 m catchment; the Dutch
  street-space study; Ljubljana's Kavalir and zone; Basel's Rhine swimming; Paris's 10 mm rain rule;
  Rotterdam's roof colours and the water square's 1 700 m³; guard heights; Brighton's swift bricks;
  Paris's shared gardens; Paris schools and class sizes; BB103 classrooms; hotel ratios; BCO 2018's
  9.6 m²; Paris's café-restaurant density, housing stock and vacancy; Paris's commercial-frontage
  protection; Turin's Brino plan; Cities: Skylines II packs; Hitman's Paris; Wu et al. 2019;
  Seestadt Aspern; Luxembourg's and Strasbourg's tram depots; US 7,932,625; EP 4 739 531; Envac's
  patents; the voie engins 60 m guidance.
- **Refused or empty:** OECD (403), Bloomberg (403), streetsblog (certificate), barcelona.cat
  infobarcelona (certificate), ajuntament dretssocials (404), stateofgreen.com (403), basel.com (403),
  darksky.org (403), edf.fr (403), ac-paris.fr (403), dezeen (403), justia (403), berlin.de (404),
  Wikipedia "Grassed track", "Superblock (Barcelona)", "Perimeter block", "Urban consolidation center",
  "Lyon Confluence" (en) (404), ISGlobal (PDF, not decoded), UAB repository (bot wall), EUR-Lex 2006/7/EC
  (empty), ecf.com (empty), NPR (timeout), the DeepLayout project page (no output).
- **From Forge, read in the repository:** `docs/research/city-generation.md` (all), `civilisations-styles.md`
  §3, `procedural-content.md` §3.3 and §4, `population-simulation.md` (summary, recommendation, patents),
  `assets-and-licences.md` §5, `rivers.md` and `water.md` (recommendations), `docs/DECISIONS.md` (D-039,
  D-053, D-058's GLO-90 region), `CLAUDE.md`.
- **Numbers to re-check before a spec:** every *(estimate)* (the street profile widths, the waterfront
  dimensions, the event and market areas, the interior sizes not from a source, the tier budgets, the
  per-1 000 design ranges, the room counts and bytes); the tram capacity arithmetic; the bridge spacing
  derived from 37 bridges over 13 km.
