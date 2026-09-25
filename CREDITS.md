# Credits

Forge is built on other people's work. This file names them:
- the libraries and tools in the build;
- the assets;
- the published techniques the code implements.

The Rust crates are listed with their authors and licences in
[`docs/credits-crates.md`](docs/credits-crates.md), which `cargo run -p credits` generates.
A game built on Forge carries these names in its credits.

A new dependency, asset or technique gets its line here in the same commit that brings it
in. CI checks the crate list.

## When a build ships

- **NVIDIA DLSS** (the `dlss` feature): the DLSS SDK's licence asks every application that
  uses it to attribute the SDK and to show the NVIDIA marks:
  - on the splash screen;
  - in the about box;
  - in a game's credits.

  The clause is 7.1(b) of the supplement to the DLSS licence that comes with the Streamline
  SDK (`bin/x64/nvngx_dlss.license.txt`).
  Development builds show none of it (owner, 2026-09-25): the splash and the about box come
  with the first public release, with the other tools and frameworks that ask for the same.
- **Permissive licences** (MIT, Apache-2.0, BSD, Zlib, ISC, BSL-1.0): the notices and licence
  texts go with the binaries. A tool such as `cargo-about` builds that file from the same
  metadata as the crate list.
- **JetBrains Mono** ships with its `OFL.txt` and is never sold on its own.

## Libraries and tools

