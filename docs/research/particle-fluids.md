# Research — Real-time particle liquids: solvers, GPU building blocks, rendering, Jolt coupling

The owner's request of 2026-10-03 (issue #155, for the glass tanks of #156): a particle liquid in
real time for the physics lab, seen through glass walls and floor with its volume drawn, the
camera crossing its surface, and whatever helps the island's streams and rivers. This goes deeper
and later than [physics-fluids.md](physics-fluids.md) §5, which surveyed SPH, PBF, FLIP/APIC and
MLS-MPM in a few paragraphs. A research agent in the cloud wrote it; the decision it proposes is
in `docs/DECISIONS.md` (🟡, the owner's to take).

*Research note, 2026-10-03. Sources were read with WebSearch and WebFetch only. A **†** marks a claim I could confirm only through a search-result snippet, not by reading the source. **(est.)** marks my own estimate, which still has to be measured.*

**Summary and recommendation.**

**Glass-tank lab: solver.** Build a hybrid particle–grid liquid:
- APIC transfers with quadratic B-splines, the same transfers MLS-MPM uses;
- a pressure projection on a dense grid that covers the lab;
- a particle-density (volume) correction, so the water level and the volume stay right;
- written as Vulkan compute in Slang;
- particle-to-grid transfers done with fixed-point integer atomics. This avoids float atomics, which matters on AMD, and makes the solver deterministic on one GPU.

Keep EA SEED's Position-Based MPM (BSD-3, WebGPU reference) as the alternative pressure model, behind the same data structures. Use it if the projection does not fit the budget.

**Tank rendering.** Ray-march the solver's grid density, as Unreal's Niagara Fluids does for its 3D FLIP, and treat the glass walls as analytic planes. This one model gives:
- refraction through glass and water;
- absorption by the volume under the surface;
- total internal reflection;
- the camera inside the water, and the half-under view with a meniscus line.

Spray, foam and bubbles are diffuse particles in the style of Ihmsen et al. (2012).

**Jolt coupling.**
- First: moving bodies are written into the grid as solid velocities. Back from the GPU come submerged volume, centre of buoyancy and local fluid velocity, which feed Jolt's volume-based `ApplyBuoyancyImpulse`.
- Second: full momentum exchange.

**Determinism.** The GPU liquid stays visual and local to the lab. It is deterministic on one GPU for replays and is never part of network state. Gameplay water stays on the CPU.

**Island.** Keep the heightfield authoritative. Add GPU ballistic particles where it fails (Chentanez & Müller 2010) and return them to it when they land. The same diffuse-particle renderer serves both the island and the tank.

**First milestone.**
- One 1.0 × 0.5 × 0.6 m tank, about 190k particles on a 1.5 cm grid, 4 substeps per 60 Hz frame.
- Budget target on the RTX 5070 Ti: ≤ 2.5 ms simulation + ≤ 1.5 ms rendering (est.).
- A second tank variant with a gate that has a hole in it, checked against Torricelli's law.

---

## 1. The owner's links

These four PDFs are exactly the simulation references listed in Sebastian Lague's Fluid-Sim README (https://github.com/SebLague/Fluid-Sim, opened).

**M. Müller, D. Charypar, M. Gross. "Particle-Based Fluid Simulation for Interactive Applications." ACM SIGGRAPH/Eurographics Symposium on Computer Animation (SCA), 2003.**
URL: https://matthias-research.github.io/pages/publications/sca03.pdf — [paper]
- **Access:** reachable, but the fetch tool returned the raw PDF and could not extract its text.
- **Method:** the founding "interactive SPH" paper. Force densities come straight from Navier–Stokes, plus a surface-tension term. The free surface is shown with point splatting or marching cubes.
- **Reported numbers:** 2,200 particles at 20 fps on a 1.8 GHz Pentium 4; 3,000 particles with a marching-cubes surface at 5 fps; interaction with up to about 5,000 particles†.
- The method is explicit and weakly compressible, so it needs small steps.

*Bearing:* this is history, not a candidate. Its kernels (poly6, spiky, viscosity) are still what most hobby GPU SPH codes use, including Lague's.

**S. Clavet, P. Beaudoin, P. Poulin. "Particle-based Viscoelastic Fluid Simulation." SCA 2005, pp. 219–228.**
URL: https://web.archive.org/web/20250106201614/http://www.ligum.umontreal.ca/Clavet-2005-PVFS/pvfs.pdf — [paper]
- **Access: unreachable.** The fetch tool refuses web.archive.org. The original ligum.umontreal.ca URL returned an "Access Denied" bot-protection page. The ACM, Eurographics and Semantic Scholar pages returned 403. Everything below is from search snippets†.
- **Double density relaxation:** positions are moved by two opposing pressures, a density term and a "near-density" term. Together they enforce incompressibility and stop particles clustering, and surface tension, drops and filaments emerge from them.
- **Integration:** a prediction–relaxation scheme said to stay stable at large steps and in fast splashes.
- **Viscoelasticity:** springs between particles that are added and removed.
- **Objects:** interaction with dynamic objects.

*Bearing:* its near-density pressure is the anti-clustering trick in Lague's GPU SPH. It is worth borrowing for surface behaviour, but the paper has no answer for incompressibility in a deep tank.

**D. Koschier, J. Bender, B. Solenthaler, M. Teschner. "Smoothed Particle Hydrodynamics Techniques for the Physics Based Simulation of Fluids and Solids." Eurographics Tutorial, 2019 (updated notes; companion survey "A Survey on SPH Methods in Computer Graphics", Computer Graphics Forum 2022).**
URLs: https://sph-tutorial.physics-simulation.org/pdf/SPH_Tutorial.pdf ; https://sph-tutorial.physics-simulation.org/ ; https://ar5iv.labs.arxiv.org/html/2009.06944 — [paper] [still-current]
- **Access:** the PDF is over the fetch tool's 10 MB limit. I read the HTML landing page and the arXiv version through ar5iv; that rendering stops in Section 4, so I did not read the boundary and rigid-body chapters.
- **Contents:** neighbourhood search, pressure solvers, boundary handling, multiphase, viscosity, vorticity, solids, rigid bodies, and the SPlisHSPlasH simulator.
- **Stiffness:** larger stiffness constants in the equation of state "result in smaller deviations and require smaller time steps."
- **Time step:** the CFL bound is Δt ≤ λ·h̃/‖v_max‖, with λ ≈ 0.4.
- **Neighbour search:** uniform grid with cell size equal to the kernel support; compact hashing; Z-curve sorting of cells, refreshed at fixed intervals.

*Bearing:* this is the reference text for any SPH-family work in Forge, and its accompanying code (SPlisHSPlasH) is MIT.

**S. Green. "Particle Simulation using CUDA." NVIDIA CUDA SDK white paper, 2010 (versions 2008–2013).**
URLs: https://web.archive.org/web/20140725014123/https://docs.nvidia.com/cuda/samples/5_Simulations/particles/doc/particles.pdf ; copy at https://developer.download.nvidia.com/assets/cuda/files/particles.pdf — [paper] [still-current]
- **Access:** the archive link was refused by the tool. The NVIDIA copy downloaded, but the tool could not parse it. I confirmed the content from the sample's source (mirror: https://github.com/zchee/cuda-sample/tree/master/5_Simulations/particles) and from search snippets.
- **The sample:** it builds a uniform grid either with atomics or with a radix sort.
- **The sort-based pipeline (kernels in the source):**
  - `calcHashD` computes each particle's cell hash;
  - the particle indices are sorted by hash;
  - `reorderDataAndFindCellStartD` reorders the particle data and marks where each cell starts, by comparing a particle's hash with its predecessor's (loaded through shared memory);
  - `collideD` loops over the 27 neighbouring cells, with a discrete-element collision model.
- **Licence:** the 2015 copy is under the NVIDIA EULA. The current NVIDIA/cuda-samples repository is BSD-3-Clause.

*Bearing:* this is still the canonical GPU neighbour grid. Forge needs the same sort, cell-start and gather structure, in Vulkan compute.

**NVIDIA. "Particles" (Omni Physics developer guide), Omniverse Kit docs, current.**
URL: https://docs.omniverse.nvidia.com/kit/docs/omni_physics/latest/dev_guide/particles/particles.html — [engine docs] [still-current]
- **Access:** opened.
- **What it is:** PhysX GPU position-based-dynamics (PBD) particles for fluids and granular media. The particle sets "can interact with all other simulation objects such as articulations, and rigid/deformable bodies."
- **Parameters:** contact offset (default 0.05 m); rest, solid and fluid rest offsets (0.99, 0.99 and 0.594 of the contact offset); neighbourhood capped at 96 by default.
- **Limits:** GPU only, no CPU simulation. Particles in different systems do not collide. Particle cloth has been removed.
- **Render post-processing,** which does not change the dynamics (from a search snippet of the same site†): smoothing, anisotropy (spheres stretched into ellipsoids), isosurface mesh extraction, and diffuse particles (for example spray at wave crests).
- **Other pages on the site:** the extension user guide (https://docs.omniverse.nvidia.com/extensions/latest/ext_physics/physics-particles.html) returned 403.

*Bearing:* this is a shipped reference for the PBF-plus-anisotropy-plus-diffuse pipeline. It is CUDA-only, so it is a design reference, not a dependency.

---

## 2. Solvers

### 2.1 SPH family

**M. Becker, M. Teschner. "Weakly Compressible SPH for Free Surface Flows." SCA 2007.**
URL: https://cg.informatik.uni-freiburg.de/publications.htm (listing) — [paper]
- WCSPH uses a stiff Tait equation of state.
- Following the tutorial above, the stiffer the equation, the smaller the step. This is why WCSPH needs many small substeps for water-like compressibility.

*Bearing:* simplest to write on the GPU, but most expensive per simulated second. Good only for small counts or for "lively but bouncy" liquid.

**B. Solenthaler, R. Pajarola. "Predictive-Corrective Incompressible SPH." ACM SIGGRAPH 2009 (TOG 28(3)).**
URL: https://dl.acm.org/doi/10.1145/1576246.1531346 (403 for the tool) — [paper]
- Pressure is iterated (predicted, then corrected) until the target density is met.
- Reported† time steps 35× larger than WCSPH at 1% density error, and 151× at 0.1%.
- Reported† overall speed-up of 15–16×.

*Bearing:* shows that iterating pressure beats a stiff equation of state. PBF and DFSPH are the later, better forms of the same idea.

**M. Ihmsen, J. Cornelis, B. Solenthaler, C. Horvath, M. Teschner. "Implicit Incompressible SPH." IEEE TVCG 20(3), 2014.**
URL: https://cg.informatik.uni-freiburg.de/publications.htm — [paper]
- A pressure Poisson equation is discretised with SPH and solved by relaxed Jacobi.
- Density deviation down to 0.01%†, at large steps; scenes up to 40 million particles†.
- The tutorial's IISPH chapter (read) uses relaxed Jacobi with ω = 0.5.

*Bearing:* Jacobi suits the GPU, but production IISPH runs offline. No real-time particle counts were found.

**J. Bender, D. Koschier. "Divergence-Free Smoothed Particle Hydrodynamics." SCA 2015; "Divergence-Free SPH for Incompressible and Viscous Fluids." IEEE TVCG 23(3), 1193–1206, 2017.**
URLs: https://animation.rwth-aachen.de/publication/051/ ; https://github.com/InteractiveComputerGraphics/SPlisHSPlasH — [paper] [still-current]
- Two implicit solvers run per step: one for low compression, one for a divergence-free velocity field.
- The authors say the divergence-free solve "drastically reduces the number of required solver iterations and increases simulation stability", which allows larger steps and fewer neighbour searches.
- 5 million fluid particles and 40 million boundary particles at about 5 s per step, with 0.01% compression†.

*Bearing:* the best SPH for accuracy at large steps, but two solves per step and offline-oriented numbers. For a 60 fps lab it costs more than PBF or grid hybrids.

**N. Akinci, M. Ihmsen, G. Akinci, B. Solenthaler, M. Teschner. "Versatile Rigid-Fluid Coupling for Incompressible SPH." ACM TOG 31(4) (SIGGRAPH), 2012.**
URL: https://cgl.ethz.ch/publications/papers/paperSol12.php — [paper] [still-current]
- Rigid surfaces are sampled with boundary particles, each weighted by its local volume, so uneven sampling works.
- Thin structures one layer thick and non-manifold geometry work "without additional treatment"†.
- Two-way coupling is momentum-conserving, through hydrodynamic forces†.

*Bearing:* the standard SPH and PBF boundary. A glass wall one particle layer thick works, and a hole is just missing boundary particles.

**D. Koschier, J. Bender. "Density Maps for Improved SPH Boundary Handling." SCA 2017. J. Bender, T. Kugelstadt, M. Weiler, D. Koschier. "Volume Maps: An Implicit Boundary Representation for SPH." ACM Motion, Interaction and Games (MIG) 2019.**
URLs: https://animation.rwth-aachen.de/media/papers/kb17.pdf ; https://dl.acm.org/doi/fullHtml/10.1145/3359566.3360077 — [paper]
- The boundary's contribution to density (or volume) is precomputed on a narrow-band grid from the signed distance field (SDF).
- This "removes the necessity to sample boundary surfaces with particles" and decouples particle size from boundary sampling†.
- Volume maps leave the kernel out of the map, so one map serves several kernels†.

*Bearing:* SDF boundaries are the modern choice. Forge can rasterise analytic SDFs (planes, boxes, cylinders for the hole) every frame instead of precomputing.

**C. Gissler, A. Peer, S. Band, J. Bender, M. Teschner. "Interlinked SPH Pressure Solvers for Strong Fluid-Rigid Coupling." ACM TOG 38(1), 2019.**
URL: https://cg.informatik.uni-freiburg.de/publications.htm — [paper]
- A second SPH solver at the rigid-body particles is interlinked with the fluid pressure solve.
- This stabilises the fluid–rigid interface and allows larger steps†.

*Bearing:* only needed if light bodies jitter under strong coupling. Too heavy for the first milestone.

**M. Waseem, M. Hong. "Real-Time Two-Way Fluid–Rigid Body Interaction via SDF Coupling with GPU-Accelerated SPH and Volumetric Rendering." Mathematics 14(11):1845, 2026.**
URL: https://www.mdpi.com/2227-7390/14/11/1845 (403 for the tool) — [paper]
- WCSPH in Unity compute.
- O(n) counting-sort spatial hash.
- SDF coupling for spheres, cylinders and tori, with "lock-free atomic compare-and-swap impulse accumulation".
- GPU stream compaction for foam, spray and bubbles.
- Density splatted into a 3D texture for volume rendering.
- Reported above 54 fps at 4 million particles on a "consumer-grade GPU"†.

*Bearing:* close to what Forge wants (SDF two-way coupling plus grid volume rendering). But compare-and-swap float accumulation is order-dependent; Forge should use fixed-point instead.

### 2.2 Position-based and relaxation methods

**M. Macklin, M. Müller. "Position Based Fluids." ACM TOG 32(4) (SIGGRAPH), 2013.**
URLs: https://history.siggraph.org/learning/position-based-fluids-by-macklin-and-muller-fischer/ ; https://graphics.stanford.edu/courses/cs348c-17-fall/PA1_PBF2016/index.html — [paper] [still-current]
- **Method:** density constraints solved inside PBD, giving incompressibility "similar" to modern SPH with PBD's stability at large steps.
- **Extras:** an artificial-pressure term (s_corr) against clumping, which also gives surface tension; vorticity confinement against energy loss; XSPH viscosity.
- **Reported numbers:** 128k particles, 2 substeps × 3 density iterations, about 10 ms per frame†. The GPU is not confirmed; a 2013 article says the NVIDIA demos ran on a single GTX 680.
- **Stanford course parameters (read):** h = 0.1 m, ε = 600, s_corr k = 1e-4, Δq = 0.03 m, n = 4, 4 iterations, dt = 0.0083 s. The course warns "tuning parameters can be a real pain".

*Bearing:* the most proven real-time particle liquid (FleX, PhysX 5). It needs no grid, so domains are unbounded. Weaknesses: compression with few iterations, and tuning by hand.

**M. Macklin, M. Müller, N. Chentanez, T.-Y. Kim. "Unified Particle Physics for Real-Time Applications." ACM TOG 33(4) (SIGGRAPH), 2014. Code: NVIDIA FleX 1.2.**
URLs: https://matthias-research.github.io/pages/publications/publications.html ; https://github.com/NVIDIAGameWorks/FleX — [paper] [code]
- One parallel PBD constraint solver handles liquids, gases, cloth, deformables and rigid bodies (as particle shapes), all with two-way interaction†.
- **FleX repository:** v1.2.0, CUDA, DX11 and DX12 compute, little activity.
- **FleX licence:** the "Nvidia Source Code License (1-Way Commercial)". It is royalty-free and redistributable under the same licence, with patent-termination clauses.

*Bearing:* the design behind FleX and PhysX 5. Rigid bodies as particle shapes would duplicate Jolt, so Forge should take only the fluid parts.

**M. Köster, A. Krüger. "Adaptive Position-Based Fluids." International Journal of Computer Graphics & Animation 6(3), 2016.**
URL: https://arxiv.org/abs/1608.04721 — [paper]
- Solver iterations are adapted per particle.
- Tested on NVIDIA GTX 680 and AMD HD 7850†.
- Baseline PBF dam break: 216k particles, about 20 ms with 3 iterations on the GTX 680†.

*Bearing:* PBF has run on both vendors since 2016. Adaptive iterations are a later optimisation.

**M. Macklin, K. Storey, M. Lu, P. Terdiman, N. Chentanez, S. Jeschke, M. Müller. "Small Steps in Physics Simulation." SCA 2019.**
URL: https://mmacklin.com/smallsteps.pdf (listed at https://blog.mmacklin.com/publications/) — [paper] [still-current]
- n substeps with one XPBD iteration each beat one step with n iterations, with less error and less damping†.

*Bearing:* for PBF and PB-MPM in Forge, prefer substeps over iterations.

### 2.3 Hybrid particle–grid methods: FLIP, APIC, MPM

**Y. Zhu, R. Bridson. "Animating Sand as a Fluid." ACM TOG (SIGGRAPH), 2005.**
URL: https://dl.acm.org/doi/10.1145/1073204.1073298 — [paper]
- The PIC/FLIP blend: particles carry the material; a grid does the pressure projection and boundaries†.

*Bearing:* the base of Niagara's 3D liquid and of LiquiGen.

**C. Jiang, C. Schroeder, A. Selle, J. Teran, A. Stomakhin. "The Affine Particle-In-Cell Method." ACM SIGGRAPH 2015.**
URL: https://disneyanimation.com/publications/the-affine-particle-in-cell-method/ — [paper] [still-current]
- Each particle carries a locally affine velocity.
- Transfers conserve angular momentum, without PIC's dissipation or FLIP's noise and instability.

*Bearing:* use APIC transfers. One affine matrix per particle also serves MLS-MPM and PB-MPM.

**T. Kugelstadt, A. Longva, N. Thuerey, J. Bender. "Implicit Density Projection for Volume Conserving Liquids." IEEE TVCG 27(4), 2019.**
URL: https://animation.rwth-aachen.de/publication/0566/ — [paper]
- A second implicit projection on particle density recovers lost volume.
- Particles are pushed out of solids without losing volume, and particle distributions stay uniform†.
- Blub (Rust/wgpu) implements it.

*Bearing:* this fixes the "water level drifts" failure of plain FLIP, which is the owner's level complaint in grid form.

**Y. Hu, Y. Fang, Z. Ge, Z. Qu, Y. Zhu, A. Pradhana, C. Jiang. "A Moving Least Squares Material Point Method with Displacement Discontinuity and Two-Way Rigid Body Coupling." ACM TOG 37(4):150 (SIGGRAPH), 2018.**
URLs: https://github.com/yuanming-hu/taichi_mpm ; https://github.com/taichi-dev/taichi/blob/master/python/taichi/examples/simulation/mpm88.py — [paper] [code] [still-current]
- **Method:** MLS-MPM makes MPM about 2× faster†, and adds rigid-body coupling and cutting (CPIC).
- **Code:** taichi_mpm is MIT; Taichi is Apache-2.0.
- **mpm88 (2D, read):** 8,192 particles, 128² grid, dt = 2e-4, 50 substeps per frame. The fluid uses the equation-of-state stress −dt·4·E·V·(J−1)/dx² with E = 400.
- **Atomics:** Taichi turns `+=` on fields into atomic adds†.
- **Companion guide:** nialltl's MLS-MPM guide (https://nialltl.neocities.org/articles/mpm_guide, MIT code): the grid is "just a scratch-pad", and recomputing density lets "you get away with a significantly higher timestep".

*Bearing:* the simplest GPU liquid there is: three kernels (particle-to-grid P2G, grid update, grid-to-particle G2P), and no neighbour search. Explicit equation-of-state MPM is weakly compressible, though.

**C. Lewin. "A Position Based Material Point Method." ACM SIGGRAPH 2024 Talks (EA SEED).**
URLs: https://www.ea.com/seed/news/siggraph2024-pbmpm ; https://github.com/electronicarts/pbmpm — [paper] [code] [still-current]
- **Method:** a "semi-implicit compliant constraint formulation" of MPM that is "stable at any time-step while remaining as easy to implement as an explicit integrator". It runs several P2G–G2P cycles per step and solves a material constraint per particle (CRESSim description, read).
- **Drawbacks:** like PBD, it relies on non-physical parameters, and stiffness depends on the iteration count.
- **Reference code:** WebGPU, BSD-3-Clause (read). It uses fixed-point atomics for P2G†.
- **Student DirectX 12 port "Breakpoint"** (https://github.com/danieljgerhardt/Breakpoint, MIT, read):
  - about 50k particles in real time on an RTX 3070 with 3 substeps × 5 iterations;
  - slows sharply at 250k+ in a 64³ grid;
  - bucketed tiles ("bukkits") limited by 32 KB of shared memory;
  - mesh-shader marching cubes at about 2.1 ms.

*Bearing:* the main alternative for Forge. It needs no global Poisson solve, it is stable at any step, BSD-3, and its WGSL ports to Slang without CUDA or wave-size assumptions.

**M. Gao, X. Wang, K. Wu, A. Pradhana, E. Sifakis, C. Yuksel, C. Jiang. "GPU Optimization of Material Point Methods." ACM TOG 37(6) (SIGGRAPH Asia), 2018.**
URL: https://github.com/kuiwuchn/GPUMPM — [paper] [code]
- Sparse grid, a modified histogram sort of particles, and SVD computed on the fly.
- Supports FLIP, APIC and MLS transfers; ten million particles in under a minute per frame (offline).
- **Licence: GPLv3** ("for commercial use, please email authors").

*Bearing:* read for the ideas (sparse blocks, particle binning), not for code.

**K. Wu, N. Truong, C. Yuksel, R. Hoetzlein. "Fast Fluid Simulations with Sparse Volumes on the GPU." Computer Graphics Forum 37(2) (Eurographics), 2018.**
URL: https://ramakarl.com/flip-sim/ — [paper]
- FLIP on GVDB sparse voxels, tens of millions of particles.
- Work-efficient particle-to-voxel gather; matrix-free conjugate gradient (CG) on sparse grids†.

*Bearing:* the path for unbounded FLIP later (rivers). Not needed for a tank.

**P. S. Centeno, J. M. Pereira. "Fluid Implicit Particle Simulation for CPU and GPU." arXiv:2404.01931, 2024.**
URL: https://arxiv.org/html/2404.01931v1 — [paper]
- CUDA FLIP with gather-based P2G (each grid node loops over the particles near it) and a red-black Gauss–Seidel pressure solve.
- On a GTX 1050 Ti: 100k particles on a 32³ grid at 18 ms per step; 500k on 64³ at 83 ms; 1M on 128³ at 204 ms. That is about 20× their CPU.

*Bearing:* a gather-based P2G and red-black Gauss–Seidel are both deterministic. The numbers come from a small 2016 card.

**H. Ou et al. (CRESSim-MPM). "CRESSim-MPM: A Material Point Method Library for Surgical Soft Body Simulation with Cutting and Suturing." arXiv:2502.18437, 2025.**
URL: https://arxiv.org/html/2502.18437v3 — [paper]
- Explains PB-MPM.
- Couples rigid bodies "at grid level": nodal velocities inside a body's SDF are modified, and the momentum impulses are recorded and applied back to the body.

*Bearing:* this is the coupling recipe to copy for Jolt (§7).

**C. Yu, W. Du, Z. Zong, A. Castro, C. Jiang, X. Han. "A Convex Formulation of Material Points and Rigid Bodies with GPU-Accelerated Async-Coupling for Interactive Simulation." arXiv:2503.05046, 2025.**
URL: https://arxiv.org/abs/2503.05046 — [paper]
- An "asynchronous time-splitting scheme" lets MPM and rigid bodies run at different step sizes, at interactive rates.

*Bearing:* supports Forge's plan to run fluid substeps on the GPU while Jolt keeps its own fixed step.

### 2.4 Heightfield plus particles, tall cells, waves

**N. Chentanez, M. Müller. "Real-time Simulation of Large Bodies of Water with Small Scale Details." SCA 2010.**
URL: https://diglib.eg.org/items/d0320015-4b07-416b-8f41-047485c9f7f3 — [paper] [still-current]
- A shallow-water solver for any terrain slope and depth, with wet–dry tracking.
- Where the heightfield cannot represent the liquid (breaking waves, waterfalls, splashes from bodies), it is converted into spray, splash and foam particles. These are "non-interacting point masses" that exchange mass and momentum with the heightfield.
- Small procedural waves are advected with the flow†.

*Bearing:* exactly the island pattern. It also says a correct shallow-water solver must track wet–dry regions, which is relevant to the column model's "stuck above level" symptom.

**N. Chentanez, M. Müller. "Real-time Eulerian Water Simulation Using a Restricted Tall Cell Grid." ACM TOG 30(4) (SIGGRAPH), 2011.**
URL: https://matthias-research.github.io/pages/publications/publications.html — [paper]
- Regular cubic cells sit on top of a layer of tall cells.
- A specialised multigrid Poisson solver, with solver changes for stability at large steps; real-time 3D liquids at large scale†.

*Bearing:* the grid version of "deep but cheap". A candidate for lakes and rivers once the tank solver exists.

**N. Chentanez, M. Müller, T.-Y. Kim. "Coupling 3D Eulerian, Heightfield and Particle Methods for Interactive Simulation of Large Scale Liquid Phenomena." SCA 2014 / IEEE TVCG 2015.**
URL: https://diglib.eg.org/items/e8fb33da-d8a6-4b58-bc8d-87a65e2a42f4 — [paper]
- Water near the surface is particles or grid depending on the region of interest.
- Coupling works by adding the particles' density field to the grid's; outside the 3D domain, shallow-water equations run on a heightfield†.

*Bearing:* the long-term shape of "particles near the player, heightfield far away".

**F. Narita, N. Ochiai, T. Kanai, R. Ando. "Quadtree Tall Cells for Eulerian Liquid Simulation." SIGGRAPH 2025 Conference Papers.**
URL: https://graphics.c.u-tokyo.ac.jp/hp/en/archives/3252 — [paper]
- Tall cells are also subdivided horizontally.
- A variational pressure solve with CG, and monolithic two-way coupled rigid bodies†.

*Bearing:* the 2025 state of tall cells. Offline-leaning, so worth watching rather than adopting.

**S. Jeschke, C. Wojtan. "Generalizing Shallow Water Simulations with Dispersive Surface Waves." ACM TOG (SIGGRAPH), 2023.**
URL: https://history.siggraph.org/learning/generalizing-shallow-water-simulations-with-dispersive-surface-waves-by-jeschke-and-wojtan/ — [paper] [still-current]
- Each step splits a heightfield into bulk shallow-water flow plus Airy surface waves, then recombines them.
- This handles both "a boat wake sloshing up onto a beach" and "a dam break producing wave interference patterns".

*Bearing:* the heightfield upgrade for lakes and rivers. It complements particles; it does not replace them.

**S. Wang et al. "Hamiltonian Two-Way Coupling of Nonlinear Waves and 3D Flows." ACM TOG (SIGGRAPH Asia 2026).**
URL: https://arxiv.org/abs/2608.25203 — [paper]
- A local 3D solver is coupled to a nonlinear dispersive 2D wave model.
- Reported "over 4× faster than a pure GPU NB-FLIP simulation". The page makes no real-time claim.

*Bearing:* the newest version of the hybrid. Watch it.

**L. Huang, Z. Qu, X. Tan, X. Zhang, D. L. Michels, C. Jiang. "Ships, Splashes, and Waves on a Vast Ocean." arXiv:2108.05481, 2021 (SIGGRAPH Asia).**
URL: https://arxiv.org/abs/2108.05481 — [paper]
- FLIP near moving objects, a boundary-element wave model elsewhere. Offline.

*Bearing:* confirms the split (3D particles locally, surface waves globally) at production scale.

### 2.5 Comparison table

| Solver | Stability at large steps | Incompressibility | Real-time counts reported (hardware) | GPU fit | Rigid coupling | Boundaries (glass walls, hole) | Determinism | Best open implementation (licence) |
|---|---|---|---|---|---|---|---|---|
| SPH (Müller 2003) / WCSPH | Poor: stiff EOS ⇒ small Δt | Weak by design | 2,200 @ 20 fps, P4 CPU†; 4M @ 160 fps RTX 4090, CUDA (fluids3; formulation and timing scope not stated); 32k+ "mid-range GPU" (Godot add-on); ~30k iGPU (WebGPU-Ocean SPH mode) | Good: gather over sorted neighbours | Boundary particles (Akinci) or SDF forces | Particles or SDF/volume maps; hole = geometry | Same-GPU if the sort is stable | SPlisHSPlasH (MIT, CPU); fluids3 (MIT, CUDA); Lague Fluid-Sim (MIT, HLSL) |
| PCISPH | Better: 35× WCSPH Δt at 1%† | Iterative, ~1% | None found | Good | Akinci 2012 | As SPH | As SPH | SPlisHSPlasH (MIT) |
| IISPH | Large Δt† | ~0.01%† | None found (offline up to 40M†) | Fair: Jacobi, many iterations | Akinci; Gissler 2019 | As SPH | As SPH | SPlisHSPlasH (MIT) |
| DFSPH | Best SPH (divergence-free) | ≤0.01%† | None (5M @ 5 s/step, offline†) | Fair: two solves per step | Via PBD library (SPlisHSPlasH) | Density/volume maps | As SPH | SPlisHSPlasH (MIT); Salva (Apache-2.0, Rust CPU) |
| Clavet 2005 | Good (prediction–relaxation)† | Approximate | None confirmed | Fair (GPU users adopt near-density inside SPH) | Dynamic objects† | Particles/SDF | As SPH | Lague Fluid-Sim (MIT) borrows near-density |
| PBF / FleX | Good (PBD); substeps help | 3–4 iterations, small compression | 128k, 2×3 iterations, ~10 ms (2013)†; 216k @ 20 ms GTX 680† | Good (FleX, PhysX ship it) | Two-way; FleX rigids as particle shapes | Planes/SDF trivial; hole = geometry | Same-GPU with stable sort | PhysX 5 (BSD-3, CUDA); FleX (NVIDIA 1-way commercial, CUDA/DX) |
| FLIP / PIC | CFL-limited, robust | Exact grid projection; drifts without correction | 100k / 32³: 18 ms per step, GTX 1050 Ti CUDA (2024) | Good: P2G + Poisson | Solid cells + grid momentum | Voxel mask; hole needs ≥ ~5 cells across | Deterministic with fixed-point P2G or gather + fixed-order reductions | Blub (MIT, Rust/wgpu, APIC); Ten Minute Physics (MIT, 2D JS); Warp examples (Apache-2.0, CUDA) |
| APIC + density projection | As FLIP, less noise | Exact + volume-conserving | None found for 3D real time | As FLIP | As FLIP | As FLIP | As FLIP | Blub (MIT) |
| MLS-MPM (explicit, EOS) | Δt limited by EOS stiffness (WebGPU-Ocean: 2 steps per frame) | Weak (EOS) | ~100k iGPU, ~300k "decent GPU", in browser (WebGPU-Ocean); ~1.6M near memory limit (Splash) | Excellent: no neighbour search | CPIC two-way (paper); grid SDF | Grid mask | Fixed-point P2G ⇒ same-GPU deterministic | taichi_mpm (MIT); Taichi (Apache-2.0); WebGPU-Ocean (MIT) |
| PB-MPM | Stable at any Δt | Iteration-dependent stiffness | ~50k real time RTX 3070, 3×5 iterations (student DX12 port) | Good: iterations × (P2G + G2P) | Grid-level | Grid mask | Fixed-point P2G (reference)† | electronicarts/pbmpm (BSD-3, WebGPU) |
| Heightfield + ballistic particles (Chentanez 2010) | Very good | n/a (2.5D) | "Real time" (numbers not retrieved) | Excellent | Buoyancy from heightfield | No overhangs, no tank holes | CPU-deterministic possible | Ten Minute Physics heightfield tutorial (licence not checked) |
| Restricted tall cells | Good† | Exact | Real time (numbers not retrieved) | Good but complex (multigrid) | Monolithic two-way (2025)† | Terrain-shaped | As FLIP | None found open |

---

## 3. GPU building blocks

**R. Hoetzlein. "Fast Fixed-Radius Nearest Neighbors: Interactive Million-Particle Fluids." GPU Technology Conference, 2014. Code: Fluids v3–5.**
URLs: https://ramakarl.com/pdfs/2014_Hoetzlein_FastFixedRadius_Neighbors.pdf (not opened, PDF) ; https://github.com/ramakarl/fluids3 — [talk] [code] [still-current]
- The talk replaces radix sort with a counting sort that uses atomics for bin counts and particle slots†.
- **fluids3:** MIT, CUDA. README: "insertion-sort with atomic operations".
- **fluids3 on an RTX 4090, 4M particles:** v3.2 at 32 fps; v4.0 at 98 fps; v5.0 at 160 fps. The README does not say whether rendering is included.

*Bearing:* counting sort is fastest. But atomic slot assignment gives a different order inside each cell on every run, which breaks bitwise determinism. Forge needs a stable sort, or a sort inside each cell by particle id.

**R. Levien. "Prefix sum on Vulkan." Blog, 2020-04-30 (updated 2021).**
URL: https://raphlinus.github.io/gpu/2020/04/30/prefix-sum.html — [blog] [still-current]
- Decoupled look-back on Vulkan compute: 64Mi uint32 in 2.05 ms on a GTX 1080 (about 82% of bandwidth).
- **Forward progress:** Vulkan's guarantees are "not strong enough to reliably run the prefix sum algorithm as written"; workgroups can deadlock waiting for others.
- **Subgroup sizes:** they vary (AMD defaulting to 64, Intel 8/16/32); `VK_EXT_subgroup_size_control` exists to query and control them.

*Bearing:* do not depend on single-pass look-back without a fallback. A reduce-then-scan prefix sum is simple and safe on both vendors.

**T. Smith (b0nes164). GPUPrefixSums. GitHub, current.**
URL: https://github.com/b0nes164/GPUPrefixSums — [code]
- MIT.
- Implements reduce-then-scan, chained scan with decoupled look-back, and "Decoupled Fallback", which lets devices "without forward thread progress guarantees" complete the scan.
- Claims to be "completely agnostic of wave size" (tested at 4, 16, 32 and 64).
- D3D12, CUDA and Unity versions; the wgpu port is marked "TESTING ONLY".

*Bearing:* the reference for a portable, wave-size-agnostic scan. Port its HLSL to Slang.

**AMD. FidelityFX Parallel Sort. GPUOpen, current.**
URL: https://gpuopen.com/fidelityfx-parallel-sort/ — [code] [still-current]
- MIT, DX12 and Vulkan.
- A radix sort in 4-bit passes (count, reduce, scan, scatter); 8 passes for 32-bit keys.
- Direct or indirect dispatch; "RDNA architecture-optimized", using wave operations.

*Bearing:* a ready, licence-compatible, cross-vendor sort. LSD radix sort is stable, which gives deterministic order inside cells. A 2^18-cell lab grid needs only 5 passes.

**Khronos. Vulkan Guide: "Atomics" and "Subgroups." docs.vulkan.org, current.**
URLs: https://docs.vulkan.org/guide/latest/atomics.html ; https://docs.vulkan.org/guide/latest/subgroups.html — [engine docs] [still-current]
- **Atomics:** core Vulkan guarantees 32-bit integer atomics. `VK_KHR_shader_atomic_int64` (core in 1.2) adds 64-bit integer atomics on buffers and shared memory, but each device must report support. `VK_EXT_shader_atomic_float` adds float atomics only as optional feature bits.
- **Subgroups:** "the size of a subgroup can be dynamic for an implementation". `VK_EXT_subgroup_size_control` is core in 1.3.
- **AMD float-atomic support (snippet):** AMD added `VK_EXT_shader_atomic_float`, including `shaderBufferFloat32AtomicAdd`, for the RX 7000 series in driver 23.4.2†. I could not confirm RDNA 4: the AMD release-notes page timed out.

*Bearing:* write P2G and reductions with 32-bit integer (fixed-point) atomics. They are guaranteed everywhere, order-independent, and therefore deterministic.

**WebGPU limits as a portability template.**
URLs: https://web3dsurvey.com/webgpu/limits/maxComputeWorkgroupStorageSize ; https://github.com/matsuoka-601/WebGPU-Ocean — [engine docs] [code]
- WebGPU's default `maxComputeWorkgroupStorageSize` is 16,384 bytes†.
- WebGPU has no float atomics. WebGPU-Ocean therefore stores P2G sums as integers, "multiplied by constants (e.g., 1e-7)", and uses `atomicAdd`.

*Bearing:* WebGPU codes (WebGPU-Ocean, pbmpm) already respect tighter limits than Forge's: no float atomics, no wave-size assumption, 16 KB shared memory. They port to Slang and Vulkan with little change.

**S. Shanmugavelu et al. "Impacts of floating-point non-associativity on reproducibility for HPC and deep learning applications." arXiv:2408.05148, 2024.**
URL: https://arxiv.org/abs/2408.05148 — [paper]
- "Run to run variability in parallel programs caused by floating-point non-associativity" affects iterative algorithms.
- Deterministic replacements for atomics cost performance.

*Bearing:* any float atomic or race-ordered sum makes the fluid differ from run to run, and an iterative solver amplifies the difference.

**Forge recipe (synthesis, not from one source).**
1. **Buffers.** Particle data in structure-of-arrays form; stable particle ids. The lab uses a dense grid, keyed by the linear cell index; no hash needed. Island splashes use a hashed or sparse key.
2. **Sort.** A stable LSD radix sort of (cell key → particle index), FidelityFX style, every frame or every N frames. Reorder the particle data afterwards, for coherent P2G and G2P.
3. **Cell ranges.** Either Green's "find cell start" (compare with the predecessor's key) or counts plus a reduce-then-scan prefix sum. No decoupled look-back without a fallback.
4. **P2G.** Scatter with 32-bit fixed-point `atomicAdd`, with one scale per channel and headroom against overflow; or 64-bit integer atomics where the device reports them. Alternatives: a per-node gather over sorted particles, or shared-memory tile accumulation then a flush (bukkits, under 32 KB).
5. **Reductions** (per-body forces, CG dot products, volume totals). Fixed-point atomics, or a fixed two-level tree. Never float atomics.
6. **Subgroups.** Only through Vulkan 1.3 `requiredSubgroupSize` where the device supports it, with a shared-memory path otherwise. Never hard-code 32.
7. **Variable counts.** Indirect dispatch for spray and foam, whose counts change.
8. **AMD check.** Test early on the RDNA 2 iGPU of the dev machine for AMD driver behaviour (wave64 defaults, LDS use), before any 9070 XT is available.

---

## 4. Rendering a particle liquid

### 4.1 Screen-space fluid rendering (SSF)

**W. J. van der Laan, S. Green, M. Sainz. "Screen Space Fluid Rendering with Curvature Flow." ACM I3D 2009, pp. 91–98.**
URL: https://www.semanticscholar.org/paper/Screen-space-fluid-rendering-with-curvature-flow-Laan-Green/19865d92faa033632cc1c8ecf95d12a7400c34f1 (403 for the tool) — [paper] [still-current]
- Particles are drawn as spheres into a depth buffer, smoothed by screen-space curvature flow, then shaded.
- A thickness pass accumulates sphere thickness with blending.
- There is no polygonisation and no marching-cubes grid artefacts, and the level of detail follows the view†.

*Bearing:* the baseline SSF. Cheap, but it shows only the near surface from outside the liquid, so on its own it cannot do underwater views.

**S. Green. "Screen Space Fluid Rendering for Games." GDC / SIGGRAPH 2010 (NVIDIA).**
URLs: https://developer.download.nvidia.com/presentations/2010/gdc/Direct3D_Effects.pdf (not parsed) ; https://www.geeks3d.com/20100809/siggraph-2010-screen-space-fluid-rendering-for-games/ — [talk]
- I could not read the slides. The Geeks3D page confirms only that it covers screen-space rendering of SPH fluids.
- WebGPU-Ocean says it follows "the GDC 2010 slides", using a bilateral filter.

*Bearing:* the version games copied. Use the 2018 filter below instead of its bilateral filter.

**N. Truong, C. Yuksel. "A Narrow-Range Filter for Screen-Space Fluid Rendering." Proc. ACM on Computer Graphics and Interactive Techniques 1(1) (I3D), 2018.**
URLs: https://ttnghia.github.io/posts/narrow-range-filter/ ; https://github.com/ttnghia/RealTimeFluidRendering — [paper] [code] [still-current]
- The depth map is smoothed using only depths "in a narrow range". Values outside the range are handled specially, so edges at discontinuities are kept.
- Better smoothness than prior filters, at low cost.
- **Demo code:** Qt and OpenGL, with dependencies that are hard to build. No licence is stated, so treat the code as reference only.
- Matsuoka's Splash (MIT) and Blub (MIT) both implement this filter.

*Bearing:* the filter to use for any SSF in Forge (island splashes, close-ups).

**F. Oliveira, A. Paiva. "Narrow-Band Screen-Space Fluid Rendering." Computer Graphics Forum 41(6):82–93, 2022.**
URL: https://diglib.eg.org/items/dfec37ea-41d7-47e7-93da-6a6ea079c155 — [paper]
- Particles are filtered only in a narrow band around the boundary particles, "to provide a smooth liquid surface with volumetric rendering effects"†.

*Bearing:* a cheaper SSF for large particle counts, if SSF is used at scale.

**N. Truong, K. Martin, S. Arikatla, S. Jhaveri, W. Schroeder, A. Enquobahrie. "Screen-Space Fluid Rendering in VTK." Kitware blog, 2019-12-18.**
URL: https://www.kitware.com/screen-space-fluid-rendering-vtk/ — [blog]
- The pipeline:
  - depth pass of spheres;
  - thickness pass with blending;
  - filter (bilateral, or narrow-range by default);
  - composition.
- Refraction and reflection are "approximately sampling the environment texture", not ray traced.
- "More than 10 million particles in real-time" at 1080p on a GTX 1080 Ti.

*Bearing:* a clean open description of the full SSF pipeline. Its own limitation (no true refraction) is the reason to ray-march in the tank.

### 4.2 Surfaces from particles: anisotropic kernels, meshes, grid ray-marching

**J. Yu, G. Turk. "Reconstructing Surfaces of Particle-Based Fluids Using Anisotropic Kernels." ACM TOG 32(1):5, 2013.**
URL: https://dl.acm.org/doi/10.1145/2421636.2421641 (403 for the tool) — [paper] [still-current]
- The implicit surface is a sum of anisotropic kernels, oriented by PCA of each particle's neighbours, after a smoothing step that moves the kernel centres†.
- PhysX/Omniverse ship this as "anisotropy" (§1).

*Bearing:* use it in the particle-to-render-grid splat to get flat, crisp surfaces from few particles.

**Y. Nishidate, I. Fujishiro. "Efficient Particle-Based Fluid Surface Reconstruction Using Mesh Shaders and Bidirectional Two-Level Grids." Proc. ACM CGIT (I3D), 2024.**
URL: https://dl.acm.org/doi/10.1145/3651285 — [paper]
- Marching cubes in mesh shaders, so no triangle mesh is stored in global memory.
- A two-level grid speeds up the search for surface cells and handles vertex overflow†.
- Breakpoint (above) reports its version at about 2.1 ms.

*Bearing:* the mesh route, if Forge later wants the liquid in a BLAS for ray-traced shadows and refraction. Forge targets mesh shaders on both vendors (the RDNA 2 iGPU has them).

**Epic Games. "Fluid Simulation in Unreal Engine – Overview." UE documentation, current.**
URL: https://dev.epicgames.com/documentation/en-us/unreal-engine/fluid-simulation-in-unreal-engine---overview — [engine docs] [still-current]
- 3D FLIP "is rendered by splatting particles into a grid, then rendering the grid as a surface using ray marching".

*Bearing:* precedent in a shipped engine for the grid ray-march proposed for the tank.

**K. Crane, I. Llamas, S. Tariq. "Real-Time Simulation and Rendering of 3D Fluids." GPU Gems 3, ch. 30, 2007.**
URL: https://developer.nvidia.com/gpugems/gpugems3/part-v-physics-simulation/chapter-30-real-time-simulation-and-rendering-3d-fluids — [paper] [still-current]
- To render liquid, march the volume looking "for the first place along the ray where φ = 0".
- Shade with ∇φ, using "tricubic interpolation" to hide the grid.
- Fake refraction by sampling the background image near the hit.
- Obstacles use an inside–outside voxelisation plus a velocity texture.

*Bearing:* the base of the tank renderer. Forge replaces the fake refraction with analytic glass planes and Snell refraction at the free surface.

### 4.3 Foam, spray and bubbles

**M. Ihmsen, N. Akinci, G. Akinci, M. Teschner. "Unified Spray, Foam and Air Bubbles for Particle-Based Fluids." The Visual Computer 28, 2012.**
URLs: https://www.physicsbasedanimation.com/2012/05/12/unified-spray-foam-and-bubbles-for-particle-based-fluids/ ; https://splishsplash.readthedocs.io/en/latest/FoamGenerator.html — [paper] [code] [still-current]
- **Model:** diffuse particles, generated from potentials (trapped air, wave crest, kinetic energy) and classified by their number of fluid neighbours: fewer than 6 is spray, more than 20 is bubbles, otherwise foam†.
- **Cost:** "interparticle forces and the influence of diffuse material onto the fluid are neglected", so large numbers are cheap.
- **SPlisHSPlasH FoamGenerator (MIT, read):** implements it as a post-process. Defaults: trapped-air factor 4000, wave-crest factor 50000, lifetimes 2–5 s.

*Bearing:* one diffuse-particle system for both the tank and the island. Run it on the GPU after the solver step, with the counts in an indirect dispatch.

### 4.4 Underwater and the medium transition

**Wave Harmonic. "Underwater" (Crest Ocean System 4 docs) and Crest repository.**
URLs: https://crest.readthedocs.io/en/stable/user/underwater.html ; https://github.com/wave-harmonic/crest — [engine docs] [code] [still-current]
- The Underwater Renderer is "a fullscreen underwater effect between the transparent pass and post-processing pass".
- The surface's underside renders with culling off (double-sided).
- A meniscus "renders a subtle line at the intersection between the camera lens and the water".
- Portal, Volume and "Volume (Fly-Through)" modes render underwater "from a provided mesh", which suits a tank or a pool.
- Transparent materials may not render correctly underwater.
- **Crest 4 repository:** MIT. Crest 5 is commercial.

*Bearing:* the design to copy for the half-under view, and the Volume mode is the tank case. The MIT code can be read for the meniscus.

**Unity Technologies. HDRP Water System: "Underwater view" and "Caustics." HDRP 17 docs, current.**
URLs: https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/water-underwater-view.html ; https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/water-caustics-in-the-water-system.html — [engine docs]
- **Underwater view:** a full-screen post effect, using either an analytic absorption formula or volumetric fog "supporting light shafts". A boundary is drawn "when transitioning from below to above".
- **Finite volumes:** use a collider's "Volume Bounds".
- **Caustics:** computed from a wave band, with a "virtual plane distance", and not applied to transparent objects underwater.

*Bearing:* confirms the same structure (mask, full-screen absorption, line at the waterline). Caustics there are a texture trick, not light transport.

**R. M. Pope, E. S. Fry. Absorption spectrum of pure water, 1997 (data via OMLC).**
URL: https://omlc.org/spectra/water/data/pope97.txt — [paper] [still-current]
- Absorption in 1/cm: 450 nm 0.0000922; 500 nm 0.000204; 550 nm 0.000565; 600 nm 0.002224; 650 nm 0.0034; 700 nm 0.00624.
- In 1/m that is: blue 0.009, green 0.057, red 0.34.

*Bearing:* through 1 m of pure water red keeps about 71%, green 94%, blue 99%. Glass-tank water is almost clear and only slightly cyan. "Proper underwater colour" in the lab will need a scattering term and an artistic absorption scale (beauty over realism). Island lakes need much stronger, turbid coefficients.

### 4.5 Glass, refraction, caustics

**C. Wyman. "An Approximate Image-Space Approach for Interactive Refraction." ACM TOG 24(3) (SIGGRAPH), 2005.**
URL: https://cwyman.org/papers/sig05_approxISRefr.pdf (not opened, PDF) — [paper]
- Two GPU passes refract through two interfaces using back-face depth†.
- Related (†, not opened): Imai et al. 2016 reach up to four refractions in screen space (https://dl.acm.org/doi/10.1002/cav.1707).

*Bearing:* image-space two-interface refraction is the fallback if ray-marching is too slow. Glass walls are flat, so Forge can be exact there analytically.

**E. Wallace. "Rendering Realtime Caustics in WebGL." Medium, 2016; WebGL Water demo.**
URLs: https://medium.com/@evanwallace/rendering-realtime-caustics-in-webgl-2a99a29a0b2c (403) ; https://madebyevan.com/webgl-water/ ; https://github.com/evanw/webgl-water ; https://blog.maximeheckel.com/posts/caustics-in-webgl/ — [blog] [code]
- The water-surface mesh is projected along refracted light rays onto the pool floor.
- Brightness is the ratio of original to projected triangle area, computed with screen-space derivatives ("partial derivatives along the x and y axes for the original and the refracted position", per Heckel's write-up, which is "based on" Wallace).
- The demo also has heightfield water, ray-traced reflections and refractions, and analytic ambient occlusion.
- No licence was found on the demo page.

*Bearing:* exactly fits a tank with a flat glass floor. Render the free surface's normals from the light, refract them to the floor plane, and accumulate the area ratio. Milestone 2.

**M. Shah, J. Konttinen, S. Pattanaik. "Caustics Mapping: An Image-Space Technique for Real-Time Caustics." IEEE TVCG 13(2), 2007.**
URL: https://dl.acm.org/doi/10.1109/TVCG.2007.32 — [paper]
- Positions are rendered from the light, and refracted rays are intersected with the receivers in image space, iteratively†.

*Bearing:* the general form, for non-planar receivers (rocks in a lake).

**A. Mihut. "Vulkan Ray Tracing Best Practices for Hybrid Rendering." Khronos blog, 2020-11-23.**
URL: https://www.khronos.org/blog/vulkan-ray-tracing-best-practices-for-hybrid-rendering — [blog]
- BLASes for animated objects and particles "require updating each frame", and need a rebuild when sizes change.
- Grouping geometry saved 60% of BLAS count in Wolfenstein: Youngblood.

*Bearing:* a liquid mesh in Forge's ray tracing means a per-frame BLAS rebuild. Ray-marching the grid avoids that, and shadow rays can march the same grid.

---

## 5. What shipped engines and tools do

**Unreal Engine 5 Niagara Fluids.**
URLs: https://dev.epicgames.com/documentation/en-us/unreal-engine/niagara-fluids-in-unreal-engine ; overview page above — [engine docs] [still-current]
- Templates: 2D gas, 2D liquid, 3D gas, 3D liquid (FLIP), shallow water.
- "2D systems… are best suited for games"; 3D is for "hero effects… and for cinematics".
- Colliders (static meshes, Geometry Collections, depth maps) are boundary conditions, and moving objects push fluid by their velocity.
- Rendering: particles are splatted to a grid, which is then ray-marched.
- I found no two-way physics coupling in the pages I read.

*Bearing:* the closest shipped analogue of the tank plan. Note it treats 3D liquid as hero or cinematic content.

**Sebastian Lague. "Coding Adventure: Simulating Fluids" (2023) and "Coding Adventure: Rendering Fluids" (2024-12-05†); Fluid-Sim repository.**
URL: https://github.com/SebLague/Fluid-Sim — [code] [talk]
- MIT, Unity 2022.3, compute-shader SPH in 2D and 3D.
- Density and near-density pressure (Clavet-style), a spatial hash, and a GPU bitonic sort†.
- The rendering video tries marching cubes, ray-marching and SSF, plus reflections, refractions, and foam, spray and bubbles†.
- Its references are the owner's four PDFs, plus Green 2010 and Ihmsen 2012.
- Ports: jeantimex/fluid (MIT, WebGPU SPH and PIC/FLIP, "tens of thousands of particles at 60 FPS", https://github.com/jeantimex/fluid).

*Bearing:* a readable reference for an all-GPU SPH pipeline and its rendering choices. Not the solver for a deep tank.

**Godot: "3D SPH Fluid Simulation" add-on (deni1000), Godot Asset Library, 2026-05-04.**
URL: https://godotengine.org/asset-library/asset/5116 — [code]
- MIT.
- "32k+ particles on mid-range GPUs"; GPU prefix-sum spatial hashing; "Texture-based 3D SDF" collisions; MultiMesh drawing; RenderingDevice (Vulkan).

*Bearing:* shows the SDF-collision SPH pattern on Vulkan compute; the counts are modest.

**Notch.**
URL: https://www.notch.one/features/particles-simulations-volumetrics — [engine docs]
- "FLIP, SPH, and MPM particle physics", "up to 16 Million Particles", "completely GPU-powered".
- Manual pages exist for a Fluid SPH Affector and a Fluid MPM Affector (https://manual.notch.one/2026.1/en/docs/reference/nodes/particles/affectors/fluid-mpm-affector/), but their text did not render for the tool. A search snippet says the MPM affector is "faster and more stable" and "better at settling over time"†.

*Bearing:* a commercial real-time tool that offers MPM next to SPH for stability and settling; it is closed source.

**WebGPU demos: matsuoka-601 WebGPU-Ocean and Splash; holtsetio Flow (three.js).**
URLs: https://github.com/matsuoka-601/WebGPU-Ocean ; https://github.com/matsuoka-601/Splash ; https://github.com/acmeproducts/flow (fork of holtsetio/flow) — [code] [still-current]
- **WebGPU-Ocean** (MIT): MLS-MPM following nialltl, ~100k particles on integrated GPUs and ~300k on "decent GPUs"; fixed-point P2G atomics; 2 steps per frame; SSF with a bilateral filter. Neighbour search for its SPH mode follows Hoetzlein.
- **Splash** (MIT): one substep per frame, Tait equation with γ = 1 "for improved stability"; narrow-range filter, ray-marched density shadows; a "million" branch at ~1.6M particles near the memory limit.
- **Flow** (three.js WebGPURenderer): MLS-MPM "guided by" WebGPU-Ocean.

*Bearing:* the best small, permissive, Vulkan-portable references for MLS-MPM plus SSF. Port them to Slang as the first prototype.

**NVIDIA PhysX 5 (PBD particles) and Omniverse.**
URLs: https://github.com/NVIDIA-Omniverse/PhysX ; https://nvidia-omniverse.github.io/PhysX/physx/5.1.3/docs/ParticleSystem.html ; https://www.cgchannel.com/2025/04/nvidia-open-sources-physxs-gpu-simulation-code/ ; https://docs.isaacsim.omniverse.nvidia.com/6.0.0/overview/release_notes.html — [engine docs] [code]
- **Licence:** BSD-3-Clause. The GPU code was opened with PhysX 5.6 (2025-04-22): "over 500 kernels written for CUDA".
- **Particle system:** "requires a CUDA capable GPU"; 96 neighbours maximum; diffuse-particle buffers.
- **Removals:** particle cloth, rigids and volumes are being removed, and PBD lift and drag are now no-ops (Isaac Sim 6.0 notes). Fluid and granular PBD remain.

*Bearing:* readable BSD-3 CUDA for PBF in production (neighbour lists, SDF collision, diffuse particles). It cannot be used directly (CUDA), but it can be ported.

**NVIDIA Warp.**
URL: https://github.com/NVIDIA/warp — [code]
- Apache-2.0, Python kernels on CUDA or CPU.
- Examples include `example_sph.py`, `example_fluid.py` and `example_apic_fluid.py`.

*Bearing:* good for prototyping and checking a solver on the CPU or an NVIDIA GPU against Forge's Slang version. Not a runtime.

**JangaFX LiquiGen.**
URL: https://www.cgchannel.com/2025/07/jangafx-releases-liquigen-1-0/ (also /2024/07/liquigen-is-now-in-public-alpha/ from search) — [engine docs]
- 1.0 released 2025-07-29.
- PIC/FLIP; it moved from APIC to PIC/FLIP in 0.3†. "Simulate and mesh liquids in near-real time."
- Whitewater (spray, foam, bubbles); exports flipbooks, VAT, Alembic and VDB.
- Minimum GPU: GTX 1060 or RX 580. Commercial.

*Bearing:* a dedicated liquid tool chose PIC/FLIP plus whitewater, which supports the recommendation. Baked VATs are an option for scripted island set pieces such as a dam burst.

**Other code bases worth knowing.**
- **SPlisHSPlasH** (MIT; https://github.com/InteractiveComputerGraphics/SPlisHSPlasH): WCSPH, PCISPH, PBF, IISPH, DFSPH and PF; Akinci, density-map and volume-map boundaries; CUDA neighbour search (cuNSearch). The accuracy reference.
- **Blub** (MIT; https://github.com/Wumpf/blub): Rust/wgpu APIC with PCG and Kugelstadt density projection.
  - P2G: particles are pushed into linked lists by atomic exchange, then grid cells gather from them, giving a ">4x" speed-up with shared memory.
  - Narrow-range SSF; boundaries by real-time hull voxelisation.
  - The closest code to Forge's language and API.
  - Its P2G order depends on atomic exchange, so it is not deterministic as written.
- **Salva** (Apache-2.0; https://github.com/dimforge/salva): Rust CPU DFSPH and IISPH, two-way coupled with Rapier. A model for a CPU-side coupling API.
- **Ten Minute Physics** (MIT header in 18-flip.html; https://matthias-research.github.io/pages/tenMinutePhysics/index.html): FLIP (#18, with drift compensation), Eulerian fluid (#17), heightfield water (#20).
- **Genesis** (Apache-2.0; https://github.com/Genesis-Embodied-AI/Genesis): SPH, MPM and PBD on CUDA, ROCm, Metal or Vulkan through its compiler. A robotics framework, not a game runtime.
- **Zibra Liquids** (commercial; https://www.zibra.ai/blog-posts/approaches-to-real-time-fluid-simulation-in-visual-effects, 2025-10-01): MLS-MPM with neural SDF colliders; "tens of millions" of particles on high-end GPUs and "hundreds of thousands" on laptops.
- **Tencent course** "Building a Real-Time System on GPUs for Simulation and Rendering of Realistic 3D Liquid in Video Games" (SIGGRAPH 2023 Courses, https://dl.acm.org/doi/10.1145/3587423.3595537): "millions of particles", integrated into UE5†. Not read.

| Code base | Method | API | Licence |
|---|---|---|---|
| SPlisHSPlasH | SPH family | C++ (CPU, CUDA neighbour search) | MIT |
| fluids3 | SPH | CUDA | MIT |
| Lague Fluid-Sim | SPH (near-density) | Unity HLSL | MIT |
| WebGPU-Ocean / Splash | MLS-MPM (+SPH) | WebGPU/WGSL | MIT |
| electronicarts/pbmpm | PB-MPM | WebGPU/WGSL | BSD-3 |
| Breakpoint | PB-MPM + mesh-shader marching cubes | DX12 | MIT (+BSD-3 parts) |
| Blub | APIC + PCG | Rust/wgpu | MIT |
| taichi_mpm / Taichi | MLS-MPM | C++/Taichi | MIT / Apache-2.0 |
| PhysX 5 | PBF | CUDA | BSD-3 |
| FleX | PBF unified | CUDA/DX | NVIDIA Source Code License (1-way commercial) |
| GPUMPM | MPM | CUDA | GPLv3 (avoid) |
| Warp | various | CUDA/CPU | Apache-2.0 |
| Salva | DFSPH/IISPH | Rust CPU | Apache-2.0 |
| Crest 4 | Ocean + underwater | Unity | MIT |
| GPUPrefixSums | Scans | HLSL/CUDA | MIT |
| FidelityFX Parallel Sort | Radix sort | HLSL/Vulkan | MIT |
| Godot SPH add-on | SPH | GLSL/Godot | MIT |
| cuda-samples (particles) | Uniform grid | CUDA | BSD-3 |

---

## 6. Particles in big outdoor water

**Further sources.**
- **P. Kipfer, R. Westermann. "Realistic and Interactive Simulation of Rivers." Graphics Interface 2006** (https://www.researchgate.net/publication/221474856) — [paper]. GPU SPH particles over a heightfield for rivers, at interactive rates but with "very low density of particles"†. *Bearing:* an early sign that full-river particles are too expensive; use particles only where the heightfield fails.
- **C. Yuksel, D. House, J. Keyser. "Wave Particles." ACM SIGGRAPH 2007** (https://www.cemyuksel.com/research/waveparticles/) — [paper]. Moving objects emit wave particles, which are converted to a heightfield on the GPU. "Unconditionally stable", with hundreds of objects in real time†. *Bearing:* wakes behind boats and swimmers on lakes, with no fluid solver.
- **S. Jeschke et al. "Water Surface Wavelets." ACM TOG 37(4), 2018** (https://research-explorer.ista.ac.at/record/134) — [paper]. Avoids the CFL and Nyquist limits of heightfield waves through amplitude functions over space, frequency and direction†. *Bearing:* detail waves around obstacles in large lakes.

**Plan for the island (synthesis).** The heightfield stays the bulk model: the CPU column model is authoritative, and the GPU shallow-water layer runs near the player, as already planned. Particles cover only what a heightfield cannot.

**Spawn particles where the heightfield fails** (Chentanez & Müller 2010):
- at waterfall lips: flow over a drop larger than a threshold;
- at steep surface gradients or breaking fronts: the surface slope or the vertical acceleration exceeds a limit;
- at impacts: a rigid body or the player enters the water fast;
- in rapids: high speed over rocks, or a large curl of the velocity.

**Each spawned particle:**
- removes its volume from the heightfield cell, so mass is conserved;
- inherits the local velocity plus a random part;
- flies ballistically with drag, with no particle–particle forces.

**When a particle lands** back on the water surface, it deposits its mass and momentum into the heightfield and becomes foam on the surface texture. When it lands on terrain, it soaks away, or is counted as lost by design.

**Diffuse layer.** Ihmsen classification:
- spray (few neighbours) is drawn as lit, motion-blurred sprites;
- foam rides the heightfield and is advected by its velocity;
- bubbles appear only in the tank or in clear pools.

**Waterfalls.** A falling sheet is best drawn as a mesh or ribbon from the lip's flow rate and velocity. Ballistic particles add mist, and the plunge pool spawns foam and bubbles. A local PBF cluster (grid-free) is an option for a close-up hero waterfall.

**Wakes and objects in lakes.** Use wave particles or the heightfield's own response, and Jolt's plane-based `ApplyBuoyancyImpulse` overload for floating bodies. The tank's particle solver is not needed outdoors.

**Precedent.** Niagara offers the same split: shallow water as "height fields… rendered as displacements" for wakes and simple interaction, and 3D FLIP only for hero effects.

---

## 7. Coupling with Jolt

**J. Rouwé. Jolt Physics: `Body::ApplyBuoyancyImpulse`, `GetSubmergedVolume`; Architecture ("Deterministic simulation").**
URLs: https://jrouwe.github.io/JoltPhysics/class_body.html ; https://github.com/jrouwe/JoltPhysics/blob/master/Docs/Architecture.md — [engine docs] [still-current]
- **Overload 1:** takes a surface position and normal (a fluid plane).
- **Overload 2:** takes `inTotalVolume`, `inSubmergedVolume`, `inRelativeCenterOfBuoyancy`, `inBuoyancy`, `inLinearDrag`, `inAngularDrag`, `inFluidVelocity`, `inGravity` and `inDeltaTime`.
- Buoyancy factor: 1 is neutral, below 1 sinks, above 1 floats†.
- `GetSubmergedVolume` computes the volumes for a plane.
- (Determinism is in §8.)

*Bearing:* overload 2 lets a GPU fluid of any shape drive Jolt without Forge writing its own pressure integration.

**Proposed coupling scheme.** This is my synthesis, combining Niagara's moving colliders, CRESSim's grid-level impulses and Jolt's overload 2.

1. **Bodies into the fluid** (bodies push the liquid). Once per Jolt step, upload the pose, linear and angular velocity, and shape of each awake body near the liquid. Analytic SDFs are enough for boxes, spheres and capsules; meshes use precomputed SDF textures. In every fluid substep:
   - rasterise solid fraction and solid velocity into the grid;
   - in the projection (or in PB-MPM's grid update), set node velocities inside and near a body to the body's velocity (no-penetration), as Niagara and CRESSim do.
   - The gate and the lid of the hole are kinematic Jolt bodies.
2. **Fluid back onto the bodies, milestone 1** (buoyancy and drag). After the frame's substeps, a reduction pass per body gives:
   - submerged volume (the sum over the body's cells of fluid fraction × cell volume);
   - total volume;
   - centre of buoyancy;
   - mean fluid velocity in a shell around the body.

   All are fixed-point, so deterministic. Read them back asynchronously, one frame late. Jolt calls overload 2 each step. Jolt integrates buoyancy and drag itself, so a one-frame lag in a slowly varying submerged volume is harmless.
3. **Fluid back onto the bodies, milestone 2** (full momentum exchange). Also accumulate the impulse the grid applied to enforce each body's boundary, as a force at the centre of mass plus a torque, following CRESSim and MLS-MPM's CPIC. Apply it with `AddForce` / `AddTorque`, clamped and filtered. This is what makes a block get shoved by the dam-break front. Light bodies may need the interlinked or implicit treatment of Gissler 2019, or more substeps.
4. **Sleeping and wake.** A body that becomes submerged, or that the wave front reaches, must be woken through `BodyInterface`. The overload on `Body` does not wake it.
5. **Body–body contacts stay in Jolt.** Do not simulate rigid bodies as particle shapes (FleX style); that would split the rigid authority.

---

## 8. Determinism and networking

**Jolt's rules (Architecture.md, read).** Jolt is deterministic when the API calls are made in the same order and "the same binary code is used". `CROSS_PLATFORM_DETERMINISTIC` extends this across compilers, OSes and CPU architectures, "approximately 8% slower".

**GPU floating point.** The same GPU, driver and SPIR-V can be made bitwise repeatable. Different vendors, or even different drivers, generally cannot: fused multiply-add choices, transcendental precision and compiler reordering differ, and the reproducibility literature above stresses that summation order dominates.

**Options for Forge's GPU liquid.**

| Option | What it gives | Cost | Use |
|---|---|---|---|
| A. Visual only | Never touches gameplay state; replays may differ visually | None | Island splashes, foam, all multiplayer |
| B. Deterministic on one GPU | Bit-identical replays and digests on the same machine and driver | Fixed-point P2G and reductions; stable radix sort (no atomic slot order); fixed iteration counts; no float atomics; NoContraction or `precise` on the few sensitive sums (to verify in Slang); same shader binary | Lab replays, A/B harness, `tools/verify.sh` tiers on the 5070 Ti |
| C. CPU-authoritative at low resolution | Network- and replay-safe gameplay water | A coarse CPU model (the fixed column model, or a coarse CPU grid or particles under Jolt-style rules); the GPU fluid is slaved to it visually | Any water that moves gameplay bodies in multiplayer |

**How others handle networking (weak evidence).**
- The best I found is forum material†. Unreal water-plugin users sync server time so every client evaluates the same Gerstner waves, and compute buoyancy on the server. Players speculate that Sea of Thieves sends wave parameters plus server time.
- I found no shipped game that networks a 3D particle liquid's state.
- Niagara is a VFX system, and PhysX GPU particles need CUDA. My inference is that both are used as client-side effects in multiplayer; no page I read says so explicitly.

**Recommendation.**
- Option A for the island and for anything networked. Gameplay buoyancy comes from the CPU heightfield model, through Jolt overload 1.
- Option B inside the single-player lab, so the tank scenes become repeatable tests on the 5070 Ti.
- Cross-GPU runs (3080, RDNA 2 iGPU, a future 9070 XT) are compared with physical metrics, not digests: water volume, final level, front position over time, outflow rate.
- If tank-like water ever matters in multiplayer, add option C, with the server running the coarse CPU model.

---

## 9. Recommendation for Forge

### 9.1 Solver for the glass-tank lab

**The solver.**
- Hybrid particle–grid with APIC transfers, using MLS-MPM's quadratic B-spline weights and affine matrix.
- Incompressible by a grid pressure projection, with a particle-density correction (Kugelstadt 2019, or Ten Minute Physics' "drift compensation").
- Dense collocated or MAC grid over the whole lab: tanks, outflow basin and table.
- Fixed-point P2G.
- 2–4 substeps per 60 Hz frame, chosen by CFL ≤ ~1.5 cells per substep.
- Pressure solve: red-black Gauss–Seidel or Jacobi with warm start at first; a multigrid V-cycle if the iteration count must drop.

**Why this solver:**
- True incompressibility and volume conservation give the right final level and the right outflow through a hole. That is the owner's complaint, and it is not something an equation of state gives cheaply.
- No neighbour search.
- The grid is reused for rendering, coupling and boundaries.
- Shipped precedent: Niagara, LiquiGen. Open Rust and wgpu precedent: Blub (MIT).

**Alternative behind the same data.** PB-MPM (EA, BSD-3, stable at any Δt, no global solve). Decide by an A/B at milestone 2 on: final-level error, look of the dam-break front, and ms.

**Not chosen.**
- PBF needs neighbour search plus tuning, and is compressible at 3–4 iterations. Keep it for grid-free splash clusters.
- DFSPH and IISPH are too costly per frame.
- Explicit MLS-MPM with an equation of state is the fastest way to bring the pipeline up, but it is visibly compressible.

**Boundaries.**
- Glass walls and floor are analytic planes, as domain faces of the grid.
- The gate is a kinematic Jolt box.
- The circular hole is a cylinder subtracted from the wall's solid mask: 8 cm across is about 5 cells at 1.5 cm, or 8 cells at 1 cm.
- Particles leaving the grid become ballistic spray, or are deleted and counted.

### 9.2 Solver for splashes in the island's rivers

- Ballistic GPU particles spawned from the heightfield and returned to it (§6), plus Ihmsen diffuse classification.
- No pressure solver.
- Optional later: a grid-free PBF cluster for a hero waterfall, sharing the sort and cell-range kernels with the lab.

### 9.3 Surface rendering, underwater and the medium transition

**Tank liquid.** A ray-marching compute pass over a render density field. The field is the solver's B-spline-splatted mass, optionally at 2× resolution with anisotropic kernels, and lightly blurred against particle noise. Per pixel:
1. **Entry.** Analytic ray–box entry into the tank. Snell refraction at the glass–water planes: flat glass shifts the ray sideways but leaves only the air–water direction change. Inside air above the waterline, no bend.
2. **March.** Steps of half a cell. At φ = 0 crossings, the normal comes from tricubic or B-spline ∇φ. Split by Fresnel:
   - reflection goes to probes, or to a ray query in milestone 2;
   - refraction continues, and when Snell fails, total internal reflection happens (Snell's window seen from below).
3. **Absorption and scattering.** Beer–Lambert with the Pope & Fry absorption times an artistic scale, plus single-scatter in-scattering towards the sun.
4. **Exit.** Sample the background behind along the refracted ray: screen space first, a ray query against the scene TLAS later.

**What this gives together:** views from the side, from above and from below (glass floor); seeing the volume; the camera inside the water.

**Medium transition.**
- A per-pixel near-plane test, sampling φ (tank) or the heightfield (lakes), builds an "is underwater" mask.
- A full-screen underwater pass (fog, absorption, scattering for opaque geometry) runs before post-processing, as in Crest and HDRP.
- A meniscus line is drawn where the mask changes along the near plane. Optional droplets on the lens follow after emerging.

**Diffuse particles.** Foam, spray and bubbles as sprites, depth-tested. Bubbles are visible only inside the water mask, and are attenuated.

**TAA.** Motion vectors for the liquid come from the grid velocity at the first hit. Sprite sizes are kept above about 1–2 px to avoid shimmer, which the owner notices at once.

**Island splashes.** Narrow-range SSF for coherent sheets and chunks; sprites for spray.

**Caustics (milestone 2).** Light-space render of the free-surface normals, then Wallace's area-ratio projection onto the planar tank floor. Shadows of the liquid come from a light-space transmittance march of the same grid.

**Fallback if the march is too slow or blobby.** Marching cubes in mesh shaders (Nishidate & Fujishiro 2024) into a per-frame BLAS, with ray-query refraction.

### 9.4 Coupling with Jolt

As in §7. Milestone 1: bodies as grid boundaries, and Jolt overload 2 fed with submerged volume, centre of buoyancy and fluid velocity from the GPU, one frame late. Milestone 2: grid-level momentum exchange.

### 9.5 Determinism and networking

As in §8. Lab: option B on the 5070 Ti (digests), with metric-based checks on other GPUs. World and multiplayer: option A, with CPU water authoritative.

### 9.6 First milestone: "Glass tank 1"

**Scene.**
- One tank, inner size 1.0 × 0.5 × 0.6 m, with 1 cm glass walls and floor, on a table.
- A water block of 0.4 × 0.5 × 0.4 m behind a gate, which is a kinematic Jolt box lifted at t = 1 s.
- Three wooden boxes (density about 600 kg/m³) and one stone block inside.
- Variant B: the same tank, but the gate is a fixed wall with an 8 cm circular hole, either central or 5 cm above the floor. A plug is pulled at t = 1 s, and the water drains into a lower open basin.

**Simulation.**
- Grid 1.5 cm over tank plus basin, about 100 × 40 × 47, so ~190k cells.
- Particles at 8 per water cell, about 190k (0.08 m³ of water / 3.4 cm³ per cell ≈ 23.7k cells).
- 4 substeps per frame. The dam-break front reaches about 2√(g·h₀) ≈ 4 m/s, which is 6.7 cm per frame.
- Fixed-point P2G; Jacobi or red-black Gauss–Seidel with a fixed iteration count plus density correction.

**Rendering.** Grid ray-march at half resolution, upscaled. Diffuse particles up to about 50k. Underwater mask plus meniscus. No caustics yet.

**Jolt.** Overload-2 buoyancy and drag; bodies as grid boundaries.

**Expected numbers (est.).**
- Simulation ≤ 2.5 ms per frame and liquid rendering ≤ 1.5 ms at 1440p on the RTX 5070 Ti, alongside ray-traced shadows, GI and TAA.
- Basis: WebGPU-Ocean's ~300k MLS-MPM particles inside a browser on a "decent GPU", and FLIP at 100k particles / 32³ in 18 ms per step on a GTX 1050 Ti.
- This is an extrapolation, not a measurement. The kill criterion is above 5 ms in total.

**Checks** (tiered `tools/verify.sh`):
- **Volume:** conserved within 1% after settling.
- **Final level:** within 2 mm of V / A (0.16 m for the dam break). The analytic check that the column model failed.
- **Dam-break front:** position over time, plotted against the shallow-water Ritter front speed 2√(g·h₀) as a sanity bound.
- **Outflow (variant B):** jet speed near √(2gh) and a discharge coefficient near the textbook ~0.6 for a sharp-edged orifice.
- **Same-GPU determinism:** three runs give identical digests at frames 300, 600 and 900.
- **Cross-vendor:** runs on the RDNA 2 iGPU (and the 3080) pass the metric checks, with no NVIDIA-only feature.
- **Visual:** A/B captures for shimmer at the free surface and the waterline.

**Milestone 2.**
- Momentum exchange with Jolt.
- Caustics on the floor.
- A/B of PB-MPM against projection.
- Two tanks in one scene.
- Optional ray-query refraction exit.
- 1 cm grid (~640k particles) if the budget allows.

---

## 10. Proposed decision (🟡) and open questions

🟡 **Proposed decision D-0xx: "Particle liquid for the lab; heightfield plus particles for the world."**
1. The physics lab gets a GPU hybrid particle–grid liquid with the following properties:
   - **Transfers:** APIC (MLS quadratic B-spline).
   - **Incompressibility:** a grid pressure projection with a particle-density volume correction.
   - **Grid:** a dense grid over the lab scene.
   - **Implementation:** Vulkan compute in Slang, with no CUDA, no float atomics and no wave-size assumption.
   - **Atomics:** 32-bit fixed-point atomics for P2G and every reduction.
   - **Sort:** a stable radix sort for any particle ordering.

   PB-MPM (EA SEED, BSD-3) is kept as the alternative pressure model, behind the same data structures, and decided by A/B at milestone 2.
2. The tank renders by ray-marching the solver's density grid, with:
   - analytic glass planes;
   - Snell refraction;
   - Beer–Lambert absorption (scaled Pope & Fry);
   - single scattering;
   - a near-plane underwater mask with a meniscus line;
   - motion vectors from the grid velocity.

   Spray, foam and bubbles are Ihmsen diffuse particles, shared with the island.
3. Jolt coupling: bodies are written into the grid as moving solids. The GPU returns submerged volume, centre of buoyancy and fluid velocity per body, one frame late, into `Body::ApplyBuoyancyImpulse` (volume overload). Full momentum exchange is milestone 2.
4. The GPU liquid is never network or gameplay state outside the single-player lab. In the lab it must be bit-deterministic on one GPU (replay digests) and metric-equivalent across GPUs.
5. The island keeps the heightfield authoritative (CPU column model, GPU shallow water near the player). GPU ballistic particles are spawned where it fails and returned to it on landing.
6. Licences: port from MIT, BSD-3 and Apache-2.0 code with credit (WebGPU-Ocean, pbmpm, Blub, FidelityFX Parallel Sort, GPUPrefixSums, Crest 4 meniscus). Do not use GPUMPM (GPLv3). PhysX and FleX are reference only (CUDA).

**Open questions only the owner can answer.**
1. **Particle versus hybrid.** Does a particle–grid hybrid (particles carry the water, a grid does the pressure) meet "particle fluid simulation"? Or must the solver be grid-free (PBF or SPH), at the cost of more compression and tuning?
2. **Gameplay authority.** May the lab liquid move Jolt bodies in ways that matter beyond the lab, or should it stay visual and lab-only as proposed?
3. **Tank size and detail.** Is 1.0 × 0.5 × 0.6 m at 1.5 cm (~190k particles) the right first target, or does the owner want bigger tanks or finer detail (1 cm, about 640k)?
4. **Frame budget.** How many ms of the 16.7 ms frame may the liquid take (simulation and rendering) when ray-traced shadows, GI and TAA are on?
5. **Look.** Should tank water be physically pale (pure water barely tints over 1 m), or stylised with stronger blue-green and some turbidity? Should the hole's jet show foam or bubbles?
6. **Column model.** Fix the column model's "stuck above level" bug in parallel (wet–dry tracking, obstacle faces), or freeze it until the lab liquid exists?
7. **Hardware.** Is testing on the RDNA 2 iGPU acceptable as the AMD check until a 9070 XT is available?

---

## 11. Verification notes

**Owner's links.**

| Link | Result |
|---|---|
| sca03.pdf | Reached. The tool returned the binary PDF without text. Content confirmed by search snippets† (2,200 particles at 20 fps, etc.). |
| Clavet (web.archive.org) | **Unreachable.** The tool refuses web.archive.org. The original ligum.umontreal.ca URL returned "Access Denied" (bot protection). ACM, Eurographics and Semantic Scholar pages returned 403. Content from snippets only†. |
| SPH_Tutorial.pdf | **Too large** (over 10 MB). Read the HTML landing page and arXiv 2009.06944 through ar5iv (Sections 1–4 only; it truncates before boundary handling and rigid bodies). |
| NVIDIA particles.pdf (web.archive.org) | **Refused.** The developer.download.nvidia.com copy downloaded but was not parsed. Confirmed through the sample source on a GitHub mirror and snippets. |
| Omniverse particles page | Opened. Other pages on the site: the extension guide returned 403; the post-processing description is from a snippet†. |

**Files written to disk without my asking.** When it could not parse a PDF, WebFetch saved the binary in Claude's tool-results folder, `C:\Users\gaelb\.claude\projects\D--sources-workspace-engines\3a668638-03a4-4999-b798-66625e39474d\tool-results\`. Five files:
- `webfetch-1791013678192-mo4vic.pdf` (sca03);
- `webfetch-1791013695762-83383a.pdf` (NVIDIA particles);
- `webfetch-1791013716169-x3wldw.pdf` (PBF);
- `webfetch-1791013717559-6gbbbp.pdf` (tall cells);
- `webfetch-1791013716546-yyl88i.pdf` (Chentanez 2014 hybrid).

I did not open them and they can be deleted. I stopped fetching PDFs after that.

**Opened and read.**
- **Papers and projects:** arXiv pages for 2403.11156, 2404.01931, 2502.18437, 2503.05046, 1608.04721, 2108.05481, 2608.25203, 2408.05148; Disney APIC; the RWTH DFSPH page; the ETH Akinci page; the Freiburg publication list; SIGGRAPH History (PBF, Jeschke & Wojtan 2023); the Stanford CS348C PBF page; the EA SEED PB-MPM page; ramakarl flip-sim.
- **GitHub:** SPlisHSPlasH (and its readthedocs FoamGenerator page), WebGPU-Ocean, Splash, Fluid-Sim, Blub, pbmpm, Breakpoint, taichi_mpm, mpm88, FleX (+ LICENSE), PhysX, cuda-samples LICENSE and the particles mirror, fluids3, GPUMPM, Warp, Salva, Genesis, Crest, GPUPrefixSums (+ LICENSE), jeantimex/fluid, acmeproducts/flow, Ten Minute Physics 18-flip.html.
- **Documentation:** Unreal fluid docs; Unity HDRP underwater and caustics; Crest underwater; PhysX 5.1.3 particle docs; Jolt Body class and Architecture.md; Vulkan Guide atomics and subgroups; GPUOpen FidelityFX Parallel Sort; Khronos ray-tracing blog; GPU Gems 3 ch. 30; Kitware SSF; Godot asset 5116; Notch features page; the Zibra blog; CGChannel (PhysX 5.6, LiquiGen 1.0); Isaac Sim 6.0 notes; the OMLC Pope & Fry data; Levien's prefix-sum blog; the nialltl MPM guide; the ttnghia narrow-range page; 80.lv on WebGPU-Ocean; Maxime Heckel's caustics post; the WebGL Water page; HN item 10859689 (date only); physicsbasedanimation (Ihmsen abstract).

**Failed: HTTP 403.** ACM DL, Wiley, ScienceDirect, MDPI, Semantic Scholar (and its API: 429), Eurographics diglib, Codrops (the matsuoka write-up), Medium (Wallace), OC3D, the Omniverse extension page.

**Failed: 404 or other.**
- 404: research.nvidia.com PBF page; yuanming-hu.github.io/mls-mpm; two cuda-samples paths.
- Timeouts: the AMD Vulkan release notes, twice.
- Notch manual pages rendered without content.

**Resting on search snippets only (†).**
- Numbers and claims: Müller 2003 numbers; PBF 128k at 10 ms (its GPU is not confirmed); Köster 216k at 20 ms on a GTX 680; PCISPH 35×, 151× and 15–16×; IISPH 0.01% and 40M; DFSPH 5M at 5 s.
- Abstract details: Akinci single-layer boundaries; density and volume maps abstracts; Gissler 2019; Ihmsen neighbour thresholds (6 / 20).
- Paper summaries: van der Laan 2009; Yu & Turk 2013; Oliveira & Paiva 2022; Nishidate & Fujishiro 2024; Wyman 2005; Imai 2016; Shah 2007; Kipfer & Westermann 2006; Wave Particles; Water Surface Wavelets; the Chentanez 2010, 2011 and 2014 abstracts; Narita 2025; Zhu & Bridson 2005; Kugelstadt 2019; MLS-MPM's "2× faster"; Gao 2018 paper details.
- Hardware and platform: AMD float atomics in driver 23.4.2 (RDNA 4 unconfirmed); the WebGPU 16 KB default limit; Taichi `+=` being atomic; pbmpm's fixed-point P2G.
- Projects and videos: Waseem & Hong 2026 (4M particles at > 54 fps, GPU unnamed); the Tencent 2023 course; Lague's video dates and his near-density and bitonic sort; Notch MPM wording; LiquiGen's APIC→FLIP history; Jolt buoyancy-factor semantics.
- Networking: the Unreal and Sea of Thieves forum claims.

**Not confirmed at all.** Particle counts and timings for Niagara 3D FLIP, PhysX 5 particles, FleX, Chentanez 2010 and tall cells. The Notch MPM affector text. Licences for the narrow-range demo code, WebGL Water, and the Ten Minute Physics heightfield tutorial (only the FLIP file's MIT header was checked). Every "(est.)" figure in §9 is my extrapolation and must be measured.

**Textbook relations used without a source:** Snell's law and total internal reflection, Torricelli's law and the ~0.6 orifice discharge coefficient, the Ritter dam-break front speed 2√(gh₀).
