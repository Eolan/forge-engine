# Research — Terrain genesis: synthesis, hydrology, erosion, rendering, texturing, determinism

> Companion to `large-worlds.md` §8 (the far-field representation, D-014) and to the terrain half of
> `procedural.md` §1, the bibliography carried over from the previous projects. Written 2026-09-25
> for Phase 2's `island` demo: a 16 km island generated from a seed, later a cube-sphere planet.
> Every citation was checked that day against a reachable page or, where the network proxy refused
> the host, against the search engine's record of it; the distinction is kept per entry under
> [Verification notes](#verification-notes), and what could not be found is under
> [Checked and left out](#checked-and-left-out). Entries that `procedural.md` already carries are
> re-verified here rather than repeated at length, and placed where the island pipeline needs them.

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
seconds to a minute at the island's resolution, it is deterministic if the receivers, the queue
order and the transcendental functions are pinned, and its output is a heightfield the existing
cluster-DAG cook already turns into 8 M-triangle ground. What the games add is discipline about
what is baked (everything geological) and what is evaluated at run time (placement and materials
from fields and rules), and what the rendering literature adds since Forge's last note is that the
DAG the city uses is a legitimate terrain LOD, with CDLOD or a concurrent binary tree as the
planet-scale far field when one mesh stops fitting.

> **State of the art in five sentences.** Large terrain is generated as a process, not drawn as
> noise: an uplift field competes with fluvial erosion under the stream-power law, solved
> implicitly and unconditionally stably in O(n) per step by ordering cells from the outlets upstream
> (Braun & Willett 2013), which Cordonnier et al. brought to graphics in 2016 and whose successors
> now run interactively on the GPU (Schott 2023), analytically without time steps (Tzathas 2024),
> or with flow routing in O(log n) parallel iterations (FastFlow 2024). Drainage is made correct by
> construction with priority flood (Barnes 2014, O(n) on integer heights) or a linear-time basin
> graph (Cordonnier, Bovy & Braun 2019), lakes come from the depression hierarchy (Fill–Spill–Merge
> 2021), and rivers are the cells above a catchment threshold, widened by rule. Fine detail is a
> second, local pass — thermal and shallow-water hydraulic erosion in the cellular form of Mei
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
<https://cel.hal.science/LJK_GI_IMAGINE/hal-01262376>)

"The first method in computer graphics that combines uplift and hydraulic erosion to generate
visually plausible terrains": "given a user-painted uplift map, the method generates a stream
graph over the entire domain embedding elevation information and stream flow", solved with the
stream-power equation "at a low computational cost", with "high-level control over the large scale
dendritic structures of the resulting river networks, watersheds, and mountains ridges". The stream
graph is Braun & Willett's structure on an irregular point set with the lake-overflow handling the
grid version lacks.
*Bearing:* the design of Forge's large-scale pass is this paper on a regular grid: the uplift map
is the one authored (or seeded) input, and the river network is a by-product of the solve, never a
post-process. Its timings were not re-read today (the PDF hosts are blocked); `procedural.md`
recorded them as seconds for continental domains.

**Guillaume Cordonnier, Marie-Paule Cani, Bedrich Benes, Jean Braun, Éric Galin. "Sculpting
Mountains: Interactive Terrain Modeling Based on Subsurface Geology." *IEEE Transactions on
Visualization and Computer Graphics* 24(5), 2018, 1756–1769.** [paper] [still-current]
<https://hal.science/hal-01517343> (DOI 10.1109/TVCG.2017.2689022)

Uplift from plates instead of a painted map: "hands-on control on the shape and motion of tectonic
plates using a geologically-inspired model for the Earth crust", which "generates a volumetric
uplift map representing the growth rate of subsurface layers, and jointly simulates erosion and
uplift movement to generate the terrain with stratigraphy that allows rendering of folded strata
on eroded cliffs".
*Bearing:* the uplift generator for the planet variant (plates on a sphere) and the source of a
*hardness* field per layer, which the erosion pass should read so that cliffs and soft valleys
differ; for the island a seeded ridge field is enough.

**Richard Barnes. "Accelerating a fluvial incision and landscape evolution model with
parallelism." *Geomorphology* 330, 2019, 28–39.** [paper] [code] [recent]
<https://arxiv.org/abs/1803.02977> (DOI 10.1016/j.geomorph.2019.01.002; code
<https://github.com/r-barnes/Barnes2019-Landscape>)

Braun & Willett's model reworked for "GPUs, many-core processors, and SIMD instructions": the
implementation "runs 43 x faster (70 s vs. 3,000 s on a 10,000 x 10,000 input) than the previous
state of the art and exhibits sublinear scaling with input size". The repository holds the
variants (basic, with routing, GPU) in C++, Fortran and CUDA with a correctness comparison script.
*Bearing:* the cost reference for the whole-map pass: 10⁸ cells in about a minute on a 2019 GPU,
so the island's 1.7 × 10⁷ cells at 4 m are a CPU job of the same order once basins run in parallel.
The paper's device-parallel stack construction is the recipe if genesis ever moves to compute.

**Guillaume Cordonnier, Benoît Bovy, Jean Braun. "A versatile, linear complexity algorithm for
flow routing in topographies with depressions." *Earth Surface Dynamics* 7(2), 2019, 549–562; with
fastscapelib (GFZ Potsdam, C++/Python, GPL-3.0).** [paper] [code] [recent]
<https://esurf.copernicus.org/articles/7/549/2019/> (DOI 10.5194/esurf-7-549-2019; paper code
<https://github.com/fastscape-lem/flow-routing-depressions>; library
<https://github.com/fastscape-lem/fastscapelib>)

Depressions without filling the whole map: the algorithm computes flow paths "both within and
across depressions through the construction of a graph connecting all adjacent drainage basins",
in linear time, and "allows users to choose among different strategies of flow path enforcement
within the depressions, such as filling versus carving". The paper repository is archived
read-only; the maintained form is fastscapelib, "a C++/Python library of efficient and reusable
algorithms for landscape evolution modeling", supported by GFZ's Earth Surface Process Modelling
group.
*Bearing:* the depression step inside the erosion loop, where priority flood every iteration would
dominate: a basin graph once per step, carved so water leaves closed basins the way the real
landscape's would. fastscapelib is GPL, so it is a reference to read, not a dependency.

**Hugo Schott, Axel Paris, Lucie Fournier, Éric Guérin, Éric Galin. "Large-scale Terrain Authoring
through Interactive Erosion Simulation." *ACM Transactions on Graphics* 42(5), Article 162, 2023.**
[paper] [recent]
<https://dl.acm.org/doi/10.1145/3592787> (DOI 10.1145/3592787)

The GPU version fast enough to edit against: it "bridges the gap between large-scale erosion
simulation and authoring into an efficient framework", motivated by "the need for authoring
techniques offering hydrological consistency without sacrificing user control". Stream-power
erosion, thermal erosion, hillslope diffusion and deposition run interactively while uplift and
material maps are painted.
*Bearing:* the authoring mode for a later terrain editor and the paper to port the discretisation
from; for the seeded island the same operators run once on the CPU.

**Hugo Schott, Éric Galin, Éric Guérin, Adrien Peytavie, Axel Paris. "Terrain Amplification using
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

A method "both physically-based and procedural": "contrary to simulation-based approaches, the
algorithm does not rely on a time-stepping scheme but uses the analytical solutions of the stream
power law", giving "generation and control of consistent large-scale mountain ranges" without the
thousands of iterations a simulation needs.
*Bearing:* the candidate replacement for the iterative loop if its cost ever matters: a steady-state
profile evaluated directly along the drainage tree. Worth a spike after the iterative version
exists, since it removes the one stage of the pipeline that is hard to bound in time.

**Aryamaan Jain, Bernhard Kerbl, James Gain, Brandon Finley, Guillaume Cordonnier. "FastFlow: GPU
Acceleration of Flow and Depression Routing for Landscape Simulation." *Computer Graphics Forum*
43(7) (Pacific Graphics 2024), e15243; with Jain, Benes, Cordonnier, "Efficient Debris-flow
Simulation for Steep Terrain Erosion", *ACM Transactions on Graphics* 43(4), 2024.** [paper]
[recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.15243> · <https://dl.acm.org/doi/10.1145/3658213>

FastFlow "computes the water discharge in O(log n) iterations for a terrain with n vertices
(assuming n processors)" and routes water out of depressions in "O(log² n) iterations", replacing
the sequential stack for GPU pipelines. The debris-flow paper adds "a new formulation and GPU
algorithm for the unified modeling of fluvial and debris flow erosion", the steep-slope process
that carves gullies and fans the stream-power law alone does not produce.
*Bearing:* the GPU path when genesis moves to compute (authoring, or planets amplified at
stream-in), and the reason not to design the CPU version around the sequential stack forever;
debris flow is a later operator for the volcanic and alpine islands.

**Guillaume Cordonnier, Guillaume Jouvet, Adrien Peytavie, Jean Braun, Marie-Paule Cani, Bedrich
Benes, Éric Galin, Éric Guérin, James Gain. "Forming Terrains by Glacial Erosion." *ACM
Transactions on Graphics* 42(4), 2023.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3592422> (DOI 10.1145/3592422)

Glaciers as a second erosive agent: "a learning-based estimation of ice flow, a multi-scale
advection scheme, and finer-scale erosive phenomena to model the formation of glacial features such
as U-shaped and hanging valleys, fjords, and glacial lakes" over several glacial cycles.
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

The guarantee and its speed-ups. Priority-flood floods the DEM "inward from their edges using a
priority queue" and is "optimal for both integer and floating-point data, working in O(n) and
O(n lg n) time, respectively". Zhou et al. process the cells outside depressions and flats with
"two plain queues", so the priority queue sees only the rest, "44.6% faster" than the 2014
algorithm. Barnes 2016 subdivides the DEM into tiles, fills each, and reconciles them through a
graph of tile edges, "able to efficiently fill depressions in DEMs with more than a trillion cells",
with "∼60% strong and weak scaling efficiencies up to 48 cores" and code in RichDEM.
*Bearing:* two uses in Forge. On the finished field, the fill that makes every cell drain
(ε-gradient variant, seeded from the coast rather than the grid edge, since the island's outlet is
the sea). And the proof that filling *tiles*: the 2016 tile graph is exact, so a planet's faces can
be filled per tile and reconciled, which is the answer to "what needs the whole map" for this step
(only the tile-edge graph does).

**Richard Barnes, Kerry L. Callaghan, Andrew D. Wickert. "Computing water flow through complex
landscapes – Part 3: Fill–Spill–Merge: flow routing in depression hierarchies." *Earth Surface
Dynamics* 9, 2021, 105–121.** [paper] [recent]
<https://esurf.copernicus.org/articles/9/105/2021/> (DOI 10.5194/esurf-9-105-2021)

Lakes instead of flattened bowls: depressions "can take the form of lakes and wetlands", and
"traditional models that eliminate depressions through filling or breaching can produce
unrealistic results"; Fill–Spill–Merge "utilizes a depression hierarchy data structure to rapidly
process and distribute runoff", filling each depression to the volume its catchment's runoff
supplies, spilling the excess to the neighbour and merging depressions that overflow into one.
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
drainage network represented as a geometric graph over a given input domain, which is then
analyzed to construct watersheds and characterize the different types and trajectories of rivers",
with the terrain synthesised afterwards to fit the network from "a simple initial sketch" and "a
few parameters".
*Bearing:* not Forge's primary path (erosion gives the network for free) but the vocabulary for the
river post-process — river types by slope and order, watershed labelling, the carving of a bed
profile per reach — and the fallback for designer-drawn rivers that the erosion must then respect.
Peytavie et al. 2019 (riverscapes) and Paris et al. 2023 (meanders) carry this on; both are in
`procedural.md` §1.

**Amit Patel. "Polygonal Map Generation for Games." Red Blob Games, September 2010; and "Making
maps with noise functions", Red Blob Games, 7 July 2015.** [web] [foundational] [still-current]
<http://www-cs-students.stanford.edu/~amitp/game-programming/polygon-map-generation/> ·
<https://www.redblobgames.com/maps/terrain-from-noise/>

The island-shape and coastline reference everyone starts from. The 2010 generator works on a
Voronoi graph rather than noise, "which led to putting rivers at the beginning of the generation
process"; "the coastline being all edges where land and water meet", with elevation from distance
to the coast and moisture from the rivers, then biomes. The 2015 article gives the island mask as
a distance function ("d = 1 − (1−nx²) · (1−ny²) for square maps") and a shaping function that
"outputs land" at the centre, "water" at the border and "both land and water" between, applied to
redistributed octave noise.
*Bearing:* stage one of the island pipeline, almost verbatim: a distance-to-centre mask warped by
low-frequency noise sets where the sea is; the uplift field is then drawn inside it. Patel's later
posts (2022) on improving island shaping show the mask is still where most of the "does it look
like an island" quality comes from.

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

The "virtual pipes" model: "the velocity field of running water created with an efficient
shallow-water fluid model" drives "the erosion and deposition process and the sediment
transportation process", "designed to be implemented totally on GPU". Four fluxes per cell, a
velocity, a sediment capacity from velocity and slope, semi-Lagrangian sediment advection.
*Bearing:* the implementable hydraulic detail pass, cellular and deterministic in a fixed
iteration order (§7), the same code on the CPU for the server and in a compute shader for
authoring previews.

**Ondřej Šťava, Bedřich Beneš, Matthew Brisbin, Jaroslav Křivánek. "Interactive Terrain Modeling
Using Hydraulic Erosion." *ACM SIGGRAPH/Eurographics Symposium on Computer Animation* 2008
(Dublin), 201–210.** [paper] [foundational]
<https://dl.acm.org/doi/10.5555/1632592.1632622> (open copy
<https://dcgi.fel.cvut.cz/en/publications/2008/stava-sca-erosion/>)

The pipe model on "a terrain composed of layers of materials", coupling "two hydraulic erosion
algorithms for running water, where areas with slow motion become more eroded by dissolution
erosion while areas with faster motion experience force-based erosion", with "slippage effects
where river banks fall into the water".
*Bearing:* the layer stack (rock, regolith, sand) that makes erosion output read as geology rather
than melted wax, and the bank-slip term rivers need; the layers are also what the material rules in
§6 read.

**Balázs Jákó, Balázs Tóth. "Fast Hydraulic and Thermal Erosion on the GPU." CESCG 2011 (Central
European Seminar on Computer Graphics), Budapest; also a Eurographics 2011 short paper.** [paper]
[still-current]
<https://old.cescg.org/CESCG-2011/papers/TUBudapest-Jako-Balazs.pdf>

Both operators in one GPU pass over "a predefined height field terrain", "designed to be executed
interactively on parallel architectures like graphics processors": the pipes model for water,
thermal erosion for talus, in a form short enough that it has WebGPU and Rust ports.
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
"an implementation of Hans Theobald Beyer's algorithm"); Lague's Unity project shows the look after
"70,000 erosion iterations"; McDonald's C++ system "extends simple particle based hydraulic
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
specific scale", split into the individual nodes "HF Erode Thermal, HF Erode Hydro, and HF
Precipitation", and writes "several output layers", among them "eroded height, sediment, debris,
flow and flowdir" — "the sediment layer contains the amount of deposited sediment from hydro
erosion", "the debris layer contains the amount of deposited debris from thermal erosion".
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
<https://www.gdcvault.com/play/1025557/Procedural-World-Generation-of-Far> (write-up:
<https://80.lv/articles/houdini-procedural-world-generation-of-far-cry-5>)

The pipeline "developed for Far Cry 5 using Houdini and Houdini Engine", with tools "to generate
biomes, texture the terrain, setup freshwater networks, generate cliff rocks and more"; the
problem was "filling up 100 square km of wilderness with a terrain changing every day", and the
team "regenerated the entire game world every night on special build machines".
*Bearing:* the offline model at AAA scale: the terrain is authored, the derived layers (biomes,
water networks, cliffs, roads, fences) are regenerated by graph whenever it changes. Forge's
equivalent is a content-hashed cache per stage so that changing the uplift re-runs only what
depends on it.

**Jaap van Muijden (Guerrilla). "GPU-Based Run-Time Procedural Placement in 'Horizon: Zero Dawn'."
GDC 2017; with Guerrilla's GDC 2022–2024 session lists.** [talk] [web] [still-current]
<https://gdcvault.com/play/1024700/GPU-Based-Run-Time-Procedural> ·
<https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn> ·
<https://www.guerrilla-games.com/read/guerrilla-at-gdc-2022>

The pipeline "from the graph editor where artists can define procedural placement rules to the GPU
algorithms that create a dense world around the player on the fly", after the team "moved from the
traditional CPU-based placement system to real-time". Guerrilla's later GDC pages list deferred
texturing for foliage (McLaren, 2022), the volumetric superstorms and the relic ruins of Forbidden
West, but no terrain-generation talk: the terrain stays authored, the placement stays a GPU rule
evaluation.
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

"A step-by-step breakdown of the generation pipeline, from voxel-based world generation, through
polygonization and texturing, to eventual population and simulation", all "continuously in
real-time", with the stated aim to "enable our artists to produce more, rather than replacing them
with an algorithm".
*Bearing:* the pure run-time end of the spectrum: voxel noise polygonised around the player. Its
planets have no drainage because nothing global was ever computed; Forge's planet variant keeps a
coarse global genesis precisely to avoid that.

**Doc Ross (Frontier Developments), interviewed by 80.lv. "Generating the Universe in Elite:
Dangerous." 80.lv, 2018.** [web] [still-current]
<https://80.lv/articles/generating-the-universe-in-elite-dangerous>

Landable planets "start as a cube with square sub-dividing faces that behave as quadtrees";
"planet-specific average properties are packed into a buffer sent to the GPU, where they modulate
noise functions combined to form geological shapes"; the first-release planets were "perfect
spheres with height difference provided by normal mapping".
*Bearing:* the cube-sphere quadtree with GPU noise per tile is the run-time half of Forge's planet
plan; Elite shows what it looks like without a baked global pass (no rivers, no basins), which is
what the coarse genesis adds.

**Cloud Imperium Games. "Planet Tech v4." Star Citizen Wiki, citing CIG's Alpha 3.8 material and
CitizenCon.** [web] [recent]
<https://starcitizen.tools/Planet_Tech_v4>

Delivered "in stages starting with Alpha 3.8", it added "terrain texture blending, objects
scattering and biome transitions" over v2/v3, "terrain simulation and displacement of sand and
soil based on wind", and replaced "pre-baked color textures with two layers of climate data":
"temperature and humidity maps infer biome selection". The CitizenCon slide reported a terrain
system that "seamlessly supports spherical terrain at a planetary scale".
*Bearing:* the closest shipped analogue to `planet-environment.md`'s climate-first materials:
two climate fields drive biome and material selection at run time. Secondary source; pull the
comm-links before any number enters a spec.

**Microsoft / Asobo Studio with Blackshark.ai. Microsoft Flight Simulator (2020): Bing Maps data
and machine-learned reconstruction.** [web] [recent]
<https://techcrunch.com/2020/08/17/meet-the-startup-that-helped-microsoft-build-the-world-of-flight-simulator>
· <https://www.bellingcat.com/resources/case-studies/2020/08/24/cleared-for-takeoff-exploring-microsoft-flight-simulator-2020s-research-potential/>

The contrast case: the world is "2 petabytes of data from Bing Maps"; Blackshark.ai's "deep
learning neural network segments and classifies buildings, vegetation, and roads globally" and
reconstructs "over 1.5 billion building footprints", processing "the entire planet" on "hundreds
of virtual machines" in "72 hours".
*Bearing:* photogrammetry plus ML is the right answer when the planet exists; for a seeded world it
sets the bar for coherence (roads meet rivers at bridges, vegetation follows terrain) that the rule
layers must reach without data.

**Brano Kemen et al. Outerra: planet engine blog, 2008–2015.** [web] [still-current]
<https://outerra.blogspot.com/2009/02/procedural-terrain-algorithm.html> ·
<https://outerra.blogspot.com/2015/05/evaluation-of-30m-elevation-data-in.html>

A real-scale Earth "created from real elevation data with resolution 90m where available, 1km
resolution for oceans", "further refined by fractal-based procedural techniques down to
centimeter-level details"; the 2015 post evaluates 30 m data and notes the newer dataset "comes with
much better erosion shapes".
*Bearing:* the two-scale pattern Forge's planet uses — a coarse global field (here generated
rather than measured) and deterministic fractal refinement per tile — and a reminder that the
refinement must respect the coarse drainage (Schott 2024) or the added detail contradicts the
valleys.

**Epic Games. "Landscape Technical Guide", "Using Nanite with Landscapes", "Landscape Splines",
Unreal Engine 5.x documentation; and the 5.3 release notes.** [docs] [still-current] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-technical-guide-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/using-nanite-with-landscapes-in-unreal-engine>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/landscape-splines-in-unreal-engine>
· <https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5.3-release-notes?application_version=5.3>

A landscape is "divided into multiple Landscape Components", "the same size and always square",
each of sections whose vertex count "must be a power of two (with a maximum of 256x256) so that
the different LOD levels can be stored in mipmaps"; "63×63 quads is a good, performant choice for
section size" and Epic recommends "no more than 1,024 Landscape components for the largest
Landscapes" (so the largest recommended landscape is a few thousand vertices a side; the exact
table was not re-read, §11). Since 5.3 "Nanite can now be enabled in landscape actors, at parity
with normal landscape rendering", with the meshes "rebuilt in the background"; Nanite tessellation
(5.4, experimental) displaces at run time by tessellating "the mesh at runtime into additional
triangles to conform to the detail of the displacement map". Landscape splines create "any linear
feature that needs to conform to a Landscape, and can even push and pull the terrain".
*Bearing:* Epic's own move of the landscape onto its cluster DAG is the strongest endorsement of
what Forge's city already does; its component/section limits are what a 16 km island at 2 m
(8193²) exceeds, which is why Forge tiles the cook rather than the world. Splines that deform the
field and carry meshes are the road/river authoring model.

