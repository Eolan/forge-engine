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
- **NVIDIA NRD** (the `nrd` feature, D-049): a game that ships NRD's SIGMA or REBLUR ships `NRD.dll` as
  object code only, under terms at least as protective of NVIDIA as the NVIDIA RTX SDKs License
  (`nrd-sdk/src/LICENSE.txt`), and never under an open-source licence. NRD never enters this
  repository; `tools/fetch-nrd.sh` builds it. This is not a legal reading.
- **NVIDIA RTXDI** (`--lamp-restir`, `--street-restir`, D-065): its shader code is compiled
  into `restir.slang`'s SPIR-V. A build that ships it ships it as object code, under the NVIDIA
  RTX SDKs License (`rtxdi-sdk/LICENSE.txt`). It is never under an open-source licence, and
  modified files keep NVIDIA's notice. RTXDI never enters this repository. This is not a legal
  reading.
- **AMD's FSR DLLs** (the `ffx` feature, `--fsr-amd`, D-082): `amd_fidelityfx_loader_vk.dll`
  and `amd_fidelityfx_upscaler_vk.dll` are expected on the player's computer, as Streamline's
  are. Forge uses them locally and never publishes them (the owner, 2026-10-29): nothing of
  AMD's FSR SDK, its ML upscaler included, enters this repository (`fsr-sdk/`, git-ignored).
  Their notice is the SDK's MIT one (`Kits/FidelityFX/docs/license.md`), which grants no patent
  licence: AMD's patents on its ML upscaling are to be looked up before a release relies on them.
