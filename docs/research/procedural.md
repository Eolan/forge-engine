> **Provenance.** This is the procedural-generation bibliography written for the `tropical-island` /
> `generative-core` projects (September 2026), kept intact as `docs/research/procedural.md` when
> Forge started. Its D-numbers refer to *those* projects' decisions, not to Forge's `DECISIONS.md`;
> the techniques and verdicts still stand and Forge's procgen phase builds on them.

# Research

An annotated bibliography for the generative core: procedural terrain, rule-based generation, and the
data-oriented architecture underneath. Organised by problem, not by date. Companion to
[DECISIONS.md](DECISIONS.md) — where a decision here has already been taken, the D-number is noted.

**How to read the labels.** Every entry is tagged with what kind of source it is —
**[paper]** peer-reviewed, **[book]**, **[talk]** conference presentation, **[web]** engineering
write-up or reference — and with how it has aged:

- **foundational** — decades old, still correct, still the thing to read.
- **still-current** — the standard reference for its problem today.
- **recent** — 2019 or later, represents where the field is now.

Every citation in this document was checked against a publisher page, author page, or canonical PDF
before being written down. Where something is a preprint rather than a peer-reviewed paper, or a talk
rather than a paper, it says so. A few candidate references turned out not to exist as remembered;
those are listed in [§7](#7-checked-and-left-out) rather than quietly dropped.

**On "recent".** The user asked for recent work, and §§1–4 do deliver it — the terrain literature in
particular is alive, with strong papers every year through 2024. But a fair chunk of the best material
here is thirty or forty years old and has not been improved on: Perlin 1985, Musgrave 1989,
Prusinkiewicz & Lindenmayer 1990, O'Callaghan & Mark 1984. Where that is the case the entry says so
plainly instead of padding the list with newer work that says less.

**Contents**

1. [Terrain that is consistent by construction](#1-terrain-that-is-consistent-by-construction)
2. [Rule-based and constraint-based generation](#2-rule-based-and-constraint-based-generation)
3. [Consistency across seeds and across scale](#3-consistency-across-seeds-and-across-scale)
4. [Ecosystems and settlements as rule systems](#4-ecosystems-and-settlements-as-rule-systems)
5. [Procedural texturing and painting with maths](#5-procedural-texturing-and-painting-with-maths)
6. [Data-oriented design](#6-data-oriented-design)
7. [Checked and left out](#7-checked-and-left-out)
8. [What I would read first](#8-what-i-would-read-first--my-judgement-not-a-consensus)

---

## 1. Terrain that is consistent by construction

The project already learned the central lesson the hard way: rivers are correct when *drainage is
guaranteed*, not when the noise looks right. Fill every depression (priority flood), route flow, then
accumulate — and the network that falls out is consistent because it could not have been otherwise.
This section is the literature that generalises that idea: build terrain from the process, and
consistency is free.

### The two entry points

**Eric Galin, Eric Guérin, Adrien Peytavie, Guillaume Cordonnier, Marie-Paule Cani, Bedrich Benes,
James Gain. "A Review of Digital Terrain Modeling." *Computer Graphics Forum* 38(2), 2019, 553–577.**
[paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.13657> (open copy:
<https://www.cs.purdue.edu/cgvlab/www/publications/Galin19CGF/>)

The state-of-the-art report for terrain, by the group that wrote most of the papers in it. It splits
the field into three families — procedural (noise, faulting, subdivision), simulation (erosion,
tectonics, hydrology) and example-based (from scanned DEMs) — and, more usefully, tabulates them
against control, scale, and computational cost. Read it to find out which paper solves your problem
rather than to learn a technique.
*Bearing:* the fastest way to decide which terrain algorithm a given rule should be expressed in.

**Ruben M. Smelik, Tim Tutenel, Rafael Bidarra, Bedrich Benes. "A Survey on Procedural Modelling for
Virtual Worlds." *Computer Graphics Forum* 33(6), 2014, 31–50.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.12276>

Broader than terrain: covers terrain, vegetation, rivers, roads, buildings and cities in one frame,
and — the reason to read it — explicitly argues for *declarative* and *integrated* world modelling,
where the designer states what the world should be like and the system finds a configuration that
satisfies it. That is the thesis the user is describing.
*Bearing:* the best single statement of "rule-based, data-driven world generation" as a research
programme, with the sub-problems named.

### Hydrology first

**Jean-David Génevaux, Éric Galin, Eric Guérin, Adrien Peytavie, Bedrich Benes. "Terrain Generation
Using Procedural Models Based on Hydrology." *ACM Transactions on Graphics* (SIGGRAPH) 32(4), 2013,
143:1–143:13.** [paper] [foundational for this approach]
<https://dl.acm.org/doi/10.1145/2461912.2461996> (PDF:
<https://www.cs.purdue.edu/cgvlab/www/resources/papers/Genevaux-ACM_Trans_Graph-2013-Terrain_Generation_Using_Procedural_Models_Based_on_Hydrology.pdf>)

Inverts the usual order: generate the river network *first* as a geometric graph grown from the
coastline inwards under a small set of expansion rules (river continues, river forks symmetrically,
river forks asymmetrically), derive watersheds from it, classify each reach by type, and only then
synthesise elevation by blending procedural terrain patches and carving river patches along the graph.
The terrain is stored as a construction tree — an analytic, continuous representation evaluable at any
level of detail — rather than a heightfield.
*Bearing:* the purest example of the project's own drainage lesson taken to its conclusion; the
expansion rules are literally a rule set, and the construction tree is a data-driven representation
you can serialise.

**Adrien Peytavie, Thibault Dupont, Eric Guérin, Yann Cortial, Bedrich Benes, James Gain, Eric Galin.
"Procedural Riverscapes." *Computer Graphics Forum* (Pacific Graphics) 38(7), 2019, 35–46.** [paper]
[recent]
<https://perso.liris.cnrs.fr/eric.galin/Articles/2019-riverscapes.pdf>

Takes a bare heightfield, derives hydrologically-plausible river trajectories over it, carves beds
whose width/depth/shape are *derived from* catchment area and river type, and then emits a "blend-flow
tree" that animates the water surface in real time to match the geometry it just carved. Geometry and
animation come from one description, so they cannot disagree.
*Bearing:* directly applicable — it is the step after flow accumulation, turning accumulation numbers
into bed geometry and visible flow by rule.

**Axel Paris, Eric Guérin, Pauline Collon, Eric Galin. "Authoring and Simulating Meandering Rivers."
*ACM Transactions on Graphics* (SIGGRAPH Asia) 42(6), 2023.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3618350>

Simulates river migration over time — meander growth, cutoff, oxbow lake formation — from a
centreline representation plus a migration model, at interactive rates. Gives you floodplain features
(scroll bars, abandoned channels) that no noise function produces and that read instantly as "a real
river was here".
*Bearing:* if a river is the spine of a region, this is how the region gets its history; run it for N
steps from the seed and the floodplain is consistent with the channel.

### The algorithms underneath (from hydrology, not graphics)

**Richard Barnes, Clarence Lehman, David Mulla. "Priority-flood: An optimal depression-filling and
watershed-labeling algorithm for digital elevation models." *Computers & Geosciences* 62, 2014,
117–127.** [paper] [still-current]
<https://doi.org/10.1016/j.cageo.2013.04.024> (open preprint: <https://arxiv.org/abs/1511.04463>)

The canonical treatment of the algorithm the project already uses: flood the DEM inwards from its
edges with a priority queue so that every cell is guaranteed to drain. The paper's real value is the
comparison of variants — plain flooding, ε-gradient flooding to break flats, the two-pass and
integer-optimised forms — with complexity bounds and measured timings, so you can choose the variant
that suits your grid size rather than rediscovering it.
*Bearing:* the guarantee. This is the paper to cite in DECISIONS for why drainage is correct by
construction rather than by tuning.

**John F. O'Callaghan, David M. Mark. "The extraction of drainage networks from digital elevation
data." *Computer Vision, Graphics, and Image Processing* 28(3), 1984, 323–344.** [paper]
[foundational]
<https://doi.org/10.1016/S0734-189X(84)80011-0>

The origin of D8 flow routing and flow accumulation: each cell drains to its steepest downslope
neighbour, accumulate upslope counts, threshold the accumulation to get a channel network. Forty years
on, this is still the two-paragraph algorithm at the heart of every terrain-with-rivers system,
including this one.
*Bearing:* worth reading once for the assumptions D8 makes (single flow direction, grid bias) —
because those assumptions are exactly where a generated river will look wrong.

### Tectonics, erosion and the long game

**Guillaume Cordonnier, Jean Braun, Marie-Paule Cani, Bedrich Benes, Eric Galin, Adrien Peytavie, Eric
Guérin. "Large Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion." *Computer Graphics
Forum* (Eurographics) 35(2), 2016, 165–175.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.12820> (PDF:
<https://www.cs.purdue.edu/cgvlab/www/resources/papers/Cordonnier-Computer_Graphics_Forum-2016-Large_Scale_Terrain_Generation_from_Tectonic_Uplift_and_Fluvial_.pdf>)

Takes a painted uplift map as *the* input and runs the stream power equation from geomorphology over a
stream graph built on the terrain, alternating uplift and erosion until mountains, valleys and
drainage emerge together. Crucially it is cheap: the stream graph is O(n) per step, so you get
continental-scale terrain in seconds, not hours.
*Bearing:* this is the cleanest "rule + field in, consistent world out" terrain model in the
literature — the rule is one equation, the data is one map, and the drainage network is a by-product
rather than a post-process.

**Guillaume Cordonnier, Eric Galin, James Gain, Bedrich Benes, Eric Guérin, Adrien Peytavie,
Marie-Paule Cani. "Authoring Landscapes by Combining Ecosystem and Terrain Erosion Simulation." *ACM
Transactions on Graphics* (SIGGRAPH) 36(4), 2017, 134:1–134:12.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/3072959.3073667>

Couples the erosion simulation to a vegetation simulation in both directions: vegetation stabilises
slopes and slows erosion; erosion, moisture and slope decide what can grow. Layers of rock, sand,
humus, grass, shrubs and trees are each simulated and each feed back, so the landscape and what covers
it are produced by one process.
*Bearing:* the bridge between §1 and §4 — biome as an *outcome* of terrain rules rather than a
separate noise field, which is the direction D-028 already points.

**Hugo Schott, Axel Paris, Lucie Fournier, Eric Guérin, Eric Galin. "Large-scale terrain authoring
through interactive erosion simulation." *ACM Transactions on Graphics* 42(5), 2023.** [paper]
[recent]
<https://dl.acm.org/doi/10.1145/3592787>

A GPU erosion simulation fast enough to edit against: stream power erosion, thermal erosion,
hillslope diffusion and deposition, running interactively on large domains while the user paints
uplift and material properties. The paper is explicit about the discretisations and the stability
constraints, which is what you need to port it.
*Bearing:* the practical path to "run erosion as part of generation" — the rules stay data (uplift,
hardness, rainfall maps), the simulation turns them into terrain.

**Hugo Schott, Eric Galin, Eric Guérin, Adrien Peytavie, Axel Paris. "Terrain Amplification using
Multi-scale Erosion." *ACM Transactions on Graphics* (SIGGRAPH) 43(4), 2024.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3658200> — code: <https://github.com/H-Schott/MultiScaleErosion>

Amplifies a coarse terrain into a detailed one that is *hydrologically consistent with the coarse
input*: fast approximations of thermal erosion, stream power erosion and deposition are applied at
successive scales so the fine detail obeys the same drainage the coarse terrain established. Source is
released.
*Bearing:* this is the answer to "global structure, cheap local evaluation" for terrain specifically —
generate coarse once, amplify per chunk, and the chunks agree because the process, not a random seed,
decides the detail. Note the year: it is 2024, not 2023.

**Axel Paris, Eric Galin, Adrien Peytavie, Eric Guérin, James Gain. "Terrain Amplification with
Implicit 3D Features." *ACM Transactions on Graphics* 38(5), Article 147, 2019 (presented at SIGGRAPH
Asia 2019).** [paper] [recent]
<https://doi.org/10.1145/3342765> — code: <https://github.com/aparis69/Implicit-Volumetric-Terrains>

Adds features a heightfield cannot express — arches, overhangs, caves, karst — by amplifying the
terrain with implicit (signed-distance) primitives placed where the terrain analysis says they belong,
then meshing the combined field. Bridges §1 and §5: the same SDF vocabulary used for shading is used
for geometry.
*Bearing:* the route to caves and overhangs without abandoning the heightfield the rest of the system
depends on.

**Oscar Argudo, Eric Galin, Adrien Peytavie, Axel Paris, James Gain, Eric Guérin. "Orometry-based
Terrain Analysis and Synthesis." *ACM Transactions on Graphics* (SIGGRAPH Asia) 38(6), 2019.** [paper]
[recent]
<https://doi.org/10.1145/3355089.3356535>

Synthesises mountain ranges by matching *orometric statistics* — peak prominence, isolation, dominance,
and the divide tree connecting them — taken from real ranges. You give it the statistical character of
a place ("like the Dolomites") and a divide tree, and it produces terrain with that character.
*Bearing:* an unusually clean example of "rules as data": the rule set is a handful of distributions,
and the same generator produces a different but equally coherent range per seed.

### The classics, and the practical erosion papers

**F. Kenton Musgrave, Craig E. Kolb, Robert S. Mace. "The synthesis and rendering of eroded fractal
terrains." *Computer Graphics* (SIGGRAPH) 23(3), 1989, 41–50.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/74333.74337>

Introduced multifractal terrain — fractal dimension varying with altitude and slope, so peaks are
rough and valleys smooth — together with the first (crude but fast) hydraulic and thermal erosion
passes over a heightfield. Almost every "ridged multifractal" in every engine descends from this.
*Bearing:* read it to understand what noise-only terrain can and cannot do, which is the argument for
everything else in this section.

**Xing Mei, Philippe Decaudin, Bao-Gang Hu. "Fast Hydraulic Erosion Simulation and Visualization on
GPU." *Pacific Graphics*, 2007, 47–56.** [paper] [foundational, still the usual implementation]
<https://doi.org/10.1109/PG.2007.15> — project page: <http://evasion.imag.fr/Publications/2007/MDH07/>

The shallow-water "virtual pipes" model: four flux values per cell, a velocity field, then sediment
capacity from velocity and slope, erode/deposit, and advect sediment semi-Lagrangianly. It is short,
fully specified, and maps directly onto compute shaders — this is the paper most GPU erosion
implementations actually follow.
*Bearing:* the implementable baseline if you want erosion at chunk granularity in wgpu.

**Ondřej Šťava, Bedřich Beneš, Matthew Brisbin, Jaroslav Křivánek. "Interactive Terrain Modeling Using
Hydraulic Erosion." *ACM SIGGRAPH/Eurographics Symposium on Computer Animation (SCA)*, 2008,
201–210.** [paper] [foundational]
<https://diglib.eg.org/items/60afda0c-a666-4df8-90dd-9b80afc554c2>

Extends the pipe model to *layered* materials and adds force-based erosion, dissolution and bank
slippage, so different rock/soil layers erode at different rates and undercut banks collapse. The
layered representation is the reason the results stop looking like melted wax.
*Bearing:* material layers are data; erosion is the rule that reads them — a good template for
"data-driven simulation" in this project's sense.

---

## 2. Rule-based and constraint-based generation

The question the user is really asking: *how do you express a world as rules and still get variety?*
The literature has three answers that have survived — rewriting (grammars and L-systems), constraint
satisfaction (model synthesis / WaveFunctionCollapse / ASP), and search over a generate-and-test loop.
They compose: grammars propose, constraints dispose.

### Rewriting: grammars and L-systems

**Przemyslaw Prusinkiewicz, Aristid Lindenmayer. *The Algorithmic Beauty of Plants.* Springer, 1990.**
[book] [foundational]
Free PDF: <http://algorithmicbotany.org/papers/#abop>

The book on L-systems, and still the clearest explanation of the idea that matters most here: a tiny
rewriting rule set, applied repeatedly with parameters and context, yields unbounded structured
variety. Beyond branching plants it covers parametric, context-sensitive and stochastic L-systems,
phyllotaxis, and developmental models where the *same* grammar produces a plant at every stage of its
life.
*Bearing:* the canonical answer to "rules plus a seed, consistent variety out"; the parametric and
stochastic extensions in Ch. 1 are the ones to implement first.

**George Stiny, James Gips. "Shape Grammars and the Generative Specification of Painting and
Sculpture." *Information Processing 71* (Proc. IFIP Congress), North-Holland, 1971, 1460–1465.**
[paper] [foundational]
<https://www.semanticscholar.org/paper/c8f7baf704f7d7713eee196de6cb90cbfb7fc4cd>

Where shape grammars began: rules that rewrite *geometry* rather than symbols, matched up to
transformation, with a separate marker/label mechanism controlling where a rule may fire. Short, and
the ideas are the ones every architectural grammar since has re-used.
*Bearing:* read for the concept of rule application under transformation with labels as control —
which is exactly how a building or ruin grammar should be structured in data.

**Peter Wonka, Michael Wimmer, François Sillion, William Ribarsky. "Instant Architecture." *ACM
Transactions on Graphics* (SIGGRAPH) 22(3), 2003, 669–677.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/882262.882324>

Introduces *split grammars* — rules that subdivide a shape into smaller shapes (a façade into floors,
a floor into bays, a bay into window and wall) — plus a separate **control grammar** that distributes
attributes over the building so the right rules fire in the right places, and attribute matching to
choose among applicable rules. The split/control separation is the key structural idea.
*Bearing:* the pattern for buildings, and by extension for any nested decomposition (a village into
plots, a plot into structures); the control grammar is how you keep local rules globally coherent.

**Paul Merrell. "Example-Based Procedural Modeling Using Graph Grammars." *ACM Transactions on
Graphics* (SIGGRAPH) 42(4), 2023.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3592119> — PDF:
<https://paulmerrell.org/wp-content/uploads/2023/08/ProcModelUsingGraphGram.pdf> — code:
<https://github.com/merrell42/Procedural-Modeling-Using-Graph-Grammars>

Derives the grammar *from an example* instead of asking a human to write it: decompose the input model
into primitives, organise all locally-similar graphs into a hierarchy, and extract the rewrite rules
that generate exactly the locally-similar family. Technical companion with the details is on arXiv
(<https://arxiv.org/abs/2309.00275>).
*Bearing:* the credible answer to "writing grammars by hand is too much work" — author one example
structure, get the rule set that varies it.

### Constraint satisfaction: model synthesis and WaveFunctionCollapse

**Paul Merrell. "Example-Based Model Synthesis." *ACM SIGGRAPH Symposium on Interactive 3D Graphics and
Games (I3D)*, 2007, 105–112.** [paper] [foundational — and it predates WFC]
PDF: <https://paulmerrell.org/wp-content/uploads/2022/03/model_synthesis.pdf>

The original algorithm: from one example model, extract the set of locally-legal adjacencies, then
assign labels to a grid so that every local neighbourhood also occurs in the example, propagating
constraints as you go. **The part that matters for this project and that WFC omitted:** it generates
large outputs *in overlapping blocks*, re-solving one block at a time against already-fixed
neighbours, which is what makes arbitrarily large — in principle infinite — output tractable and
locally consistent.

**Paul Merrell, Dinesh Manocha. "Model Synthesis: A General Procedural Modeling Algorithm." *IEEE
Transactions on Visualization and Computer Graphics* 17(6), 2011.** [paper] [still-current]
PDF: <https://paulmerrell.org/wp-content/uploads/2021/06/tvcg.pdf>

The full journal treatment, with the constraint formulation, the block-based ("modify in pieces")
scheme, and extensions to continuous rather than grid-aligned models.
*Bearing:* together with the 2007 paper, this is the formal foundation for chunked, locally-evaluable
rule-based placement — the thing you want for an island that streams.

**Paul Merrell. "Comparing Model Synthesis and Wave Function Collapse." 2021.** [web — author's own
technical note, not peer-reviewed]
<https://paulmerrell.org/wp-content/uploads/2021/07/comparison.pdf>

A short, pointed comparison by the author of model synthesis: WFC re-derived the same constraint
propagation independently and popularised it, but dropped the block-wise modification scheme, which is
why naive WFC fails or stalls on large outputs. Reading it will save you from implementing the
version with the known scaling problem.
*Bearing:* decides for you which variant to implement. Short; read it before writing any WFC code.

**Isaac Karth, Adam M. Smith. "WaveFunctionCollapse is Constraint Solving in the Wild." *Foundations of
Digital Games (FDG)*, 2017, 68:1–68:10.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/3102071.3110566> — open copy:
<https://escholarship.org/uc/item/1f29235t>

Reconstructs Gumin's WFC as a constraint-solving system, separating the three things it conflates:
the *pattern extraction* from an example, the *constraint model*, and the *solver* (observe lowest
entropy, propagate, backtrack or restart). Once separated, you can swap any of the three — e.g. hand
authoring the constraints instead of learning them.
*Bearing:* the paper that lets you treat WFC as a component of a rule system rather than a black box;
essential if you want designer-written adjacency rules rather than example-derived ones.

**Isaac Karth, Adam M. Smith. "WaveFunctionCollapse: Content Generation via Constraint Solving and
Machine Learning." *IEEE Transactions on Games* 14(3), 2022, 364–376.** [paper] [recent]
<https://doi.org/10.1109/TG.2021.3076368>

The extended journal version: a rational reconstruction of the whole WFC family, a survey of the
variants developers have built, and a clear account of which design decisions (pattern size, symmetry,
entropy heuristic, backtracking policy) control which output properties.
*Bearing:* the reference to keep open while tuning a constraint generator — it names the knobs.

**Adam M. Smith, Michael Mateas. "Answer Set Programming for Procedural Content Generation: A Design
Space Approach." *IEEE Transactions on Computational Intelligence and AI in Games* 3(3), 2011,
187–200.** [paper] [still-current]
<https://doi.org/10.1109/TCIAIG.2011.2158545>

Makes the case for declaring content *as a design space*: write the rules and constraints in a
logic-programming language (ASP), and let an off-the-shelf solver enumerate — uniformly at random, if
you like — the worlds that satisfy them. The worked examples cover mazes, levels and item sets.
*Bearing:* the purest form of "rule-based, data-driven": the rules are a text file, variety comes from
sampling solutions, and consistency is a proof rather than a hope. Even if you never ship a solver,
this reframes how to write the rules.

**Seth Cooper. "Sturgeon: Tile-Based Procedural Level Generation via Learned and Designed
Constraints." *AIIDE* 18(1), 2022, 26–36.** [paper] [recent]
<https://doi.org/10.1609/aiide.v18i1.21944>

A practical constraint-based generator that mixes *learned* pattern constraints (extracted from
examples, WFC-style) with *designed* constraints written by hand — including non-local ones like "a
path must exist from entrance to exit". Compiles both down to a SAT/ASP problem.
*Bearing:* this is the missing piece for authored places (D-029): reachability and playability are
constraints, not things you hope the generator got right.

**Yuhe Nie, Shaoming Zheng, Zhan Zhuang, Xuan Song. "Extend Wave Function Collapse to Large-Scale
Content Generation." *IEEE Conference on Games (CoG)*, 2023.** [paper] [recent]
<https://arxiv.org/abs/2308.07307>

Proposes Nested WFC to cut the complexity of large outputs, and — the part worth the read — gives
*tileset preparation strategies* that guarantee a tileset can tile the plane without conflicts, which
is what makes deterministic infinite content possible. Also describes a weight-brush system for
biasing the solver spatially.
*Bearing:* directly relevant to an endless or streamed world: conflict-free tilesets plus nesting is
how constraint solving survives contact with "the player keeps walking".

**"Punch Out Model Synthesis: A Stochastic Algorithm for Constraint Based Tiling Generation."
arXiv:2501.14786, 2025.** [preprint — **not peer-reviewed**; author listed as a pseudonym]
<https://arxiv.org/abs/2501.14786>

Resolves a large grid block by block and, when a block fails to resolve, stochastically "punches out"
(erodes) part of the fixed boundary and retries — which removes the global restart that kills naive
WFC. Introduces *tile correlation length* as the quantity that should determine block size.
*Bearing:* a concrete recipe for large-world constraint solving with bounded failure cost. Flagged
honestly: it is a preprint and should be treated as an idea to test, not an established result.

### The textbook

**Noor Shaker, Julian Togelius, Mark J. Nelson. *Procedural Content Generation in Games.* Springer,
2016.** [book] [still-current]
Free chapter PDFs: <http://pcgbook.com/>

The only real textbook on PCG. Ch. 3 (constructive methods), Ch. 4 (fractals, noise and agents),
Ch. 5 (grammars and L-systems), Ch. 8 (ASP) and Ch. 12 (evaluating generators) are the relevant ones
here. Written to be taught from, so each chapter is short and has the algorithm in it.
*Bearing:* the single best orientation if you want the whole map of rule-based generation before
choosing. Ch. 12, on how to tell whether a generator is any good, is the most under-read chapter and
the most useful one for a project that will run its generator thousands of times.

**Julian Togelius, Georgios N. Yannakakis, Kenneth O. Stanley, Cameron Browne. "Search-Based
Procedural Content Generation: A Taxonomy and Survey." *IEEE Transactions on Computational
Intelligence and AI in Games* 3(3), 2011, 172–186.** [paper] [still-current]
<https://doi.org/10.1109/TCIAIG.2011.2148116>

The taxonomy that gave the field its vocabulary — online/offline, necessary/optional,
random-seeds/parameter-vectors, stochastic/deterministic, constructive/generate-and-test — plus the
generate-and-test framing: generate candidates, score them with a fitness function, keep the good
ones.
*Bearing:* the vocabulary is worth adopting in DECISIONS; and generate-and-test is the cheap way to
add *composition* (D-029's "a ridge that hides a valley") — score candidate placements, keep the best.

### The practitioner benchmark

**Étienne Carrier. "Procedural World Generation of *Far Cry 5*." *GDC 2018*.** [talk] [still-current]
<https://www.gdcvault.com/play/1025557/Procedural-World-Generation-of-Far>
(A related, freely viewable version from Houdini HIVE Utrecht 2018:
<https://www.youtube.com/watch?v=NfizT369g60>)

The most complete public account of a shipped, rule-based world pipeline: Houdini node graphs,
embedded in the game editor via Houdini Engine, generating biomes, terrain texturing, freshwater
networks, cliffs and vegetation from artist-controlled rules, with the artist able to override
anywhere and have the rules respect the override. The "lessons learned" section — about iteration time
and about artists needing to *not* lose their hand-work when rules re-run — is the valuable part.
*Bearing:* the closest thing to a reference implementation of D-029's split between authored places
and generated connective tissue, from a team that shipped it.

---

## 3. Consistency across seeds and across scale

Honest framing first: **this is the area with the least academic literature.** There is no body of work
titled "consistency across seeds". What exists is a set of mechanisms — stateless hashing, locally
evaluable procedural functions, global control fields, and block-wise constraint solving — that
together produce it. The sources below are the good ones for each mechanism, and the section is
deliberately mixed — four papers, two talks, two engineering write-ups — because this is one of the
few places in this document where the shipped games are ahead of the papers.

**Mathieu Gaillard, Bedrich Benes, Eric Guérin, Eric Galin, Damien Rohmer, Marie-Paule Cani. "Dendry:
A Procedural Model for Dendritic Patterns." *ACM SIGGRAPH Symposium on Interactive 3D Graphics and
Games (I3D)*, 2019.** [paper] [recent] — **the key paper for this section**
<https://perso.liris.cnrs.fr/eric.galin/Articles/2019-branching.pdf> — HAL:
<https://hal.science/hal-02150651>

A *locally computable* function that returns the distance to a branching structure — a drainage
network, a crack pattern, a lightning figure — which it constructs implicitly, on the fly, around the
query point. No global data structure, small memory footprint, evaluable in parallel at arbitrary
points and scales. Critically, it is steered by a **global control function** that defines the overall
shape and seeds the local minima, so large-scale structure is authored while evaluation stays local.
*Bearing:* this is the paper that answers the user's hardest requirement directly — global structure,
cheap local evaluation, deterministic from a seed, and it happens to produce exactly the river-like
patterns this world needs.

**Mark Jarzynski, Marc Olano. "Hash Functions for GPU Rendering." *Journal of Computer Graphics
Techniques* 9(3), 2020, 21–38.** [paper] [recent]
<https://jcgt.org/published/0009/03/02/>

Benchmarks the hash functions everyone copies from Shadertoy against the TestU01 statistical suite and
against GPU execution speed, plots the Pareto frontier, and recommends specific hashes per
quality/speed budget — including a new multi-dimensional-input hash (`pcg3d`/`pcg4d`) that is now the
usual choice. Several widely-used one-liners fail badly.
*Bearing:* the foundation of seed-based, locally-evaluable generation: `hash(seed, cell_x, cell_z)`
must be a *good* hash or "consistent across seeds" degrades into visible structure. This tells you
which one to use, with evidence.

**Squirrel Eiserloh. "Math for Game Programmers: Noise-Based RNG." *GDC 2017*.** [talk] [still-current]
<https://www.gdcvault.com/play/1024365/Math-for-Game-Programmers-Noise> — free video:
<https://www.youtube.com/watch?v=LWFzPP8ZbdU>

Argues, convincingly and at implementation level, for replacing stateful RNGs with stateless noise
functions of position and seed. The benefits are exactly the ones this project needs: unordered
access (ask for cell 5,000,000 without generating the first five million), trivial reseeding,
record/replay, network-loss tolerance, and lock-free parallel generation — all while being smaller and
faster than a Mersenne twister.
*Bearing:* the single clearest statement of the discipline that makes a world identical on client and
server without shipping data (D-021). If any part of generation still uses a sequential RNG, this is
the argument for removing it.

**Michael F. Cohen, Jonathan Shade, Stefan Hiller, Oliver Deussen. "Wang Tiles for Image and Texture
Generation." *ACM Transactions on Graphics* (SIGGRAPH) 22(3), 2003, 287–294.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/882262.882265>

A small set of square tiles with coloured edges, plus the rule that adjacent edges must match, tiles
the plane aperiodically — and the tile at any position can be chosen by a hash of the position, so the
tiling is *stochastic, non-repeating, and evaluable at any point without generating its neighbours*.
The paper also shows how to fill tiles with texture, Poisson point sets, or geometry.
*Bearing:* the oldest and simplest answer to "locally evaluable but globally consistent"; still the
right tool when the rule set is small enough to precompute into edge-compatible tiles.

**Innes McKendrick. "Continuous World Generation in *No Man's Sky*." *GDC 2017*.** [talk]
[still-current]
<https://www.gdcvault.com/play/1024265/Continuous-World-Generation-in-No>

The technical architecture of a world that is generated identically on every machine from a seed,
continuously, while the player moves: voxel generation, polygonisation, texturing, and then population
and simulation, all layered so that each stage is a pure function of the stage before it plus the
seed. The talk is candid about where that purity had to be broken and what it cost.
*Bearing:* the closest public precedent for this project's exact constraint — no world data crosses
the network, both sides generate and provably agree.

**Giliam de Carpentier. "Scape: Procedural Extensions." 2012.** [web — engineering write-up]
<https://www.decarpentier.nl/scape-procedural-extensions>

Two noise variants — "Swiss turbulence" and "Jordan turbulence" — that fold erosion-like behaviour
into the fBm summation itself: use the *analytic derivative* of the noise at each octave to warp the
domain along the slope and to damp amplitude where the terrain is already steep or flat. Includes the
derivative-returning Perlin implementation, which is the part most people are missing. This is the
family of noise No Man's Sky used.
*Bearing:* when a full erosion simulation is too expensive for a streamed chunk, this gets
erosion-*looking* terrain from a pure, locally-evaluable function — the pragmatic middle of the
consistency/cost trade.

**Glenn Fiedler. "Floating Point Determinism." *Gaffer On Games*, 2010.** [web — engineering write-up]
[still-current]
<https://gafferongames.com/post/floating_point_determinism/>

Works through when identical floating-point results across machines are achievable and when they are
not: x87 vs SSE, extended precision, compiler fast-math and reassociation, differing transcendental
implementations in libm, and debug vs release. The conclusion is "yes, if" — and the *ifs* are the
useful content.
*Bearing:* load-bearing for D-021. Client and server both generate the island from one seed; every
`sin`, `powf` and `exp` in the generator is a place where they can silently diverge. Read it before
assuming they agree.

**Arnaud Emilien, Ulysse Vimont, Marie-Paule Cani, Pierre Poulin, Bedrich Benes. "WorldBrush:
Interactive Example-Based Synthesis of Procedural Virtual Worlds." *ACM Transactions on Graphics*
(SIGGRAPH) 34(4), 2015.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/2766975> — HAL: <https://inria.hal.science/hal-01147913>

Analyses a selected region of an example world into *statistical distributions* — element densities,
pairwise interaction distributions, relations to terrain features — stores them in a palette like
colours, and lets you paint those distributions elsewhere, with the synthesis matching the statistics
rather than copying the content. Also supports blending two palettes and painting gradients between
them.
*Bearing:* a concrete mechanism for "the same world character under a different seed": the seed
changes the instance, the palette fixes the statistics, and statistics are what a player perceives as
the identity of a place.
---

## 4. Ecosystems and settlements as rule systems

Two sides of the same problem: what covers the ground, and what people built on it. Both are placement
problems, and both go wrong the same way — scatter by density alone and you get wallpaper; place by
rules that read the terrain and each other, and you get somewhere.

### Vegetation and ecosystems

**Oliver Deussen, Patrick Hanrahan, Bernd Lintermann, Radomír Měch, Matt Pharr, Przemyslaw
Prusinkiewicz. "Realistic Modeling and Rendering of Plant Ecosystems." *SIGGRAPH '98*, 1998,
275–286.** [paper] [foundational]
<https://algorithmicbotany.org/papers/ecosys.sig98.pdf>

Establishes the pipeline everything since has followed: terrain → plant *distribution* (painted, or
simulated by individual-based competition over circular ecological neighbourhoods, or both) →
procedural plant models → rendering. The rendering half contributes *approximate instancing*:
cluster similar plants and plant parts, substitute representatives, and billions of primitives fit in
memory.
*Bearing:* the reference architecture for separating "the rules that place plants" from "the rules
that grow one plant" — a separation this project's scatter already half has.

**Brendan Lane, Przemyslaw Prusinkiewicz. "Generating Spatial Distributions for Multilevel Models of
Plant Communities." *Graphics Interface 2002*, 69–80.** [paper] [foundational]
<https://algorithmicbotany.org/papers/eco.gi2002.html>

Formalises both directions in one framework — *local-to-global* (plants compete, die, seed, and a
distribution emerges) and *global-to-local* (a density field is given, synthesise positions that match
it) — using **multiset L-systems**, where the state is an unordered multiset of plant symbols and
productions fire on neighbourhood queries.
*Bearing:* the global-to-local direction is exactly what this project needs: biome weights dictate
density, and the rules must deterministically fill it. The multiset formulation maps cleanly onto an
ECS.

**Wojciech Pałubicki, Kipp Horel, Steven Longay, Adam Runions, Brendan Lane, Radomír Měch, Przemyslaw
Prusinkiewicz. "Self-organizing tree models for image synthesis." *ACM Transactions on Graphics*
(SIGGRAPH) 28(3), 2009.** [paper] [still-current]
<https://algorithmicbotany.org/papers/selforg.sig2009.html>

Trees as self-organisation rather than as a fixed grammar: buds compete for light and space (space
colonisation, or a shadow-propagation light model), and a two-pass signal flow up and down the
skeleton allocates vigour to the buds that won. Gives the per-cycle update rules explicitly, plus
interactive pruning and bending on top of the same simulation.
*Bearing:* one rule set plus one resource field yields endless consistent-but-varied trees from a
seed — the archetype of the thing the user is describing, at the scale of a single object.

**Sören Pirk, Ondřej Šťava, Julian Kratt, Michel Abdul Massih Said, Boris Neubert, Radomír Měch,
Bedřich Beneš, Oliver Deussen. "Plastic trees: interactive self-adapting botanical tree models." *ACM
Transactions on Graphics* (SIGGRAPH) 31(4), 2012.** [paper] [still-current]
<https://doi.org/10.1145/2185520.2185546>

Takes an existing tree skeleton and *deforms* it in response to local light and nearby obstacles,
touching only the affected sub-branches — far cheaper than re-running a growth simulation. A tree
leans away from the cliff it grows beside because the rule read the cliff.
*Bearing:* the affordable version of environmental response: store one skeleton per species, adapt it
per site at load time, and props stop looking stamped.

**Miłosz Makowski, Torsten Hädrich, Jan Scheffczyk, Dominik L. Michels, Sören Pirk, Wojciech
Pałubicki. "Synthetic Silviculture: Multi-scale Modeling of Plant Ecosystems." *ACM Transactions on
Graphics* (SIGGRAPH) 38(4), 2019.** [paper] [still-current]
<https://research.google/pubs/pub49196/>

A genuine per-plant simulation — growth, tropisms, resource competition, seeding, death — that still
scales to hundreds of thousands of plants, by exploiting self-similarity within and between plants to
reuse geometry. Ships parameter sets for nine plant ecologies, which is a ready-made data table.
*Bearing:* shows that "simulate the ecosystem" and "stream a large world" are not mutually exclusive,
and hands you the parameter schema.

**Wojtek Pałubicki, Miłosz Makowski, Weronika Gajda, Torsten Hädrich, Dominik L. Michels, Sören Pirk.
"Ecoclimates: climate-response modeling of vegetation." *ACM Transactions on Graphics* (SIGGRAPH)
41(4), 2022.** [paper] [recent]
<https://doi.org/10.1145/3528223.3530146>

Couples vegetation, soil and atmosphere with the water cycle as the mediating variable, so local
weather emerges *from* the vegetation and terrain rather than being painted on. Reproduces windward/
leeward asymmetry, the Foehn effect, forest-edge effects and banded vegetation patterning.
*Bearing:* the principled way to derive a moisture field from the island's own shape instead of from a
second noise function — which is what would make D-011's biomes feel caused rather than assigned. A
tropical island with a volcano is precisely the case where windward and leeward should differ.

**Konrad Kapp, James Gain, Eric Guérin, Eric Galin, Adrien Peytavie. "Data-driven authoring of
large-scale ecosystems." *ACM Transactions on Graphics* (SIGGRAPH Asia) 39(6), 2020.** [paper]
[recent]
<https://doi.org/10.1145/3414685.3417848>

Predicts a canopy height model for unseen terrain from real LiDAR, then fits individual canopy trees
to that field under a target species distribution, and synthesises the understorey separately from
biome-specific undergrowth distributions.
*Bearing:* even without the learned part, the two ideas worth stealing are the **canopy/understorey
split** and "fit discrete instances to a continuous target field" — a fast, deterministic alternative
to running a full simulation per chunk.

**James Gain, Harry Long, Guillaume Cordonnier, Marie-Paule Cani. "EcoBrush: Interactive Control of
Visually Consistent Large-Scale Ecosystems." *Computer Graphics Forum* (Eurographics) 36(2), 2017,
63–73.** [paper] [still-current]
<https://people.cs.uct.ac.za/~Jgain/index.htm>

Semantic brushes that edit ecosystem *properties* — age, density, variability — locally, while
keeping the result ecologically consistent with the simulation it came from.
*Bearing:* the mechanism for authored overrides on top of generated ground, which D-029 will need the
moment a designer wants a specific grove in a specific place.

### Settlements, roads and buildings

**Yoav I. H. Parish, Pascal Müller. "Procedural Modeling of Cities." *SIGGRAPH '01*, 2001, 301–308.**
[paper] [foundational]
<https://cgl.ethz.ch/Downloads/Publications/Papers/2001/p_Par01.pdf>

The CityEngine origin paper, and still the cleanest statement of the pattern that matters: extend
L-systems with **global goals** (population density, elevation, water, street-pattern templates) and
**local constraints** (each proposed street segment is adjusted, snapped, or rejected against terrain,
water and existing roads). Blocks are then subdivided into lots and extruded.
*Bearing:* "rules propose, terrain disposes" — a propose → validate → commit loop over a graph, which
is directly portable and is the right shape for placing anything that must fit the ground.

**Pascal Müller, Peter Wonka, Simon Haegler, Andreas Ulmer, Luc Van Gool. "Procedural Modeling of
Buildings." *ACM Transactions on Graphics* (SIGGRAPH) 25(3), 2006, 614–623.** [paper] [foundational]
<https://doi.org/10.1145/1141911.1141931>

Defines **CGA shape**: a split/repeat/component-split grammar over a hierarchy of oriented scopes,
with context-sensitive rules for occlusion and snapping so that facades line up and mass models of
arbitrary orientation stay coherent. Demonstrated by rebuilding Pompeii.
*Bearing:* the canonical data-driven building generator — one grammar file per architectural style,
one deterministic derivation per seed. This is the closest published thing to D-027's "one building
system builds everything".

**Arnaud Emilien, Adrien Bernhardt, Adrien Peytavie, Marie-Paule Cani, Eric Galin. "Procedural
Generation of Villages on Arbitrary Terrains." *The Visual Computer* 28(6–8), 2012, 809–818.**
[paper] [foundational for villages — still the best match]
<https://doi.org/10.1007/s00371-012-0699-7> — PDF:
<https://perso.liris.cnrs.fr/eric.galin/Articles/2012-villages.pdf>

Three stages, all of them implementable: (1) grow settlement seeds and connecting roads from
**interest maps** derived from the terrain — slope, sun exposure, water proximity, defensibility,
accessibility from existing roads — so roads attract houses and houses extend roads, co-evolving;
(2) an **anisotropic conquest** region-growing that carves land into parcels around seeds, with the
anisotropy following slope and road direction; (3) an open shape grammar emitting houses adapted to
local slope. Validated against real alpine and fishing villages.
*Bearing:* the single closest paper to what this project wants for settlements: villages placed *from*
terrain and hydrology by weighted rule maps, not by hand.

**Eric Galin, Adrien Peytavie, Eric Guérin, Bedřich Beneš. "Authoring Hierarchical Road Networks."
*Computer Graphics Forum* (Pacific Graphics) 30(7), 2011, 2021–2030.** [paper] [still-current]
<https://doi.org/10.1111/j.1467-8659.2011.02055.x>

Connects settlements with a hierarchy of highways, roads and tracks by shortest path under a
**non-Euclidean cost metric** over the terrain (slope, water crossings, land cover), then merges
nearly-parallel paths into proper junctions and generates the cuttings, embankments and bridges the
route implies.
*Bearing:* gives the actual cost function and merge step for paths that follow terrain — the natural
companion to Emilien's village seeds, and the thing that makes a settlement network read as a system
rather than as dots.

**Christoph Salge, Michael Cerny Green, Rodrigo Canaan, Julian Togelius. "Generative Design in
Minecraft (GDMC): Settlement Generation Competition." *Foundations of Digital Games*, 2018.** [paper]
[still-current]
<https://arxiv.org/abs/1803.09853>

Less an algorithm than a *definition of done*: the competition's rubric scores generated settlements
on adaptivity (does it respond to *this* terrain), functionality, evocative narrative, and aesthetics.
It is the clearest articulation anywhere of what adaptive, holistic generation should mean.
*Bearing:* adopt the rubric as acceptance criteria. "Adaptivity" is precisely the property a
seed-varying world needs and the one that is easiest to fail silently.

**Arthur van der Staaij, Jelmer Prins, Vincent L. Prins, Julian Poelsma, Thera Smit, Matthias
Müller-Brockhausen, Mike Preuss. "Believable Minecraft Settlements by Means of Decentralised Iterative
Planning." *IEEE Conference on Games (CoG)*, 2023.** [paper] [recent]
<https://arxiv.org/abs/2309.10871>

The 2022 GDMC winner written up: instead of a global layout solver, many local agents repeatedly
propose and revise placements against the actual voxel terrain until the settlement converges. The
authors are explicit that the method generalises beyond Minecraft.
*Bearing:* a decentralised incremental planner is far easier to make streaming-friendly and
seed-deterministic than a global optimiser, and it degrades gracefully when the terrain is hostile.

### The one that generates everything

**Alexander Raistrick, Lahav Lipson, Zeyu Ma, Lingjie Mei, Mingzhe Wang, Yiming Zuo, Karhan Kayan,
Hongyu Wen, Beining Han, Yihan Wang, Alejandro Newell, Hei Law, Ankit Goyal, Kaiyu Yang, Jia Deng.
"Infinite Photorealistic Worlds Using Procedural Generation" (Infinigen). *CVPR 2023*, 12630–12641.**
[paper + open source] [recent]
<https://infinigen.org> — code: <https://github.com/princeton-vl/infinigen>

Every asset — terrain, plants, creatures, fire, cloud, rain, snow — is generated from randomised
mathematical rules with **no external asset library at all**, and composed into complete scenes. It
was built to make training data, but the artefact that matters here is the codebase: a large,
readable corpus of real generator rules and parameter distributions for natural-world content.
*Bearing:* the closest existing proof that "everything procedural, nothing hand-authored" works at
scene scale, and the most directly mineable thing on this list — read the rules, port the ones that
apply.

**Jaap van Muijden. "GPU-Based Procedural Placement in *Horizon Zero Dawn*." *GDC 2017*.** [talk]
[still-current]
<https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn>
(video: <https://www.youtube.com/watch?v=ToCozpl1sYY>)

The shipped architecture: artists author placement *rules* in a graph editor; compute shaders evaluate
those rules at run time, in rings around the player, to populate the world with vegetation, sounds,
effects, wildlife and gameplay elements. Nothing is baked. Covers why CPU placement was abandoned, the
artist workflow, and the GPU pipeline.
*Bearing:* the best public reference for making rule-based placement real-time and streaming, which is
where this project's scatter is heading; the rule-graph editor is also a strong model for the
data-driven authoring layer.

---

## 5. Procedural texturing and painting with maths

The user's phrase, "use math for painting", has a precise lineage. It starts in 1985 with the idea
that a texture is a *function of position* rather than an image, and it arrives at signed distance
fields, where the geometry is a function too.

### The classics

**Ken Perlin. "An Image Synthesizer." *Computer Graphics* (SIGGRAPH) 19(3), 1985, 287–296.** [paper]
[foundational]
<https://doi.org/10.1145/325334.325247>

Introduces the Pixel Stream Editor and, with it, gradient noise and solid texturing: a band-limited
pseudo-random function of 3D position from a hashed gradient lattice, composed through turbulence and
fBm sums into marble, wood and flame. The lasting contribution is not the noise function but the
*compositional idiom* — a handful of non-linear primitives layered into arbitrary appearance.
*Bearing:* the origin of texture-as-function-of-position, evaluated on demand at any scale. This is
the document the whole "procedural textures" goal descends from.

**Ken Perlin. "Improving Noise." *ACM Transactions on Graphics* (SIGGRAPH) 21(3), 2002, 681–682.**
[paper] [foundational, and still the practical default]
<https://mrl.cs.nyu.edu/~perlin/paper445.pdf>

Two pages, two fixes: the quintic fade 6t⁵−15t⁴+10t³ in place of the cubic, so second derivatives are
continuous and the grid-axis creasing disappears from normals; and 12 fixed gradients toward cube-edge
midpoints chosen by bit arithmetic instead of a random table. Faster *and* better.
*Bearing:* if you hand-roll noise in Rust or WGSL, implement this version. Derivative continuity stops
mattering the instant you compute normals analytically from the field — which this project will.

**Steven Worley. "A Cellular Texture Basis Function." *SIGGRAPH '96*, 1996, 291–294.** [paper]
[foundational]
<https://doi.org/10.1145/237170.237267>

Defines the F₁, F₂ … basis: scatter feature points by a Poisson process over a virtual grid and return
the distance to the *n*th nearest. F₂−F₁ gives Voronoi edges; changing the metric and the points-per-
cell distribution gives flagstone, scales, crumpled organic and caustic patterns. Includes the
cell-walking evaluation that keeps it O(1).
*Bearing:* the second fundamental basis alongside noise. Everything with discrete cells — rock, bark,
sand grain, path stones, slime membrane — comes from here, tile-free and locally evaluable.

**David S. Ebert, F. Kenton Musgrave, Darwyn Peachey, Ken Perlin, Steven Worley. *Texturing & Modeling:
A Procedural Approach.* 3rd edition, Morgan Kaufmann, 2003.** [book] [foundational]
<https://shop.elsevier.com/books/texturing-and-modeling/ebert/978-1-55860-848-1>

The densest single source for this whole area: antialiasing procedural textures (frequency clamping,
filtered noise), fractal and multifractal terrain (Musgrave's ridged and hybrid multifractals),
volumetric clouds and atmospherics, cellular texturing, hypertextures, plus — new to the 3rd edition —
Bill Mark on real-time procedural shading and John Hart on procedural geometric instancing.
*Bearing:* effectively a specification for "rule-based world material": terrain, cloud and surface
appearance treated as one family of composable functions with explicit control parameters. If one book
had to be on the desk for this project's rendering side, it is this one.

**Robert L. Cook, Tony DeRose. "Wavelet Noise." *ACM Transactions on Graphics* (SIGGRAPH) 24(3), 2005,
803–811.** [paper] [foundational — the aliasing argument is still correct]
<https://graphics.pixar.com/people/derose/publications/WaveletNoise/paper.pdf>

Shows analytically that Perlin noise is neither properly band-limited nor free of detail loss, and
that both get much worse when a 3D noise is sliced by a 2D surface. The fix: build each noise band by
downsample-then-upsample and take the difference, giving clean frequency separation so bands can be
clamped against the pixel footprint.
*Bearing:* the paper that explains how procedural detail *filters* correctly as the camera pulls back
— unavoidable reading if the world's material is generated rather than mip-mapped.

**Ares Lagae, Sylvain Lefebvre, George Drettakis, Philip Dutré. "Procedural Noise using Sparse Gabor
Convolution." *ACM Transactions on Graphics* (SIGGRAPH) 28(3), 2009.** [paper] [still-current]
<https://www-sop.inria.fr/reves/Basilic/2009/LLDD09/LLDD09PNSGC_paper.pdf>

Sparse convolution noise with a Gabor kernel, so the power spectrum is parameterised *directly* by
principal frequency, bandwidth and orientation — you dial the spectrum instead of guessing octave
weights. Being a sparse convolution, it evaluates in object or surface space without texture
coordinates, and the known spectrum gives good anisotropic filtering for free.
*Bearing:* the right tool when a rule says "streaky along the slope direction, at this scale" —
spectral control is what turns an authoring intent into a maths expression.

**Ares Lagae, Sylvain Lefebvre, Rob Cook, Tony DeRose, George Drettakis, David S. Ebert, J. P. Lewis,
Ken Perlin, Matthias Zwicker. "A Survey of Procedural Noise Functions." *Computer Graphics Forum*
29(8), 2010, 2579–2600.** [paper] [still the standard survey]
<https://www.cs.umd.edu/~zwicker/publications/SurveyProceduralNoise-CGF10.pdf>

Formalises procedural noise as a stochastic process, then classifies every major construction —
lattice gradient, value, sparse convolution, spot, wavelet, spectral, explicit — and compares them on
spectral control, anisotropy, filterability, memory and evaluation cost.
*Bearing:* the decision table for "which noise, and what do I give up". Worth pasting the comparison
into a design note before committing to one.

### Infinite non-repeating material

**Eric Heitz, Fabrice Neyret. "High-Performance By-Example Noise using a Histogram-Preserving Blending
Operator." *Proc. ACM Computer Graphics and Interactive Techniques* (HPG 2018) — Best Paper.**
[paper] [still-current]
<https://eheitzresearch.wordpress.com/722-2/>

Takes one small stochastic exemplar, maps its histogram to a Gaussian, tiles the plane with a
randomised triangle grid, blends three samples with *variance-preserving* weights (divide by √Σw²
rather than Σw, which is why contrast survives), then inverts the histogram transform through a LUT.
Infinite non-repeating texture from one small input, three taps per fragment.
*Bearing:* the maths answer to "cover an island in a material with no visible tiling and no texture
budget".

**Thomas Deliot, Eric Heitz. "Procedural Stochastic Textures by Tiling and Blending." In *GPU Zen 2:
Advanced Rendering Techniques*, ed. Wolfgang Engel, 2019.** [book chapter — practitioner]
[still-current]
<https://eheitzresearch.wordpress.com/738-2/>

The engineering follow-up, written from user feedback on the paper above: three 1D histogram
transforms in the eigenspace instead of one full 3D transform (orders of magnitude cheaper), a LUT
prefiltering scheme that fixes the colour drift under mipmapping, and notes on surviving block
compression. Ships runnable OpenGL code.
*Bearing:* this, not the paper, is the version to port to wgpu — the precompute and the shader are
both spelled out.

**Brent Burley. "On Histogram-Preserving Blending for Randomized Texture Tiling." *Journal of Computer
Graphics Techniques* 8(4), 2019, 31–53.** [paper — practitioner journal, peer-reviewed]
[still-current]
<https://www.jcgt.org/published/0008/04/02/paper.pdf>

Analyses what histogram-preserving blending fixes and what it does not: contrast is preserved, but
*ghosting* — two exemplar structures visible at once — remains. Proposes exponentiated blend weights
to sharpen the transition, per channel, without the original's lengthy precomputation.
*Bearing:* the cheap quality knob on top of the two entries above, and it matters as soon as rules
blend several stochastic materials in one shader.

**Morten S. Mikkelsen. "Practical Real-Time Hex-Tiling." *Journal of Computer Graphics Techniques*
11(3), 2022, 77–94.** [paper — practitioner journal, peer-reviewed] [still-current]
<https://jcgt.org/published/0011/03/05/> — demo: <https://github.com/mmikk/hextile-demo>

Adapts the above into something shippable for *regular* materials, not just random-phase ones:
hexagonal tiling with per-tile random rotation and offset, a contrast-preserving blend with a
controllable falloff, and — the practical part — correct handling of normal maps and derivatives so
bump detail does not break at tile seams.
*Bearing:* one rock or sand material, derivative-correct, tiled forever, driven entirely by hashes of
position. This is the technique most engines ended up copying.

**Bartlomiej Wronski. "GPU-Friendly Laplacian Texture Blending." *Journal of Computer Graphics
Techniques* 14(1), 2025, 21–39.** [paper — practitioner journal, peer-reviewed] [recent]
<https://arxiv.org/abs/2502.13945>

Blends textures in a Laplacian-pyramid sense using only the mip chain that already exists: sample a
few extra coarse levels, blend low frequencies smoothly and high frequencies by sharper selection.
Removes both the seams of hard masks and the contrast loss of linear blending, with no precomputation,
no extra memory, no LUTs and no network.
*Bearing:* the lowest-friction option for the sand → rock → grass transitions a biome rule produces,
and it composes with hex-tiling.

### Signed distance fields

**John C. Hart. "Sphere tracing: a geometric method for the antialiased ray tracing of implicit
surfaces." *The Visual Computer* 12(10), 1996, 527–545.** [paper] [foundational]
<https://doi.org/10.1007/s003710050084> — PDF:
<https://graphics.stanford.edu/courses/cs348b-20-spring-content/uploads/hart.pdf>

The algorithm behind every SDF renderer: given a field with a bounded Lipschitz constant, march along
the ray in steps equal to the field value and you can never overshoot the surface. Also derives
distance functions for primitives and CSG operations, and shows how the same bound yields cheap
antialiasing and soft shadows.
*Bearing:* the formal licence for "the world is a maths function" — and the same Lipschitz reasoning
governs whether your composed rules still form a valid distance field.

**Sarah F. Frisken, Ronald N. Perry, Alyn P. Rockwood, Thouis R. Jones. "Adaptively Sampled Distance
Fields: A General Representation of Shape for Computer Graphics." *SIGGRAPH 2000*.** [paper]
[foundational]
<https://www.merl.com/publications/TR2000-15>

Stores a distance field in an octree with detail-directed sampling — dense where the field curves,
sparse where it is near-linear — and shows what that unlocks: rendering, sculpting and carving, level
of detail, offsetting, collision.
*Bearing:* the data structure for a buildable, diggable world (D-027's building system, caves from
§1): rules generate a field, the tree stores only where it matters, edits stay local. Direct ancestor
of modern sparse brick-based SDF engines.

**Chris Green. "Improved Alpha-Tested Magnification for Vector Textures and Special Effects."
*SIGGRAPH 2007 course: Advanced Real-Time Rendering in 3D Graphics and Games*, 9–18.** [talk/course
notes — practitioner] [foundational, still shipped everywhere]
<https://cdn.akamai.steamstatic.com/apps/valve/2007/SIGGRAPH2007_AlphaTestedMagnification.pdf>

Generate a distance field from a high-resolution binary image, store it in one channel of a much
smaller texture, and let bilinear filtering plus alpha test reconstruct crisp edges at huge
magnification. Thresholding the same field at different values gives outlines, glows and drop shadows
for free.
*Bearing:* the cheapest "maths instead of pixels" win available — glyphs, decals, and any hard-edged
marking on the world stay sharp from a tiny source.

**Inigo Quilez — distance functions, smooth minimum, raymarching, fBm, and the *Painting with Math*
series.** [web + video — practitioner, but canonical in this area] [still-current, continuously
updated]
- SDF primitives and operators: <https://iquilezles.org/articles/distfunctions/>
- Smooth minimum: <https://iquilezles.org/articles/smin/>
- Raymarching distance fields: <https://iquilezles.org/articles/raymarchingdf/>
- fBm: <https://iquilezles.org/articles/fbm/>
- *Painting with Math* videos, including the landscape breakdown:
  <https://www.youtube.com/watch?v=BFld4EBO2RE> (index: <https://iquilezles.org/live/>)

Exact distance functions for every primitive you will want, plus union/subtraction/intersection and
their smooth variants, domain repetition (infinite and limited), symmetry, elongation, rounding,
onioning, revolution and extrusion — as GLSL, with explicit notes on which are exact and which are
only bounds. The `smin` article analyses the polynomial, exponential and circular families for
rigidity, locality, whether they remain conservative for sphere tracing, and associativity, and shows
how to carry material IDs through a blend. The fBm article derives the gain G = 2^(−H) and explains
why 0.5 became the default.
*Bearing:* this is the literal source of the user's phrase and the most directly copyable material in
the document. An SDF/noise vocabulary is what a rule-based generator *emits*; this is the vocabulary.

**Alex Evans. "Learning from Failure: a Survey of Promising, Unconventional and Mostly Abandoned
Renderers for *Dreams PS4*, a Geometrically Dense, Painterly UGC Game." *SIGGRAPH 2015 course:
Advances in Real-Time Rendering in Games*.** [talk — practitioner] [still-current for this
architecture]
<https://advances.realtimerendering.com/s2015/AlexEvans_SIGGRAPH-2015-sml.pdf>

A candid tour of roughly eight renderer architectures tried and discarded, landing on this: the scene
*is* an edit list — a CSG tree of soft and hard brush primitives, operationally transformed so several
people can edit it at once — evaluated on GPU compute into a sparse hierarchy of SDF bricks, from
which multi-resolution point clouds are generated and splatted. The failures are documented as
carefully as the success.
*Bearing:* the closest published precedent for a **co-op, user-editable, fully procedural world where
the authoritative data is a small rule/edit list and all geometry is derived** — which is very close
to what this project is trying to be.

**Sebastian Aaltonen. "GPU-Based Clay Simulation and Ray-Tracing Tech in *Claybook*." *GDC 2018*
(Advanced Graphics Techniques Tutorial).** [talk — practitioner] [still-current on the cost model]
<https://media.gdcvault.com/gdc2018/presentations/Aaltonen_Sebastian_GPU_Based_Clay.pdf>

The performance-engineering counterpart to the above: SDF modelling of a fully deformable world,
ray-traced at 60 fps on 2018 console hardware, with the acceleration structure, step heuristics and
cost control spelled out, the simulation entirely in compute with async overlap, and notes on
integrating all of it into an existing engine.
*Bearing:* concrete numbers for whether a generated, deformable, non-mesh world can actually run.

### Fitting procedural material to a reference

**Liang Shi, Beichen Li, Miloš Hašan, Kalyan Sunkavalli, Radomír Měch, Tamy Boubekeur, Wojciech
Matusik. "MATch: Differentiable Material Graphs for Procedural Material Capture." *ACM Transactions on
Graphics* (SIGGRAPH Asia) 39(6), 2020.** [paper] [recent]
<https://perso.telecom-paristech.fr/boubek/papers/MATch/>

Provides DiffMat — differentiable implementations of Substance-style material graph nodes — plus
automatic translation of large real node graphs into differentiable form, so a graph's parameters can
be gradient-descended until its render matches a single photograph. The output stays a
resolution-independent, editable procedural material.
*Bearing:* the bridge from "rules and maths" to "matches this reference", without falling back to a
baked bitmap.

**Yiwei Hu, Paul Guerrero, Miloš Hašan, Holly Rushmeier, Valentin Deschaintre. "Node Graph Optimization
Using Differentiable Proxies." *SIGGRAPH 2022 Conference Proceedings*.** [paper] [recent]
<https://arxiv.org/abs/2207.07684>

Fixes MATch's main limitation — that many useful generator nodes (brick, tile, arc pavement) are
non-differentiable black boxes — by training a small network per node to mimic its parameter→output
mapping, so gradients flow through the whole graph, with a multi-stage optimisation matching structure
first and appearance second.
*Bearing:* shows how to keep discrete, rule-shaped generators (tilings, scattering, bricks) in the
pipeline while still fitting their parameters automatically.
---

## 6. Data-oriented design

**Read this section with its caveat attached.** [PERF.md](PERF.md) measures the simulation tick at
**0.915 ms — 1.8% of the 50 ms tick** at today's 3 202 actors. Nothing in this section will make the
game faster in any way a player notices today.

And the honest version of that caveat has a second half. PERF.md *does* name the simulation as the
constraint at scale — the cost is superlinear (`tick ∝ actors^1.14`), one core reaches a fifth of the
tick at about 26 700 actors, and the fix it identifies is **simulation level of detail**. That is an
algorithmic change, not a memory-layout one. So data-oriented design is not the lever for the problem
this project actually has; a better algorithm is.

The reasons to read this section anyway are different ones, and the good practitioner literature says
them out loud: *modularity, testability, and the ability to change the rules without recompiling your
mental model.* For a generator whose whole premise is "the rules live in data", the payoff is that
rules become data you can query, diff, hot-reload and test — not cycles. Judge every entry below
against that, and be suspicious of any of them that promises speed.

**One more caveat about the sources.** This area is almost entirely **practitioner literature** —
conference talks and engineering blogs, not peer review. That is not a defect (the practitioners are
the people who ship engines), but it is worth knowing: exactly one entry below is an academic paper.

### The canon

**Richard Fabian. *Data-Oriented Design: Software Engineering for Limited Resources and Short
Schedules.* Self-published, 2018. Free online edition.** [book] [still-current]
<https://www.dataorienteddesign.com/dodbook/>

The reference text. The two chapters that matter most here are **Relational Databases** — model game
state as normalised tables — and **Existential Processing** — replace `if (alive)` with membership in
a table, so the branch disappears and the data answers the question. The rest covers component
objects, hierarchical LOD, searching, sorting and compiler-facing optimisation.
*Bearing:* the relational framing is the most transferable idea in the whole section. Generator rules
become **queries and joins over tables** rather than methods on objects, which is very close to what
the user means by "rule based, data driven". The benefit is expressiveness, not cycles.

**Mike Acton. "Data-Oriented Design and C++." *CppCon 2014* keynote.** [talk] [foundational]
<https://isocpp.org/blog/2015/01/cppcon-2014-data-oriented-design-and-c-mike-acton> — video:
<https://www.youtube.com/watch?v=rX0ItVEVjHc>

The canonical polemic: the transformation of data is the only purpose of any program; the three lies
are that software is a platform, that code should be designed around a model of the world, and that
data is not the problem. Concrete cache-line budgeting, and the discipline of asking what the data
actually *is*, how much of it there is, and what happens to it.
*Bearing:* cite it for the **method** (know your data before you design), and be explicit that its
central argument is a hardware-throughput argument, which is not the argument this project needs.

**Noel Llopis. "Data-Oriented Design (Or Why You Might Be Shooting Yourself in the Foot With OOP)."
*Game Developer* magazine, September 2009; reprinted on Games From Within.** [magazine article /
engineering blog] [foundational, still correct]
<https://gamesfromwithin.com/data-oriented-design>

Short and unusually balanced. Lists DOD's benefits as parallelisation, cache efficiency, **modularity
(smaller functions, fewer dependencies) and testability** — and concedes outright that OOP is still
fine for singletons and GUI.
*Bearing:* the best short citation for the honest framing, because Llopis himself ranked modularity
and unit-testability alongside performance in 2009. This is the paragraph to quote in DECISIONS.

**Robert Nystrom. "Data Locality." In *Game Programming Patterns*, Genever Benning, 2014. Free
online.** [book chapter] [still-current]
<https://gameprogrammingpatterns.com/data-locality.html>

Teaches the three layouts you would actually implement — component arrays, packed live-prefix arrays,
and hot/cold splitting — and gates the whole pattern up front: the first guideline for using it is
that you have a performance problem, it is not worth applying to infrequently executed code, and you
will sacrifice abstractions to get it.
*Bearing:* **the counterweight citation.** It is the most quotable reputable source for "we profiled,
the sim is 1.8% of the tick, so we are not doing this for speed" — while still explaining exactly how
to do it when you are.

**Ulrich Drepper. "What Every Programmer Should Know About Memory." Red Hat / LWN.net, 2007.**
[technical report] [foundational; principles hold, numbers are dated]
<https://lwn.net/Articles/250967/> — full PDF: <https://www.akkadia.org/drepper/cpumemory.pdf>

About a hundred pages on cache hierarchies, TLBs, NUMA, prefetching and how to measure any of it, with
the numbers that justify structure-of-arrays layouts.
*Bearing:* reference material for the memory-layout section of a design doc — not a justification for
restructuring anything here. The absolute latencies are nearly twenty years old.

**Scott Meyers. "CPU Caches and Why You Care." *code::dive 2014* (also NDC 2014, ACCU 2011, C++ and
Beyond 2010).** [talk] [foundational]
<https://www.aristeia.com/presentations.html> — slides:
<https://www.aristeia.com/TalkNotes/codedive-CPUCachesHandouts.pdf> — video:
<https://www.youtube.com/watch?v=WDIkqP4JbkE>

A digestible tenth-length substitute for Drepper: data/instruction/TLB caches, cache lines,
speculative prefetching, false sharing, and cache-friendly traversal order.
*Bearing:* the accessible "why layout matters" citation. The false-sharing material becomes relevant
the moment world generation is threaded across cores, which it already is.

**Stoyan Nikolov. "OOP Is Dead, Long Live Data-oriented Design." *CppCon 2018*.** [talk]
[still-current]
<https://isocpp.org/blog/2019/08/cppcon-2018-oop-is-dead-long-live-data-oriented-design-stoyan-nikolov>
— video: <https://www.youtube.com/watch?v=yy8jQgmhbAU>

A real, measured migration rather than a manifesto: Coherent Labs' Hummingbird HTML renderer
re-architected data-oriented, compared against Chrome's OOP design, with before/after production code.
Claims gains in performance, scalability, **maintainability and testability**.
*Bearing:* the "we did this on a shipping product and here is what changed" citation; the
maintainability and testability half is the half that applies here.

**Aras Pranckevičius. "Entity Component Systems and Data Oriented Design." Unity internal training
academy, September 2018.** [talk slides + sample code] [still-current]
<https://aras-p.info/texts/files/2018Academy%20-%20ECS-DoD.pdf> — code:
<https://github.com/aras-p/dod-playground>

An explicitly non-Unity-specific teaching deck paired with a small C++ project refactored step by step
from OOP to DOD.
*Bearing:* the best pedagogical entry point — the one link to hand a new contributor.

### ECS, and why it is everywhere in Rust

**Catherine West (kyren). "Using Rust For Game Development." *RustConf 2018* closing keynote.**
[talk + edited transcript] [still-current — the best Rust entry here]
<https://kyren.github.io/2018/09/14/rustconf-talk.html> — video:
<https://www.youtube.com/watch?v=aKLntZcp27M>

Derives ECS from first principles by refactoring a Starbound-like OOP `Player` class with far more
methods than fields, showing precisely where Rust's ownership rules break object-oriented game code,
and introduces **generational-index arenas** as the safe entity handle. It then declines to prescribe
ECS: the advice is to start from a large, procedural, single-purpose engine written in "a much much
nicer C".
*Bearing:* **this is the answer to "why is ECS so common in Rust" — ownership pressure, not
performance.** It is simultaneously the best argument for the data-structure discipline and against
adopting a framework before you need one. Given D-012 already says storage moves to an ECS only when
component variety demands it, this talk is the one that agrees with you.

**Sander Mertens. "Entity Component System FAQ." flecs.dev.** [web — maintained community FAQ]
[still-current]
<https://www.flecs.dev/ecs-faq/>

Written by the author of Flecs, actively maintained. Sections on general questions, how-to,
data-oriented design questions, a glossary, and a survey of frameworks.
*Bearing:* use it to fix vocabulary — archetype, table, sparse set, relationship, tag — so that
"component" and "system" mean one thing across DECISIONS and ARCHITECTURE.

**Carter Anderson et al. "Bevy 0.5 release notes, §Bevy ECS V2." bevy.org, 6 April 2021.**
[engineering blog / release notes] [still-current mechanism; 2021 numbers]
<https://bevy.org/news/bevy-0-5/> — implementation: <https://github.com/bevyengine/bevy/pull/1525>

The clearest free write-up of the archetype-versus-sparse-set trade-off *and* a way out of it:
archetypal storage iterates fast because the layout is cache-friendly, but adding or removing a
component moves the entity; sparse sets add and remove in constant time but iterate worse. Bevy ships
both with per-component opt-in, plus an archetype graph and cached queries.
*Bearing:* directly relevant — a generator that adds and removes components while building entities
pays exactly the archetype-move cost described here, and per-component storage choice is a real lever.

**Louis Cox, Benjamin Williams, James Vickers, Davin Ward, Christopher Headleand. "Run-time
Performance Comparison of Sparse-set and Archetype Entity-Component Systems." *CGVC 2025: Computer
Graphics & Visual Computing* (Eurographics UK Chapter).** [paper — **the only peer-reviewed entry in
this section**] [recent]
<https://eprints.staffs.ac.uk/9315/>

Purpose-built C++20 implementations of both architectures, measured under controlled conditions, with
recommendations by workload: sparse sets make entity modification cheap but scale poorly during
iteration; archetypes excel at large-scale iteration but charge more for composition changes.
*Bearing:* worth including precisely because it confirms the practitioner folklore above under
controlled conditions — and because its existence lets this bibliography say honestly that everything
else in the section is practitioner material.

**Michele Caini (skypjack). "ECS back and forth" (13+ parts). skypjack on software, from February
2019.** [engineering blog series] [still-current]
<https://skypjack.github.io/2019-02-14-ecs-baf-part-1/> — consolidated PDF:
<https://skypjack.github.io/pdf/ecs_back_and_forth.pdf>

The definitive free deep-dive, by the author of EnTT. Opens by stating that ECS's primary benefit is
code organisation and *not only* performance, then builds up implementations — maps of containers,
entity-as-index packed arrays, sparse sets, grouping — with the trade-offs of each, concluding that
archetypes suit low-level systems such as rendering while sparse sets suit high-level gameplay.
*Bearing:* both the storage reference and an explicit endorsement, from an ECS library author, of
exactly the framing this project needs.

**Benjamin Saunders (Ralith). `hecs` — README and crate documentation. MIT/Apache-2.0.** [library
design notes] [recent, actively maintained]
<https://github.com/Ralith/hecs> — <https://docs.rs/hecs/>

Documents a deliberately minimal archetype ECS: columnar dense per-archetype component arrays, **no
System abstraction and no scheduler** — a library, not a framework. Honest about its own scope: ECS
may be overkill for games that do not call for batch processing of entities, and most games keep
significant state outside the world.
*Bearing:* the strongest demonstration that you can take ECS *storage* without ECS *ceremony*, which
matters if the rule engine wants to own its own scheduling.

**Niklas Frykholm. "Building a Data-Oriented Entity System", Parts 1–4. Bitsquid development blog,
August–October 2014.** [engineering blog series] [foundational]
<https://bitsquid.blogspot.com/2014/08/building-data-oriented-entity-system.html>
(Part 2: <https://bitsquid.blogspot.com/2014/09/building-data-oriented-entity-system.html> ·
Part 3: <https://bitsquid.blogspot.com/2014/10/building-data-oriented-entity-system.html> ·
Part 4: <https://bitsquid.blogspot.com/2014/10/building-data-oriented-entity-system_10.html>)

A shipping engine's actual design, written up honestly. Entities are 30-bit IDs split into 22 index
bits plus 8 generation bits — the generational handle, four years before kyren's talk — and there is
deliberately **no central list of an entity's components**: each component type has its own manager,
free to lay its data out however suits it. Part 4 covers compiling entity definitions authored *as
data* into runtime resources.
*Bearing:* Part 4 is the data-driven authoring piece. The decoupled-managers design is also the
argument that you can be data-oriented without a monolithic ECS framework — which is where `ti-sim`
already is.

**Adam Martin. "Entity Systems are the future of MMOG development", Part 1. T-machine.org, 3 September
2007.** [engineering blog series] [foundational — historical citation only]
Original host **unreachable**; archived:
<https://web.archive.org/web/20240109024510/https://t-machine.org/index.php/2007/09/03/entity-systems-are-the-future-of-mmog-development-part-1/>

The series that named the pattern and fixed its vocabulary: entities as bare IDs, components as pure
data, systems as the only code.
*Bearing:* cite for provenance, link the archive, and expect nothing from it that later sources do not
say better.

### Data-driven engine architecture — the most relevant part

**Niklas Gray. "The Story behind The Truth: Designing a Data Model." Our Machinery blog, 7 November
2018.** [engineering blog] [still-current as design; **site dead, cite the archive**]
<https://ruby0x1.github.io/machinery_blog_archive/post/the-story-behind-the-truth-designing-a-data-model/index.html>

The best single design document for a data-driven engine's core. "The Truth" is an in-memory
authoritative data model of typed objects with properties (bools, ints, floats, strings, buffers,
references, sub-objects, sets), reflective enough to clone and serialise generically. Every mutation is
a uniform `(object, property, old value, new value)` tuple — which yields undo/redo, change
notification and multi-user collaboration *for free, across every tool*. References are stable UUIDs,
never paths.
*Bearing:* **the highest-value entry in this section for this project.** If world-generation rules are
going to live in data, this is the shape that data model should have — and note that the payoff Gray
argues for is entirely iteration speed and tooling uniformity, with no performance claim at all.

**Tobias Persson. "Creation Graphs." Our Machinery blog, 4 April 2019.** [engineering blog]
[still-current as design; **site dead, cite the archive**]
<https://ruby0x1.github.io/machinery_blog_archive/post/creation-graphs/index.html>

A node-graph system where reusable engine mechanisms are exposed as nodes and assembled *in data*
within the editor rather than in code, with validity-hash caching of CPU and GPU results, and the same
graphs usable at runtime for user-generated content.
*Bearing:* the closest verified match to "a data-driven rule system, defined in data, reloadable". A
world-generation rule graph is the same object — authored as data, evaluated by fixed engine
mechanisms, cached by hash. Compare Far Cry 5 (§2) and Horizon Zero Dawn (§4), which are the same idea
shipped.

**Niklas Gray. "DLL Hot Reloading in Theory and Practice." Our Machinery blog, 14 August 2017.**
[engineering blog] [principle current, mechanism dated for Rust; **site dead, cite the archive**]
<https://ruby0x1.github.io/machinery_blog_archive/post/dll-hot-reloading-in-theory-and-practice/index.html>

Practical hot reload: C interfaces as structs of function pointers so reloading is pointer
replacement; state carried across by `memcpy` rather than serialisation; header discipline to keep
builds fast; and honesty about what stays broken (debugger file locks) and about the social
requirement that the whole team use it or reload bugs accumulate.
*Bearing:* the concrete mechanism behind the iteration-speed thesis. The DLL specifics do not port
cleanly to Rust, but the architectural precondition — narrow data interfaces and relocatable state —
is exactly what a data-oriented rewrite buys.

### The honest counterweight

**Bobby Anguelov. "Game Engine Entity/Object Models." Self-published recorded lecture, 28 December
2020 (~2h34m).** [talk — long-form video, no slides or paper] [still-current]
<https://www.youtube.com/watch?v=jjEsB611kxs>

Walks OOP → object-component → ECS, credits what ECS genuinely fixes, then argues it is oversold for
gameplay: real games are not "update position by velocity"; special-casing entities that share
components pushes you into tag-driven branching *inside* systems, which erodes the cache argument that
justified ECS in the first place; and one-to-many relationships and hierarchies fit the
one-component-per-type-per-entity model badly.
*Bearing:* the strongest "ECS is not a silver bullet, from someone who ships engines" source. It bites
hardest exactly where this project lives — world-generation rules are full of one-to-many
relationships and special cases. Cite it as a talk, not as evidence; it is unstructured and
unsourced.

**If only three things are quoted in the "why DOD when the sim is 1.8%" argument**, use Nystrom's
"when you have a performance problem" gate, Caini's "organisation and not (only) the performance", and
the `hecs` README's own "may be overkill" — with Llopis as the 2009 precedent for ranking modularity
and testability as first-class DOD benefits.

### Two dead hosts, noted

`ourmachinery.com` and `t-machine.org` were both unreachable when this document was written. The
archive links above work today; the machinery archive is a third-party public-service mirror and could
itself disappear. If the Truth and Creation Graphs posts end up load-bearing for the architecture,
vendor a copy into `docs/` rather than relying on the mirror.
---

## 7. Checked and left out

Kept deliberately, because a bibliography that only lists what it found is not auditable. These are
things that were looked for and are *not* above, with the reason.

- **"Efficient Interactive Ecosystem Simulation"** — no paper with this title was found. The nearest
  real work appears to be **Beneš et al., "Interactive Modeling of Virtual Ecosystems", Eurographics
  Workshop on Natural Phenomena, 2009**, which was located but whose author list could not be
  confirmed. Left out rather than cited half-verified; verify the authors before using it.
- **A "Burley hex-tiling" paper** — does not exist. Two separate works were being conflated: Burley's
  contribution is histogram-preserving blending (JCGT 2019, §5); hex-tiling is Mikkelsen's (JCGT 2022,
  §5). Both are above under their correct authors.
- **Sander Mertens' "Building an ECS" Medium series** — almost certainly genuine and widely linked,
  but Medium refused every automated fetch, so titles, dates and contents are unverified. His Flecs
  ECS FAQ (§6) is verified and covers similar ground.
- **A 2020s paper on settlement placement driven specifically by hydrology features** (river
  confluences, fertile land) — none found. Emilien et al. 2012 and Galin et al. 2011 (§4) remain the
  state of the art for this exact combination as far as could be established. If such a paper exists
  it is likely in a Visual Computer or CGF issue that did not surface.
- **Texture bombing and triplanar mapping** — no primary source of the same standing as the §5
  entries. The canonical texture-bombing reference is a GPU Gems chapter and triplanar mapping has no
  authoritative paper at all. The nearest rigorous treatment is **Mikkelsen, "Surface Gradient Based
  Bump Mapping Framework", JCGT 9(3), 2020** (<http://jcgt.org/published/0009/03/04/>) — the correct
  way to combine procedural, triplanar and normal-map detail from several UV spaces on one surface —
  but it could only be verified from the author's own index page, so it is recorded here as a lead
  rather than as a §5 entry.
- **Jonathan Blow / Jai material on iteration speed** — nothing found that is specifically about
  *data-driven* iteration rather than language design. Gray's hot-reload post (§6) makes the argument
  better and is verifiable.
- **A separate Cordonnier paper on interactive ecosystem authoring** distinct from the 2017
  erosion-plus-ecosystem paper — not found. That 2017 paper is in §1, and it is the one that covers
  the ground; Gain et al.'s EcoBrush (§4) is the nearest thing to a dedicated authoring paper.

**One date correction worth carrying.** Schott et al., "Terrain Amplification using Multi-scale
Erosion", is **SIGGRAPH 2024**, not 2023. The 2023 Schott paper is a different one — "Large-scale
terrain authoring through interactive erosion simulation". Both are in §1, under their real years.

**Fetching caveat, for the record.** ACM DL, Wiley, dblp and jcgt.org all reject automated fetching.
Entries published there were confirmed through at least one independently reachable page each — the
ACM SIGGRAPH History Archives, a publisher shop page, an institutional repository (INRIA, MERL,
Purdue CGVLab, UCT, Illinois, Staffordshire), the author's own site, HAL, or arXiv — with DOIs
recorded from those pages. If a link here rots, the DOI is the durable handle.

---

## 8. What I would read first — my judgement, not a consensus

*This section is opinion. Everything above is sourced; this is a recommendation, and it is shaped by
what this project has already built and decided rather than by what is most cited.*

**1. Gaillard et al., "Dendry" (I3D 2019) — §3.**
Because it is the only paper in this bibliography that answers the user's hardest requirement head-on.
A *locally computable* function, deterministic from a seed, that builds a branching structure
implicitly around the query point, steered by a **global control function** that fixes the large-scale
shape. Global authorship, local evaluation, no global data structure, parallel, small memory — and it
happens to produce drainage-like patterns. If one idea from this document changes the architecture,
it is this one: it shows that "consistent by construction" and "cheap to evaluate anywhere" are not in
tension, provided the rule is expressed as a function of position rather than as a simulation over a
grid.

**2. Cordonnier et al., "Large Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion"
(CGF 2016) — §1.**
Because it is the smallest rule set in the literature that produces a whole coherent world: one
painted uplift map, one stream power equation, and mountains, valleys and a correct drainage network
emerge *together* at continental scale in seconds. It is the general form of the lesson this project
already learned about priority flood and flow accumulation — and it makes "add rivers" and "make the
island bigger" parameter changes rather than rework, which D-029 explicitly wants to preserve. Read it
with Schott et al. 2024 (multi-scale amplification) as the follow-up for how to get chunk-level detail
that stays consistent with the coarse result.

**3. Merrell, "Example-Based Model Synthesis" (I3D 2007) together with his 2021 note "Comparing Model
Synthesis and Wave Function Collapse" — §2.**
Because this is the rule engine, and because reading them in this order will save real time. WFC is
what everyone reaches for; model synthesis is the same constraint propagation *plus* the block-wise
modification scheme that WFC dropped, which is precisely what makes large and streamed output
tractable. The comparison note is three pages and tells you which variant to build. Follow with Karth
& Smith (FDG 2017) if you want designer-authored adjacency rules rather than example-derived ones —
which, given the stated goal of expressing the world as rules, you probably do.

**4. Emilien et al., "Procedural Generation of Villages on Arbitrary Terrains" (The Visual Computer,
2012) — §4.**
Because it is the closest published match to where D-027 and D-029 are already heading, and because it
is unusually implementable: interest maps derived from slope, water proximity, sun and accessibility;
seeds and roads that co-evolve; anisotropic region growing into parcels; a shape grammar that adapts
each building to its slope. It is the worked example of "authored places, procedural placement", and
it is the paper that turns a village from a scatter of props into a settlement that looks like someone
chose that spot.

**And the half-hour that pays for itself immediately:** Squirrel Eiserloh's GDC 2017 talk
**"Noise-Based RNG"** (§3). Stateless hash-of-position instead of stateful RNGs is the discipline that
makes a world identical on client and server, generable out of order, parallel without locks, and
reproducible from a seed alone. It is a precondition for almost everything else in this document, it
is thirty minutes, and it is free.

**Deliberately not in this list, and why.** The surveys (Galin 2019, Smelik 2014, Lagae 2010) are
excellent and should be on the shelf, but they are reference works — read them when choosing, not to
get started. The data-oriented design material in §6 is worth reading *after* there is a rule system
to organise, not before: the simulation measures at 1.8% of the tick today, and the constraint PERF.md
actually identifies — superlinear cost at scale — is answered by simulation level of detail, not by
memory layout. DOD here buys iteration speed and clarity, and those are only worth spending on once
there is something to iterate on.
