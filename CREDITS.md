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
| [windows-sys](https://github.com/microsoft/windows-rs) | Microsoft | Windows' display configuration: the HDR switch and the SDR white level (`forge-gpu`, issue #94) | MIT OR Apache-2.0 |
| [glam](https://github.com/bitshifter/glam-rs) | Cameron Hart and contributors | vector and matrix maths | MIT OR Apache-2.0 |
| [crossbeam](https://github.com/crossbeam-rs/crossbeam) | the crossbeam contributors | the work-stealing deques and channels under `forge-task` | MIT OR Apache-2.0 |
| [Tracy](https://github.com/wolfpld/tracy) | Bartosz Taudul | the profiler behind `--features profiling` | BSD-3-Clause |
| [tracy-client](https://github.com/nagisa/rust_tracy_client) | Simonas Kazlauskas | Tracy from Rust | MIT OR Apache-2.0 |
| [Streamline](https://github.com/NVIDIA-RTX/Streamline) and DLSS | NVIDIA | DLSS as an option next to TAA (`dlss` feature, D-024) | Streamline: MIT, parts under NVIDIA's Nsight SDK licences; DLSS: NVIDIA RTX SDKs licence |
| [ab_glyph](https://github.com/alexheretic/ab-glyph) | Alex Butler | the overlay's font rasteriser | Apache-2.0 |
| [image](https://github.com/image-rs/image) | the image-rs developers | PNG captures and `imgdiff` | MIT OR Apache-2.0 |
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | Contributors to the OpenColorIO Project (Academy Software Foundation) | ACES 2.0's output transform, ported from its ACES2 code, v2.5.2, with its notice kept (`crates/forge-render/src/aces2.rs`, `shaders/aces2.slang`, issue #76) | BSD-3-Clause |
| [ꟻLIP](https://github.com/NVlabs/flip) | Pontus Ebelin (formerly Andersson), Jim Nilsson, Tomas Akenine-Möller, Magnus Oskarsson, Kalle Åström and Mark D. Fairchild (NVIDIA, Lund University, RIT) | LDR-ꟻLIP in `imgdiff`, ported from `FLIP.h` v1.7 with its notice kept (issue #75), and HDR-ꟻLIP (Pontus Andersson, Jim Nilsson, Peter Shirley and Tomas Akenine-Möller, Eurographics 2021) for the HDR captures (issue #126); its magma colour map is matplotlib's, by Nathaniel J. Smith and Stéfan van der Walt (CC0) | BSD-3-Clause |
| [FidelityFX Super Resolution 1](https://github.com/GPUOpen-Effects/FidelityFX-FSR) | AMD | its RCAS (robust contrast-adaptive sharpening), ported to `tools/sharpness --rcas` to preview a sharpening pass on a capture (issue #159), then to TAA's sharpening pass, `sharpen_main` in `shaders/taa.slang` (D-045) | MIT |
| [xxhash-rust](https://github.com/DoumanAsh/xxhash-rust), after [xxHash](https://github.com/Cyan4973/xxHash) | Douman; the XXH3 algorithm by Yann Collet | cache keys for shaders and cooked meshes | BSL-1.0 |
| [Jolt Physics](https://github.com/jrouwe/JoltPhysics) | Jorrit Rouwé and the Jolt contributors | rigid bodies (`forge-physics`, D-009, issue #136): v5.6.0's library sources vendored in `third_party/jolt` with its licence, built with `CROSS_PLATFORM_DETERMINISTIC` and double precision | MIT |
| [JoltC](https://github.com/SecondHalfGames/JoltC) | Second Half Games (Lucien Greathouse and contributors) | the model for `forge-physics`' C layer: opaque shape handles, the layer set-up | MIT OR Apache-2.0 |
| [gltf](https://github.com/gltf-rs/gltf) | David Harvey-Macaulay and the gltf-rs contributors | reading glTF 2.0 models (`forge_geom::model`, #138) | MIT OR Apache-2.0 |
| [Blender](https://www.blender.org/) | the Blender Foundation and its contributors | a tool, not in the build: `assets/blender/boat.py`, `car.py`, `plane.py`, `creatures.py` and `ship.py` model the lab's boat, car, aeroplane, mannequin, dog and spaceship in it and export them as glTF (#138, #140, #141, #143, #150) | GPL-2.0-or-later (the tool; what it makes is ours) |

## Assets

| Asset | People | Where | Licence |
|---|---|---|---|
| [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | the JetBrains Mono Project Authors | the profiler overlay's text | SIL OFL 1.1 (`assets/fonts/jetbrains-mono/OFL.txt`) |
| The lab's boat (`assets/models/boat.glb`) | made for Forge by `assets/blender/boat.py` (#138) | `physics-lab --lab sea` | the project's (MIT OR Apache-2.0) |
| The lab's spaceship (`assets/models/ship.glb`) | made for Forge by `assets/blender/ship.py` (#150, 2026-10-03) | `physics-lab --lab space` | the project's (MIT OR Apache-2.0) |

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
- **Superquadrics.** Alan H. Barr, "Superquadrics and Angle-Preserving Transformations", *IEEE
  Computer Graphics and Applications* 1(1), 1981: the superellipsoids the island's stones are
  shaped from, rounder for granite, squarer for limestone (`forge_geom::stone`, #130).
- **Smooth minimum.** Inigo Quilez, ["Smooth Minimum"](https://iquilezles.org/articles/smin/),
  2013: the quadratic polynomial smooth minimum that wears
  the edges of the rivers' broken stones round (`forge_geom::stone`, #133).
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
  contributors; MIT, notice in `shaders/third-party/XeGTAO-LICENSE.txt`). XeGTAO's sample noise,
  which combines a Hilbert curve with Martin Roberts' R2 sequence ("The Unreasonable
  Effectiveness of Quasirandom Sequences", 2018), still drives the clouds', the dust's and the
  dither's samples. GTAO's own samples use Jorge Jimenez's interleaved gradient noise ("Next
  Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014).
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
  - AgX: Troy Sobotka. Forge uses the minimal real-time form by Benjamin Wrensch (2023), and
    his "punchy" look (`--tonemap agx-punchy`), its values checked against Google Filament's
    `AgxToneMapper` (Apache 2.0).
  - ACES 1.x: the Academy of Motion Picture Arts and Sciences, through Stephen Hill's fit
    (from BakingLab, MIT).
  - PBR Neutral: the Khronos Group.
  - ACES 2.0's output transform: the Academy's ACES project (the CTL, `aces-aswf/aces-core`,
    Apache-2.0). Its tonescale is Daniele Siragusano's, and its appearance model a
    simplified form of Luke Hellwig and Mark D. Fairchild's 2022 revision of CAM16. Forge
    ports OpenColorIO's implementation (see the table above) and checks it against OCIO's
    test values (issue #76). Its HDR outputs follow the Academy's presets (issue #94).
- **HDR signals.** SMPTE ST 2084, the perceptual quantizer (Scott Miller, Mahdi Nezamabadi and
  Scott Daly, "Perceptual Signal Coding for More Efficient Usage of Bit Codes", *SMPTE Motion
  Imaging Journal* 122(4), 2013), and ITU-R BT.2100, BT.2087 and BT.2408 (Rec.2020 primaries,
  the conversion from Rec.709, the 203-nit reference white): the HDR10 output
  (`shaders/tonemap.slang`, `crates/forge-render/src/aces2.rs`, issue #94). CTA-861.3's
  MaxCLL and MaxFALL, measured from the frames shown (`shaders/hdr_metadata.slang`, #125).
- **HDR calibration.** The HDR Gaming Interest Group's guidelines ("For a Better HDR Gaming
  Experience": MaxTML, MinTML, the mark that disappears) and Unity's HDR Calibration Sample
  (Unity Technologies: the peak, black and paper-white pages): the calibration pages
  (`crates/forge-app/src/calibration.rs`, #125).
- **Bloom.** Jorge Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare",
  SIGGRAPH 2014: the downsample and upsample chain, and its firefly weighting.
- **TAA.**
  - Brian Karis, "High-Quality Temporal Supersampling", SIGGRAPH 2014.
  - Jorge Jimenez, "Filmic SMAA", SIGGRAPH 2016.
  - Lasse Jon Fuglsang Pedersen (Playdead), "Temporal Reprojection Anti-Aliasing in INSIDE",
    GDC 2016.
- **Measuring sharpness.** Peter D. Burns, "Slanted-Edge MTF for Digital Camera and Scanner
  Analysis", IS&T PICS 2000, pp. 135–138, the method of ISO 12233's e-SFR: `tools/sharpness`
  and the physics lab's sharpness room (#159).

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
- **Stream-power erosion.** Jean Braun, Sean D. Willett, "A very efficient O(n), implicit and
  parallel method to solve the stream power equation governing fluvial incision and landscape
  evolution", *Geomorphology* 180–181, 2013: the D8 receivers, the downstream-first stack and
  the implicit update of `forge_procgen::erosion`.
- **Priority flood.** Richard Barnes, Clarence Lehman, David Mulla, "Priority-flood: An optimal
  depression-filling and watershed-labeling algorithm for digital elevation models", *Computers
  & Geosciences* 62, 2014: `forge_procgen::flow::priority_flood`.
- **The basin graph.** Guillaume Cordonnier, Benoît Bovy, Jean Braun, "A versatile, linear
  complexity algorithm for flow routing in topographies with depressions", *Earth Surface
  Dynamics* 7, 2019: `forge_procgen::flow::drain` (the pit basins, their lowest passes, the
  spanning tree from the sea, carving).
- **Strahler orders and river widths.** Arthur N. Strahler, "Quantitative analysis of watershed
  geomorphology", *Transactions of the American Geophysical Union* 38, 1957; Luna B. Leopold,
  Thomas Maddock Jr., "The hydraulic geometry of stream channels and some physiographic
  implications", USGS Professional Paper 252, 1953: `forge_procgen::hydrology`.
- **The distance transform.** Pedro F. Felzenszwalb, Daniel P. Huttenlocher, "Distance
  Transforms of Sampled Functions", *Theory of Computing* 8, 2012: `forge_procgen::coast`.
- **Amplification by erosion.** Hugo Schott, Éric Galin, Éric Guérin, Axel Paris, Adrien
  Peytavie, "Terrain Amplification using Multi-scale Erosion", *ACM Transactions on Graphics*
  43(4), 2024 (DOI 10.1145/3658200): upsampling ×2 and eroding at the finer level under the
  coarser level's drainage, `forge_procgen::amplify`; with the talus rule of F. Kenton Musgrave,
  Craig E. Kolb, Robert S. Mace, "The synthesis and rendering of eroded fractal terrains",
  *Computer Graphics* 23(3) (SIGGRAPH '89), 41–50.
- **The topographic wetness index.** Keith J. Beven, Michael J. Kirkby, "A physically based,
  variable contributing area model of basin hydrology", *Hydrological Sciences Bulletin* 24(1),
  1979, 43–69 (DOI 10.1080/02626667909491834): ln(a / tan β), `forge_procgen::layers::wetness`.
- **Ocean waves.** Jerry Tessendorf, "Simulating Ocean Water", SIGGRAPH course notes,
  2001–2004 (the Fourier synthesis, the choppy displacement, the Jacobian's foam); Klaus
  Hasselmann et al., "Measurements of wind-wave growth and swell decay during the Joint North
  Sea Wave Project (JONSWAP)", *Deutsche Hydrographische Zeitschrift* Ergänzungsheft A8, 1973;
  Evert Bouws, Hans Günther, Wolfgang Rosenthal, Cornelis L. Vincent, "Similarity of the wind
  wave spectrum in finite depth water", *Journal of Geophysical Research* 90, 1985 (the TMA
  factor); Christopher J. Horvath, "Empirical directional wave spectra for computer graphics",
  *DigiPro* 2015 (the spreading with a swell parameter): `forge_procgen::ocean`. On the GPU
  (`shaders/water.slang`), Stockham's autosort FFT as laid out for graphics processors by
  Naga K. Govindaraju, Brandon Lloyd, Yuri Dotsenko, Burton Smith and John Manferdelli, "High
  Performance Discrete Fourier Transforms on Graphics Processors", SC 2008.
- **The sea's surface** (`shaders/water.slang`, issue #105). Frank Losasso and Hugues Hoppe,
  "Geometry clipmaps: terrain rendering using nested regular grids", *ACM Transactions on
  Graphics* 23(3), SIGGRAPH 2004 (the nested grids and their transitions); Eric Bruneton,
  Fabrice Neyret and Nicolas Holzschuch, "Real-time Realistic Ocean Lighting using Seamless
  Transitions from Geometry to BRDF", *Computer Graphics Forum* 29(2), 2010, 487–496 (the
  slopes' variance as roughness); Bruce Walter, Stephen R. Marschner, Hongsong Li and Kenneth
  E. Torrance, "Microfacet Models for Refraction through Rough Surfaces", EGSR 2007 (GGX), with
  Eric Heitz, "Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs",
  *JCGT* 3(2), 2014 (the height-correlated masking).
- **The shore's waves** (`forge_procgen::shore`, `shaders/water.slang`, issue #105). Trains of
  waves timed along the coast distance, after Carlos Gonzalez-Ochoa and Doug Holder, "Water
  Technology of Uncharted", GDC 2012; Gerstner's trochoidal waves (Franz Joseph von Gerstner,
  "Theorie der Wellen", 1802), as brought to graphics by Alain Fournier and William T. Reeves,
  "A simple model of ocean waves", SIGGRAPH 1986; their shoaling by the conservation of energy
  flux, which in shallow water is George Green, "On the motion of waves in a variable canal of
  small depth and width", *Transactions of the Cambridge Philosophical Society* 6, 1838,
  457–462; and the breaker index 0.78 of John McCowan, "On the highest wave of permanent
  type", *Philosophical Magazine* 38, 1894, 351–358.
- **Wet sand** (`shaders/meshlet.slang`, issue #105). Sébastien Lagarde, "Water drop 3a/3b –
  Physically based wet surfaces", blog, 2013
  (<https://seblagarde.wordpress.com/2013/03/19/water-drop-3a-physically-based-wet-surfaces/>):
  the water film darkens a porous surface and smooths it to water's reflection.
- **The rivers** (`forge_procgen::river`, `shaders/water.slang`, issue #105). Flow maps after
  Alex Vlachos, "Water Flow in Portal 2", SIGGRAPH 2010, *Advances in Real-Time Rendering in 3D
  Graphics and Games* (two phases of a texture advected along the flow and cross-faded); the
  courses smoothed by George M. Chaikin, "An algorithm for high speed curve generation",
  *Computer Graphics and Image Processing* 3, 1974, 346–349; the depth by the hydraulic
  geometry of Leopold and Maddock (1953, above) and the speed by Chézy's formula (Antoine de
  Chézy, 1775); far away, a ribbon kept a pixel wide with its coverage scaled, after Emil
  Persson's "Phone-Wire AA" demo, 2012 (<https://www.humus.name/index.php?page=3D&ID=89>).
- **The rivers' channels, mouths and stones** (`forge_procgen::channel`, `shaders/water.slang`,
  issue #105). The ground around the channels through the spline of Edwin Catmull and Raphael
  Rom, "A class of local interpolating splines", in *Computer Aided Geometric Design*
  (R. E. Barnhill and R. F. Riesenfeld, eds.), Academic Press, 1974, 317–326; the plumes at the
  mouths as turbulent plane jets, whose width grows linearly and whose centre speed falls as the
  root of the distance (Stephen B. Pope, *Turbulent Flows*, Cambridge University Press, 2000,
  chapter 5); the water around a stone as the potential flow past a cylinder (G. K. Batchelor,
  *An Introduction to Fluid Dynamics*, Cambridge University Press, 1967).
- **Wakes** (`shaders/wakes.slang`, `forge_render::wakes`, issue #107). Cem Yuksel, Donald H.
  House and John Keyser, "Wave Particles", *ACM Transactions on Graphics* 26(3), SIGGRAPH 2007
  (DOI 10.1145/1276377.1276501): particles that carry a piece of a wave front out from what
  moves through the water, split as the front spreads and are splatted into a height field.
  Each particle here carries a short packet of waves rather than one bump, as the packets of
  Stefan Jeschke and Chris Wojtan, "Water Wave Packets", *ACM Transactions on Graphics* 36(4),
  SIGGRAPH 2017 (<https://visualcomputing.ist.ac.at/publications/2017/WWP/>) carry a group of
  wave trains.
- **Splashes** (`shaders/splashes.slang`, `forge_render::splashes`, issue #107;
  `docs/research/water.md` §7).
  - **The spray as ballistic particles with drag, born where the water splashes:** Nuttapong
    Chentanez and Matthias Müller, "Real-time Simulation of Large Bodies of Water with Small
    Scale Details", SCA 2010 (<https://matthias-research.github.io/pages/publications/hfFluid.pdf>),
    whose emission from bodies sets the crown's speeds (0.2–0.6 of the body's).
  - **When an impact makes a crown:** Cyril Duez, Christophe Ybert, Christophe Clanet and
    Lydéric Bocquet, "Making a splash with water repellency", *Nature Physics* 3, 2007
    (<https://arxiv.org/abs/cond-mat/0701093>): only above a few metres a second.
  - **When the jet rises:** at 2 √(R / g), the cavity's pinch-off, from Tadd T. Truscott, Brenden
    P. Epps and Jesse Belden, "Water Entry of Projectiles", *Annual Review of Fluid Mechanics* 46,
    2014 (DOI 10.1146/annurev-fluid-011212-140753), and Rafsan Rabbi et al., "Impact force
    reduction by consecutive water entry of spheres", *Journal of Fluid Mechanics*, 2021
    (<https://arxiv.org/abs/2007.01943>). It is weaker for a buoyant body: Jeffrey M. Aristoff
    et al., "The water entry of decelerating spheres", *Physics of Fluids* 22, 2010.
  - **A bow's spray by its Froude number:** J. R. Chaplin and P. Teigen, "Steady flow past a
    vertical surface-piercing circular cylinder", *Journal of Fluids and Structures* 18, 2003.
  - **A fall's sheet breaking up over 6 q^0.32 m:** P. Horeni (1956), as given by Luis G.
    Castillo, José M. Carrillo and Álvaro Blázquez, "Plunge pool dynamic pressures: a temporal
    analysis in the nappe flow case", *Journal of Hydraulic Research* 53(1), 2015.
  - **Drops at least a pixel wide, their alpha scaled by the area they lack:** Emil Persson,
    "Phone-wire AA", 2012 (<https://www.humus.name/index.php?page=3D&ID=89>).
  - **Fast drops as streaks over the shutter:** Sarah Tariq, "Rain", NVIDIA DirectX 10 SDK
    whitepaper, 2007.
  - **Soft edges against the scene:** Tristan Lorach, "Soft Particles", NVIDIA DirectX 10 SDK
    whitepaper, 2007.
  - **A reactive mask that tells TAA to trust the current frame where the spray is:** as AMD's
    FidelityFX Super Resolution 2 does for its transparencies
    (<https://github.com/GPUOpen-Effects/FidelityFX-FSR2>), its value capped at 0.9.
  - **Each drop's random numbers:** the `pcg3d` hash of Mark Jarzynski and Marc Olano, "Hash
    Functions for GPU Rendering", *Journal of Computer Graphics Techniques* 9(3), 2020.

**Simulation and networking** (`forge-sim`, `forge-physics`, the physics lab):
- **The fixed tick.** Glenn Fiedler, "Fix Your Timestep!", Gaffer On Games, 2004: the
  simulation advanced in fixed steps from an accumulator, drawn between the last two (#136).
- **Prediction and reconciliation.** Yahn W. Bernier, "Latency Compensating Methods in
  Client/Server In-game Protocol Design and Optimization", GDC 2001, and Gabriel Gambetta,
  "Fast-Paced Multiplayer": the client applies its own commands at once and, corrected by the
  server, replays those not yet acknowledged (#137).
- **Rollback over a deterministic simulation.** Timothy Ford, "Overwatch Gameplay Architecture
  and Netcode", GDC 2017, and Jared Cone, "It IS Rocket Science! The Physics of Rocket League
  Detailed", GDC 2018: the predicted state taken back to the server's and run forward again
  (#137); Forge skips the replay when the snapshot's digest is the one predicted.
- **Networked physics.** Glenn Fiedler, "Introduction to Networked Physics" and the series
  after it, Gaffer On Games, 2014–2015: the link conditioner and the commands sent again until
  acknowledged (#137).
- **Floating bodies.** Jacques Kerner, "Water interaction model for boats in video games",
  Game Developer (Gamasutra), 2015: closed hulls cut at the water's surface, each submerged
  piece pushed by the pressure at its depth and dragged by the water it moves through
  (`forge_physics::buoyancy`, #138).
- **Lift and drag.** Thin-aerofoil theory and Prandtl's lifting line, as taught in John D.
  Anderson, *Fundamentals of Aerodynamics* (McGraw-Hill): a lift slope of 2π a radian, the drag
  a wing's lift induces by its aspect ratio with an Oswald efficiency, a flat plate's lift and
  drag past the stall (`forge_physics::aero`, #141).
- **Vehicles.** Jolt's `VehicleConstraint` and `WheeledVehicleController` (Jorrit Rouwé), set up
  after Jolt's own vehicle sample: the wheels, the anti-roll bars, the cylinder cast (#140).
- **Fracture.** Pieces cut ahead of time as the Voronoi cells of points inside the solid and
  swapped in for it on a blow, the common practice of game destruction (Blender's Cell Fracture
  add-on, NVIDIA's Blast); see Matthias Müller, Nuttapong Chentanez and Tae-Yong Kim, "Real
  Time Dynamic Fracture with Volumetric Approximate Convex Decompositions", SIGGRAPH 2013, for
  Voronoi fracture patterns and convex pieces (`forge_geom::fracture`, #142). Joints that break
  past a load or a strain, as engines' breakable constraints do (#142).
- **Powered ragdolls.** Jolt's `Ragdoll`, `Skeleton` and motorised swing-twist and hinge
  constraints (Jorrit Rouwé), set up after Jolt's ragdoll samples (#143); D-012's physics layer.
- **Clouds.** Andrew Schneider and Nathan Vos, "The Real-time Volumetric Cloudscapes of Horizon:
  Zero Dawn", SIGGRAPH 2015 Advances in Real-Time Rendering: a layer from a weather map,
  Perlin–Worley shapes eroded by Worley noise, Beer's law with the powder term, temporal
  reprojection; Sébastien Hillaire, "Physically Based Sky, Atmosphere and Cloud Rendering in
  Frostbite", SIGGRAPH 2016: multiple scattering as Magnus Wrenninge's octaves ("Oz: The Great
  and Volumetric", SIGGRAPH 2013 Talks) (`forge_render::clouds`, #145).
- **Shallow water.** Matthias Müller-Fischer, "Fast Water Simulation for Games Using Height
  Fields", GDC 2008: depths on a grid, velocities on its faces carried along semi-Lagrangian,
  upwind fluxes kept from overdrawing a cell (`forge_physics::shallow`, #144). A. Ritter's
  dam-break solution (1892) for its test.
- **The lab's particle liquid** (`forge_render::liquid`, #156, D-044):
  - **APIC:** Chenfanfu Jiang, Craig Schroeder, Andrew Selle, Joseph Teran and Alexey
    Stomakhin, "The Affine Particle-In-Cell Method", ACM SIGGRAPH 2015. The particles carry their
    velocity's gradient to and from a MAC grid, with trilinear weights.
  - **The volume:** after Tassilo Kugelstadt, Andreas Longva, Nils Thuerey and Jan Bender,
    "Implicit Density Projection for Volume Conserving Liquids", IEEE TVCG 27(4), 2019, and
    Matthias Müller's Ten Minute Physics FLIP tutorial (#18, its drift compensation, MIT).
    Forge moves the particles down their crowding's gradient instead (no code taken).
  - **The pressure's multigrid** (opt-in, `--liquid-cycles`): after Aleka McAdams, Eftychios
    Sifakis and Joseph Teran, "A Parallel Multigrid Poisson Solver for Fluids Simulation on Large
    Grids", SCA 2010. A coarse cell is air where any of its eight is, and the correction comes
    back trilinear (no code taken).
  - **Pure water's absorption:** Robin M. Pope and Edward S. Fry, "Absorption spectrum (380–700
    nm) of pure water. II. Integrating cavity measurements", Applied Optics 36(33), 1997.
  - **The bench:** a floor of squares in four tints, a plain background and tinted water to tune
    by, after Sebastian Lague's fluid simulation videos (the owner's pointer, 2026-10-03; nothing
    taken but the idea).