**Unity Technologies. "Terrain", "Working with Heightmaps", "Terrain Layers". Unity Manual.**
[docs] [still-current]
<https://docs.unity3d.com/Manual/terrain-Heightmaps.html> ·
<https://docs.unity3d.com/6000.4/Documentation/Manual/class-TerrainLayer.html>

`TerrainData` "stores heightmaps, detail mesh positions, tree instances, and terrain texture alpha
maps"; a heightmap side is a power of two plus one ("512 + 1 = 513"), imported and exported as RAW,
"a 16-bit grayscale format"; "each Splatmap is capable of containing 4 textures", a fifth adds "a
second splatmap" and "1 additional shader pass"; a 1024 px control texture on the example terrain
puts "each pixel of the splatmap" at "0.5 x 0.5m".
*Bearing:* the baseline any terrain system is measured against, and its limits are instructive:
four-layer splat maps and per-tile terrains are what procedural materials (§6) and a single
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
video memory and incrementally refilled as the viewpoint moves", giving "visual continuity,
uniform frame rate, complexity throttling, and graceful degradation"; the dataset was "a 40GB
height map of the United States, with a compressed image pyramid reducing the size by a factor of
100 so it fits entirely in memory".
*Bearing:* the structure for the ocean and for a flat island's far field if the DAG ever costs too
much; the compression figure (100:1 on a residual pyramid) is the reference for shipping a planet's
coarse height data.