- **Steam Audio** (the `steam-audio` feature, #267): `phonon.dll` is Valve's (Apache-2.0) and
  carries other people's code under their terms, listed in the SDK's `THIRDPARTY.md`: Intel IPP
  (the Intel Simplified Software License: redistributed unmodified with its notice, no reverse
  engineering), FFTS and PFFFT (BSD), MySOFA (BSD), Intel Embree (Apache-2.0), AMD Radeon Rays
  (MIT), AMD TrueAudio Next, the CIPIC HRTF database and Google's spherical harmonics library.
  A build that ships the library ships that file with it. The SDK never enters this repository.
- **Permissive licences** (MIT, Apache-2.0, BSD, Zlib, ISC, BSL-1.0): the notices and licence
  texts go with the binaries. A tool such as `cargo-about` builds that file from the same
  metadata as the crate list.
- **JetBrains Mono** ships with its `OFL.txt` and is never sold on its own.

## Giving back

The owner's commitment (2026-10-10): if something built with Forge is sold, and it relied
heavily on a free, open-source tool, library or asset, part of the profits goes to that project's
authors, where they accept money (a donation, sponsorship or their foundation), as far as the
profits allow. The projects in this file that a release leaned on most are the first to check.
This is a promise of intent, not a licence term.

On the list since 2026-10-30: Bevy (the Bevy Foundation takes donations) and kajiya's
authors at Embark Studios, whose caches D-088's surface cache is ported from.

## Libraries and tools

| Project | People | What Forge uses it for | Licence |
|---|---|---|---|
| [Vulkan](https://www.vulkan.org/) and the [Vulkan SDK](https://vulkan.lunarg.com/) | The Khronos Group; LunarG | the GPU API; the validation layers behind every `--validate` run | Apache-2.0 and MIT components |
| [ash](https://github.com/ash-rs/ash) | Maik Klein, Benjamin Saunders, Marijn Suijten and contributors | Vulkan from Rust (`forge-gpu`), and `ash-window`: their master branch, pinned by commit in `Cargo.toml` (#361) | MIT OR Apache-2.0 |
| [Slang](https://github.com/shader-slang/slang) | the Slang contributors (a Khronos project, started by Yong He, Kayvon Fatahalian and Tim Foley) | every shader in `shaders/`, compiled by `slangc` | Apache-2.0 WITH LLVM-exception |
| [meshoptimizer](https://github.com/zeux/meshoptimizer) | Arseny Kapoulkine | meshlets, simplification and cluster partitioning for the LOD DAG, following its `clusterlod.h` (`forge-geom`) | MIT |
| [meshopt](https://github.com/gwihlidal/meshopt-rs) | Graham Wihlidal | meshoptimizer from Rust | MIT OR Apache-2.0 |
| [gpu-allocator](https://github.com/Traverse-Research/gpu-allocator) | Traverse Research | GPU memory (`forge-gpu`): 0.28.0 vendored in `third_party/gpu-allocator` with its licences, two calls renamed for ash's master (#361) | MIT OR Apache-2.0 |
| [intel_tex_2](https://github.com/Traverse-Research/intel-tex-rs-2) over Intel's [ISPC Texture Compressor](https://github.com/GameTechDev/ISPCTextureCompressor) | Traverse Research; Intel | BC7 encoding of the normal maps' rebuilt levels (`forge_render::dds::bc5_normals_to_bc7`, D-086) | MIT OR Apache-2.0 (the bindings); MIT (the compressor) |
| [winit](https://github.com/rust-windowing/winit) | Pierre Krieger and the winit contributors | windows and input (`forge-app`) | Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) | Microsoft | Windows' display configuration: the HDR switch and the SDR white level (`forge-gpu`, issue #94) | MIT OR Apache-2.0 |
| [glam](https://github.com/bitshifter/glam-rs) | Cameron Hart and contributors | vector and matrix maths | MIT OR Apache-2.0 |
| [crossbeam](https://github.com/crossbeam-rs/crossbeam) | the crossbeam contributors | the work-stealing deques and channels under `forge-task` | MIT OR Apache-2.0 |
| [Tracy](https://github.com/wolfpld/tracy) | Bartosz Taudul | the profiler behind `--features profiling` | BSD-3-Clause |
| [tracy-client](https://github.com/nagisa/rust_tracy_client) | Simonas Kazlauskas | Tracy from Rust | MIT OR Apache-2.0 |
| [Streamline](https://github.com/NVIDIA-RTX/Streamline) and DLSS | NVIDIA | DLSS as an option next to TAA (`dlss` feature, D-024) | Streamline: MIT, parts under NVIDIA's Nsight SDK licences; DLSS: NVIDIA RTX SDKs licence |
| [FidelityFX Denoiser](https://github.com/GPUOpen-Effects/FidelityFX-Denoiser), its shadow denoiser (commit d7dfecb) | Advanced Micro Devices | the sun's soft shadows denoised where NRD is absent: `shaders/ffx_shadows.slang`, ported to Slang with AMD's notice (D-049, #173) | MIT |
| [NRD](https://github.com/NVIDIA-RTX/NRD) (Real-time Denoisers) v4.18.0 (master at d3df343, #210) | NVIDIA | SIGMA, the sun's soft shadows denoised (`nrd` feature, D-049, #172), and REBLUR, D-088's diffuse bounce denoised (`r.gi.trace`); fetched and built outside git by `tools/fetch-nrd.sh`, loaded at run time | NVIDIA RTX SDKs License |
| [RTXDI](https://github.com/NVIDIA-RTX/RTXDI-Library) (RTXDI-Library 3.1, commit f12037f) | NVIDIA | ReSTIR DI over the lamps (D-065): the comparison with the light lists (`--lamp-restir`) and the street lamps, however far (`--street-restir`); `shaders/restir.slang` includes its headers from `rtxdi-sdk/`, kept outside git, through Forge's own bridge | NVIDIA RTX SDKs License |
| [Bevy](https://github.com/bevyengine/bevy)'s Solari (v0.20.1, commit a0577b5) | the Bevy contributors | D-088's surface cache: its world cache (`crates/bevy_solari/src/realtime/world_cache_*.wesl`) ported to `shaders/surface_cache.slang`, `surface_cache_update.slang` and `surface_cache_trace_main` (`meshlet.slang`), the changes in their header; licence in `shaders/third-party/Bevy-LICENSE-APACHE.txt` | MIT OR Apache-2.0 (taken under Apache-2.0) |
| [kajiya](https://github.com/EmbarkStudios/kajiya) (commit 9fdec0f, archived) | Embark Studios and contributors | D-088's surface cache: its irradiance cache's dense index of camera-centred 32³ levels (`assets/shaders/ircache/ircache_grid.hlsl`) in `shaders/surface_cache.slang`; licence in `shaders/third-party/kajiya-LICENSE-APACHE.txt` | MIT OR Apache-2.0 (taken under Apache-2.0) |
| [ab_glyph](https://github.com/alexheretic/ab-glyph) | Alex Butler | the overlay's font rasteriser | Apache-2.0 |
| [image](https://github.com/image-rs/image) | the image-rs developers | PNG captures and `imgdiff`; decoding the PNG and JPEG images inside glTF models (`forge_render::textures::decode_image`, D-047) | MIT OR Apache-2.0 |
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | Contributors to the OpenColorIO Project (Academy Software Foundation) | ACES 2.0's output transform, ported from its ACES2 code, v2.5.2, with its notice kept (`crates/forge-render/src/aces2.rs`, `shaders/aces2.slang`, issue #76) | BSD-3-Clause |
| [ꟻLIP](https://github.com/NVlabs/flip) | Pontus Ebelin (formerly Andersson), Jim Nilsson, Tomas Akenine-Möller, Magnus Oskarsson, Kalle Åström and Mark D. Fairchild (NVIDIA, Lund University, RIT) | LDR-ꟻLIP in `imgdiff`, ported from `FLIP.h` v1.7 with its notice kept (issue #75), and HDR-ꟻLIP (Pontus Andersson, Jim Nilsson, Peter Shirley and Tomas Akenine-Möller, Eurographics 2021) for the HDR captures (issue #126); its magma colour map is matplotlib's, by Nathaniel J. Smith and Stéfan van der Walt (CC0) | BSD-3-Clause |
| [FidelityFX SDK v1.1.4](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK), FSR 3.1.4's upscaler | AMD | FSR 3.1 in place of TAA on every GPU (D-070, #286): AMD's shader headers in `shaders/third-party/fidelityfx/` (the upscaler's, the single-pass downsampler's, FSR 1's RCAS and the core), compiled through Forge's bridge; one change marked in `ffx_fsr3upscaler_accumulate.h` (a min/max history box); notice in `shaders/third-party/FidelityFX-SDK-LICENSE.txt` | MIT |
| [FidelityFX SDK 2.4.0](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) (tag `v2.4.0`), its signed upscaler DLLs | AMD | under `r.aa=fsr` on any GPU wherever the system has them, FSR 4 where they offer it, else their FSR 3.1.6, loaded at run time (D-082, #361): `amd_fidelityfx_loader_vk.dll` and `amd_fidelityfx_upscaler_vk.dll` from `fsr-sdk/` (git-ignored), the FFX API's types declared in `crates/forge-gpu/src/ffx.rs` after the SDK's C headers | MIT |
| [FidelityFX Super Resolution 1](https://github.com/GPUOpen-Effects/FidelityFX-FSR) | AMD | its RCAS (robust contrast-adaptive sharpening), ported to `tools/sharpness --rcas` to preview a sharpening pass on a capture (issue #159), then to TAA's sharpening pass, `sharpen_main` in `shaders/taa.slang` (D-045) | MIT |
| [xxhash-rust](https://github.com/DoumanAsh/xxhash-rust), after [xxHash](https://github.com/Cyan4973/xxHash) | Douman; the XXH3 algorithm by Yann Collet | cache keys for shaders, cooked meshes and derived products (#208), and their checksums | BSL-1.0 |
| [lz4_flex](https://github.com/pseitz/lz4_flex), after [LZ4](https://github.com/lz4/lz4) | Pascal Seitz, Arthur Silva and ticki; the LZ4 format by Yann Collet | the caches' packing (`forge_core::pack`, #215): the derived products' frames and each cooked mesh page | MIT |
| [toml](https://github.com/toml-rs/toml) | Ed Page, Alex Crichton and the toml-rs contributors | the world files (`assets/worlds/`, D-053, issue #211) | MIT OR Apache-2.0 |
| [ron](https://github.com/ron-rs/ron) (Rusty Object Notation) | Thomas Schaller, Dzmitry Malyshau, Christopher Durham, Juniper Tyree and the ron-rs contributors | the content records' format (D-035), first the missions (`forge-mission`, D-072, issue #287) | MIT OR Apache-2.0 |
| [mlua](https://github.com/mlua-rs/mlua) (Lua and Luau bindings for Rust) | Aleksandr Orlenko, kyren and the mlua contributors | the sandboxed Luau host (`forge-script`, D-072, issue #292) | MIT |
| [Luau](https://github.com/luau-lang/luau), through [luau0-src](https://github.com/mlua-rs/luau-src-rs) | Roblox Corporation; Lua by Lua.org, PUC-Rio; luau0-src by Aleksandr Orlenko | the script language (D-072), built from its sources | MIT |
| [luau-lsp](https://github.com/JohnnyMorganz/luau-lsp) 1.70.1 | JohnnyMorganz and the luau-lsp contributors | type-checks the game scripts against the game API's definitions (`tools/luau-check.sh`, in the gate and CI; D-072, issue #319), downloaded on the owner's go, not shipped | MIT |
| [Jolt Physics](https://github.com/jrouwe/JoltPhysics) | Jorrit Rouwé and the Jolt contributors | rigid bodies (`forge-physics`, D-009, issue #136): v5.6.0's library sources vendored in `third_party/jolt` with its licence, built with `CROSS_PLATFORM_DETERMINISTIC` and double precision | MIT |
| [JoltC](https://github.com/SecondHalfGames/JoltC) | Second Half Games (Lucien Greathouse and contributors) | the model for `forge-physics`' C layer: opaque shape handles, the layer set-up | MIT OR Apache-2.0 |
| [gltf](https://github.com/gltf-rs/gltf) | David Harvey-Macaulay and the gltf-rs contributors | reading glTF 2.0 models (`forge_geom::model`, #138) | MIT OR Apache-2.0 |
| [zune-jpeg](https://github.com/etemesi254/zune-image) | Caleb Etemesi | the JPEG decoder under `image`, for the JPEG images models embed (D-047) | MIT OR Apache-2.0 OR Zlib |
| [i_overlay](https://github.com/iShape-Rust/iOverlay) | Nail Sharipov and the iShape contributors | polygon booleans and strokes for the city's street ground (`forge_procgen::city::ground`, D-061): the streets' union, carriageways, crossings and blocks, in its integer mode | MIT OR Apache-2.0 |
| [quinn](https://github.com/quinn-rs/quinn) | Benjamin Saunders, Dirkjan Ochtman and the quinn contributors | QUIC with its streams and datagrams (`forge-net`, D-010, #268); the link conditioner sits under its socket trait | MIT OR Apache-2.0 |
| [rustls](https://github.com/rustls/rustls) | Joseph Birr-Pixton, Dirkjan Ochtman, Daniel McCarney, Josh Aas and the rustls contributors | TLS 1.3 under QUIC (`forge-net`) | Apache-2.0 OR ISC OR MIT |
| [ring](https://github.com/briansmith/ring) | Brian Smith; parts from BoringSSL (Google) and OpenSSL | the TLS cryptography, and Ed25519 for the connection tokens (`forge_net::token`) | Apache-2.0 AND ISC |
| [rcgen](https://github.com/rustls/rcgen) | est31 and the rustls contributors | self-signed server certificates made at run time (`forge_net::ServerIdentity`) | MIT OR Apache-2.0 |
| [tokio](https://github.com/tokio-rs/tokio) | Carl Lerche and the tokio contributors | the network's I/O runtime (`forge-net`) | MIT |
| [bytes](https://github.com/tokio-rs/bytes) | Carl Lerche, Sean McArthur and the tokio contributors | messages' buffers (`forge-net`) | MIT |
| [rtrb](https://github.com/mgeier/rtrb) | Matthias Geier and Stjepan Glavina | the wait-free single-producer, single-consumer rings between the game, decode and audio threads (`forge-audio`, #267) | MIT OR Apache-2.0 |
| [lewton](https://github.com/RustAudio/lewton) and [ogg](https://github.com/RustAudio/ogg) | est31 and the RustAudio contributors | Ogg Vorbis decoding, and the Ogg pages under the Opus decoder (`forge_audio::decode`, #267) | lewton: MIT OR Apache-2.0; ogg: BSD-3-Clause |
| [opus-rs](https://github.com/restsend/opus-rs) | jinti (restsend), porting libopus 1.6 by the Xiph.Org Foundation, Skype, Octasic, Jean-Marc Valin, Timothy B. Terriberry and the other Opus authors | Opus decoding in pure Rust (`forge_audio::decode::OggOpusDecoder`, #267); the Opus patents of Xiph.Org, Broadcom and Microsoft are licensed royalty-free (`docs/research/audio.md` §12) | BSD-3-Clause |
| [cpal](https://github.com/RustAudio/cpal) | the RustAudio contributors | the audio device (WASAPI) under `forge-audio`'s `device` feature, on Windows only: the app's sound (#318) and the `audio-demo` example; with Microsoft's `windows` crates under it (#267) | Apache-2.0 (the `windows` crates: MIT OR Apache-2.0) |
| [Steam Audio](https://github.com/ValveSoftware/steam-audio) 4.8.1 | Valve Corporation, with the third parties of its `THIRDPARTY.md` | the HRTF behind `forge_audio::steam::SteamSpatializer` (the `steam-audio` feature, #267, D-011): `phonon.dll` from the SDK, kept in `steam-audio-sdk/` and never committed | Apache-2.0 (its third parties: see "When a build ships") |
| [audionimbus](https://github.com/MaxenceMaire/audionimbus) | Maxence Maire | the safe Rust bindings to Steam Audio, generated by `bindgen` (#267) | MIT OR Apache-2.0 |
| [LLVM](https://llvm.org/) | the LLVM Project | a tool, not in the build: its `libclang` lets `bindgen` read Steam Audio's header where `steam-audio` builds (#267) | Apache-2.0 with LLVM exceptions |
| [Blender](https://www.blender.org/) | the Blender Foundation and its contributors | a tool, not in the build: `assets/blender/boat.py`, `car.py`, `plane.py` and `ship.py` model the lab's boat, car, aeroplane and spaceship in it and export them as glTF (#138, #140, #141, #150); `skinned_creatures.py` the mannequin and the dog as skinned bodies with their armatures, bone-heat weights and clips (#143, #165), unwrapped by its smart UV projection and their textures baked by Cycles (#166); its audaspace and FFmpeg (libvorbis, libopus) encoded the tone `forge-audio` made into the Ogg Vorbis and Ogg Opus test files in `crates/forge-audio/tests/data/` (#267) | GPL-2.0-or-later (the tool; what it makes is ours) |

## Assets

| Asset | People | Where | Licence |
|---|---|---|---|
| [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) | the JetBrains Mono Project Authors | the profiler overlay's text | SIL OFL 1.1 (`assets/fonts/jetbrains-mono/OFL.txt`) |
| The lab's boat (`assets/models/boat.glb`) | made for Forge by `assets/blender/boat.py` (#138) | `physics-lab --lab sea` | the project's (MIT OR Apache-2.0) |
| The lab's spaceship (`assets/models/ship.glb`) | made for Forge by `assets/blender/ship.py` (#150, 2026-10-03) | `physics-lab --lab space` | the project's (MIT OR Apache-2.0) |
| The lab's creatures (`assets/models/skinned-creatures.glb`: the mannequin and the dog, their armatures and clips, their wood and fur textures) | made for Forge by `assets/blender/skinned_creatures.py` (#165, #166, 2026-10-04) | `physics-lab --lab creatures` | the project's (MIT OR Apache-2.0) |
| [The Yale Bright Star Catalogue](https://heasarc.gsfc.nasa.gov/W3Browse/star-catalog/bsc5p.html), 5th revised edition (`assets/sky/bsc5p.txt`: positions, magnitudes, B − V) | Dorrit Hoffleit and Wayne H. Warren Jr. (1991), NASA HEASARC's copy (BSC5P) | the real night sky (`--real-sky`, #164, D-046) | public: a NASA data product, freely distributed |
| The labs' external models (D-048, #170): not in the repository, fetched by `tools/fetch-assets.sh` from the [Khronos glTF Sample Assets](https://github.com/KhronosGroup/glTF-Sample-Assets) at commit `edc7c9e`, each with its `LICENSE.md` | TextureCoordinateTest, TextureSettingsTest, NormalTangentTest, NormalTangentMirrorTest and MetalRoughSpheres by Ed Mackey (Analytical Graphics, Inc.); TextureTransformTest and WaterBottle by Microsoft; Fox by PixelMannen (model), tomkranis (rigging and animation), AsoboStudio and scurest (glTF conversion); CesiumMan by Cesium; FlightHelmet converted from Maya by Gary Hsu | `physics-lab --lab models`, for the labs and the tests only | CC0 1.0 or CC-BY 4.0, per model (the Cesium logo excluded) |
| Amazon Lumberyard Bistro, Open Research Content Archive (ORCA), v5.2 (D-083): not in the repository, fetched by hand from [NVIDIA's ORCA page](https://developer.nvidia.com/orca/amazon-lumberyard-bistro) into `assets/external/bistro/` and converted by `tools/fbx-to-gltf.py`; the page's three exterior renders kept beside it as references | Amazon Lumberyard (the scene, revealed at GDC 2017); NVIDIA (ORCA); the sky map from [HDRI Haven](https://hdrihaven.com) | `physics-lab --lab bistro`, an authored scene to measure Forge against | CC-BY 4.0 |
| Sponza, a reference model (D-048: fetched only with `--reference-only`, never committed, never in the engine or a game) | Crytek; Frank Meinl, after Marko Dabrovic's original | `physics-lab --lab models --model Sponza` | Cryengine Limited License Agreement |
| [NASA's CGI Moon Kit](https://svs.gsfc.nasa.gov/4720) (`assets/sky/moon-albedo-512x256.r8`, its 2025 colour map's luminance, by `assets/blender/moon_albedo.py`) | NASA's Scientific Visualization Studio, from the Lunar Reconnaissance Orbiter Camera's data | the real Moon (`--real-sky`) | NASA imagery, credit "NASA's Scientific Visualization Studio" |
| [NOAA's ETOPO 2022 Global Relief Model](https://www.ncei.noaa.gov/products/etopo-global-relief-model), 60 arc-seconds, ice surface: not in the repository, fetched into `assets/planets/earth/` by `tools/fetch-planets.sh` and converted by `assets/blender/planet_elevation.py` | NOAA National Centers for Environmental Information (2022) | the Earth of the `planet` demo (#220, D-056) | public domain (U.S. government work) |
| [Copernicus DEM GLO-90](https://dataspace.copernicus.eu/explore-data/data-collections/copernicus-contributing-missions/collections-description/COP-DEM), 3 arc-seconds, 25 one-degree tiles from 41° to 47° N and 5° to 10° E ([AWS Open Data](https://registry.opendata.aws/copernicus-dem/)): not in the repository, fetched into `assets/planets/earth/glo90/` by `tools/fetch-planets.sh` and mosaicked by `assets/blender/planet_region.py` | Produced using Copernicus WorldDEM-90 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved | the Alps, the Côte d'Azur and Corsica at 90 m in the `planet` demo's tour (#220) | free under the Copernicus licence (ESA's licence for the Copernicus DEM) |
| [NASA's Blue Marble Next Generation](https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/), July 2004, without its relief shaded, 21 600 × 10 800 (`world.200407.3x21600x10800.jpg`): not in the repository, fetched into `assets/planets/earth/` by `tools/fetch-planets.sh` and resampled to 16 384 × 8 192 by `assets/blender/planet_colour.py` | NASA Earth Observatory (Reto Stöckli and colleagues, from the MODIS instruments' data) | the Earth's colour from afar in the `planet` demo | NASA imagery, credit "NASA Earth Observatory" |
| [NASA's Deep Star Maps 2020](https://svs.gsfc.nasa.gov/4851), the 8K celestial map (`starmap_2020_8k.exr`): not in the repository, fetched into `assets/planets/sky/` by `tools/fetch-planets.sh` | NASA/Goddard Space Flight Center Scientific Visualization Studio, from ESA's Gaia DR2 and Hipparcos and other catalogues (ESA/Gaia/DPAC) | the stars and the Milky Way of the `planet` demo (`forge_render::SkyBox`) | NASA imagery, credit "NASA/Goddard Space Flight Center Scientific Visualization Studio"; Gaia data credit "ESA/Gaia/DPAC" |
| [NASA's CGI Moon Kit](https://svs.gsfc.nasa.gov/4720): its elevation at 16 samples a degree (`ldem_16_uint.tif`) and its 2025 colour map at 4K: not in the repository, fetched into `assets/planets/moon/` by `tools/fetch-planets.sh` and converted by `assets/blender/planet_elevation.py` and `planet_colour.py` | NASA's Scientific Visualization Studio, from the Lunar Reconnaissance Orbiter's laser altimeter (LOLA) and camera (LROC) | the Moon of the `planet` demo (`--world assets/worlds/moon.toml`) | NASA imagery, credit "NASA's Scientific Visualization Studio" |
| A blackbird's song ([Common Blackbird song (Turdus merula).ogg](https://commons.wikimedia.org/wiki/File:Common_Blackbird_song_(Turdus_merula).ogg), from [xeno-canto XC1059970](https://xeno-canto.org/1059970)): not in the repository, fetched into `assets/sounds/birds/` by `tools/fetch-sounds.sh` | Diana Tudor (2022) | the birds bed's blackbird (`forge_audio::ambience::BirdCalls`, #267) | CC BY 4.0 |
| A house sparrow's *tschilp* call ([Joseph Sardin - Passer domesticus tschilp call.oga](https://commons.wikimedia.org/wiki/File:Joseph_Sardin_-_Passer_domesticus_tschilp_call.oga)) and a morning chorus of sparrows and pigeons ([Réveil des oiseaux.ogg](https://commons.wikimedia.org/wiki/File:R%C3%A9veil_des_oiseaux.ogg)): fetched the same way | Joseph Sardin | the birds bed's sparrow and its far dawn chorus (#267) | CC0 1.0 |
| A *Cicada orni* singing by the Hérault river, Saint-Étienne-d'Issensac, 9 July 2006 ([Cicada orni.ogg](https://commons.wikimedia.org/wiki/File:Cicada_orni.ogg)): not in the repository, fetched into `assets/sounds/cicadas/` by `tools/fetch-sounds.sh` | DavidDelon | the cicada bed's chorus (`forge_audio::ambience::CicadaSong`, #267) | public domain |
| A serin's song ([Gulhämpling - Serinus serinus. Ystad-Sweden-2012.ogg](https://commons.wikimedia.org/wiki/File:Gulh%C3%A4mpling_-_Serinus_serinus._Ystad-Sweden-2012.ogg)): fetched the same way | Jonn Leffmann (2012) | the birds bed's serin (#267) | CC BY 3.0 |

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
- **Packed caches** (`forge_core::pack`, #215).
  - The float grids' coding follows lossless image coding: each sample predicted from its
    left, upper and upper-left neighbours as `a + b − c`, lossless JPEG's predictor 4
    (ITU-T T.81, 1992).
  - The byte planes follow Blosc's shuffle filter (Francesc Alted).
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
  - The twilight test's tables (`atmosphere.rs`, #347), every 10 nm from 380 to 780 nm: from
    Bruneton's demo (BSD licence), the ozone cross-sections (the University of Bremen's 2011
    spectra at 233 K) and the sun above the air (ASTM G-173's extraterrestrial column); the
    CIE 1931 colour-matching functions (a CIE data set). Red's CIE-weighted coefficients come
    from them (`docs/research/twilight-colour.md`).
- **Physical light units and pre-exposure.** Sébastien Lagarde, Charles de Rousiers, "Moving
  Frostbite to Physically Based Rendering", SIGGRAPH 2014.
- **Automatic exposure.** Krzysztof Narkowicz, "Automatic Exposure", 2016.
- **The surface model** (D-086): GGX, Bruce Walter, Stephen Marschner, Hongsong Li, Kenneth
  Torrance, "Microfacet Models for Refraction through Rough Surfaces", EGSR 2007 (after
  Trowbridge and Reitz 1975); the height-correlated Smith visibility, Eric Heitz,
  "Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs", JCGT 2014;
  Christophe Schlick's Fresnel, 1994; the split sum, Brian Karis, "Real Shading in Unreal
  Engine 4", SIGGRAPH 2013, its DFG term by Dimitar Lazarov's analytic fit (Karis, "Physically
  Based Shading on Mobile", 2014); multiple scattering, Carmelo J. Fdez-Agüera, "A
  Multiple-Scattering Microfacet Model for Real-Time Image-based Lighting", JCGT 2019; Toksvig's
  normal-length variance, Michael Toksvig, "Mipmapping Normal Maps", 2005.
- **ACES's fitted curve** (`aces-narkowicz`, the Bistro's, D-084). Krzysztof Narkowicz, "ACES
  Filmic Tone Mapping Curve", 2016, with the 0.6 pre-exposure Falcor's ToneMapper applies.
- **Local exposure** (`shaders/local_exposure*.slang`, D-067, #275):
  - Frédo Durand, Julie Dorsey, "Fast Bilateral Filtering for the Display of High-Dynamic-Range
    Images", SIGGRAPH 2002: the base and detail layers, only the base's contrast reduced.
  - Jiawen Chen, Sylvain Paris, Frédo Durand, "Real-time Edge-Aware Image Processing with the
    Bilateral Grid", SIGGRAPH 2007: the grid of log luminance, splatted, blurred and read per
    pixel at its own luminance.
  - Jasmin Patry, "Real-Time Samurai Cinema: Lighting, Atmosphere, and Tonemapping in Ghost of
    Tsushima", SIGGRAPH 2021 Advances: the grid blended with a wide Gaussian of the cells' means,
    upsampled by a cubic B-spline, and the contrast around a midpoint.
  - Christian Sigg, Markus Hadwiger, "Fast Third-Order Texture Filtering", GPU Gems 2, 2005: the
    B-spline from four bilinear taps.
- **Sky light.** Ravi Ramamoorthi, Pat Hanrahan, "An Efficient Representation for Irradiance
  Environment Maps", SIGGRAPH 2001: the sky's irradiance as nine spherical-harmonic
  coefficients.
- **Reflections.** Christophe Schlick, "An Inexpensive BRDF Model for Physically-based
  Rendering", Eurographics 1994: the Fresnel approximation. The specular occlusion follows
  Lagarde and de Rousiers (2014, above).
- **The mirror rays' history** (`steadied_reflection` in `shaders/meshlet.slang`, D-050, #176):
  a flat mirror's reflection reprojected through its *virtual* point, the hit seen behind the
  mirror on the view ray. Forge writes it from the description in NVIDIA's NRD README ("Primary
  Surface Replacement", v4.17.3) and the idea of AMD's FidelityFX reflection denoiser
  (`FFX_DNSR_Reflections_GetHitPositionReprojection`, MIT); neither's code is used.
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
- **The surface cache** (D-088, `r.gi.cache`). Bevy Solari's world cache (the Bevy
  contributors, Bevy 0.20.1): entries that live while they are read, one cosine ray an entry a
  frame whose hit reads the cache again, its samples blended up to 32, and the lookup jittered in
  the surface's plane (after Tom Clabault's ReGIR notes, 2025). Its hashed index is replaced by
  kajiya's (Embark Studios): levels of 32³ cells round the camera, each twice the last, the cell
  moved half a cell along the normal. Ported under Apache-2.0, the changes in
  `shaders/surface_cache.slang`'s header.
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
- **Leaf cards' mips** (`forge_render::textures::leaves`, D-013). Ignacio Castaño, "Computing
  Alpha Mipmaps", his blog, written for The Witness, 2010: each mip's alpha scaled so an alpha
  test keeps the share of texels level 0 keeps.
- **The light through leaves** (`meshlet.slang`'s `leaf_transmission`, #346). Barré-Brisebois and
  Bouchard's translucency approximation (GDC 2011, above), the light's direction bent by the
  normal and a lobe round the view, as Frostbite uses it for thin foliage.

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
- **Grass near the camera** (`forge_render::grass`, `shaders/meshlet.slang`, #300, D-075). Blades
  generated on the GPU from a map of the ground, their normals and contrast converging to their
  clump's with distance, a colour a clump: E. Wohllaib, "Procedural Grass in 'Ghost of
  Tsushima'", GDC 2021. Plain opaque triangles darkened towards the root: Angelo Pesce,
  "Vegetation system" (notes on Call of Duty: Black Ops 4's grass). Blades at least a pixel wide,
  their covered share kept by noise: Emil Persson, "Phone-wire AA", 2012.
- **Impostors** (`forge_render::impostor`, `shaders/impostor.slang`, #302, D-075 step 4). The
  upper hemisphere's views in a hemi-octahedral layout, three frames blended by a view's
  barycentric weights: Ryan Brucks, "Octahedral Impostors", 2018 (shaderbits.com). Read for the
  method, no code taken.
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
  *An Introduction to Fluid Dynamics*, Cambridge University Press, 1967). The bar of sand past a
  confluence's downstream corner (#119's polish) lies in its separation zone: James L. Best and
  Ian Reid, "Separation Zone at Open-Channel Junctions", *Journal of Hydraulic Engineering*
  110(11), 1984; James L. Best, "Flow dynamics at river channel confluences: implications for
  sediment transport and bed morphology", in *Recent Developments in Fluvial Sedimentology*, SEPM
  Special Publication 39, 1987, 27–35.
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

**Particles** (`forge_render::particles`, D-064, #271), grown from the splashes and taking their
soft edges, their pixel-wide minimum, their reactive mask and their hash:
- **A swirl without sources or sinks:** the curl of a potential, after Robert Bridson, Jim
  Hourihan and Marcus Nordenstam, "Curl-Noise for Procedural Fluid Flow", *ACM Transactions on
  Graphics* 26(3), SIGGRAPH 2007; here the potential is three sums of sines, not their noise.
- **One blend for smoke and embers:** premultiplied alpha, after Thomas Porter and Tom Duff,
  "Compositing Digital Images", SIGGRAPH 1984: a particle with no opacity adds its light.
- **Smoke's scattering:** Henyey and Greenstein's phase function (above), with an even part for
  the light scattered more than once.
- **The baked smoke's simulation** (`forge_render::smoke_bake`):
  - **Stable fluids:** Jos Stam, "Stable Fluids", SIGGRAPH 1999. The semi-Lagrangian advection
    and the pressure projection; US 6,266,071 B1 expired in 2018.
  - **Vorticity confinement**, as in Ronald Fedkiw, Jos Stam and Henrik Wann Jensen, "Visual
    Simulation of Smoke", SIGGRAPH 2001.
  - **Growth from the heat:** a divergence source in the pressure solve, after Bryan E. Feldman,
    James F. O'Brien and Okan Arikan, "Animating Suspended Particle Explosions", SIGGRAPH 2003.
- **Six-way lighting:** the layout of Unity's six-way smoke lighting: right, top and back with
  the alpha in one map; left, bottom and front in the other. It is described in Unity
  Technologies' "Realistic smoke lighting with 6-way lighting in VFX Graph", January 2023, and
  is used in games under several names. The frames and the code are our own.
- **The baked fire:** the same simulation's heat drawn as an emission. Its glow grows with the
  fourth power of the heat, after the Stefan–Boltzmann law's power of temperature. The colour
  ramp from red to yellow-white is our own.

**Lamps** (`forge_render::lamps`, D-065 steps 1–3, #270):
- **A bulb's colour from its temperature:** Tanner Helland's fit of the blackbody locus in sRGB,
  "How to Convert Temperature (K) to RGB", 2012
  (<https://tannerhelland.com/2012/09/18/convert-temperature-rgb-algorithm-code.html>),
  decoded to linear and scaled to luminance 1. The colours are then balanced for 3 200 K by
  dividing by that temperature's colour, a von Kries-style white balance.
- **Soft shadows from one ray a frame**, a point of the light's disc that moves with the frame's
  noise and TAA's average, as the sun's (#54).
- **One light picked by its unshadowed contribution**, its light divided by its chance: Peter
  Shirley, Changyaw Wang, Kurt Zimmerman, "Monte Carlo Techniques for Direct Lighting
  Calculations", *ACM Transactions on Graphics* 15(1), 1996. For the lamps beyond a pixel's
  three strongest and for the probes' rays.
- **Lights listed per region of space:** after Ola Olsson, Markus Billeter, Ulf Assarsson,
  "Clustered Deferred and Forward Shading", HPG 2012. Here a house's rooms (the probes' regions,
  D-068) are the clusters.
- **The penumbra filter** (`lamp_rays_main`, `lamp_filter_main`, `research/lamp-penumbra-denoising.md`):
  - *The ratio estimator:* Eric Heitz, Stephen Hill, Morgan McGuire, "Combining Analytic Direct
    Illumination and Stochastic Shadows", I3D 2018. The shadowed and unshadowed light of the same
    rays, their ratio filtered and multiplied into the exact, unshadowed light.
  - *Interleaved sampling:* Alexander Keller, Wolfgang Heidrich, "Interleaved Sampling",
    Eurographics Workshop on Rendering 2001. A run of the lamps' disc points spread over 4 × 4
    pixels, in the order of Bryce E. Bayer's ordered-dither matrix ("An Optimum Method for
    Two-Level Rendition of Continuous-Tone Pictures", IEEE International Conference on
    Communications, 1973).
  - *The disc points:* the first 128 points of the two-dimensional sequence of I. M. Sobol', "On
    the Distribution of Points in a Cube and the Approximate Evaluation of Integrals", *USSR
    Computational Mathematics and Mathematical Physics* 7(4), 1967, mapped onto the disc by Peter
    Shirley and Kenneth Chiu's concentric map, "A Low Distortion Map Between Disk and Square",
    *Journal of Graphics Tools* 2(3), 1997.
  - The tent (the 4 × 4 box twice over), the strongest lamp's paired runs and the filter's edge
    weights are our own.

**City plans** (`forge_procgen::city`, D-059):
- **Streets from a tensor field.** Guoning Chen, Gregory Esch, Peter Wonka, Pascal Müller,
  Eugene Zhang, "Interactive Procedural Street Modeling", *ACM Transactions on Graphics* 27(3),
  SIGGRAPH 2008: the grid, radial and boundary basis fields, their weighted sum, and the
  streamlines traced along the eigenvectors with a separation and a join.
- **Lots.** Carlos A. Vanegas, Tom Kelly, Basil Weber, Jan Halatsch, Daniel G. Aliaga, Pascal
  Müller, "Procedural Generation of Parcels in Urban Modeling", *Computer Graphics Forum* 31(2),
  Eurographics 2012: recursive splits across each piece's oriented box, lots on a street.
- **Parks within reach.** Cecil C. Konijnendijk, "Evidence-based guidelines for greener,
  healthier, more resilient neighbourhoods: Introducing the 3–30–300 rule", *Journal of Forestry
  Research* 34(3), 2023: a park of at least a hectare within 300 m of every home.
- **Convex hulls.** A. M. Andrew, "Another efficient algorithm for convex hulls in two
  dimensions", *Information Processing Letters* 9(5), 1979: the monotone chain.
- **Buildings from a split grammar** (`forge_procgen::grammar`, `city::facades`, D-039). Pascal
  Müller, Peter Wonka, Simon Haegler, Andreas Ulmer, Luc Van Gool, "Procedural Modeling of
  Buildings", *ACM Transactions on Graphics* 25(3), SIGGRAPH 2006 (CGA shape): scopes, split
  and repeat with absolute, relative and floating sizes, the component split of a mass into
  faces, conditions on the context, and terminals that insert modules. The research found no
  patent on the grammar itself (`city-generation.md`, Patents).
- **Railings as gratings** (`grating_coverage` in `shaders/meshlet.slang`). Darwyn Peachey,
  "Building Procedural Textures", in David S. Ebert et al., *Texturing and Modeling: A
  Procedural Approach*, 1994: the box-filtered pulse train, the bars' share of a pixel's
  footprint. Emil Persson's phone-wire antialiasing (GPU Pro 5, 2014): thin geometry fading by
  its coverage instead of thinning under a pixel; here the coverage is dithered against the
  frame's noise for TAA to average.
- **Rooms behind the windows** (`interior_room` in `shaders/meshlet.slang`, D-060). Joost van
  Dongen, "Interior Mapping: A new technique for rendering realistic buildings", Computer
  Graphics International 2008: the view ray traced into a room box behind the glass, in the
  window's own space. No patent found on the technique itself (`building-structure.md`).
- **Timetabled runs** (`forge_sim::transit`). The General Transit Feed Specification's model
  (GTFS, gtfs.org): routes, trips, stop times and frequencies, a vehicle's place a function of
  its schedule.
- **Polylines and polygons.** David H. Douglas, Thomas K. Peucker, "Algorithms for the reduction
  of the number of points required to represent a digitized line or its caricature", *The
  Canadian Cartographer* 10(2), 1973; Ivan E. Sutherland, Gary W. Hodgman, "Reentrant polygon
  clipping", *Communications of the ACM* 17(1), 1974.

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
- **QUIC.** Jana Iyengar, Martin Thomson (eds.), RFC 9000, 2021; Martin Thomson, Sean Turner,
  RFC 9001; Tommy Pauly, Eric Kinnear, David Schinazi, RFC 9221 (unreliable datagrams), 2022:
  the transport under `forge-net` (#268).
- **Connection tokens.** Glenn Fiedler, the netcode 1.02 standard (`mas-bandwidth/netcode`): a
  short-lived token naming the client, its servers and their times, issued by a web login and
  checked by the game server offline (`forge_net::token`, #268), signed here with Ed25519
  (Daniel J. Bernstein, Niels Duif, Tanja Lange, Peter Schwabe, Bo-Yin Yang, "High-speed
  high-security signatures", 2011; RFC 8032).
- **Fragments.** Glenn Fiedler, "Packet Fragmentation and Reassembly", Gaffer On Games, 2016:
  a message's number, a fragment's index and the count in each fragment, the message lost whole
  with any fragment (`forge_net::fragment`, #268).
- **Snapshots.** Glenn Fiedler, "Snapshot Compression" and "State Synchronization", Gaffer On
  Games, 2015: smallest-three quaternions, bounded quantised positions, deltas against the
  state the receiver acknowledged, quantising on both sides, and the priority accumulator;
  the Quake III network model (id Software, 1999; Fabien Sanglard's review, 2012): deltas
  against the client's last acknowledged snapshot; Mark Frohnmayer and Tim Gift, "The TRIBES
  Engine Networking Model", 1999: scoped, prioritised partial updates (`forge_net::snapshot`,
  `forge_net::replicate`, #268).
- **Lag compensation.** Yahn W. Bernier, "Latency Compensating Methods in Client/Server In-game
  Protocol Design and Optimization", GDC 2001: the server keeps its recent hit shapes and tests
  a shot against them as its shooter saw them (`forge_sim::rewind`, #268), and the client draws
  the others a fixed delay behind, between the two states about that moment; Glenn Fiedler,
  "Snapshot Interpolation", Gaffer On Games, 2014: the playout delay that covers the snapshots'
  gaps and losses (`forge_net::interpolate`).
- **Pacing the client** (`forge_sim::PaceClock`, #268). Timothy Ford, "Overwatch Gameplay
  Architecture and Netcode", GDC 2017: the client's simulation run faster or slower on the
  server's feedback about its inputs' arrival; Unity's NetCode for Entities (2019–20), its command
  age and time scale. Redundant inputs resent until acknowledged, as QuakeWorld and Quake 2 resent
  their commands (id Software, 1996–97).
- **Authority between workers** (`forge_sim::authority`, #268). Richard M. Fujimoto, *Parallel
  and Distributed Simulation Systems*, Wiley, 2000: conservative synchronisation with one tick of
  lookahead, so the split world steps as one. Fencing by epochs, as Viewstamped Replication's view
  numbers (Oki and Liskov, 1988), Chubby's sequencers (Mike Burrows, OSDI 2006) and Raft's terms
  (Ongaro and Ousterhout, 2014) fence a stale leader. Border ghosts promoted on crossing, as
  Linden's regions (US 2003/0195735) and BigWorld's real and ghost entities; the replication
  layer and its workers, as Cloud Imperium describes server meshing (2021–2023).
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
- **Skinning** (`forge_render::skin`, `forge-anim`, #165):
  - **Linear blend skinning:** Nadia Magnenat-Thalmann, Richard Laperrière and Daniel Thalmann,
    "Joint-dependent Local Deformations for Hand Animation and Object Grasping", Graphics
    Interface 1988. Four joints a vertex, as glTF 2.0's skins give them; the keys' cubic
    splines are glTF's (Khronos, the specification's appendix C).
  - **Dual quaternion skinning,** a mesh's option (#169, D-052): Ladislav Kavan, Steven
    Collins, Jiří Žára and Carol O'Sullivan, "Skinning with Dual Quaternions", I3D 2007. The
    rotation from each joint's matrix by Shepperd's method: Stanley W. Shepperd, "Quaternion
    from Rotation Matrix", *Journal of Guidance and Control* 1(3), 1978.
  - **Morph targets** (#169): glTF 2.0's morph targets and mesh weights (Khronos), their names
    from the `targetNames` extras Blender's exporter writes. The lab's elbow correctives are
    pose space deformation's idea, a shape added by how far a joint is posed: J. P. Lewis, Matt
    Cordner and Nickson Fong, "Pose Space Deformation: A Unified Approach to Shape
    Interpolation and Skeleton-Driven Deformation", SIGGRAPH 2000.
  - **Normalised lerp between rotation keys:** as ozz-animation does between dense keys
    (Guillaume Blanc).
  - **The creatures' weights:** Blender's bone heat, after Ilya Baran and Jovan Popović,
    "Automatic Rigging and Animation of 3D Characters", SIGGRAPH 2007.
- **Playing clips** (`forge-anim`, #167):
  - **Inertialized switches:** David Bollo, "Inertialization: High-Performance Animation
    Transitions in *Gears of War*", GDC 2016: the offset at a switch dying away along his
    quintic (`forge_anim::Inertializer`).
  - **Packed clips:** after Nicholas Frechette's Animation Compression Library (ACL, MIT):
    per-track range reduction and variable bit rates, the error measured on virtual vertices
    round the joints (`forge_anim::PackedClip`). The approach only; none of ACL's code.
  - **Powered ragdolls held upright:** a damped spring on the root towards its stance, as games
    keep their active ragdolls standing (the lab's dogs).
- **Models' textures** (D-047, #166):
  - **Their materials:** glTF 2.0's metallic-roughness model and its `KHR_texture_transform`
    (Khronos, the specification and the extension's sample code).
  - **The tangent frame from derivatives:** Christian Schüler, "Followup: Normal Mapping
    Without Precomputed Tangents", 2013 (after his "Normal Mapping without Precomputed
    Tangents", ShaderX5, 2006): the cotangent frame of the position's and the UVs' screen
    derivatives, so no tangents are stored.
  - **UVs quantised per cluster:** Brian Karis, Rune Stubbe and Graham Wihlidal, "Nanite: A Deep
    Dive", SIGGRAPH 2021 Advances in Real-Time Rendering: attributes stored per cluster in
    a few bits over the cluster's range.
  - **Cut-outs in their own raster** (#171): the same talk's programmable raster bins.
    Masked materials' clusters are drawn by a pipeline of their own, so the others keep the
    early depth test. Their alpha is tested in the ray queries' candidate loops, as Vulkan's
    `VK_KHR_ray_query` lets an inline query stand in for an any-hit shader.
  - **Opacity micromaps** (#333, `forge_render::opacity`): the cut-out triangles' opacity in
    Vulkan's `VK_EXT_opacity_micromap` maps, baked by Forge on the CPU. The micro-triangles'
    order is the Vulkan specification's reference code, `BarycentricsToSpaceFillingCurveIndex`
    (© The Khronos Group Inc., CC-BY-4.0, appendix `VK_EXT_opacity_micromap`), ported to Rust.
    The conservative 4-state maps for shadow rays follow NVIDIA's "Path Tracing Optimizations
    in Indiana Jones: Opacity MicroMaps and Compaction of Dynamic BLASs" (2024). None of
    NVIDIA's OMM SDK.
- **Clouds.** Andrew Schneider and Nathan Vos, "The Real-time Volumetric Cloudscapes of Horizon:
  Zero Dawn", SIGGRAPH 2015 Advances in Real-Time Rendering: a layer from a weather map,
  Perlin–Worley shapes eroded by Worley noise, Beer's law with the powder term, temporal
  reprojection; Sébastien Hillaire, "Physically Based Sky, Atmosphere and Cloud Rendering in
  Frostbite", SIGGRAPH 2016: multiple scattering as Magnus Wrenninge's octaves ("Oz: The Great
  and Volumetric", SIGGRAPH 2013 Talks) (`forge_render::clouds`, #145).
- **Shallow water.** Matthias Müller-Fischer, "Fast Water Simulation for Games Using Height
  Fields", GDC 2008: depths on a grid, velocities on its faces carried along semi-Lagrangian,
  upwind fluxes kept from overdrawing a cell (`forge_physics::shallow`, #144; on the GPU,
  `forge_render::shallow`, #162). A. Ritter's dam-break solution (1892) for its test.
- **Deformable snow and sand.** Colin Barré-Brisebois, "Deformable Snow Rendering in Batman:
  Arkham Origins", GDC 2014: a height map of the snow that what walks through it writes into
  and the renderer displaces the ground by (D-007; `forge_physics::deform` and the skin pass's
  height fields, #185). The prints slump by Musgrave, Kolb and Mace's talus rule (above).
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
- **The night sky** (`forge_render::night`, #164, D-046):
  - **The night's layers:** Henrik Wann Jensen, Frédo Durand, Julie Dorsey, Michael M. Stark,
    Peter Shirley and Simon Premože, "A Physically-Based Night Sky Model", SIGGRAPH 2001: the
    Moon lit by the sun, the stars by magnitude and colour, airglow and the Milky Way.
  - **The Moon:** its phase law after C. W. Allen, *Astrophysical Quantities* (the magnitude
    `-12.73 + 0.026 α + 4e-9 α⁴`); its disc shaded with the Lommel–Seeliger law.
  - **The stars' colours:** F. J. Ballesteros, "New insights into black bodies", EPL 97 (2012),
    for a temperature from B − V; Chris Wyman, Peter-Pike Sloan and Peter Shirley, "Simple
    Analytic Approximations to the CIE XYZ Color Matching Functions", JCGT 2(2), 2013.
  - **The Purkinje shift:** Adam G. Kirk and James F. O'Brien, "Perceptually Based Tone Mapping
    for Low-Light Conditions", SIGGRAPH 2011 (the effect); the scotopic luminance after Frédo
    Durand and Julie Dorsey, "Interactive Tone Mapping", EGWR 2000.
  - **Galactic coordinates:** the IAU's J2000 equatorial-to-galactic rotation.
- **The planet** (`forge_terrain::planet`, `demos/planet`, #220, D-056):
  - **3-D gradient noise:** Ken Perlin, "Improving Noise", SIGGRAPH 2002: the twelve edge
    gradients and the quintic fade.
  - **Skirts under the tiles' edges:** Thatcher Ulrich, "Rendering Massive Terrains using Chunked
    Level of Detail Control", SIGGRAPH 2002 course notes.
  - **The tiles on the cube sphere and their cut:** the recommendation of
    `docs/research/planet-terrain.md` (Cesium's quantized-mesh skirts, S2's cells).
  - **The far air marched per pixel:** the aerial-perspective volume of Sébastien Hillaire, "A
    Scalable and Production Ready Sky and Atmosphere Rendering Technique", EGSR 2020, with the
    pixels beyond its far end marched on their own (Forge's, for the view from orbit).
  - **The heights in integers** (D-058's step 2, `forge_core::fixed`, `shaders/fixed.slang`):
    the CORDIC, Jack E. Volder, "The CORDIC Trigonometric Computing Technique", IRE
    Transactions on Electronic Computers, 1959, for the sines, cosines and angles in shifts and
    adds.
- **The flora** (`forge-flora`, #269; `docs/research/flora.md`):
  - **Space colonisation:** Adam Runions, Brendan Lane and Przemyslaw Prusinkiewicz, "Modeling
    Trees with a Space Colonization Algorithm", Eurographics Workshop on Natural Phenomena 2007:
    markers in the crown's envelope claimed by the buds that see them.
  - **Self-organising trees:** Wojciech Pałubicki, Kipp Horel, Steven Longay, Adam Runions,
    Brendan Lane, Radomír Měch and Przemyslaw Prusinkiewicz, "Self-organizing tree models for
    image synthesis", SIGGRAPH 2009: the light summed down, the vigour handed up, shoots of
    ⌊vigour⌋ metamers, shedding and pruning.
  - **The vigour's split at a fork:** Rolf Borchert and Hisao Honda, "Control of Development in
    the Bifurcating Branch System of *Tabebuia rosea*: A Computer Simulation", *Botanical
    Gazette* 145, 1984.
  - **The pipe model:** K. Shinozaki, K. Yoda, K. Hozumi and T. Kira, "A quantitative analysis
    of plant form — the pipe model theory", *Japanese Journal of Ecology* 14, 1964: the radii.
  - **The growth curve:** F. J. Richards, "A Flexible Growth Function for Empirical Use",
    *Journal of Experimental Botany* 10, 1959 (the Chapman–Richards form foresters fit).
  - **Closest points of two segments:** Christer Ericson, *Real-Time Collision Detection*, 2005,
    §5.1.9.
  - **The species' sizes:** Van den Berk Nurseries' tree pages, the Royal Horticultural Society's
    plant pages, the American Conifer Society, the Sacramento Tree Foundation, EUFORGEN and
    Wikipedia, as `docs/research/flora.md` §2 lists them.
  - **Darts for the placement:** Robert Bridson, "Fast Poisson Disk Sampling in Arbitrary
    Dimensions", SIGGRAPH 2007 Sketches (one candidate per cell of a grid); Li-Yi Wei,
    "Multi-Class Blue Noise Sampling", SIGGRAPH 2010 (a spacing per pair of species; Microsoft's
    patent on it, US 8,219,517 B2, has lapsed).
  - **The ground's fields:** the topographic wetness index of Beven and Kirkby (above, as
    `forge_procgen::layers::wetness` uses it) for the moisture; Andrew Weiss, "Topographic
    Position and Landforms Analysis", ESRI User Conference 2001, for the ridges and the valleys.
- **The fauna** (`forge-fauna`, #269; `docs/research/fauna.md`):
  - **Flocks and steering:** Craig W. Reynolds, "Flocks, Herds, and Schools: A Distributed
    Behavioral Model", SIGGRAPH 1987, and "Steering Behaviors For Autonomous Characters", GDC
    1999 (obstacle avoidance by looking ahead).
  - **The zones:** Iain D. Couzin, Jens Krause, Richard James, Graeme D. Ruxton and Nigel R.
    Franks, "Collective Memory and Spatial Sorting in Animal Groups", *Journal of Theoretical
    Biology* 218, 2002: repulsion first, then alignment and attraction.
  - **Seven neighbours:** Michele Ballerini and co-authors, "Interaction ruling animal collective
    behavior depends on topological rather than metric distance", *PNAS* 105, 2008.
  - **A constant cruise and a turn:** Charlotte K. Hemelrijk and Hanno Hildenbrandt, "Some
    Causes of the Variable Shape of Flocks of Birds", *PLoS ONE*, 2011 (StarDisplay).
  - **Flight distances and the animals' numbers:** Díaz et al. 2013 (urban and rural birds across
    Europe), García-Arroyo et al. 2023 (house sparrows), Legagneux and Ducatez 2013 (birds and
    speed limits), Henningsson, Johansson and Hedenström 2010 (swifts), Alerstam et al. 2007
    (flight speeds), Kays et al. 2020 and Jensen et al. 2022 (pet cats), Chebli et al. 2022 and
    the Filabres study (goats), as `docs/research/fauna.md` lists them.

**Navigation** (`forge-nav`, D-076, #317):
- **The walk field.** Mikko Mononen, Recast & Detour (zlib, read, not used as code): the solids
  rasterised into spans per column, a walkable top under the walker's height, neighbours joined
  within its step and a chamfer distance field for its radius.
- **The search.** Peter E. Hart, Nils J. Nilsson and Bertram Raphael, "A Formal Basis for the
  Heuristic Determination of Minimum Cost Paths", 1968 (A*), with an octile heuristic.
- **The crossing's warp.** Andrew Witkin and Zoran Popović, "Motion Warping", SIGGRAPH 1995: a
  clip's root path scaled per axis between targets (`forge_nav::CrossingMotion`, #316).
- **Avoidance.** Jur van den Berg, Stephen J. Guy, Ming Lin and Dinesh Manocha, "Reciprocal
  n-Body Collision Avoidance", 2011 (ORCA), and its library RVO2 (UNC, Apache-2.0, read, not
  used as code): the half-planes, the incremental linear programs and the nudge against
  symmetric meetings (`forge_nav::Crowd`).

**Sound** (`forge-audio`, #267, D-011):
- **The audio thread's rules.** Ross Bencina, "Real-time audio programming 101: time waits for
  nothing", 2011: no lock, allocation, file or unbounded call on the audio thread; what it is
  done with goes back to be freed elsewhere.
- **The limiter.** Geraint Luff, "Designing a straightforward limiter", Signalsmith Audio, 2022:
  the gain's minimum held over the look-ahead, then averaged over the same length, the audio
  delayed to match (`forge_audio::dsp::Limiter`).
- **The reverb.** Jean-Marc Jot and Antoine Chaigne, "Digital Delay Networks for Designing
  Artificial Reverberators", AES 90th Convention, 1991: a feedback delay network through an
  orthogonal matrix, each line's gain set from the decay time and damped by a one-pole filter;
  the feedback delay network itself after John Stautner and Miller Puckette, "Designing
  Multi-Channel Reverberators", *Computer Music Journal* 6(1), 1982 (`forge_audio::dsp::Reverb`).
- **Band gains and air.** Valve's Steam Audio (Apache-2.0), read in its source: the three bands
  (800 Hz, 8 kHz), the default air absorption per metre, and the band EQ as a low shelf, a
  peaking filter and a high shelf with the gains normalised and crossfaded when they change
  (`core/src/core/eq_effect.cpp`, `iir.cpp`, `air_absorption.cpp`), written again in Rust
  (`forge_audio::dsp::ThreeBand`, `spatial::AIR_ABSORPTION`); its eleven acoustic material
  presets and the way it applies their transmission to the occluded share of the direct sound
  (`phonon.h`, `direct_effect.cpp`; `spatial::AcousticMaterial`, `Occlusion::through`).
- **Reverb zones.** The listener-driven zones of the engines: Unity's Audio Reverb Zone and FMOD's
  3D reverb (a shape where the reverb is whole and a distance over which it fades), blended here
  in the log domain of the decay time (`forge_audio::zones`).
- **The filters.** Robert Bristow-Johnson, "Cookbook formulae for audio EQ biquad filter
  coefficients" (the shelves and the peaking filter).
- **Resampling.** The four-point, third-order Hermite (Catmull–Rom) interpolator: Edwin Catmull
  and Raphael Rom, "A Class of Local Interpolating Splines", 1974, as Olli Niemitalo sets it out
  for audio in "Polynomial Interpolators for High-Quality Resampling of Oversampled Audio",
  2001.
- **Distance.** The OpenAL 1.1 specification's clamped inverse distance model (Creative Labs,
  2005).
- **Voices.** The real and virtual voices of FMOD and Wwise as their documentation describes
  them: a budget kept by priority and then audibility, virtual voices that keep their time.
- **Variation.** Wwise's random container and FMOD's multi instrument as their documentation
  describes them: a take not among the last few played, its pitch and volume drawn within a range
  (`forge_audio::variation`).
- **Ogg Vorbis and Opus.** The Xiph.Org Foundation's Vorbis I specification (granule positions
  and the trimming they imply, A.2); RFC 7845 (Ogg Opus: the head, the pre-skip, the granule
  positions) and RFC 6716 (Opus).
- **Footsteps and impacts.** Kees van den Doel, Paul G. Kry and Dinesh K. Pai,
  "FoleyAutomatic: Physically-Based Sound Effects for Interactive Simulation and Animation",
  SIGGRAPH 2001 (a bank of modal resonators driven by the contact force); Perry R. Cook,
  "Physically Informed Sonic Modeling (PhISM): Synthesis of Percussive Sounds", *Computer Music
  Journal* 21(3), 1997 (stochastic grains: gravel, shakers); Andy Farnell, *Designing Sound*, MIT
  Press, 2010 (a step as a heel and a ball contact) (`forge_audio::foley`).
- **Bubbles and rain.** Marcel Minnaert, "On musical air-bubbles and the sounds of running
  water", *Philosophical Magazine* 16, 1933 (a bubble's pitch from its radius); Kees van den
  Doel, "Physically Based Models for Liquid Sounds", *ACM Transactions on Applied Perception*
  2(4), 2005 (its damping and rising pitch); Charles Verron and George Drettakis, "Procedural
  Audio Modeling for Particle-Based Environmental Effects", AES 133rd Convention, 2012, and
  Shiguang Liu, Haonan Cheng and Yiying Tong, "Physically-Based Statistical Simulation of Rain
  Sound", SIGGRAPH 2019 (rain as drops and bubbles in numbers set by the rate)
  (`dsp::synth::Grain`, `ambience::beds::Rain`).
- **Cicadas.** Pinto-Juma, Simões, Seabra and Quartau, "Calling song structure and
  geographical variation in *Cicada orni* Linnaeus (Hemiptera: Cicadidae)", *Zoological Studies*
  44(1), 2005: the echeme of 80 ms every 230 ms and the peak near 4.8 kHz the cicadas are made from
  (`ambience::beds::Cicadas`).
- **Pink noise.** Paul Kellet's three-pole filter (the music-dsp mailing list, 1999, public
  domain) (`dsp::synth::PinkNoise`).
- **The medium filter.** Andrew Simper, "Linear Trap Integrated SVF", Cytomic, 2013: a
  state-variable filter that can be retuned every block (`dsp::SvfLowPass`, `dsp::BusFilter`).
