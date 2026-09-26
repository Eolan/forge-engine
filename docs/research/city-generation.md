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

**Contents**

1. [Road networks and layout](#1-road-networks-and-layout)
2. [Blocks, lots and parcels; districts and landmarks](#2-blocks-lots-and-parcels-districts-and-landmarks)
3. [Buildings: grammars, kits, constraint solvers, interiors](#3-buildings-grammars-kits-constraint-solvers-interiors)
4. [Rendering a city of modules through the cluster DAG](#4-rendering-a-city-of-modules-through-the-cluster-dag)
5. [What professional engines do](#5-what-professional-engines-do)
6. [The decision #86 asks for: grammar, kits or hybrid](#6-the-decision-86-asks-for-grammar-kits-or-hybrid)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [What the numbers say](#what-the-numbers-say)
9. [Checked and left out](#checked-and-left-out)
10. [Verification notes](#verification-notes)

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