**Thatcher Ulrich. "Rendering Massive Terrains using Chunked Level of Detail Control." SIGGRAPH
2002 course notes.** [talk] [foundational]
<http://vterrain.org/LOD/Papers/> (index; the notes are linked from tulrich.com)

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

"A binary heap (a 1D array) that explicitly stores the sum-reduction tree of a bitfield, where each
one-valued bit represents a leaf node of the binary tree", used "to accelerate a
longest-edge-bisection-based algorithm that computes and renders adaptive geometry for
large-scale terrains entirely on the GPU". The 2024 paper takes bisection "to arbitrary polygon
meshes rather than just squares" by "mapping a triangular subdivision primitive to each halfedge";
libleb provides the algorithms "on multicore processors, including GPUs" in C, GLSL and HLSL,
released to the public domain.
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
functions" and octree simplification that "requires no crack patching". Transvoxel "solves crack
problems by inserting special transition cells at the boundary between high-resolution and
low-resolution voxel data": "512 cases → 73 equivalence classes".
*Bearing:* unchanged from D-014: the near-field mesher for caves, overhangs and edits, with the
heightfield converted to a signed field only inside edited or feature bricks; the seam between the
brick and the DAG ground is a Transvoxel transition, not a stitch.

**Éric Bruneton, Fabrice Neyret. "Real-Time Rendering and Editing of Vector-based Terrains."
*Computer Graphics Forum* 27(2) (Eurographics 2008).** [paper] [still-current]
<https://maverick.inria.fr/Publications/2008/BN08/> (DOI 10.1111/j.1467-8659.2008.01128.x)

