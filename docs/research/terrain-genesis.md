# Research — Terrain genesis: synthesis, hydrology, erosion, rendering, texturing, determinism

> Companion to `large-worlds.md` §8 (the far-field representation, D-014) and to the terrain half of
> `procedural.md` §1, the bibliography carried over from the previous projects. Written 2026-09-25
> for Phase 2's `island` demo: a 16 km island generated from a seed, later a cube-sphere planet.
> Entries that `procedural.md` already carries are re-verified here rather than repeated at length,
> and placed where the island pipeline needs them.

The question is what the strongest published work says about generating a large terrain from a
seed and drawing it, so that Forge's pipeline can be fixed in order before any code: how the large
shape is made (tectonic uplift against stream-power erosion), how water is routed over it (priority
flood, drainage, lakes, coasts), where the small-scale erosion belongs (after the large pass, on
tiles), what shipped games generate offline and what at run time, how a heightfield of this size
reaches the screen (clipmaps, CDLOD, concurrent binary trees, or the cluster DAG Forge already
has), how it is textured without the repeats the owner sees at once, and how the same field comes
out identical on a client and a server. The short answer: the geomorphology of the last fifteen
years — Braun & Willett's implicit O(n) solver for the stream-power law, Barnes's priority flood,
Cordonnier's stream graph and basin graph — is the generation pipeline, it runs on the CPU in
about a minute or two at the island's resolution if the solver is parallel across basins (the
published serial code would take some twenty minutes), it is deterministic if the receivers, the
queue order and the transcendental functions are pinned, and its output is a heightfield the
existing cluster-DAG cook already turns into 8 M-triangle ground. What the games add is discipline about
what is baked (everything geological) and what is evaluated at run time (placement and materials
from fields and rules), and what the rendering literature adds since Forge's last note is that the
DAG the city uses is a legitimate terrain LOD, with CDLOD or a concurrent binary tree as the
planet-scale far field when one mesh stops fitting.