| Project | People | What Forge uses it for | Licence |
|---|---|---|---|
| [Vulkan](https://www.vulkan.org/) and the [Vulkan SDK](https://vulkan.lunarg.com/) | The Khronos Group; LunarG | the GPU API; the validation layers behind every `--validate` run | Apache-2.0 and MIT components |
| [ash](https://github.com/ash-rs/ash) | Maik Klein, Benjamin Saunders, Marijn Suijten and contributors | Vulkan from Rust (`forge-gpu`) | MIT OR Apache-2.0 |
| [Slang](https://github.com/shader-slang/slang) | the Slang contributors (a Khronos project, started by Yong He, Kayvon Fatahalian and Tim Foley) | every shader in `shaders/`, compiled by `slangc` | Apache-2.0 WITH LLVM-exception |
| [meshoptimizer](https://github.com/zeux/meshoptimizer) | Arseny Kapoulkine | meshlets, simplification and cluster partitioning for the LOD DAG, following its `clusterlod.h` (`forge-geom`) | MIT |
| [meshopt](https://github.com/gwihlidal/meshopt-rs) | Graham Wihlidal | meshoptimizer from Rust | MIT OR Apache-2.0 |
| [gpu-allocator](https://github.com/Traverse-Research/gpu-allocator) | Traverse Research | GPU memory (`forge-gpu`) | MIT OR Apache-2.0 |
| [winit](https://github.com/rust-windowing/winit) | Pierre Krieger and the winit contributors | windows and input (`forge-app`) | Apache-2.0 |
| [glam](https://github.com/bitshifter/glam-rs) | Cameron Hart and contributors | vector and matrix maths | MIT OR Apache-2.0 |
| [crossbeam](https://github.com/crossbeam-rs/crossbeam) | the crossbeam contributors | the work-stealing deques and channels under `forge-task` | MIT OR Apache-2.0 |
| [Tracy](https://github.com/wolfpld/tracy) | Bartosz Taudul | the profiler behind `--features profiling` | BSD-3-Clause |
| [tracy-client](https://github.com/nagisa/rust_tracy_client) | Simonas Kazlauskas | Tracy from Rust | MIT OR Apache-2.0 |
| [Streamline](https://github.com/NVIDIA-RTX/Streamline) and DLSS | NVIDIA | DLSS as an option next to TAA (`dlss` feature, D-024) | Streamline: MIT, parts under NVIDIA's Nsight SDK licences; DLSS: NVIDIA RTX SDKs licence |
| [ab_glyph](https://github.com/alexheretic/ab-glyph) | Alex Butler | the overlay's font rasteriser | Apache-2.0 |
| [image](https://github.com/image-rs/image) | the image-rs developers | PNG captures and `imgdiff` | MIT OR Apache-2.0 |
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | Contributors to the OpenColorIO Project (Academy Software Foundation) | ACES 2.0's output transform, ported from its ACES2 code, v2.5.2, with its notice kept (`crates/forge-render/src/aces2.rs`, `shaders/aces2.slang`, issue #76) | BSD-3-Clause |
| [ꟻLIP](https://github.com/NVlabs/flip) | Pontus Ebelin (formerly Andersson), Jim Nilsson, Tomas Akenine-Möller, Magnus Oskarsson, Kalle Åström and Mark D. Fairchild (NVIDIA, Lund University, RIT) | LDR-ꟻLIP in `imgdiff`, ported from `FLIP.h` v1.7 with its notice kept (issue #75); its magma colour map is matplotlib's, by Nathaniel J. Smith and Stéfan van der Walt (CC0) | BSD-3-Clause |
| [xxhash-rust](https://github.com/DoumanAsh/xxhash-rust), after [xxHash](https://github.com/Cyan4973/xxHash) | Douman; the XXH3 algorithm by Yann Collet | cache keys for shaders and cooked meshes | BSL-1.0 |

## Assets

| Asset | People | Where | Licence |
|---|---|---|---|
| [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | the JetBrains Mono Project Authors | the profiler overlay's text | SIL OFL 1.1 (`assets/fonts/jetbrains-mono/OFL.txt`) |

## Techniques

The published work the code follows. The full references, with links, are in the research
files (`docs/research/`) and the decisions (`docs/DECISIONS.md`).

**Geometry** (`forge-geom`, `shaders/meshlet.slang`):
- **Nanite.** Brian Karis, Rune Stubbe, Graham Wihlidal, "A Deep Dive into Nanite
  Virtualized Geometry", SIGGRAPH 2021. Forge follows it for:
  - the cluster LOD DAG and its cut;
  - the software rasteriser on 64-bit atomics;
  - 128 KiB cluster pages and their streaming.
- **The DAG's build.** Arseny Kapoulkine, meshoptimizer's `clusterlod.h`.
- **GPU-driven culling.**
  - Ulrich Haar, Sebastian Aaltonen, "GPU-Driven Rendering Pipelines", SIGGRAPH 2015: the
    two-pass occlusion against a depth pyramid.
  - Graham Wihlidal, "Optimizing the Graphics Pipeline with Compute", GDC 2016: cluster
    culling in compute.
- **Ordered appends.** Duane Merrill, Michael Garland, "Single-pass Parallel Prefix Scan with
  Decoupled Look-back", 2016: the culls' ordered appends.
- **Morton order.** G. M. Morton, "A Computer Oriented Geodetic Data Base and a New Technique
  in File Sequencing", IBM, 1966: the city's instance table sorted along the Z-order curve so
  that its cells of 64 instances are compact (issue #38).
- **The visibility buffer.**
  - Christopher A. Burns, Warren A. Hunt, "The Visibility Buffer: A Cache-Friendly Approach
    to Deferred Shading", JCGT 2013.
  - Christoph Schied, Carsten Dachsbacher, "Deferred Attribute Interpolation for
    Memory-Efficient Deferred Shading", HPG 2015.
  - John Hable, "Visibility Buffer Rendering with Material Graphs", 2021.
- **Normals in cluster pages.** Quirin Meyer et al., "On Floating-Point Normal Vectors",
  EGSR 2010: the octahedral encoding.

**Light and image** (`forge-render`, `shaders/`):
- **The atmosphere.** Sébastien Hillaire, "A Scalable and Production Ready Sky and Atmosphere
  Rendering Technique", EGSR 2020. Its Earth preset, the transmittance table's mapping and
  the planet-view table's split at the horizon (issue #26) come from Eric Bruneton,
  "Precomputed Atmospheric Scattering: a New Implementation", 2017.
- **Physical light units and pre-exposure.** Sébastien Lagarde, Charles de Rousiers, "Moving
  Frostbite to Physically Based Rendering", SIGGRAPH 2014.
- **Automatic exposure.** Krzysztof Narkowicz, "Automatic Exposure", 2016.
- **Sky light.** Ravi Ramamoorthi, Pat Hanrahan, "An Efficient Representation for Irradiance
  Environment Maps", SIGGRAPH 2001: the sky's irradiance as nine spherical-harmonic
  coefficients.
- **Reflections.** Christophe Schlick, "An Inexpensive BRDF Model for Physically-based
  Rendering", Eurographics 1994: the Fresnel approximation. The specular occlusion follows
  Lagarde and de Rousiers (2014, above).
- **Translucent ice.** Colin Barré-Brisebois, Marc Bouchard, "Approximating Translucency for a
  Fast, Cheap and Convincing Subsurface Scattering Look", GDC 2011: thickness-driven
  translucency, here measured by rays.
- **Ice density.** P. Mullen, S. G. Warren, "Theory of the Optical Properties of Lake Ice",
  JGR 1988: bubbles scatter the light inside ice and set its look. J. H. Joseph,
  W. J. Wiscombe, J. A. Weinman, "The Delta-Eddington Approximation for Radiative Flux
  Transfer", J. Atmos. Sci. 1976: the forward peak kept in the straight beam. Henrik Wann
  Jensen, Stephen R. Marschner, Marc Levoy, Pat Hanrahan, "A Practical Model for Subsurface
  Light Transport", SIGGRAPH 2001: the diffusion approximation's effective attenuation. H. C.
  van de Hulst, "Light Scattering by Small Particles", 1957: large spheres block twice their
  cross-section.
- **Volumetric dust.** Bartlomiej Wronski, "Volumetric Fog: Unified Compute Shader-Based Solution
  to Atmospheric Scattering", SIGGRAPH 2014, and Sébastien Hillaire, "Towards Unified and
  Physically-Based Volumetric Lighting in Frostbite", SIGGRAPH 2015: the froxel volume and its
  integration. The phase function is Louis G. Henyey and Jesse L. Greenstein's, "Diffuse
  Radiation in the Galaxy", 1941.
- **Ambient occlusion.** Jorge Jimenez, Xian-Chun Wu, Angelo Pesce, Adrian Jarabo, "Practical
  Real-Time Strategies for Accurate Indirect Occlusion", SIGGRAPH 2016: GTAO and its
  multi-bounce fit. Forge ports Intel's implementation, XeGTAO (Filip Strugar and
  contributors; MIT, notice in `shaders/third-party/XeGTAO-LICENSE.txt`). Its sample noise
  combines a Hilbert curve with Martin Roberts' R2 sequence ("The Unreasonable Effectiveness
  of Quasirandom Sequences", 2018).
- **Diffuse light from probes.** Zander Majercik, Jean-Philippe Guertin, Derek Nowrouzezahrai,
  Morgan McGuire, "Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields",
  *Journal of Computer Graphics Techniques* 8(2), 2019, and Zander Majercik, Adam Marrs, Josef
  Spjut, Morgan McGuire, "Scaling Probe-Based Real-Time Dynamic Global Illumination for
  Production", *JCGT* 10(2), 2021: the probes, their visibility test, relocation,
  classification and bias (issue #53). `shaders/probes.slang` is written from the papers.
  NVIDIA's RTXGI-DDGI SDK (NVIDIA RTX SDKs License) was read for its constants; none of its
  code is used. The probes' maps use the octahedral mapping of Zina H. Cigolle, Sam Donow,
  Daniel Evangelakos, Michael Mara, Morgan McGuire, Quirin Meyer, "A Survey of Efficient
  Representations for Independent Unit Vectors", *JCGT* 3(2), 2014, and their rays turn by
  Ken Shoemake's uniform random rotations ("Uniform Random Rotations", *Graphics Gems III*,
  1992).
- **The sky's reflection dimmed by the probes.** Dimitar Lazarov, "Getting More Physical in
  Call of Duty: Black Ops II", SIGGRAPH 2013 Physically Based Shading course: reflection
  probes rescaled by the local irradiance over the probe's own, at the vertex normal. Unity
  HDRP's Adaptive Probe Volumes (after Michał Drobot, "Rendering of Call of Duty: Infinite
  Warfare", Digital Dragons 2017) read the local light along the mirror direction and let
  the ratio only darken. Forge takes both terms along the mirror direction, the probes'
  irradiance over the open sky's, per channel and at most 1 (issue #68); no code is used.
- **Triplanar normal maps.** Ben Golus, "Normal Mapping for a Triplanar Shader", 2017: the
  whiteout blend the materials' normal maps use.
- **Hex-tiling.** Morten S. Mikkelsen, "Practical Real-Time Hex-Tiling", *Journal of Computer
  Graphics Techniques* 11(2), 2022, after Eric Heitz and Fabrice Neyret, "High-Performance
  By-Example Noise using a Histogram-Preserving Blending Operator", *Proc. ACM Comput. Graph.
  Interact. Tech.* 1(2), 2018. The rock's texture is sampled in random hexagonal tiles so its
  repeats do not show (issue #66). `shaders/meshlet.slang` adapts the paper's reference code,
  [hextile-demo](https://github.com/mmikk/hextile-demo) (`hextiling.h`, MIT License,
  Copyright (c) 2022 mmikk; notice in `shaders/third-party/hextile-demo-LICENSE.txt`).
- **Tone curves.**
  - AgX: Troy Sobotka. Forge uses the minimal real-time form by Benjamin Wrensch (2023).
  - ACES 1.x: the Academy of Motion Picture Arts and Sciences, through Stephen Hill's fit
    (from BakingLab, MIT).
  - PBR Neutral: the Khronos Group.
  - ACES 2.0's output transform: the Academy's ACES project (the CTL, `aces-aswf/aces-core`,
    Apache-2.0). Its tonescale is Daniele Siragusano's, and its appearance model a
    simplified form of Luke Hellwig and Mark D. Fairchild's 2022 revision of CAM16. Forge
    ports OpenColorIO's implementation (see the table above) and checks it against OCIO's
    test values (issue #76).
- **Bloom.** Jorge Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare",
  SIGGRAPH 2014: the downsample and upsample chain, and its firefly weighting.
- **TAA.**
  - Brian Karis, "High-Quality Temporal Supersampling", SIGGRAPH 2014.
  - Jorge Jimenez, "Filmic SMAA", SIGGRAPH 2016.
  - Lasse Jon Fuglsang Pedersen (Playdead), "Temporal Reprojection Anti-Aliasing in INSIDE",
    GDC 2016.

**The render graph** (`forge-gpu::graph`, D-020):
- **The frame graph.** Yuriy O'Donnell, "FrameGraph: Extensible Rendering Architecture in
  Frostbite", GDC 2017: passes declaring what they read and write, barriers and transient
  memory derived from them.
- **Queues.** Hans-Kristian Arntzen, "Render graphs and Vulkan — a deep dive", 2017 (Granite),
  and Epic Games' Render Dependency Graph documentation: the author picks a pass's queue and
  the graph derives the waits between queues (issue #77).

**Core** (`forge-core`, `forge-task`):
- **Hashes.** Mark Jarzynski, Marc Olano, "Hash Functions for GPU Rendering", JCGT 2020: the
  PCG3D and PCG4D hashes, and the one-word PCG hash of the debugging image hash
  (`shaders/debug_hash.slang`, issue #71).
- **SplitMix64.** Guy L. Steele Jr., Doug Lea, Christine H. Flood, "Fast Splittable
  Pseudorandom Number Generators", OOPSLA 2014, with David Stafford's mix13 finaliser.
- **Work stealing.** David Chase, Yossi Lev, "Dynamic Circular Work-Stealing Deque", SPAA 2005.
- **The job model.** The continuation model follows Natalya Tatarchuk, "Destiny's Multithreaded
  Rendering Architecture", GDC 2015, and the engines surveyed in
  `docs/research/task-system.md`.