"Very large terrains with very detailed features such as roads, rivers, lakes and fields" from
"vector descriptions of linear and areal features, with associated shaders to specify their
appearance (terrain color and material), their footprint (effect on terrain shape), and their
associated objects (bridges, hedges, etc.)", refined in "a view dependent quadtree refinement
scheme, with new quads generated when needed and cached on the GPU".
*Bearing:* rivers and roads as vectors with a footprint and an appearance, rasterised into the
height and the material of the tiles that need them: the model for Forge's river polylines and
lake polygons from §2, and the origin of the "decal into a runtime virtual texture" pattern in §6.

---

## 6. Texturing and materials at terrain scale

The owner sees repeating textures at once. The literature's answer has three parts: materials from
fields and rules instead of painted splat maps, a virtual texture so the evaluated result is cached
per page, and a stochastic tiling operator so that no tile is ever visibly reused.

**Johan Andersson (DICE). "Terrain Rendering in Frostbite Using Procedural Shader Splatting."
SIGGRAPH 2007 course *Advanced Real-Time Rendering in 3D Graphics and Games*.** [talk]
[foundational] [still-current]
<https://www.ea.com/frostbite/news/terrain-rendering-in-frostbite-using-procedural-shader-splatting>
· <https://dl.acm.org/doi/10.1145/1281500.1281668>

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
<https://blog.unity.com/engine-platform/procedural-stochastic-texturing-in-unity> ·
<https://jcgt.org/published/0008/04/02/>