> **State of the art in five sentences.** Large terrain is generated as a process, not drawn as
> noise: an uplift field competes with fluvial erosion under the stream-power law, solved
> implicitly and unconditionally stably in O(n) per step by ordering cells from the outlets upstream
> (Braun & Willett 2013), which Cordonnier et al. brought to graphics in 2016 and whose successors
> now run interactively on the GPU (Schott 2023), analytically with time as a parameter rather than
> a step count (Tzathas 2024), or with flow routing in O(log n) parallel iterations (FastFlow
> 2024). Drainage is made correct by construction with priority flood (Barnes 2014, O(n) on
> integer heights) or a linear-time basin graph (Cordonnier, Bovy & Braun 2019), lakes come from
> the depression hierarchy (Fill–Spill–Merge 2021), and rivers are the cells above a catchment
> threshold, widened by rule. Fine detail is a second, local pass — thermal and shallow-water hydraulic erosion in the cellular form of Mei
> 2007 and Jákó & Tóth 2011, or multi-scale amplification (Schott 2024) — which tiles with a halo,
> whereas the large pass needs the whole map. Shipped worlds bake geology offline (Far Cry 5's
> nightly Houdini regeneration, Tsushima's and Horizon's GPU rule languages for placement, Flight
> Simulator's petabytes of photogrammetry) and evaluate only placement and materials at run time,
> the exception being the planet games (Elite, No Man's Sky, Star Citizen) that generate on the GPU
> from noise and pay for it in geological plausibility. For rendering, clipmaps, CDLOD and
> concurrent binary trees are the heightfield-specific answers and a cluster DAG is the general one;
> for texturing, materials from slope, altitude and flow through a runtime virtual texture with
> histogram-preserving stochastic tiling; for determinism, integer-hashed noise, integer drainage
> areas, pinned tie-breaks and a portable `libm`.

**Contents**

1. [Large-scale synthesis: uplift against the stream-power law](#1-large-scale-synthesis-uplift-against-the-stream-power-law)
2. [Hydrology: drainage, depressions, lakes, coastlines](#2-hydrology-drainage-depressions-lakes-coastlines)
3. [Detail erosion, and where it fits](#3-detail-erosion-and-where-it-fits)
4. [Terrain in shipped games and their tooling](#4-terrain-in-shipped-games-and-their-tooling)
5. [Rendering LOD for heightfields](#5-rendering-lod-for-heightfields)
6. [Texturing and materials at terrain scale](#6-texturing-and-materials-at-terrain-scale)
7. [Determinism, seeds and tiling](#7-determinism-seeds-and-tiling)
8. [Recommendation for Forge](#recommendation-for-forge)
9. [What the numbers say](#what-the-numbers-say)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. Large-scale synthesis: uplift against the stream-power law

One equation underlies this section. The stream-power law says a river bed lowers at a rate
proportional to a power of its drainage area and a power of its local slope, `∂h/∂t = U − K·A^m·S^n`
with uplift `U`; rivers cut where much water meets a steep bed, ridges stay where little water
gathers, and the balance between `U` and `K` fixes the relief. Everything below is a way of solving
it fast, controlling it, or adding what it leaves out (hillslopes, glaciers, debris flows).

**Kelin X. Whipple, Gregory E. Tucker. "Dynamics of the stream-power river incision model:
Implications for height limits of mountain ranges, landscape response timescales, and research
needs." *Journal of Geophysical Research: Solid Earth* 104(B8), 1999, 17661–17674.** [paper]
[foundational]
<https://agupubs.onlinelibrary.wiley.com/doi/10.1029/1999JB900120> (DOI 10.1029/1999JB900120)

The geomorphology paper every graphics erosion paper cites for the law itself: bedrock channel
profiles "limit the elevation of peaks and ridges" and "communicate tectonic and climatic signals
across the landscape", and the exponents `m`, `n` and the erodibility `K` set how a range responds
to uplift and how tall it can get.
*Bearing:* the parameters Forge exposes to a designer are these three and the uplift field; the
paper is the reference for what plausible ranges of `m/n` (about 0.5) and `K` look like, so the
island's relief comes from geology rather than from a height multiplier.

**Jean Braun, Sean D. Willett. "A very efficient O(n), implicit and parallel method to solve the
stream power equation governing fluvial incision and landscape evolution." *Geomorphology*
180–181, 2013, 170–179.** [paper] [foundational] [still-current]
<https://www.sciencedirect.com/science/article/abs/pii/S0169555X12004618> (DOI
10.1016/j.geomorph.2012.10.008)

The solver. Cells are ordered from the outlets upstream along their single-flow-direction
receivers (the "stack"), so the implicit update of each cell needs only its already-updated
receiver, and one pass over the stack solves the whole time step: "computation time increases
linearly with the number of points used to discretize the landscape and is ideally suited to
parallelization", and it is "unconditionally stable because it uses an implicit scheme for the
time integration of the landscape evolution equation, which means that large time steps can be
used without sacrificing accuracy". It is the method inside FastScape, an "open-source code
available in multiple languages (Fortran, C++, and Python)", and other landscape-evolution models
(Badlands) adopt its ordering.
*Bearing:* the core loop of Forge's genesis: receivers → stack → drainage area → one ordered pass
per step. For `n = 1` the per-cell update is closed-form; independent drainage basins are
independent stacks, which is where the job system's parallelism goes (results merged by basin,
then by index, D-016).

**Guillaume Cordonnier, Jean Braun, Marie-Paule Cani, Bedrich Benes, Éric Galin, Adrien Peytavie,
Éric Guérin. "Large Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion." *Computer
Graphics Forum* 35(2) (Eurographics 2016), 165–175.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.12820> (DOI 10.1111/cgf.12820; open copy
<https://inria.hal.science/hal-01262376>)

"The first method in computer graphics that combines uplift and hydraulic erosion to generate
visually plausible terrains": "given a user-painted uplift map, we generate a stream graph over the
entire domain embedding elevation information and stream flow", which with the stream-power
equation gives "large realistic terrains at a low computational cost", with "high-level control
over the large scale dendritic structures of the resulting river networks, watersheds, and
mountains ridges". The stream
graph is Braun & Willett's structure on an irregular point set with the lake-overflow handling the
grid version lacks.
*Bearing:* the design of Forge's large-scale pass is this paper on a regular grid: the uplift map
is the one authored (or seeded) input, and the river network is a by-product of the solve, never a
post-process. Its timings were not re-read (HAL's PDF sits behind a bot check that refuses
automated fetches); `procedural.md` recorded them as seconds for continental domains.

**Guillaume Cordonnier, Marie-Paule Cani, Bedrich Benes, Jean Braun, Éric Galin. "Sculpting
Mountains: Interactive Terrain Modeling Based on Subsurface Geology." *IEEE Transactions on
Visualization and Computer Graphics* 24(5), 2018, 1756–1769.** [paper] [still-current]
<https://hal.science/hal-01517343> (DOI 10.1109/TVCG.2017.2689022)

Uplift from plates instead of a painted map: the user has "hands-on control on the shape and
motion of tectonic plates, represented using a new geologically-inspired model for the Earth
crust"; the model "generates a volumetric uplift map representing the growth rate of subsurface
layers", "erosion and uplift movement are jointly simulated to generate the terrain", and "the
stratigraphy allows us to render folded strata on eroded cliffs".
*Bearing:* the uplift generator for the planet variant (plates on a sphere) and the source of a
*hardness* field per layer, which the erosion pass should read so that cliffs and soft valleys
differ; for the island a seeded ridge field is enough.

**Richard Barnes. "Accelerating a fluvial incision and landscape evolution model with
parallelism." *Geomorphology* 330, 2019, 28–39.** [paper] [code] [recent]
<https://arxiv.org/abs/1803.02977> (DOI 10.1016/j.geomorph.2019.01.002; code
<https://github.com/r-barnes/Barnes2019-Landscape>)

Braun & Willett's model reworked for "GPUs, many-core processors, and SIMD instructions": the new
algorithm "runs 43x faster (70s vs. 3,000s on a 10,000x10,000 input) than the previous state of
the art and exhibits sublinear scaling with input size". The body pins the figure down: "All tests
were run for 120 timesteps", on a node with two 10-core POWER8 CPUs (160 hardware threads) and
"4 NVIDIA Tesla P100 GPUs"; "On the GPU, the algorithm runs 43 x faster than the serial version of
the B&W algorithm, 9 x faster the best B&W parallel implementation", and on the larger input the
GPU is 3× faster than the paper's own parallel CPU version. The repository holds the variants
(basic, with routing, GPU) in C++, Fortran and CUDA with a correctness comparison script.
*Bearing:* the cost reference for the whole-map pass: 120 steps on 10⁸ cells take 70 s on the
P100 node, about 210 s for the parallel CPU version and 3,000 s serially (25 s a step, about
250 ns per cell per step). Scaled linearly to the island's 1.7 × 10⁷ cells and 300 steps, that is
about 30 s on the GPU, 1.5 min for the parallel CPU form and 20 min serial on that 2016-era
hardware, so the basin-parallel version is not optional. The paper's device-parallel stack
construction is the recipe if genesis ever moves to compute.

**Guillaume Cordonnier, Benoît Bovy, Jean Braun. "A versatile, linear complexity algorithm for
flow routing in topographies with depressions." *Earth Surface Dynamics* 7(2), 2019, 549–562; with
fastscapelib (GFZ Potsdam, C++/Python, GPL-3.0).** [paper] [code] [recent]
<https://esurf.copernicus.org/articles/7/549/2019/> (DOI 10.5194/esurf-7-549-2019; paper code
<https://github.com/fastscape-lem/flow-routing-depressions>; library
<https://github.com/fastscape-lem/fastscapelib>)

Depressions without filling the whole map: the algorithm computes flow paths "both within and
across the depressions through the construction of a graph connecting together all adjacent
drainage basins", in linear time, and has "the advantage of letting the user choose among
different strategies of flow path enforcement within the depressions (i.e., filling vs.
carving)". The paper repository is archived read-only; the maintained form is fastscapelib, "a
C++/Python library of efficient and reusable algorithms for landscape evolution modeling",
supported by GFZ's Earth Surface Process Modelling group.
*Bearing:* the depression step inside the erosion loop, where priority flood every iteration would
dominate: a basin graph once per step, carved so water leaves closed basins the way the real
landscape's would. fastscapelib is GPL, so it is a reference to read, not a dependency.

**Hugo Schott, Axel Paris, Lucie Fournier, Éric Guérin, Éric Galin. "Large-scale Terrain Authoring
through Interactive Erosion Simulation." *ACM Transactions on Graphics* 42(5), Article 162, 2023.**
[paper] [recent]
<https://dl.acm.org/doi/10.1145/3592787> (DOI 10.1145/3592787)

The GPU version fast enough to edit against: the authors "bridge the gap between large-scale
erosion simulation and authoring into an efficient framework", motivated by "a need for authoring
techniques offering hydrological consistency without sacrificing user control". Modelling moves
"in favour of the uplift domain": stream-power erosion is simulated on "a fast yet accurate
approximation of drainage area and flow routing" so that it runs interactively while the uplift
is edited with copy-and-paste, "warping for imitating folds and faults" and point and curve
elevation constraints; the uplift can also be reconstructed from an input elevation model.
*Bearing:* the authoring mode for a later terrain editor and the paper to port the discretisation
from; for the seeded island the same operators run once on the CPU.

**Hugo Schott, Éric Galin, Éric Guérin, Axel Paris, Adrien Peytavie. "Terrain Amplification using
Multi-scale Erosion." *ACM Transactions on Graphics* 43(4), 2024.** [paper] [code] [recent]
<https://dl.acm.org/doi/10.1145/3658200> (DOI 10.1145/3658200; code, MIT, C++ with OpenGL 4.3
compute: <https://github.com/H-Schott/MultiScaleErosion>)

Coarse in, hydrologically consistent detail out. The released code applies three operators the
README names "Erosion", "Thermal erosion" and "Deposition" in sequence and amplifies by
"alternating this sequence with the x2 upsampling", so each finer level erodes under the drainage
the coarser level fixed.
*Bearing:* the bridge between Forge's whole-map pass and its tiles: generate at 4 m once, then
upsample ×2 and erode per tile with a halo, and the 2 m detail agrees across tiles because the
process, not a random seed, decides it. This is the piece that makes the planet variant possible.

**Petros Tzathas, Boris Gailleton, Philippe Steer, Guillaume Cordonnier. "Physically-based
analytical erosion for fast terrain generation." *Computer Graphics Forum* 43(2) (Eurographics
2024), e15033.** [paper] [recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.15033> (DOI 10.1111/cgf.15033; author PDF
<https://hal.science/hal-04525371>)

A method "both physically-based and procedural", built on "the analytical solutions of the stream
power law": where simulation reaches consistency "at the cost of thousands of iterations", here
"time is no longer the stopping criterion of an iterative process but acts as the parameter of a
mathematical function, a slider that controls the aging of the input terrain from a subtle erosion
to the complete replacement by a fully formed mountain range". Extending the 1D solutions to a
heightmap uses "a multigrid accelerated iterative process", with landslides and hillslope
processes added.
*Bearing:* the candidate replacement for the time-stepped loop if its cost ever matters: the
terrain at a chosen age evaluated through a multigrid-accelerated solve instead of hundreds of
erosion steps.
Worth a spike after the iterative version exists, since it bounds the one stage of the pipeline
whose cost grows with simulated time.

**Aryamaan Jain, Bernhard Kerbl, James Gain, Brandon Finley, Guillaume Cordonnier. "FastFlow: GPU
Acceleration of Flow and Depression Routing for Landscape Simulation." *Computer Graphics Forum*
43(7) (Pacific Graphics 2024), e15243; with Jain, Benes, Cordonnier, "Efficient Debris-flow
Simulation for Steep Terrain Erosion", *ACM Transactions on Graphics* 43(4), 2024.** [paper]
[recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.15243> · <https://dl.acm.org/doi/10.1145/3658213>

FastFlow "computes the water discharge in O(log n) iterations for a terrain with n vertices
(assuming n processors)" and routes water out of depressions in "O(log² n) iterations", replacing
the sequential stack for GPU pipelines; on a 1024² terrain the paper reports "a 5× speedup for
flow routing and 34 × to 52 × speedup for depression routing compared to previous work". The
debris-flow paper adds "a new mathematical formulation for debris flow erosion derived from
geomorphology and a unified GPU algorithm for erosion and deposition", the steep-slope process that
carves "erosive scars on steep slopes and cones of deposited debris", which the stream-power law
alone does not produce.
*Bearing:* the GPU path when genesis moves to compute (authoring, or planets amplified at
stream-in), and the reason not to design the CPU version around the sequential stack forever;
debris flow is a later operator for the volcanic and alpine islands.

**Guillaume Cordonnier, Guillaume Jouvet, Adrien Peytavie, Jean Braun, Marie-Paule Cani, Bedrich
Benes, Éric Galin, Éric Guérin, James Gain. "Forming Terrains by Glacial Erosion." *ACM
Transactions on Graphics* 42(4), 2023.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3592422> (DOI 10.1145/3592422)

Glaciers as a second erosive agent, over "the combination of glacial and inter-glacial cycles":
"a fast yet accurate deep learning-based estimation of highorder ice flows and a new, multi-scale
advection scheme", combined "with finer-scale erosive phenomena to account for the transport of
debris flowing from cliffs", forming shapes "ranging from U-shaped and hanging valleys to fjords
and glacial lakes".
*Bearing:* not for the tropical island; the operator to add for cold planets and fjord coasts, and
proof that the pipeline should be a list of operators over shared fields rather than one solver.

**Wolfgang Bangerth. "Massively parallel flow routing and drainage area determination."
arXiv:2606.12800, June 2026.** [paper] [recent]
<https://arxiv.org/abs/2606.12800>

The newest word on the sequential bottleneck: "the traditional algorithm for flow routing is
sequential, and attempts to parallelize this method have so far only been moderately successful";
the proposed algorithms route water on a model of "1.88 billion points" in "4.0 seconds on 12,288
processes of a computer cluster".
*Bearing:* evidence that flow routing at planet resolution is a solved parallel problem, and a
preprint to watch; for an island the sequential stack costs tens of milliseconds and needs none of
this.

**Éric Galin, Éric Guérin, Adrien Peytavie, Guillaume Cordonnier, Marie-Paule Cani, Bedrich Benes,
James Gain. "A Review of Digital Terrain Modeling." *Computer Graphics Forum* 38(2) (Eurographics
2019), 553–577.** [paper] [still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.13657> (DOI 10.1111/cgf.13657; open copy
<https://hal.science/hal-02097510>; UCT record without full text
<https://pubs.cs.uct.ac.za/id/eprint/1337/>)

The state-of-the-art report, "organized according to three categories: procedural modeling,
physically-based simulation of erosion and land formation processes, and example-based methods
driven by scanned terrain data", by the group behind most of the papers above.
*Bearing:* the map of the field for anyone joining the terrain work; its tables of control versus
cost are the fastest way to decide which operator a new feature request belongs to.

---

## 2. Hydrology: drainage, depressions, lakes, coastlines

The previous projects' lesson stands (`procedural.md` §1): rivers are right when drainage is
guaranteed, not when the noise looks right. The algorithms are from hydrology, forty years old at
the root and still being sharpened.

**John F. O'Callaghan, David M. Mark. "The extraction of drainage networks from digital elevation
data." *Computer Vision, Graphics, and Image Processing* 28(3), 1984, 323–344; with David G.
Tarboton, "A new method for the determination of flow directions and upslope areas in grid digital
elevation models", *Water Resources Research* 33(2), 1997, 309–319.** [paper] [foundational]
<https://www.sciencedirect.com/science/article/abs/pii/S0734189X84800110> ·
<https://agupubs.onlinelibrary.wiley.com/doi/pdf/10.1029/96WR03137>

D8 and D∞. O'Callaghan & Mark drain each cell to its steepest of eight neighbours, accumulate the
upslope counts and threshold them into channels, handling "artificial pits introduced by data
collection systems". Tarboton's D∞ takes the flow direction as "a single angle taken as the
steepest downward slope on the eight triangular facets centered at each grid point" and
proportions "flow between two downslope pixels", removing the grid bias of eight directions
without the "unrealistic dispersion" of slope-weighted multiple flow.
*Bearing:* the erosion solver wants D8 (one receiver per cell, so the stack exists and drainage
area is an integer count); the *rendered* river network and the moisture field want D∞ or a
multiple-flow accumulation, or straight valleys at 45° show up on the flow preview.

**Richard Barnes, Clarence Lehman, David Mulla. "Priority-flood: An optimal depression-filling and
watershed-labeling algorithm for digital elevation models." *Computers & Geosciences* 62, 2014,
117–127; with Guiyun Zhou, Zhongxuan Sun, Suhua Fu, "An efficient variant of the Priority-Flood
algorithm…", *Computers & Geosciences* 90, 2016, 87–96; and Barnes, "Parallel Priority-Flood
depression filling for trillion cell digital elevation models on desktops or clusters",
*Computers & Geosciences* 96, 2016, 56–68.** [paper] [still-current]
<https://arxiv.org/abs/1511.04463> (DOI 10.1016/j.cageo.2013.04.024) ·
<https://github.com/cageo/Zhou-2016> (DOI 10.1016/j.cageo.2016.02.021) ·
<https://arxiv.org/abs/1606.06204> (DOI 10.1016/j.cageo.2016.07.001)

The guarantee and its speed-ups. Priority-flood works "by flooding DEMs inwards from their edges
using a priority queue" and is "optimal for both integer and floating-point data, working in O(n)
and O(n lg n) time, respectively". Zhou et al. process the cells outside depressions and flats with
two plain queues, so the priority queue sees only the rest, for an average speed-up of 44.6 % over
the fastest earlier variant. Barnes 2016 parallelises Priority-Flood "by subdividing a DEM into
tiles": each tile is filled on its own, the edges of adjoining tiles "connect the individual tiles'
spillover graphs together", and a second pass raises each tile's cells to the spill elevations the
joined graph gives; the largest run had "2 trillion (2*10^12) cells" (4.8 h on 48 cores), with
"~60% strong and weak scaling efficiencies up to 48 cores", and the tiled output showed no
"deviation from the authoritative answer" of a single Priority-Flood over the whole DEM in the
paper's correctness tests.
*Bearing:* two uses in Forge. On the finished field, the fill that makes every cell drain
(ε-gradient variant, seeded from the coast rather than the grid edge, since the island's outlet is
the sea). And the proof that filling *tiles*: the 2016 tile graph is exact, so a planet's faces can
be filled per tile and reconciled, which is the answer to "what needs the whole map" for this step
(only the tile-edge graph does).

**Richard Barnes, Kerry L. Callaghan, Andrew D. Wickert. "Computing water flow through complex
landscapes – Part 3: Fill–Spill–Merge: flow routing in depression hierarchies." *Earth Surface
Dynamics* 9, 2021, 105–121.** [paper] [recent]
<https://esurf.copernicus.org/articles/9/105/2021/> (DOI 10.5194/esurf-9-105-2021)

Lakes instead of flattened bowls: "when there is sufficient moisture, depressions take the form of
lakes and wetlands", and flow models that "eliminate depressions through filling or breaching"
risk that "this can produce unrealistic results"; Fill–Spill–Merge "utilizes our depression
hierarchy data structure to rapidly process and distribute runoff": "Runoff fills depressions,
which then overflow and spill into their neighbors. If both a depression and its neighbor fill,
they merge." In the paper's two case studies it "runs 90–2600 times faster" than the Jacobi
iteration commonly used.
*Bearing:* the lake pass: a depression becomes a lake with a water level (and a shoreline) only if
the seeded rainfall over its catchment fills it, otherwise it is carved; the same hierarchy gives
the overflow point where the outlet river starts. Without this every hollow the erosion leaves is
either a lake or a hole.

**Richard Barnes et al. RichDEM. GitHub, 2013–2026 (C++ with Python bindings, GPL-3.0).** [code]
[still-current]
<https://github.com/r-barnes/richdem>

"A set of digital elevation model (DEM) hydrologic analysis tools" using "parallel processing and
state of the art algorithms to quickly process even very large DEMs": priority-flood filling and
breaching, D8, D∞ and multiple-flow directions, flow accumulation including the parallel
trillion-cell variants, slopes and curvatures; the README lists the papers each algorithm comes
from.
*Bearing:* the reference implementation to test Forge's own against (GPL, so not a dependency):
the same inputs through RichDEM and through `forge-procgen` should give the same filled field and
accumulation up to the tie-break rules §7 fixes.

**Jean-David Génevaux, Éric Galin, Éric Guérin, Adrien Peytavie, Bedrich Benes. "Terrain
Generation Using Procedural Models Based on Hydrology." *ACM Transactions on Graphics* 32(4)
(SIGGRAPH 2013), Article 143.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/2461912.2461996> (DOI 10.1145/2461912.2461996; open copy
<https://www.cs.purdue.edu/cgvlab/www/publications/Genevaux13ToG/>)

The other order: "rivers as modeling elements", grown from the coast inward as "a hierarchical
drainage network that is represented as a geometric graph over a given input domain"; "the network
is then analyzed to construct watersheds and to characterize the different types and trajectories
of rivers", and the terrain is synthesised afterwards to fit the network from "a simple initial
sketch" and "a few parameters".
*Bearing:* not Forge's primary path (erosion gives the network for free) but the vocabulary for the
river post-process — river types by slope and order, watershed labelling, the carving of a bed
profile per reach — and the fallback for designer-drawn rivers that the erosion must then respect.
Peytavie et al. 2019 (riverscapes) and Paris et al. 2023 (meanders) carry this on; both are in
`procedural.md` §1.

**Amit Patel. "Polygonal Map Generation for Games." Red Blob Games, September 2010; and "Making
maps with noise functions", Red Blob Games, 7 July 2015.** [web] [foundational] [still-current]
<http://www-cs-students.stanford.edu/~amitp/game-programming/polygon-map-generation/> ·
<https://www.redblobgames.com/maps/terrain-from-noise/> ·
<https://simblob.blogspot.com/2022/04/improving-island-shaping-for-map.html>

The island-shape and coastline reference everyone starts from. The 2010 generator works on a
Voronoi graph rather than a noise height map, using the graph "to model the things directed by
gameplay constraints (elevation, roads, river flow, quest locations, monster types)" and noise for
"the variety not constrained by gameplay"; "The coastline is then all the edges where land and
water meet", "I set elevation to be the distance from the coast", rivers run "from the coast to
the mountains", and moisture decreases "as distance from fresh water increases", then biomes. The
2015 article gives the island mask as a distance function (for a square map
`d = 1 - (1-nx²) * (1-ny²)`) and a shaping function that "always outputs land" at the centre,
"always outputs water" at the border and allows "both land and water" in between, applied to
redistributed octave noise.
*Bearing:* stage one of the island pipeline, almost verbatim: a distance-to-centre mask warped by
low-frequency noise sets where the sea is; the uplift field is then drawn inside it. Patel's
"Improving island shaping for map generation, again" (Blobs in Games, 20 April 2022) changed his
recommended distance function after full-size 3D views showed artefacts the thumbnails had hidden,
evidence that the mask and its distance function carry much of the "does it look like an island"
quality.

---

## 3. Detail erosion, and where it fits

The large pass produces valleys and ridges at the scale of the grid it ran on (4 m and up).
Gullies, talus, banks and the roughness the eye reads at walking distance come from a second pass
with different physics: cellular shallow-water models on the GPU, or particles. Both are local, so
they tile; neither should run before the drainage exists, or they erase it.

**F. Kenton Musgrave, Craig E. Kolb, Robert S. Mace. "The synthesis and rendering of eroded fractal
terrains." *Computer Graphics* 23(3) (SIGGRAPH '89), 41–50.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/74333.74337>

Multifractal terrain with "locally independent control of the frequencies composing the surface,
and thus local control of fractal dimension", varied "with altitude or other functions to yield
more realistic first approximations to eroded landscapes", plus the first thermal and hydraulic
erosion passes over a heightfield.
*Bearing:* the thermal-erosion operator (talus slides above a repose angle) is still written as
here; and the noise-only baseline the erosion pipeline must beat on the flow preview.

**Jacob Olsen. "Realtime Procedural Terrain Generation: Realtime Synthesis of Eroded Fractal
Terrain for Use in Computer Games." IMADA, University of Southern Denmark, 31 October 2004.**
[paper] [web] [foundational]
<https://web.mit.edu/cesium/Public/terrain.pdf>

An overview "of various methods for synthesis of eroded terrain for use in computer games", the
first to make "thermal erosion and hydraulic erosion" near-real-time "by emphasizing speed" — the
fast thermal variant that moves material to the single lowest neighbour is from here.
*Bearing:* the cheapest useful detail operator, a dozen lines per cell, and the first one to add
after the large pass: it breaks the polygonal look of D8 valleys at almost no cost.

**Xing Mei, Philippe Decaudin, Bao-Gang Hu. "Fast Hydraulic Erosion Simulation and Visualization on
GPU." *Pacific Graphics* 2007 (PG '07, Maui), 47–56.** [paper] [foundational] [still-current]
<https://inria.hal.science/inria-00402079> (DOI 10.1109/PG.2007.15)

The "virtual pipes" model: "the velocity field of running water, which is created with an efficient
shallow-water fluid model", drives "the erosion and deposition process, and sediment
transportation process", "designed to be implemented totally on GPU". Four fluxes per cell, a
velocity, a sediment capacity from velocity and slope, semi-Lagrangian sediment advection.
*Bearing:* the implementable hydraulic detail pass, cellular and deterministic in a fixed
iteration order (§7), the same code on the CPU for the server and in a compute shader for
authoring previews.

**Ondřej Šťava, Bedřich Beneš, Matthew Brisbin, Jaroslav Křivánek. "Interactive Terrain Modeling
Using Hydraulic Erosion." *ACM SIGGRAPH/Eurographics Symposium on Computer Animation* 2008
(Dublin), 201–210.** [paper] [foundational]
<https://dl.acm.org/doi/10.5555/1632592.1632622> ·
<https://diglib.eg.org/handle/10.2312/SCA.SCA08.201-210> (open copy
<https://dcgi.fel.cvut.cz/en/publications/2008/stava-sca-erosion/>)

The pipe model on "a terrain, composed of layers of materials", where "two hydraulic erosion
algorithms for running water are coupled": "Areas where the motion is slow become more eroded by
the dissolution erosion, whereas in the areas with faster motion, the force-based erosion
prevails", and when water under-erodes an area "slippage takes effect and the river banks fall
into the water". The GPU simulation "runs at least at 20 fps" on "grid resolution of 2048×1024
and four layers of material", and large terrains are tiled, "each tile calculated independently
on the GPU".
*Bearing:* the layer stack (rock, regolith, sand) that makes erosion output read as geology rather
than melted wax, and the bank-slip term rivers need; the layers are also what the material rules in
§6 read.

**Balázs Jákó, Balázs Tóth (Budapest University of Technology and Economics). "Fast Hydraulic and
Thermal Erosion on the GPU." CESCG 2011 (15th Central European Seminar on Computer Graphics,
Viničné, Slovakia, 2–4 May 2011); also "Fast Hydraulic and Thermal Erosion on GPU", Eurographics
2011 Short Papers, 57–60 (DOI 10.2312/EG2011/short/057-060).** [paper] [still-current]
<https://old.cescg.org/CESCG-2011/papers/TUBudapest-Jako-Balazs.pdf>

Both operators in one GPU pass over "a predefined height field terrain", "designed to be executed
interactively on parallel architectures like graphics processors": the pipes model for water,
thermal erosion for talus, in a form short enough that it has a browser WebGPU port
(<https://github.com/joshbrew/webgpu_hydraulic_thermal_erosion_Jako2011>, MIT).
*Bearing:* the closest thing to a reference implementation of the detail pass as one kernel; the
formulation Forge's `erode.slang` should follow so the CPU and GPU versions are the same algorithm.

**Hans Theobald Beyer. "Implementation of a method for hydraulic erosion." Bachelor thesis,
Technische Universität München (Informatics: Games Engineering), 2015; with Henrik Glass, erodr
(GitHub); Sebastian Lague, Hydraulic-Erosion (GitHub, MIT); Nick McDonald, SimpleHydrology
(GitHub, MIT) and "Procedural Hydrology", nickmcd.me, April 2020.** [paper] [code] [web]
[still-current]
<https://github.com/henrikglass/erodr> · <https://github.com/SebLague/Hydraulic-Erosion> ·
<https://github.com/weigert/SimpleHydrology>

The particle family: a droplet walks downhill carrying sediment, eroding where it accelerates and
depositing where it slows. Beyer's thesis is the description most implementations cite (erodr is
"an implementation of Hans Theobald Beyer's algorithm", and Lague's README credits it by title
through its copy on firespark.de); Lague's Unity project shows the look after "70,000 erosion
iterations"; McDonald's C++ system "extends simple particle based hydraulic
erosion to capture streams and pools" with "momentum and discharge maps that create realistic
river meandering".
*Bearing:* particles give the most convincing gullies per line of code, but a particle crossing a
tile border is a determinism problem (§7); the cellular models above are the safer default for
the shipped pipeline, with particles allowed inside a tile's halo and their sediment discarded at
its edge.

**SideFX. "HeightField Erode" (with HeightField Erode Hydro, Thermal, Precipitation). Houdini
documentation.** [docs] [still-current]
<https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode.html> ·
<https://www.sidefx.com/docs/houdini/heightfields/erosion.html>

The production tool the games in §4 used: the node "simulates hydraulic and thermal erosion at a
specific scale", with HeightField Erode Hydro, HeightField Erode Precipitation and HeightField
Erode Thermal as separate nodes, and "Several output layers are generated, including eroded
`height`, `sediment`, `debris`, `flow` and `flowdir`": the sediment layer holds "the amount of
deposited sediment from hydro erosion", the debris layer "the amount of deposited debris from
thermal erosion".
*Bearing:* the output contract worth copying: Forge's genesis should emit not one height but the
height plus sediment, debris, flow and flow direction layers, because that is what the material
rules, the placement rules and the water need downstream.

---

## 4. Terrain in shipped games and their tooling

The line that matters is offline versus run time. Every ground-level open world below bakes its
geology; what it evaluates at run time is placement, materials and (for planets) the noise-based
surface itself.

**Etienne Carrier (Ubisoft Montréal). "Procedural World Generation of 'Far Cry 5'." GDC 2018;
repeated at Houdini HIVE Utrecht, June 2018.** [talk] [still-current]
<https://www.gdcvault.com/play/1025557/Procedural-World-Generation-of-Far> (write-ups:
<https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/> by Carrier,
<https://80.lv/articles/houdini-procedural-world-generation-of-far-cry-5>, and talk notes
<https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/>)

"A sophisticated procedural pipeline using Houdini and Houdini Engine" (Carrier on the PlayStation
Blog), with "a set of procedural tools to generate biomes, texture the terrain, set up freshwater
networks, generate cliff rocks and more" (80.lv); the Vault abstract asks "How do you fill up 100
square km of wilderness with a terrain changing every day?". The nightly regeneration comes from
third-party notes of the GDC talk, not a page by Ubisoft: "nightly builds on build machines to
fully refresh the world daily", each machine processing a section of the map.
*Bearing:* the offline model at AAA scale: the terrain is authored, the derived layers (biomes,
water networks, cliffs, roads, fences) are regenerated by graph whenever it changes. Forge's
equivalent is a content-hashed cache per stage so that changing the uplift re-runs only what
depends on it.

**Jaap van Muijden (Guerrilla). "GPU-Based Run-Time Procedural Placement in 'Horizon: Zero Dawn'."
GDC 2017; with Guerrilla's GDC 2022–2024 session lists.** [talk] [web] [still-current]
<https://gdcvault.com/play/1024700/GPU-Based-Run-Time-Procedural> ·
<https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn> ·
<https://www.guerrilla-games.com/read/guerrilla-at-gdc-2022> ·
<https://www.guerrilla-games.com/read/guerrilla-gdc-2023> ·
<https://www.guerrilla-games.com/read/guerrilla-gdc-2024>

A GPU system that "dynamically creates the world of Horizon Zero Dawn around the player" and
"assembles fully-fledged environments while the player walks through them", presented from "the
graph editor where artists can define the procedural placement rules" to the GPU algorithms that
build "a dense world around the player on the fly". Guerrilla's later GDC pages list deferred
texturing (McLaren) and the volumetric superstorms (2022), world and asset tooling (2023) and the
relic ruins (2024) of Forbidden West, but no terrain-generation talk: the terrain stays authored,
the placement stays a GPU rule evaluation.
*Bearing:* the split Forge already follows in the city (a compute pass places a million instances
from a seed and the heightfield): genesis writes fields, rules read them at run time, and nothing
placed is stored.

**Matthew Pohlmann (Sucker Punch). "Samurai Landscapes: Building and Rendering Tsushima Island on
PS4." GDC 2021; with Adrian Bentley, "Zen of Streaming: Building and Loading 'Ghost of Tsushima'",
GDC 2021.** [talk] [recent]
<https://gdcvault.com/play/1027352/Samurai-Landscapes-Building-and-Rendering> ·
<https://media.gdcvault.com/GDC+2021/ghost_streaming_gdc2021.pdf>

"Vegetation density and draw distance requirements meant authoring millions of object instances";
the in-engine tools include "a functional programming language interpreted as bytecode on the GPU
that artists use to describe texture and placement rules for generated data". The streaming talk's
slides are public as a PDF.
*Bearing:* the second shipped GPU rule language (Horizon's is the first), and the argument for
making Forge's material and placement rules data (P6) interpreted by one compute pass rather than
compiled per biome.

**Innes McKendrick (Hello Games). "Continuous World Generation in 'No Man's Sky'." GDC 2017.**
[talk] [still-current]
<https://www.gdcvault.com/play/1024265/Continuous_World_Generation_in__No_Man_s_Sky_>

"A step-by-step breakdown of their generation pipeline, from voxel-based world generation, through
polygonization and texturing, to eventual population and simulation", with "the techniques used to
generate planets and the supporting structures allowing this to happen continuously in real-time".
*Bearing:* the pure run-time end of the spectrum: voxel noise polygonised around the player. Its
planets have no drainage because nothing global was ever computed; Forge's planet variant keeps a
coarse global genesis precisely to avoid that.

**Doc Ross (Lead Render Programmer, Frontier Developments), interviewed by 80.lv. "Generating the
Universe in Elite: Dangerous." 80.lv, 5 April 2018.** [web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous>

"The 'landable' surfaces start as a cube with square sub-dividing faces which behave as
quadtrees"; average properties "which are planet specific, are packed up into a buffer which is
sent to the GPU when calculating the surface of the planet", where "they modulate noise functions
which are combined together to form geological shapes". The planets of the initial release "are
perfect spheres", and "their sense of height difference is provided by the normal mapping of
their surfaces".
*Bearing:* the cube-sphere quadtree with GPU noise per tile is the run-time half of Forge's planet
plan; Elite shows what it looks like without a baked global pass (no rivers, no basins), which is
what the coarse genesis adds.

**Cloud Imperium Games. "Planet Tech v4" and "CitizenCon 2019 – Terra Firmer". Star Citizen Wiki
(fan-run), summarising CIG's Alpha 3.8 material and the CitizenCon 2019 panel.** [web] [recent]
<https://starcitizen.tools/Planet_Tech_v4> · <https://starcitizen.tools/CitizenCon_2019_-_Terra_Firmer>

"Delivered in stages starting with Alpha 3.8", v4 "builds upon the features of Planet Tech v2 and
v3, offering terrain texture blending, objects scattering and biome transitions", and
"Temperature and Humidity maps infer biome selection - a data-driven approach". The wiki's page on
the CitizenCon 2019 panel lists the plan behind it: "Replace pre-baked color textures with two
layers of climate data" and "Terrain simulation, displacement of sand and soil based off of wind
and such"; the v4 page itself says wind "affects the player and StarCloth only in this version".
*Bearing:* the closest shipped analogue to `planet-environment.md`'s climate-first materials:
two climate fields drive biome and material selection at run time. Secondary source; pull the
comm-links before any number enters a spec.

**Microsoft / Asobo Studio with Blackshark.ai. Microsoft Flight Simulator (2020): Bing Maps data
and machine-learned reconstruction.** [web] [recent]
<https://techcrunch.com/2020/08/17/meet-the-startup-that-helped-microsoft-build-the-world-of-flight-simulator>
· <https://www.bellingcat.com/resources/case-studies/2020/08/24/cleared-for-takeoff-exploring-microsoft-flight-simulator-2020s-research-potential/>
· <https://www.thresholdx.net/news/blkmfs>

The contrast case: the game "pulls satellite images from Bing Maps, and populates them with
objects" (Bellingcat), and Blackshark.ai "reconstructed 1.5 billion buildings from 2D satellite
images" (TechCrunch). The process figures come from the Blackshark.ai episode of Microsoft's
6 August 2020 development update, as summarised by Threshold: "Using deep learning, buildings,
vegetation, and roads around the globe are segmented and classified", "Hundreds of virtual
machines are used in parallel to process the entire planet", "Several petabytes of data are
ingested", and "This process takes a mere 72 hours".
*Bearing:* photogrammetry plus ML is the right answer when the planet exists; for a seeded world it
sets the bar for coherence (roads meet rivers at bridges, vegetation follows terrain) that the rule
layers must reach without data.

**Brano Kemen et al. Outerra: planet engine blog, 2008–2015.** [web] [still-current]
<https://outerra.blogspot.com/2009/02/procedural-terrain-algorithm.html> ·
<https://outerra.blogspot.com/2012/02/outerra-tech-demo-released.html> ·
<https://outerra.blogspot.com/2015/05/evaluation-of-30m-elevation-data-in.html>

A real-scale Earth, in 2009 from a "~150m dataset for whole Earth": "All detail below 150m is
generated procedurally"; by the 2012 tech demo "Created from real elevation data with resolution
90m where available, 1km resolution for oceans" and "Further refined by fractal-based procedural
techniques down to centimeter-level details". The 2015 post evaluates 30 m data because "the 90m
data are lacking finer erosion patterns".
*Bearing:* the two-scale pattern Forge's planet uses — a coarse global field (here generated
rather than measured) and deterministic fractal refinement per tile — and a reminder that the
refinement must respect the coarse drainage (Schott 2024) or the added detail contradicts the
valleys.

**Epic Games. "Landscape Technical Guide", "Using Nanite with Landscapes", "Landscape Splines",
Unreal Engine 5.x documentation; and the 5.3 and 5.4 release notes.** [docs] [still-current]
[recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-technical-guide-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/using-nanite-with-landscapes-in-unreal-engine>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-splines-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5.3-release-notes?application_version=5.3>
· <https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5.4-release-notes?application_version=5.4>

"Landscapes are divided into multiple Landscape Components", "the same size and are always
square", made of sections whose size in vertices "must be a power of two value (with a maximum of
256x256) so that different LOD levels can be stored in the mipmaps of the texture"; "For the
largest Landscapes, Epic recommends a maximum of 1024 Landscape Components". The guide's
Recommended Landscape Sizes table runs from 127 × 127 to 8129 × 8129 vertices, the largest being
32 × 32 components of 2 × 2 sections of 127 quads (the others use 63-quad sections). Since 5.3
"Nanite can now be enabled in landscape actors, at parity with normal landscape rendering", and
"Nanite Landscape meshes are rebuilt in the background"; Nanite tessellation (5.4, experimental)
"tessellates the mesh at runtime into additional triangles to conform to the detail of the
displacement map" (5.4 release notes). Landscape splines are "for creating roads, paths, fences,
and other long, contiguous features that conform naturally to the landscape", curves "that
automatically deform the terrain and apply meshes along the path" (5.8 page).
*Bearing:* Epic's own move of the landscape onto its cluster DAG is the strongest endorsement of
what Forge's city already does; a 16 km island at 2 m (8193²) is just over the largest size in
Epic's table (8129²), which is why Forge tiles the cook rather than the world. Splines that deform
the field and carry meshes are the road/river authoring model.

**Unity Technologies. "Terrain", "Working with Heightmaps", "Terrain Layers". Unity Manual.**
[docs] [still-current]
<https://docs.unity3d.com/Manual/terrain-Heightmaps.html> ·
<https://docs.unity3d.com/Manual/terrain-OtherSettings.html> ·
<https://docs.unity3d.com/ScriptReference/TerrainData.html> ·
<https://docs.unity3d.com/6000.4/Documentation/Manual/class-TerrainLayer.html>

"The TerrainData class stores heightmaps, detail mesh positions, tree instances, and terrain
texture alpha maps"; the heightmap resolution "must be a power of two plus one, for example, 513,
which is 512 + 1"; "A RAW file uses a 16-bit grayscale format"; the Control Texture Resolution is
"the resolution of the splatmap that controls the blending of the different Terrain Textures". In
URP and the built-in pipeline "you can use four Terrain Layers per Texture pass, with no limit on
the number of passes"; in HDRP "you can add up to eight Terrain Layers per Terrain tile", rendered
in a single pass.
*Bearing:* the baseline any terrain system is measured against, and its limits are instructive:
four-layer splat passes and per-tile terrains are what procedural materials (§6) and a single
streamed field replace.

---

## 5. Rendering LOD for heightfields

`large-worlds.md` §8 chose the representation (cube-sphere quadtree, CDLOD far, dual contouring
near); this section adds what an island-sized heightfield needs to know about the alternatives and
their seams, and records that Forge already renders terrain through its cluster DAG.

**Frank Losasso, Hugues Hoppe. "Geometry Clipmaps: Terrain Rendering Using Nested Regular Grids."
*ACM Transactions on Graphics* 23(3) (SIGGRAPH 2004), 769–776; with Arul Asirvatham, Hugues Hoppe,
"Terrain Rendering Using GPU-Based Geometry Clipmaps", *GPU Gems 2* ch. 2, 2005.** [paper] [book]
[foundational]
<https://dl.acm.org/doi/10.1145/1015706.1015799> · <https://hhoppe.com/proj/geomclipmap/> ·
<https://hhoppe.com/gpugcm.pdf>

"A set of nested regular grids centered about the viewer", "stored as vertex buffers in fast
video memory", "incrementally refilled as the viewpoint moves", giving "visual continuity, uniform
frame rate, complexity throttling, and graceful degradation"; the main dataset was "a 40GB height
map of the United States", and "A compressed image pyramid reduces the size by a remarkable factor
of 100, so that it fits entirely in memory".
*Bearing:* the structure for the ocean and for a flat island's far field if the DAG ever costs too
much; the compression figure (100:1 on a residual pyramid) is the reference for shipping a planet's
coarse height data.

**Thatcher Ulrich. "Rendering Massive Terrains using Chunked Level of Detail Control." Notes for
the SIGGRAPH 2002 course "Super-size it! Scaling up to Massive Virtual Worlds".** [talk]
[foundational]
<http://tulrich.com/geekstuff/chunklod.html> (the author's page, with the notes as
<http://tulrich.com/geekstuff/sig-notes.pdf>, slides and code) · <http://vterrain.org/LOD/Papers/>
(index)

A quadtree of "chunks", "each chunk is a rectangular, precomputed section of optimized geometry",
and "skirts to fill cracks rather than force each chunk edge to match".
*Bearing:* the crack rule in one sentence. Forge's DAG locks tile borders instead (the city's edge
"is locked like a group border", 173 roots along it), which is exact but keeps border geometry
fine; skirts are the fallback where a locked border would cost too many clusters, and CDLOD's morph
removes the problem for a heightfield-only far field.

**Filip Strugar. "Continuous Distance-Dependent Level of Detail for Rendering Heightmaps (CDLOD)."
2010; source, paper and data on GitHub (MIT).** [paper] [code] [still-current]
<https://github.com/fstrugar/CDLOD>

"A technique for GPU-based rendering of heightmap terrains" using "a quadtree of regular grids"
selected by "precise three-dimensional distance between the observer and the terrain", vertices
morphed toward the parent so there is no stitching and no pop; DirectX 9 source and test datasets
in the repository.
*Bearing:* the far field on the planet (D-014) and the simplest crack-free scheme: the same morph
function everywhere means cube-face seams need nothing special. For the island, the DAG covers
everything and CDLOD is not needed.

**Jonathan Dupuy. "Concurrent Binary Trees (with application to longest edge bisection)."
*Proceedings of the ACM on Computer Graphics and Interactive Techniques* 3(2) (HPG 2020), Article
21; with Anis Benyoub, Jonathan Dupuy, "Concurrent Binary Trees for Large-Scale Game Components",
*PACMCGIT* 7(3) (HPG 2024); and libleb (public domain).** [paper] [code] [recent]
<https://dl.acm.org/doi/10.1145/3406186> · <https://dl.acm.org/doi/10.1145/3675371>
(arXiv:2407.02215) · <https://github.com/jdupuy/libleb>

"A binary heap, i.e., a 1D array, that explicitly stores the sum-reduction tree of a bitfield. In
this bitfield, each one-valued bit represents a leaf node of the binary tree", used "to accelerate
a longest-edge-bisection-based algorithm that computes and renders adaptive geometry for
large-scale terrains entirely on the GPU". The 2024 paper takes bisection "to arbitrary polygon
meshes rather than just squares" by "mapping a triangular subdivision primitive, which we refer to
as a bisector, to each halfedge of the input mesh", and "evaluates in less than 0.2ms on
console-level hardware"; libleb provides the algorithms "on multicore processors, including GPUs"
in C, GLSL and HLSL, released to the public domain.
*Bearing:* the GPU-driven far field for the planet once the CDLOD path is proven
(`large-worlds.md` §8 keeps the 0.2 ms figure); for the island the cluster DAG is already
GPU-driven, so CBT is a comparison to measure, not a dependency.

**Brian Karis, Rune Stubbe, Graham Wihlidal. "A Deep Dive into Nanite Virtualized Geometry."
SIGGRAPH 2021 *Advances in Real-Time Rendering in Games*; with Forge's own measurement (issue
#35).** [talk] [recent]
<https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf>

The cluster DAG as a terrain LOD is not theory in Forge: the city's ground is "one mesh through the
same DAG and cache as the props", "4 km across, a sample every 2 m: 8 M triangles, 186 k clusters,
14 levels", "a single mesh has no tile borders, so it cannot crack", cooked in 12 s and loaded in
0.85 s, drawn with the props in a frame of 1.05–1.12 ms at 1600 × 900 (`docs/demos/city-blocks.md`).
Epic's 5.3 Nanite Landscape (§4) made the same choice.
*Bearing:* the island's heightfield goes to the same cook first; the open questions are size (a
2 m island is 16× the city's ground, so a tiled cook with locked shared borders and streamed pages,
#36) and whether the locked borders cost visible cluster counts at distance, which the F1 overlay
will show.

**Matt Zucker, Yosuke Higashi. "Cube-to-sphere Projections for Procedural Texturing and Beyond."
*Journal of Computer Graphics Techniques* 7(2), 2018, 1–22.** [paper] [still-current]
<https://jcgt.org/published/0007/02/01/>

"Several approximately equal-area cube-to-sphere projections that provide low-distortion UV mapping
and enable efficient generation of jittered point sets with O(1) nearest-neighbor lookup", with
GLSL; the tangent (equi-angular) warp is the recommended easy one (`large-worlds.md` §8).
*Bearing:* the planet's tile frame. Genesis on the sphere runs on the warped cube grid, so the D8
edge lengths and cell areas per face come from this projection, not from a constant `Δx`.

**Tao Ju, Frank Losasso, Scott Schaefer, Joe Warren. "Dual Contouring of Hermite Data." *ACM
Transactions on Graphics* 21(3) (SIGGRAPH 2002), 339–346; with Eric Lengyel, "Voxel-Based Terrain
for Real-Time Virtual Simulations", PhD dissertation, UC Davis, 2010 (the Transvoxel algorithm;
also "Transition Cells for Dynamic Multiresolution Marching Cubes", *JGT* 15(2), 2010).** [paper]
[foundational] [still-current]
<https://dl.acm.org/doi/10.1145/566654.566586> · <https://transvoxel.org/> ·
<https://transvoxel.org/Lengyel-VoxelTerrain.pdf>

Dual contouring contours "a signed grid whose edges are tagged by Hermite data (i.e., exact
intersection points and normals)" with a "numerically stable representation for quadratic error
functions" and octree simplification that "requires no 'crack patching'". Transvoxel "works by
inserting special transition cells in between regular cells along the boundary between voxel data
sampled at one resolution and voxel data sampled at exactly half that resolution", and its "512
cases" "fall into the 73 equivalence classes".
*Bearing:* unchanged from D-014: the near-field mesher for caves, overhangs and edits, with the
heightfield converted to a signed field only inside edited or feature bricks; the seam between the
brick and the DAG ground is a Transvoxel transition, not a stitch.

**Éric Bruneton, Fabrice Neyret. "Real-Time Rendering and Editing of Vector-based Terrains."
*Computer Graphics Forum* 27(2) (Eurographics 2008), 311–320.** [paper] [still-current]
<https://maverick.inria.fr/Publications/2008/BN08/> (DOI 10.1111/j.1467-8659.2008.01128.x)

"Very large terrains with very detailed features such as roads, rivers, lakes and fields" from
"vector descriptions of linear and areal features, with associated shaders to specify their
appearance (terrain color and material), their footprint (effect on terrain shape), and their
associated objects (bridges, hedges, etc.)", refined in "a view dependent quadtree refinement
scheme": "New quads are generated when needed and cached on the GPU".
*Bearing:* rivers and roads as vectors with a footprint and an appearance, rasterised into the
height and the material of the tiles that need them: the model for Forge's river polylines and
lake polygons from §2, and the origin of the "decal into a runtime virtual texture" pattern in §6.

---

## 6. Texturing and materials at terrain scale

The owner sees repeating textures at once. The literature's answer has three parts: materials from
fields and rules instead of painted splat maps, a virtual texture so the evaluated result is cached
per page, and a stochastic tiling operator so that no tile is ever visibly reused.

**Johan Andersson (DICE). "Terrain Rendering in Frostbite Using Procedural Shader Splatting."
SIGGRAPH 2007 course *Advanced Real-Time Rendering in 3D Graphics and Games*, course notes
38–58.** [talk] [foundational] [still-current]
<https://dl.acm.org/doi/10.1145/1281500.1281668> (DOI 10.1145/1281500.1281668) · course-notes
chapter on EA's site
<https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/chapter5-andersson-terrain-rendering-in-frostbite.pdf>
· slides <https://www.realtimerendering.com/advances/s2007/Andersson-TerrainRendering(Siggraph07).pdf>

Terrain shading "computed dynamically in shaders rather than storing static textures", from
compact masks and rules (slope, height, curvature), over "a quadtree for culling and level-of-detail
with fixed grids for the leaf nodes".
*Bearing:* the rule-based material model Forge's D-028 layered row already half-implements (a byte
per texel names a layer): the next step is to derive that byte, and the blend weights, from genesis
fields at cook time and from slope and curvature at run time.

**Ryan Geiss. "Generating Complex Procedural Terrains Using the GPU." *GPU Gems 3* ch. 1, 2007;
with Ben Golus, "Normal Mapping for a Triplanar Shader", Medium, 17 September 2017 (shaders on
GitHub).** [book] [web] [foundational] [still-current]
<https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-1-generating-complex-procedural-terrains-using-gpu>
· <https://bgolus.medium.com/normal-mapping-for-a-triplanar-shader-10bf39dca05a> ·
<https://github.com/bgolus/Normal-Mapping-for-a-Triplanar-Shader>

Geiss builds terrain from a density function on the GPU (DirectX 10 geometry shader, stream output,
3D textures) and textures it triplanarly, which is where the technique became standard for
overhanging terrain. Golus shows that triplanar mapping "is plagued by half-hearted and incorrect
implementations of normal mapping" and gives the correct blends (UDN, whiteout, reoriented normal
mapping) with example shaders.
*Bearing:* Forge's resolve already samples "textures without coordinates" by triplanar projection
of the object-space position with analytic derivatives (ARCHITECTURE §4); Golus's normal blending
is the piece to check in `shading/standard` before cliffs get normal maps.

**Eric Heitz, Fabrice Neyret. "High-Performance By-Example Noise using a Histogram-Preserving
Blending Operator." *PACMCGIT* 1(2) (HPG 2018, best paper); with Thomas Deliot, Eric Heitz,
"Procedural Stochastic Textures by Tiling and Blending", *GPU Zen 2*, 2019 (Unity blog and
plugin); and Brent Burley, "On Histogram-Preserving Blending for Randomized Texture Tiling",
*Journal of Computer Graphics Techniques* 8(4), 2019, 31–53.** [paper] [book] [code] [recent]
<https://dl.acm.org/doi/10.1145/3233304> (DOI 10.1145/3233304) ·
<https://eheitzresearch.wordpress.com/738-2/> ·
<https://unity.com/blog/engine-platform/procedural-stochastic-texturing-in-unity> ·
<https://jcgt.org/published/0008/04/02/>

"Takes as input a small example of a stochastic texture and synthesizes an infinite output with
the same appearance", for "natural textures such as moss, granite, sand, bark, etc.", "more than
20 times faster" than procedural noise of comparable quality; the insight is that "with Gaussian
inputs, histogram-preserving blending boils down to mean and variance preservation". Deliot &
Heitz simplify it for production ("replace the full 3D histogram transformation by three 1D
histogram transformations", plus "a look-up table prefiltering algorithm" for mipmaps), and
Unity's plugin covers large surfaces with small textures "without any repetition artifacts";
Burley fixes the ghosting by "exponentiating the blending weights".
*Bearing:* the direct fix for the owner's first complaint. Every tileable in Forge's `textures`
module gets a Gaussian-transformed twin and an inverse LUT, and terrain layers are sampled through
the tiling-and-blending operator; cost is three samples instead of one, paid only in the terrain
classes.

**Sean Barrett. "Sparse Virtual Texture Memory." GDC 2008; J.M.P. van Waveren, "Software Virtual
Textures" (25 February 2012), in *Virtual Texturing in Software and Hardware* (Obert, van Waveren,
Sellers), SIGGRAPH 2012 course; and Epic Games, "Runtime Virtual Texturing", Unreal Engine
documentation.** [talk] [paper] [docs] [foundational] [still-current]
<https://gdcvault.com/play/417/Sparse-Virtual-Texture> · <http://silverspaceship.com/src/svt/> ·
<https://dl.acm.org/doi/10.1145/2343483.2343488> (author's copy
<https://mrelusive.com/publications/papers/Software-Virtual-Textures.pdf>) ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/runtime-virtual-texturing-in-unreal-engine>

Barrett's talk ("All the source code is in the public domain") is the open form of virtual
texturing; RAGE used "a single 120k x 120k virtual texture for all static geometry" with pages of
128 × 128 texels, a 120 × 120 payload "surrounded by a 4-texel border", and a software page table.
Epic's runtime variant "creates its texel data on demand using the GPU at runtime", "caches
shading data over large areas", and is "a good fit for Landscape shading that uses decal-like
materials and splines".
*Bearing:* the residency model for the evaluated terrain material: rules and stochastic tiling
are expensive per texel, so they render once into a runtime virtual texture page and the resolve
samples the cache; roads and river banks are decals rendered into the same pages (Bruneton &
Neyret's appearance shaders, §5), which is how the city's layer map generalises.

---

## 7. Determinism, seeds and tiling

P3 and D-016 require that the client and the server compute the same island from the same seed.
The pitfalls are known: platform transcendental functions, summation order, hash quality, and any
algorithm whose result depends on which of two equal candidates is taken first.

**Ken Perlin. "Improving Noise." *ACM Transactions on Graphics* 21(3) (SIGGRAPH 2002), 681–682;
with Mark Jarzynski, Marc Olano, "Hash Functions for GPU Rendering", *Journal of Computer Graphics
Techniques* 9(3), 2020, 21–38.** [paper] [foundational] [recent]
<https://dl.acm.org/doi/10.1145/566654.566636> (reference implementation
<https://mrl.cs.nyu.edu/~perlin/noise/>) · <https://jcgt.org/published/0009/03/02/>

Perlin's corrected noise fixes a "second order interpolation discontinuity and unoptimal gradient
computation": the interpolant becomes 6t⁵ − 15t⁴ + 10t³ (`t * t * t * (t * (t * 6 - 15) + 10)` in
the reference implementation) and the gradient set is fixed; its lattice hashing is a permutation
of integers, which is why gradient noise is deterministic when the lattice coordinate is an
integer. Jarzynski & Olano evaluate GPU hashes "for random number
quality using the TestU01 test suite, and GPU execution speed", including "pcg3d", and recommend
by cost and quality.
*Bearing:* Forge's `hash::pcg3d`/`pcg4d` (already used by the city's placement) are the lattice
hash for every noise in genesis, keyed by integer cell coordinates and a derived seed, so noise
never depends on a float's rounding; the only float work is the interpolation, which is IEEE
arithmetic and portable.

**Bruce Dawson. "Floating-Point Determinism." Random ASCII, 16 July 2013; with Glenn Fiedler,
"Floating Point Determinism", gafferongames.com, 24 February 2010.** [web] [still-current]
<https://randomascii.wordpress.com/2013/07/16/floating-point-determinism/> ·
<https://gafferongames.com/post/floating_point_determinism/>

Is IEEE arithmetic deterministic? Dawson's answer is "an unequivocal 'yes'" and "also an
unequivocal 'no'": the basic operations are, given the same instruction sequence and rounding
mode; what differs is compilers (contraction into FMA, reassociation; "if you need determinism
across architectures you will have to avoid fmadd"), libraries ("The precise results of functions
like sin, cos, tan, etc. are not defined by the IEEE standard") and instruction sets. Fiedler's
post collects practitioners' reports: SSE use judged "too under-specified to be deterministic", a
replay saved "from a debug build" failing "in release builds", and "AMD and Intel processors"
giving "slightly different results for transcendental functions".
*Bearing:* D-016's rule list in two blog posts. For genesis specifically: `dmath` for every
transcendental (the stream-power law's `A^m` is a `pow`), no `mul_add` unless it is explicit on
both sides, drainage area as an integer cell count times the cell area (exact, order-independent),
and fields reduced in index order.

**rust-lang/libm. "A port of MUSL's libm to Rust." GitHub / crates.io (MIT or Apache-2.0,
`no_std`; now maintained inside compiler-builtins).** [code] [still-current]
<https://github.com/rust-lang/libm>

Pure-Rust implementations of the C math library ("sin, cos, exp, pow, sqrt, atan2, and other
standard math operations") that compile to the same instruction sequence on every target; the
repository is archived because the crate "has been merged into the compiler-builtins repository"
where development continues.
*Bearing:* the library behind `forge_core::dmath`, and the reason it is safe to use `pow` and `exp`
in the erosion loop on both the client and the server: the same code runs on both, whatever the
platform's `libm` does.

**Tiling, halos and what needs the whole map** (a synthesis of the entries above; no new source).
Three of the pipeline's stages are global and two are local. Priority flood is global in principle
but Barnes 2016 shows the exact tile decomposition (fill tiles, then reconcile a graph of tile
edges). The implicit stream-power solve is global in a stronger sense: a cell's new height depends
on its receiver's, all the way to the outlet, so a step must see the whole drainage tree; the
parallelism is across trees (basins), not across tiles, and the planet variant therefore runs it on
a coarse global grid. Drainage area and lake levels are global too (they are sums over catchments).
Thermal erosion, the pipe model and multi-scale amplification are local with a finite footprint per
iteration, so a tile of side `T` with a halo of `k · iterations` cells reproduces the untiled result
inside the tile exactly, at the cost of the halo's redundant work; Schott 2024 is built on this.
Particles are local per step but travel, so a particle pass tiles only if particles die at the
halo's edge, which changes the statistics near borders but not the determinism, provided the
particle seeds derive from the tile id. Pinned tie-breaks complete the picture: the D8 receiver on
equal slopes is the lowest-index neighbour, the priority queue orders by `(height, index)`, and the
stack is built by a deterministic traversal, so no result depends on heap or thread order.

---

## Recommendation for Forge

**The pipeline for `island`, in order.** Every stage is a pure function of a `Seed::derive`d seed and
a parameter record, writes named fields (`Field2<T>` with spacing, size and an `f64` frame origin),
and is cached by the content hash of its inputs, so an uplift change re-runs stages 3–7 and a
material change only 7.

1. **Island mask.** A distance-to-centre shaping function (Patel 2015) warped by two octaves of
   low-frequency `pcg3d` gradient noise, thresholded at sea level 0, plus the signed distance to
   the coast. Outputs: land mask, coast distance. Cost: one pass, milliseconds.
2. **Uplift field.** A ridge spine (warped ridged noise, or a few seeded segments with falloff)
   scaled inside the mask, zero at and beyond the coast; a hardness field (two or three layers:
   basalt, regolith, sand; Šťava 2008) and a rainfall field (orographic later, from
   `planet-environment.md`). Outputs: `uplift`, `hardness`, `rain`.
3. **Stream-power erosion, implicit.** Braun & Willett's loop at 4 m: D8 receivers with the
   lowest-index tie-break; depressions by the Cordonnier–Bovy–Braun basin graph (carve mode);
   stack from the coast (every coast cell is an outlet at base level 0); drainage area as an
   integer count, times 16 m²; the ordered implicit update with `n = 1`, `m = 0.5`, `K` from
   hardness, then a few sweeps of hillslope diffusion; uplift added per step. 200–400 steps with a
   large `Δt`, basins in parallel, merged by index. Outputs: `height` (4 m), `area`, `receiver`.
4. **Priority flood and hydrology.** On the eroded field: ε-gradient priority flood seeded from the
   coast; D8 for the network, D∞ accumulation for moisture; rivers where the catchment exceeds a
   threshold (0.5 km² = 31 250 cells at 4 m, tuned by eye on the preview), traced to polylines with
   Strahler order and a width and depth from area (Peytavie 2019's rules, `procedural.md`); lakes
   by a depression hierarchy with the Fill–Spill–Merge rule under the rainfall budget (a level and a
   shoreline per lake, an outlet where it spills); river beds carved into the height with a profile
   per reach. Outputs: `flow_dir`, `flow_acc`, `water_dist`, river polylines, lake polygons.
5. **Detail erosion and amplification to 2 m.** Schott 2024's scheme: upsample ×2 (to 8193²),
   then per 2049² tile with a 64-cell halo: fast thermal erosion (Olsen 2004) with the repose angle
   from hardness, a short pipe-model pass (Mei 2007 / Jákó & Tóth 2011) with rain from `rain` and
   the rivers held as fixed inflows, deposition; river beds re-carved; tiles in parallel, halo
   discarded. Outputs: `height` (2 m), `sediment`, `debris` (Houdini's contract).
6. **Materials from fields.** A rule table (P6, data) over slope, altitude, curvature, `flow_acc`,
   `water_dist`, `coast_dist`, `sediment`, `debris` and `hardness` → a layer byte per texel and a
   blend weight (D-028), at 2 m: sand where the coast is near and the ground low, rock above a
   slope threshold and where `debris` is thin, wet soil along `water_dist`, grass and forest by
   altitude and moisture, snow above a line. Output: `layers` (u8), `weights` (u8).
7. **Hand-off.** The 2 m height as `f32` in grid order to a `terrain_mesh`-shaped builder and the
   existing cluster-DAG cook and cache (tiled, see below); the layer map as the terrain row's
   layered material; the river polylines and lake levels to the water surface (Phase 2 step 3);
   the same heightfield to the placement pass, as the city does today.

**Sizes.** The domain is a 16.384 km square (2¹⁴ m). At 4 m it is 4097² = 16.8 M samples; at 2 m,
8193² = 67.1 M. The city's ground is 2001² samples (4 km at 2 m), 8 M triangles.

| Field | 4097² (4 m) | 8193² (2 m) |
|---|---|---|
| height, uplift, sediment (`f32` each) | 67 MB | 268 MB |
| height quantised (`u16`, 1 cm steps to 655 m; `i32` mm if taller) | 34 MB | 134 MB |
| receiver, stack, area (`u32` each) | 67 MB | — |
| layers + weights (`u8` each) | 17 MB | 67 MB |
| triangles in the mesh | 33.5 M | 134 M |

The 4 m working set of stage 3 is about six fields, 400 MB; the 2 m stage works per tile. The mesh
at 2 m is 16× the city's ground: one 134 M-triangle cook is not the city's 12 s but, if the cook
scales linearly, three to four minutes and several gigabytes, so cook 4 × 4 tiles of 2049² (each
the city's size) with shared borders locked, one cache file each, streamed through the page pool
(#36); a 4 m island (33.5 M triangles) fits a single mesh if the first demo needs it faster. The
planet's tiles are the same 2049² unit.

**CPU cost, estimated and to be measured.** Stage 3 touches 16.8 M cells about five times per step;
at a few nanoseconds a cell that would be 0.2–0.4 s per step single-threaded and 1–2 min for 300
steps on one core. The published measurement is less kind: Barnes 2019 ran 120 steps on 10⁸ cells
in 3,000 s with the serial Braun & Willett code (about 250 ns per cell per step), about 210 s with
his parallel CPU version on a 160-thread POWER8 node and 70 s on a Tesla P100. Scaled linearly to
the island's 16.8 M cells and 300 steps, that is about 20 min serial and 1.5 min for the parallel
CPU form on that 2016-era hardware. So the few-nanosecond figure is the best case a
cache-friendly implementation might reach, not a plan: stage 3 must be basin-parallel from the
start (Braun & Willett's ordering is "ideally suited to parallelization"), and its per-step time
is the first number `genesis` prints. Priority flood on integer heights is O(n): about a second at
4 m. Stage 5 is 16 tiles of 4.5 M cells with halo, a few dozen iterations each: tens of seconds in
parallel. Stage 6 is one pass. Target: the whole genesis under 60 s cold on the dev PC, which
Barnes's figures say needs the parallel solver and possibly fewer steps; milliseconds warm from
the cache; the cook adds its minutes once. Tzathas 2024's analytical solution is the spike to run
if stage 3 dominates.

**What is deterministic, and what needs D-016's rules.** Everything above is deterministic by
construction if: noise is lattice-hashed with `pcg3d` on integer coordinates; the D8 tie-break, the
`(height, index)` queue order and the stack traversal are pinned; drainage area is an integer
count; transcendental functions go through `dmath` (`libm`); parallel work is by basin or by tile
and merged by index; and the particle variant, if ever used, seeds per tile and kills particles at
the halo. Genesis runs on the CPU on both the client and the server (P8); GPU erosion is for
authoring previews only, since a compute shader's `pow` is not the server's. The CI digest at one
and six workers (D-016) covers stages 3 and 5; a golden digest per stage per seed is the test.

**The cube-sphere planet, from the same pipeline.** What changes: (1) the domain is a graph on the
six warped cube faces (Zucker & Higashi's tangent warp), so neighbours cross face edges and D8 edge
lengths and cell areas per face come from the projection; the basin graph, the stack and priority
flood are already graph algorithms, so stages 3–4 run unchanged on a *coarse global grid* — six
faces of 1025² (6.3 M cells) is 2.3 km spacing on a 1 500 km planet and 10 km on an Earth-sized
one; (2) uplift comes from plates (Sculpting Mountains) or a seeded plate field instead of an island
mask, and sea level is the outlet everywhere; (3) stage 5 becomes the *streaming-time* stage: a
2049² tile at 4 m and then 2 m is amplified from the coarse field on demand in `Low`-priority jobs
(≤ 200 µs slices, ARCHITECTURE §3), deterministic per S2-style `u64` cell id, cached on disk; a
1 km tile at 2 m is 250 k cells, a few milliseconds of thermal and pipe iterations; (4) rendering
keeps the DAG per tile with locked borders near the camera and switches to CDLOD (later CBT) per
face for the far field (D-014, `large-worlds.md` §8); (5) the per-face frame is the tile's `f64`
origin under D-004, and every field's `f32` is local to it.

**What to build first, without a GPU (best cost/benefit).**
1. `forge-procgen`: `Field2<T>`, PNG writers (16-bit height, hillshade by Horn's gradient, log
   flow accumulation, rivers and lakes over the hillshade, layer map in colours), and a `genesis`
   binary: `genesis --seed 7 --size 16384 --spacing 4 --out island/` printing per-stage timings.
   A day; it makes every later stage visible.
2. Stages 1–2 and the flow routing of stage 3 (receivers, basin graph, stack, integer area):
   the flow preview alone shows whether the drainage is right, before any erosion.
3. The implicit stream-power loop with diffusion: the mountains appear; log seconds per step.
4. Stage 4: priority flood, rivers as polylines with widths, lakes by the depression hierarchy.
5. Stage 5 at 2 m per tile with halos; the determinism digests at one and six workers.
6. Stage 6, the rule table, and the layer map through D-028's layered row.
7. Stage 7 through the existing cook: the `island` demo renders with today's renderer, TAA on, the
   F1 overlay showing the terrain's clusters; then the far-field question (CDLOD/CBT) for the planet.

---

## What the numbers say

Resolutions: shipped landscape systems live at a few thousand samples a side — Epic's guide caps
sections at 256 × 256 vertices, recommends at most 1,024 components and tops its size table at
8129 × 8129 vertices, Unity's heightmaps are a power of two plus one with a 16-bit RAW
interchange, Far Cry 5 covered 100 km² and (by notes of the talk) refreshed its world nightly;
Outerra refines 90 m (later 30 m) real data with fractal detail to centimetres, and Flight
Simulator's world was reconstructed from several petabytes of Bing imagery in 72 hours on hundreds
of machines. Forge's island at 4 m is 4097² (16.8 M cells, 67 MB per `f32` field) and at 2 m 8193²
(67 M cells, 268 MB), against the city's 2001² today. Generation times from the geomorphology side
are the useful ones: the implicit solver is linear per step and unconditionally stable (Braun &
Willett 2013); Barnes's parallel version runs 120 steps on 10⁸ cells in 70 s on a Tesla P100, 43×
the serial code (3,000 s); priority flood is O(n) on integer heights, Zhou's variant is on average
44.6 % faster than the fastest earlier variant, and the tiled version keeps ~60 % efficiency to
48 cores on a 2 × 10¹²-cell DEM; FastFlow routes flow in O(log n) parallel iterations and
Bangerth's 2026 preprint routes 1.88 × 10⁹ points in 4.0 s on 12,288 processes. The interactive
erosion frameworks (Schott 2023) edit large domains on a GPU; the amplification code (Schott 2024)
runs on OpenGL 4.3 compute. Rendering: clipmaps held a 40 GB heightmap in memory at 100:1
compression in 2004; RAGE's virtual texture was 120k × 120k in 128 × 128 pages; Forge's DAG draws
the 8 M-triangle city ground within a 1.05–1.12 ms frame at 1600 × 900. Texturing: histogram-
preserving tiling is over 20× faster than procedural noise of the same quality, at three texture
samples per lookup.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A Death Stranding terrain talk** — none found. Searches return the Decima/Horizon material
  (Guerrilla shared the engine and its placement pipeline with Kojima Productions) and press pieces;
  Death Stranding is covered only through the Horizon entry.
- **A Horizon Forbidden West terrain talk** — Guerrilla's GDC pages list deferred texturing
  (McLaren) and the volumetric superstorms (2022), the living world, cauldrons and "Scaling Tools
  for Millions of Assets" (2023) and the relic ruins (2024); nothing on terrain generation. Van
  Muijden's 2017 placement talk stands for Guerrilla.
- **Cordonnier et al. 2016's timings and grid sizes** — on 2026-09-26 the HAL page and PDF sat
  behind a bot check and Wiley refused the fetch; the abstract (read through HAL's API) is quoted,
  and `procedural.md`'s "seconds" is attributed to the earlier session.
- **Beyer's thesis itself** — a copy exists on firespark.de (the one Lague's README credits by
  title), but its certificate had expired on 2026-09-26 and it could not be read; the entry
  describes the thesis through erodr's README.
- **Claims removed on 2026-09-26 because the cited page does not contain them** — kept here so they
  are not re-added from memory:
  - Schott 2023 running "thermal erosion, hillslope diffusion and deposition" with painted material
    maps: not in the abstract (the only reachable text); the entry now describes what the abstract
    says.
  - Van Muijden's team having "moved from the traditional CPU-based placement system to
    real-time": on neither the GDC Vault page nor Guerrilla's.
  - No Man's Sky's aim to "enable our artists to produce more, rather than replacing them with an
    algorithm": not on the GDC Vault page.
  - Far Cry 5's team having "regenerated the entire game world every night on special build
    machines": not on the Vault page, Carrier's PlayStation Blog post, 80.lv or SideFX's page; the
    nightly refresh is kept, attributed to third-party notes of the talk.
  - Star Citizen's "seamlessly supports spherical terrain at a planetary scale": search records
    trace it to PC Gamer's coverage of CitizenCon 2016 (the earlier planet tech), not to v4.
  - Flight Simulator's "2 petabytes of data from Bing Maps": on neither TechCrunch nor Bellingcat,
    and no Microsoft or Asobo page with the figure was reached; the entry uses "several petabytes"
    from the development-update summary.
  - Epic's "63×63 quads is a good, performant choice for section size": not in the technical guide.
  - Unity's splat-map wording ("each Splatmap is capable of containing 4 textures", a fifth layer
    adding "a second splatmap" and "1 additional shader pass", 0.5 m per splat-map pixel at 1024
    px): on none of the current manual pages (heightmaps, terrain settings, terrain layers, in
    6000.x and 2019.4); replaced by the Terrain Layer page's per-pass limits.
  - Patel 2010's "which led to putting rivers at the beginning of the generation process": not on
    the page.
  - RAGE's "126 + 1 pixel border on all sides": from Holger Dammertz's notes on sparse virtual
    texturing, not from van Waveren; RAGE's pages have a 4-texel border.
  - A Rust port of Jákó & Tóth: none found; the WebGPU port is cited.
  - Outerra 2015's 30 m data that "comes with much better erosion shapes": not in the post, which
    says the 90 m data lack "finer erosion patterns".
- **Machine-learned terrain** — TerraFusion (Higo, Kanai, Endo, Kanamori, *Virtual Reality &
  Intelligent Hardware* 7(6), 2025; CGI 2025), StyleDEM (2023), Terrain Diffusion Network (2023) and
  Geodiffussr (2025) exist and were seen in search results. Not adopted: a diffusion model's output
  is not a pure function of a seed across GPUs and drivers (P3/P8), and none guarantees drainage.
  Listed here so nobody re-derives the omission.
- **Unerosion (SCA 2024), Perche et al. "Authoring Terrains with Spatialised Style" (CGF 2023),
  Paris et al. "Desertscape Simulation" (2019), Kristof et al. SPH erosion (2009)** — seen, out of
  scope for the island; the debris-flow and glacial entries cover the operator family.
- **World Machine and Gaea** — the other production erosion tools; no primary technical page was
  checked, so Houdini's documentation stands for the tooling contract.
- **Hardy & McRoberts 2006 "Blend maps"** (height-based splat blending) — not searched within the
  budget; height-based blending is treated as folklore and left to the implementation.
- **Cesium's quantized-mesh and its skirts** — not searched; Ulrich 2002 is the skirt reference.
- **GPU Gems 2 ch. 2 (Asirvatham & Hoppe)** — hhoppe.com's clipmap page points to the chapter and
  the PDF is served, but its text could not be extracted through WebFetch; folded into the Losasso &
  Hoppe entry.
- **Sebastian Lague's erosion video** — a video, not citation grade; the repository is cited.

---

## Verification notes

First written on 2026-09-25 from a cloud session whose proxy reached only `github.com`, so every
other source was then confirmed only through the search engine's record of it. Re-verified on
2026-09-26 for issue #99 from the owner's machine, with WebFetch and WebSearch only (no browser
pane, no YouTube pages), against the primary page wherever it could be reached.

- **Access on 2026-09-26.** Reached: arXiv and its ar5iv HTML renderings, Copernicus (esurf), the
  Crossref and OpenAlex APIs (publisher-deposited metadata and abstracts), the HAL API (abstracts
  and file lists), GDC Vault, dev.epicgames.com, docs.unity3d.com, sidefx.com, guerrilla-games.com,
  80.lv, blog.playstation.com, hhoppe.com, redblobgames.com, simblob.blogspot.com, Patel's Stanford
  page (as xenon.stanford.edu: the www-cs-students certificate does not match the host),
  outerra.blogspot.com, randomascii, gafferongames.com, NVIDIA's GPU Gems, TechCrunch, Bellingcat,
  starcitizen.tools, transvoxel.org, old.cescg.org, tulrich.com, geodynamics.org, the HPG 2018
  best-paper page. Refused: onlinelibrary.wiley.com, agupubs, sciencedirect.com, dl.acm.org,
  gfzpublic.gfz.de, diglib.eg.org and researchgate.net (HTTP 403), hal.science and
  inria.hal.science pages and PDFs and dblp.org (a bot check), bgolus.medium.com (403), jcgt.org
  (an empty page for every article), firespark.de (expired certificate), Semantic Scholar's API
  (429). The Frostbite news page on ea.com now returns 404. PDFs from hhoppe.com, tulrich.com,
  web.mit.edu, old.cescg.org, cgg.mff.cuni.cz, mrelusive.com and EA's CDN were served, but WebFetch
  could not extract their text. Paywalled papers were checked against the abstract the publisher
  deposits with Crossref (or HAL's copy of it) and their metadata against Crossref's record.
- **Fetched and read (GitHub), 2026-09-25, not re-checked:** fastscapelib,
  flow-routing-depressions, MultiScaleErosion, RichDEM, Barnes2019-Landscape, CDLOD, libleb,
  LongestEdgeBisection and its demos, SimpleHydrology, Hydraulic-Erosion (Lague), rust-lang/libm.
  Quotes from these are verbatim from their READMEs. On 2026-09-26 also read: Lague's README (it
  credits Beyer's thesis by title), Golus's shader repository, the Jákó & Tóth WebGPU port.
- **Verified on 2026-09-26 against the primary page and held as written:** Whipple & Tucker 1999
  (Crossref abstract, metadata); Bangerth 2026 (arXiv); Tarboton 1997 (Crossref abstract); Tsushima
  2021 (GDC Vault 1027352; Bentley's talk is GDC Vault 1027205, the slides PDF is served, over
  10 MB); Karis 2021 (the Advances 2021 index links the PDF); Geiss 2007 (NVIDIA's chapter); Perlin
  2002 (Crossref abstract and metadata; the fade polynomial from Perlin's reference implementation,
  now linked); Ju 2002 (Crossref abstract); Heitz & Neyret 2018 (Crossref abstract, the HPG 2018
  best-paper page; one word fixed); UE Runtime Virtual Texturing (Epic's page).
- **Metadata verified on 2026-09-26, quotes still from the search record** (the primary text was
  not reachable): Braun & Willett 2013 (Crossref and OpenAlex: Geomorphology 180–181, 170–179;
  ScienceDirect and GFZ refused; the FastScape-languages sentence confirmed on geodynamics.org);
  O'Callaghan & Mark 1984 (Crossref; no abstract deposited); Musgrave 1989 (Crossref, pp. 41–50;
  no abstract deposited); Andersson 2007 (Crossref, course notes pp. 38–58; the ea.com URL was dead
  and is replaced by EA's course-notes PDF and the Advances 2007 slides); van Waveren 2012
  (Crossref; the course's authors are Obert, van Waveren and Sellers); Jarzynski & Olano 2020
  (volume, number and pages on Jarzynski's own page); Ulrich 2002 (the author's page gives the
  course, "Super-size it! Scaling up to Massive Virtual Worlds", and serves the notes; their text
  could not be extracted). Schott 2023's "Article 162" is from the search record; Crossref gives
  pages 1–15 without an article number.
- **Not reachable on 2026-09-26 either, graded as before (search record only):** Olsen 2004 (the
  MIT-hosted PDF is served but unreadable through WebFetch; a departmental report, not
  peer-reviewed); Zucker & Higashi 2018 and Burley 2019 (jcgt.org serves empty pages; volume,
  number and pages match the search record); Golus 2017's Medium article (403; the GitHub
  repository it links is read); Beyer's thesis (expired certificate).
- **Verified on 2026-09-26 and corrected** (what was wrong → the fix):
  - Cordonnier 2016: a quote altered ("the method generates" for "we generate") and an open-copy
    URL on cel.hal.science that is not HAL's record → exact quote from HAL's abstract, URL
    inria.hal.science/hal-01262376.
  - Sculpting Mountains 2018: two quotes spliced from separate sentences → split into the exact
    sentences (HAL abstract); page range 1756–1769 now confirmed by Crossref.
  - Barnes 2019: quote spacing; the 70 s figure lacked its context → exact quote, plus "120
    timesteps", the P100 node and the 43×/9×/3× comparisons from the paper's body (ar5iv); the
    entry's bearing and the recommendation's cost paragraph changed (see "Numbers re-checked").
  - Cordonnier, Bovy & Braun 2019: two quotes reworded ("allows users to choose … such as filling
    versus carving" was the paper's "letting the user choose … (i.e., filling vs. carving)") →
    exact text from the Copernicus abstract and conclusion.
  - Schott 2023: quote altered; operators not in the abstract → rewritten from the Crossref
    abstract (moved item under "Checked and left out").
  - Schott 2024: authors out of order → Schott, Galin, Guérin, Paris, Peytavie (Crossref).
  - Tzathas 2024: the quote "contrary to simulation-based approaches, the algorithm does not rely
    on a time-stepping scheme …" is not in the abstract, and the method is not free of iteration →
    exact abstract sentences (time as the parameter of a function, "a multigrid accelerated
    iterative process"); bearing and the five-sentence summary adjusted.
  - FastFlow and debris flow 2024: the debris-flow quote was a paraphrase → exact Crossref text;
    FastFlow's measured speed-ups added.
  - Glacial erosion 2023: quote spliced and reworded → exact Crossref text.
  - Galin 2019: the UCT eprint holds no full text → open copy is HAL hal-02097510.
  - Barnes 2014 / Zhou 2016 / Barnes 2016: "inward" for "inwards"; Zhou's 44.6 % is an average
    speed-up over the fastest earlier variant, not over "the 2014 algorithm" (search record of the
    ScienceDirect abstract; Crossref confirms the metadata); Barnes 2016's "more than a trillion
    cells" quote is not in the abstract → replaced by the abstract's 2 × 10¹² cells, and the tile
    scheme and its correctness tests are now described from the paper's body (ar5iv).
  - Fill–Spill–Merge 2021: three quotes reworded → exact Copernicus text; speed-up added.
  - Génevaux 2013: quote spliced → exact Crossref text; 143:1–143:13 is consistent with Crossref's
    13 pages.
  - Patel 2010 and 2015: the 2010 rivers quote is not on the page and the coastline quote was
    reworded → exact sentences; "7 July 2015" confirmed ("Created 07 Jul 2015"); the vague "later
    posts (2022)" → the 20 April 2022 post, read.
  - Mei 2007: two quotes slightly reworded → exact HAL abstract.
  - Šťava 2008: quotes reworded → exact DCGI abstract, with the 20 fps / 2048×1024 figure and the
    tiling sentence added. Pages: the EG handle gives 201–210, DCGI's page says 30–39; the entry
    keeps 201–210 and adds the EG link.
  - Jákó & Tóth: "Budapest" was the authors' university, not CESCG's venue (Viničné, Slovakia,
    2–4 May 2011); the Eurographics 2011 short-paper version (pp. 57–60, DOI
    10.2312/EG2011/short/057-060) is now matched by the search engine's record of the EG item
    (diglib refused the fetch) and cited; the CESCG programme lists Jákó alone, hgpu.org and the EG
    record list both authors; "Rust port" → the WebGPU port that exists.
  - Houdini HeightField Erode: the node-split sentence and the layer sentences were paraphrases →
    exact text from the node page.
  - Far Cry 5: quotes reworded, and the nightly-regeneration quote is on no primary page → exact
    Vault, PlayStation Blog and 80.lv text; nightly refresh attributed to third-party talk notes.
  - Van Muijden 2017: one quote not on either page, one reworded → exact Vault/Guerrilla text;
    Guerrilla's later GDC years corrected (relic ruins is 2024, not 2022).
  - No Man's Sky 2017: one quote not on the Vault page, one reworded → exact Vault text.
  - Elite: quotes reworded; date → exact 80.lv text, 5 April 2018; "first-release planets" checked
    against the interview's context.
  - Planet Tech v4: two quotes are on the wiki's CitizenCon 2019 panel page, not the v4 page; one
    is from 2016 coverage → both wiki pages cited, the 2016 quote dropped.
  - Flight Simulator: none of the process quotes are on TechCrunch or Bellingcat → exact quotes
    from those two pages, the process figures from Threshold's summary of Microsoft's 6 August 2020
    Blackshark.ai episode (a secondary source), "2 petabytes" dropped for "several petabytes".
  - Outerra: the 90 m / 1 km / centimetre quotes are from the 2012 tech-demo post, not the 2009
    one (which says ~150 m) → both cited; the 2015 quote replaced.
  - Epic landscape pages: section and component sentences reworded; "63×63 … performant" not in
    the guide; the Landscape Splines wording was an older page's → exact 5.8 text; the Recommended
    Landscape Sizes table (up to 8129 × 8129) is now read and quoted; the Nanite-tessellation
    sentence is confirmed in Epic's 5.4 release notes (now linked), not only a community tutorial;
    the "Using Nanite with Landscapes" page (5.8) was read.
  - Unity: quotes came from other pages or none → the Scripting API, terrain-settings, heightmap
    and terrain-layer pages cited with exact text; splat-map figures dropped.
  - Losasso & Hoppe 2004: two quotes spliced → exact hhoppe.com abstract; Crossref confirms 23(3),
    769–776.
  - Dupuy 2020 and Benyoub & Dupuy 2024: spliced quote, elided quote → exact Crossref and arXiv
    text; the 0.2 ms figure `large-worlds.md` uses is in the 2024 abstract.
  - Transvoxel: quote not on transvoxel.org → exact sentence from the page.
  - Bruneton & Neyret 2008: quote spliced → exact Crossref abstract; 27(2), 311–320 confirmed.
  - Deliot & Heitz 2019: the "breaks tiling artifacts by sampling a texture multiple times" quote
    is on neither Heitz's page nor Unity's blog → exact text from both; the blog's URL updated
    (unity.com/blog).
  - Barrett 2008 / van Waveren 2012: the GDC title is "Sparse Virtual Texture Memory"; RAGE's page
    border was misattributed → 4-texel border from the search record of van Waveren's paper, whose
    own copy is now linked.
  - Dawson 2013 and Fiedler 2010: "Yes and no" and Fiedler's three phrases were paraphrases →
    exact text from both posts.
- **Forge's own numbers** (city ground 2001² at 2 m, 8 M triangles, 186 k clusters, 14 levels,
  12 s cook, 0.85 s load, 1.05–1.12 ms frames, 173 locked-edge roots) are from
  `docs/demos/city-blocks.md` as of 2026-09-25.
- **Numbers re-checked on 2026-09-26:** Barnes 2019's 70 s is 120 time steps on 10⁸ cells on a
  node with Tesla P100 GPUs (the paper's 2018 test machine), 43× the serial Braun & Willett code
  and 3× the paper's parallel CPU version; the recommendation's cost paragraph now scales from these
  three figures rather than from the GPU one alone. Epic's limits (256 × 256-vertex sections, 1,024
  components, sizes up to 8129 × 8129) are the 5.8 guide's; Barnes 2016's ~60 % efficiency to 48
  cores, Bangerth's 4.0 s for 1.88 × 10⁹ points on 12,288 processes, Heitz & Neyret's "more than 20
  times faster", Losasso & Hoppe's 40 GB and 100:1, and Benyoub & Dupuy's 0.2 ms are as the
  primary pages give them. Still to replace before a spec: every estimate in the recommendation's
  cost paragraph (per-step seconds, the linear scaling of the cook) by `genesis`'s printed timings;
  the 0.5 km² river threshold is a starting value for the preview, not a finding.