"Takes as input a small example of a stochastic texture and synthesizes an infinite output with
the same appearance", for "natural textures such as moss, granite, sand, and bark", "more than 20
times faster" than procedural noise of comparable quality; the insight is that "with Gaussian
inputs, histogram-preserving blending boils down to mean and variance preservation". Deliot &
Heitz make it a shader that "breaks tiling artifacts by sampling a texture multiple times from
different regions" and blending with the variance-preserving operator; Burley fixes the ghosting by
"exponentiating the blending weights".
*Bearing:* the direct fix for the owner's first complaint. Every tileable in Forge's `textures`
module gets a Gaussian-transformed twin and an inverse LUT, and terrain layers are sampled through
the tiling-and-blending operator; cost is three samples instead of one, paid only in the terrain
classes.

**Sean Barrett. "Sparse Virtual Textures." GDC 2008; J.M.P. van Waveren, "Software Virtual
Textures", in *Virtual Texturing in Software and Hardware* (Obert, van Waveren, Sellers), SIGGRAPH
2012 course; and Epic Games, "Runtime Virtual Texturing", Unreal Engine documentation.** [talk]
[paper] [docs] [foundational] [still-current]
<https://gdcvault.com/play/417/Sparse-Virtual-Texture> · <http://silverspaceship.com/src/svt/> ·
<https://dl.acm.org/doi/10.1145/2343483.2343488> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/runtime-virtual-texturing-in-unreal-engine>

Barrett's talk (source in the public domain) is the open form of virtual texturing; RAGE used "a
single 120k x 120k virtual texture for all static geometry" with 128 × 128 tiles ("126 + 1 pixel
border on all sides") and a software page table. Epic's runtime variant "creates its texel data on
demand using the GPU at runtime", "caches shading data over large areas", and is "a good fit for
Landscape shading that uses decal-like materials and splines".
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
<https://dl.acm.org/doi/10.1145/566654.566636> · <https://jcgt.org/published/0009/03/02/>

Perlin's corrected noise replaces the interpolant by "6t⁵−15t⁴+10t³" and fixes the gradient set;
its lattice hashing is a permutation of integers, which is why gradient noise is deterministic when
the lattice coordinate is an integer. Jarzynski & Olano evaluate GPU hashes "for random number
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

Is IEEE arithmetic deterministic? "Yes and no": the basic operations are, given the same
instruction sequence and rounding mode; what differs is compilers (contraction into FMA,
reassociation), libraries (`sin`, `exp`, `pow` differ between platforms and versions) and
instruction sets (Fiedler's "SSE and x87", "debug and release builds", "AMD and Intel").
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
at a few nanoseconds a cell that is 0.2–0.4 s per step single-threaded, so 300 steps are 1–2 min on
one core and well under a minute across the 9800X3D's basins (Barnes 2019's 70 s for 10⁸ cells on a
2019 GPU is the scale reference; Braun & Willett's ordering is "ideally suited to
parallelization"). Priority flood on integer heights is O(n): a second at 4 m. Stage 5 is 16 tiles
of 4.5 M cells with halo, a few dozen iterations each: tens of seconds in parallel. Stage 6 is one
pass. Target: the whole genesis under 60 s cold on the dev PC, milliseconds warm from the cache;
the cook adds its minutes once. Tzathas 2024's analytical solution is the spike to run if stage 3
dominates.

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
sections at 256 × 256 vertices, recommends 63 × 63 quads and at most 1,024 components, Unity's
heightmaps are a power of two plus one with a 16-bit RAW interchange, Far Cry 5 covered 100 km²
and regenerated its derived layers nightly; Outerra refines 90 m (later 30 m) real data with
fractal detail to centimetres, and Flight Simulator's world is 2 PB of Bing imagery reconstructed
in 72 hours on hundreds of machines. Forge's island at 4 m is 4097² (16.8 M cells, 67 MB per `f32`
field) and at 2 m 8193² (67 M cells, 268 MB), against the city's 2001² today. Generation times from
the geomorphology side are the useful ones: the implicit solver is linear per step and
unconditionally stable (Braun & Willett 2013); Barnes's parallel version does a 10⁸-cell step
sequence in 70 s on a 2019 GPU, 43× the serial code; priority flood is O(n) on integer heights,
Zhou's variant 44.6 % faster than the 2014 algorithm, and the tiled version keeps ~60 % efficiency
to 48 cores on trillion-cell DEMs; FastFlow routes flow in O(log n) parallel iterations and
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
- **A Horizon Forbidden West terrain or world-tooling talk** — Guerrilla's GDC 2022–2024 pages list
  deferred texturing for foliage (McLaren), the Nubis superstorms, the relic ruins and living-world
  design; nothing on terrain generation. Van Muijden's 2017 placement talk stands for Guerrilla.
- **Jákó & Tóth as a Eurographics 2011 short paper** — only the CESCG 2011 PDF was confirmed; the
  Eurographics DOI (10.2312/EG2011/short/057-060, from memory) was not verified, so the entry cites
  the CESCG version.
- **Cordonnier et al. 2016's timings and grid sizes** — the PDF hosts (Purdue, HAL, CORE) are
  blocked by the proxy; the abstract's "low computational cost" is quoted and `procedural.md`'s
  "seconds" is attributed to the earlier session.
- **Beyer's thesis itself** — no reachable copy; described through erodr's README and the search
  record. Lague's repository cites firespark.de and ranmantaru.com rather than Beyer, which is
  stated.
- **Epic's "Recommended Landscape Sizes" table** (8129², 4033², 2017², 1009² …) — remembered, not
  found in any reachable extract of the technical guide; only the component and section rules are
  quoted, and the "2^n + 1 with n from 5 to 12" formulation comes from a third-party guide.
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
- **GPU Gems 2 ch. 2 (Asirvatham & Hoppe)** — confirmed only as a listing on hhoppe.com in a search
  result; folded into the Losasso & Hoppe entry.
- **Epic's "Using Nanite with Landscapes" page** — URL confirmed by search, content not read; the
  5.3 wording comes from the release-notes extract.
- **Sebastian Lague's erosion video** — a video, not citation grade; the repository is cited.

---

## Verification notes

Checked on 2026-09-25 with WebSearch and WebFetch only; no browser pane and no YouTube pages. The
session's egress proxy refused every host tried except `github.com`: doi.org, arxiv.org,
hal.science and inria.hal.science, cs.purdue.edu, api.semanticscholar.org, sciencedirect.com,
esurf.copernicus.org, the Stanford and redblobgames.com pages, readthedocs.io, hhoppe.com,
dev.epicgames.com, docs.unity3d.com, gdcvault.com, onrendering.com, history.siggraph.org,
en.wikipedia.org, ea.com, fastscape.org, www-sop.inria.fr, team.inria.fr and files.core.ac.uk all
returned "blocked by the network egress proxy". Verification therefore has two grades.

- **Fetched and read (GitHub):** fastscapelib, flow-routing-depressions, MultiScaleErosion,
  RichDEM, Barnes2019-Landscape, CDLOD, libleb, LongestEdgeBisection and its demos (README not
  served), SimpleHydrology, Hydraulic-Erosion (Lague), rust-lang/libm. Quotes from these are
  verbatim from their READMEs.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  volume, pages, DOI, and the sentences quoted, which are the search engine's extracts of the page
  named): Whipple & Tucker 1999 (AGU/Wiley); Braun & Willett 2013 (ScienceDirect, GFZ, the
  geodynamics.org highlight for the FastScape languages); Cordonnier 2016 (Wiley, HAL, EG diglib);
  Sculpting Mountains 2018 (PubMed, HAL, Purdue); Barnes 2019 (OSTI, arXiv); Cordonnier, Bovy &
  Braun 2019 (Copernicus, HAL); Schott 2023 and 2024 (ACM DL); Tzathas 2024 (Wiley, EG diglib,
  HAL); FastFlow 2024 and the debris-flow paper (Wiley, ACM DL, INRIA); glacial erosion 2023 (ACM
  DL, SIGGRAPH history); Bangerth 2026 (arXiv listing); Galin 2019 (Wiley, UCT, NSF PAR);
  O'Callaghan & Mark 1984 (ScienceDirect); Tarboton 1997 (AGU); Barnes 2014 (dblp, arXiv,
  Minnesota); Zhou 2016 (ACM DL, the cageo GitHub mirror); Barnes 2016 (OSTI, arXiv); Fill–Spill–
  Merge 2021 (Copernicus, OSTI); Génevaux 2013 (ACM DL, SIGGRAPH history); Patel 2010 and 2015
  (the Stanford page, simblob posts, redblobgames.com with the 7 July 2015 date); Musgrave 1989
  (ACM DL, SIGGRAPH history); Olsen 2004 (the MIT-hosted PDF listing); Mei 2007 (HAL, ACM DL);
  Šťava 2008 (ACM DL, DCGI); Jákó & Tóth (CESCG PDF listing); Beyer 2015 (erodr, a thesis page);
  Houdini HeightField Erode (sidefx.com); Far Cry 5 (GDC Vault 1025557, 80.lv, PlayStation Blog);
  van Muijden 2017 (GDC Vault 1024700, guerrilla-games.com); Guerrilla GDC 2022–2024 pages;
  Tsushima 2021 (GDC Vault 1027352, the streaming PDF on media.gdcvault.com); No Man's Sky 2017
  (GDC Vault 1024265); Elite/80.lv; Planet Tech v4 (starcitizen.tools, PC Gamer); Flight Simulator
  (TechCrunch, bellingcat); Outerra (blog posts of 2009, 2012, 2015); Epic's Landscape technical
  guide, Nanite Landscape page, 5.3 release notes and Landscape Splines page; Unity's heightmap and
  terrain-layer pages; Losasso & Hoppe 2004 (ACM DL, hhoppe.com listing); Ulrich 2002 (vterrain.org
  listing and course-note citations); Dupuy 2020 (ACM DL, EG diglib, Kesen's HPG 2020 list);
  Benyoub & Dupuy 2024 (ACM DL, arXiv, Intel); Karis 2021 (the Advances 2021 PDF listing); Zucker &
  Higashi 2018 (jcgt.org); Ju 2002 (ACM DL, SIGGRAPH history); Transvoxel (transvoxel.org,
  Lengyel's dissertation PDF listing); Bruneton & Neyret 2008 (Maverick/INRIA, EG diglib, Wiley);
  Andersson 2007 (ea.com, ACM DL, the Advances 2007 PDF); Geiss 2007 (NVIDIA GPU Gems 3); Golus
  2017 (Medium, GitHub); Heitz & Neyret 2018 (ACM DL, HPG best-paper page, HAL); Deliot & Heitz
  2019 (Unity blog, Heitz's page); Burley 2019 (JCGT via Semantic Scholar and dblp listings);
  Barrett 2008 (GDC Vault 417, silverspaceship.com, archive.org); van Waveren 2012 (ACM DL, the
  SIGGRAPH 2012 session page); UE Runtime Virtual Texturing (dev.epicgames.com listing); Perlin
  2002 (ACM DL, SIGGRAPH history); Jarzynski & Olano 2020 (jcgt.org, UMBC); Dawson 2013 and Fiedler
  2010 (their blogs' listings with dates).
- **Weaker confirmations, stated plainly.** Jákó & Tóth's Eurographics venue and DOI are from
  memory. The Sculpting Mountains page range (1756–1769) is from PubMed's record. Génevaux 2013's
  page count differs between records (10 pages on ACM's, 143:1–143:13 in `procedural.md`); the
  entry cites the article number only. Bruneton & Neyret's CGF volume (27(2)) is inferred from
  Eurographics 2008; the DOI is confirmed. The Nanite-tessellation wording ("tessellates the mesh at
  runtime into additional triangles…") is from an Epic community tutorial, not the reference page.
  The Flight Simulator figures (2 PB, 1.5 billion buildings, 72 hours) come from press coverage of
  Blackshark.ai, not from Microsoft or Asobo. The Star Citizen entry rests on a fan wiki. Ulrich
  2002's text was confirmed through citations of it (a course syllabus and later papers), not the
  notes themselves. Olsen 2004 is a departmental report, not peer-reviewed. Barnes 2016's exact
  tile-edge reconciliation scheme is described from the abstract ("subdividing a DEM into tiles")
  and the RichDEM implementation's existence, not from the paper's body.
- **Forge's own numbers** (city ground 2001² at 2 m, 8 M triangles, 186 k clusters, 14 levels,
  12 s cook, 0.85 s load, 1.05–1.12 ms frames, 173 locked-edge roots) are from
  `docs/demos/city-blocks.md` as of 2026-09-25.
- **Numbers to re-check before they enter a spec:** every estimate in the recommendation's cost
  paragraph (per-step seconds, the linear scaling of the cook) is an estimate, marked as such, to
  be replaced by `genesis`'s printed timings; Barnes 2019's 70 s is on 2019 hardware; Epic's
  component limits move between 5.x releases; the 0.5 km² river threshold is a starting value for
  the preview, not a finding.
