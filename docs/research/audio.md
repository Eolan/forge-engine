# Research — Game audio engine architecture and spatial audio

> Status: v0.1, 2026-09-23. Sources checked the same day (crates.io API, GitHub, publisher and author
> pages, vendor docs). Builds on the prior project's
> [`world/docs/research/audio.md`](../../../world/docs/research/audio.md) (v0.1, 2026-09-14), which is
> read here as an input, not repeated.

The prior project got the physics right and the product wrong. Its medium model — speed of sound and
impedance from composition, pressure and temperature; ISO 9613-1 absorption generalised to any gas;
interface loss between media; a structural HRTF scaled by the medium's `c` — is correct, tested, and
worth carrying over whole. What it never wrote down is the layer a sound designer actually touches:
events, buses, real-time parameters, states, loudness-based mixing, voice limits, streaming banks and a
live profiler. That layer is what "pro" means in this industry, it is what Wwise and FMOD sell, and it
is independent of the physics. The prior survey also under-read Steam Audio (its *pathing* is baked but
its direct occlusion and reflections are real-time), got its release date wrong (4.8.1 is February 2025,
not 2026), left HRTF-dataset licensing as an open question when the three canonical datasets are all
shippable, and said nothing about music, voice chat or output formats. This file fills those gaps for
Forge — Rust, Vulkan with hardware ray tracing, procedural open worlds with forests, water, caves,
weather and vehicles, co-op multiplayer, small team — and ends with a recommendation and the demo that
would prove it.

**How to read the labels.** **[paper]** peer-reviewed, **[book]**, **[talk]** conference presentation,
**[web]** engineering write-up or interview, **[code]** repository or crate, **[docs]** vendor
documentation or standard; and **foundational** (old, still correct), **still-current** (the standard
reference today), **recent** (2019 or later).

> **State of the art in five sentences.** Shipping AAA audio is a *data-driven event system* (Wwise,
> FMOD) whose mixer is a bus tree with sends, RTPC-driven parameters, state/switch logic, loudness-based
> voice culling (DICE's HDR audio) and a live profiler — the DSP underneath is ordinary. Spatialisation
> is HRTF for direct paths plus ambisonics for beds and reverb, with SOFA the interchange format and
> three freely licensed measured datasets (MIT KEMAR, CIPIC, SADIE II) available to ship. Acoustics
> splits into two camps: *precomputed wave* simulation (Microsoft's Project Acoustics line, now
> archived) which is physically right but bakes static geometry, and *geometric* ray/path tracing (Steam
> Audio, Meta's Acoustic Ray Tracing, Schissler's diffraction pathfinding) which runs on dynamic
> scenes and is what every open engine ships. Steam Audio is Apache-2.0 since 2024 with a C API and a
> maintained Rust wrapper, which makes "own mixer + Steam Audio" the only open stack that reaches the
> middleware bar on spatialisation. The Rust ecosystem in 2026 has a solid device layer (`cpal`), a
> real-time-safe graph (Firewheel), a friendly high-level library (kira), good decoders and resamplers
> (Symphonia, rubato), and no event/RTPC/profiler layer at all — that is the part Forge has to write.

**Contents**

1. [What "pro" means: the middleware bar](#1-what-pro-means-the-middleware-bar)
2. [Spatialisation and acoustics](#2-spatialisation-and-acoustics)
3. [Mixer and DSP architecture](#3-mixer-and-dsp-architecture)
4. [Procedural and synthesised audio](#4-procedural-and-synthesised-audio)
5. [Music systems](#5-music-systems)
6. [Multiplayer voice chat](#6-multiplayer-voice-chat)
7. [The Rust audio ecosystem, 2025–2026](#7-the-rust-audio-ecosystem-20252026)
8. [Comparison](#8-comparison)
9. [Recommendation for Forge](#9-recommendation-for-forge)
10. [Checked and left out](#10-checked-and-left-out)
11. [Verification notes](#11-verification-notes)

---

## 1. What "pro" means: the middleware bar

Both commercial middlewares expose the same conceptual model, and it is the model, not the DSP, that a
team of designers depends on: an **event** is a named, data-authored graph of actions (play, stop, set
volume, set state, post another event) fired from code by name; a **bus** tree with **sends** and
**auxiliary** (reverb) buses is the mix; **RTPCs** (real-time parameter controls) map a game float to
any property through an authored curve; **states** and **switches** select content and mix snapshots;
**ducking** and **HDR** decide who wins when too much plays; **virtual voices**, priorities and playback
limits bound the CPU; **banks** group assets for streaming; and a **profiler** connects live to the
running game and shows every voice, bus level and CPU cost. The entries below establish the terms and
the price of buying them.

**Audiokinetic. "Wwise" (product; free Indie tier).** [docs] [web] [still-current]
<https://www.audiokinetic.com/en/wwise/pricing/for-games/> (403 to our fetcher; verified through
<https://gamefromscratch.com/wwise-now-free-for-indie-developers/>, 2023-02-01)

Wwise is the reference for the event/bus/RTPC/state model, Spatial Audio (rooms, portals, diffraction,
transmission, geometry-driven early reflections), interactive music (§5) and a live profiler. The free
Indie tier, added in summer 2022, covers projects with a production budget under **US$250,000**, all
platforms, unlimited sounds, but *no source access* and support on a pay-per-use basis; the paid tiers
step at US$2M. The only Rust binding, `rrise` (0.2.3, November 2022, plus `bevy-rrise`), is a raw FFI
layer that requires a licensed Wwise install and has not moved in nearly four years.
*Bearing:* study the object model and the profiler screens; do not plan on the binding. Forge's event
data model (§9) should be a strict subset of this vocabulary so a designer coming from Wwise reads it.

**Firelight Technologies. "FMOD Studio" (product; Indie licence).** [docs] [web] [still-current]
<https://www.fmod.com/licensing> (renders client-side; verified through
<https://www.gamedeveloper.com/audio/small-developers-and-creators-can-now-use-fmod-studio-for-free>,
2020-12-03, and <https://en.wikipedia.org/wiki/FMOD>)

FMOD Studio is the DAW-shaped authoring tool (events on timelines with parameter sheets, snapshots for
ducking and mix states, banks, Live Update, profiler) over the FMOD Core mixer. The Indie licence is
free below **US$200K gross revenue per year** and a title budget the 2020 announcement put at US$500K
(the current site says US$600K; Basic and Premium tiers run US$600K–1.8M and above); it requires
registering the project and an in-game attribution, and excludes gambling and simulation. Two Rust
bindings exist: `libfmod` 2.222.6 (MIT, October 2024, generated from the FMOD 2.02.22 docs) and
`fmod-oxide` 0.2.2 (MPL-2.0, July 2026, hand-written, covering Core and Studio); neither can
redistribute the FMOD libraries, which the user downloads separately.
*Bearing:* the cheaper of the two to prototype against from Rust, and the better reference for how
*timeline* events (multi-instrument, parameter automation) are authored. Its attribution and licence
tiering are acceptable for an indie title but are still a per-title negotiation for an engine product.

**David Möllerstedt (EA DICE). "Audio for Multiplayer & Beyond — Mixing Case Studies from
Battlefield: Bad Company & Frostbite." Develop, 2008.** [talk] [foundational]
<https://www.slideshare.net/slideshow/audio-for-multiplayer-beyond-mixing-case-studies-from-battlefield-bad-company-frostbite/3128141>
(companion deck, Möllerstedt & Strandberg, "Adaptive Mixing in Frostbite":
<https://www.slideshare.net/slideshow/adaptive-mixing-in-frostbite/3128152>)

The origin of **HDR audio**. Every source carries a *logical* loudness in dB spanning roughly 130 dB
from the barely audible to the pain threshold; each frame the engine measures the loudest thing at the
listener, opens a window below it, scales everything into the output range, and *culls* what falls
under the window. It is explicitly "not compression, all sounds are played uncompressed": a culling
and gain policy on loudness values, with loudness standing in for importance. The same deck describes
the master chain (high-pass, shelves, compressor, clip protection) with Home Cinema / Hi-Fi / TV
presets.
*Bearing:* the single most valuable idea in this file for a team without a mixing engineer. A gunshot
silences the birds without anyone authoring a duck; a waterfall dominates until you turn away. Wwise
later productised it as "HDR" buses; Forge should implement it natively on the bus tree.

## 2. Spatialisation and acoustics

### 2.1 Libraries and vendor SDKs

**Valve. "Steam Audio SDK" 4.8.1.** [code] [docs] [still-current]
<https://github.com/ValveSoftware/steam-audio/releases> ·
<https://valvesoftware.github.io/steam-audio/doc/capi/guide.html>

Apache-2.0 since February 2024 (4.5.2); 4.7.0 (August 2024) added impulse-response and energy-field API
objects and experimental octave bands; 4.8.1 (11 February 2025) is the latest, so the project has been
quiet for nineteen months. The C API covers: HRTF with the built-in set or a custom SOFA file
(`SimpleFreeFieldHRIR` convention); a *direct* effect with distance attenuation, frequency-dependent air
absorption, source directivity, partial occlusion by ray count and transmission as an EQ on the occluded
part; *reflections* by real-time ray tracing (built-in tracer, Intel Embree, or AMD Radeon Rays on the
GPU on 64-bit Windows) or baked into probe batches; *pathing* (diffraction around corners) baked over a
probe graph; and ambisonics encode, rotate and decode to binaural or speakers. Simulation is designed to
run on its own thread and hand results to the audio thread.
*Bearing:* the open stack's spatialiser. Real-time reflections and direct occlusion work on dynamic
geometry; only pathing needs probes, which Forge can bake per streamed chunk. The 2025 stall is the risk
to weigh against the Apache licence: you may end up maintaining a fork.

**Maxence Maire. `audionimbus` 0.16.0 (Steam Audio in Rust).** [code] [recent]
<https://github.com/MaxenceMaire/audionimbus> · <https://crates.io/crates/audionimbus>

Safe wrapper over `audionimbus-sys`, MIT OR Apache-2.0, released 8 August 2026; can download and link
the Steam Audio binaries automatically, and ships FMOD, Wwise and Bevy integrations. The 0.12.0 release
notes ("Steam Audio v4.8.1, thread safety and segfault fixes") say what the crate was like before it,
which is the honest maturity signal: one maintainer, moving fast, now covering the whole API.
*Bearing:* use it, pin it, and keep a `-sys` fallback path in mind. Its Bevy integration is a worked
example of feeding a game-thread simulation into an audio-thread effect chain.

**Microsoft. "Project Acoustics" (Project Triton).** [code] [docs] [archived]
<https://github.com/microsoft/ProjectAcoustics> · <https://www.microsoft.com/en-us/research/project/project-triton/>

The productised form of the wave-field-coding papers in §2.3: a cloud/offline wave solver bakes
per-probe perceptual parameters (occlusion, wetness, decay, arrival direction) for static geometry; the
runtime is a tiny lookup. The GitHub repository was archived on 1 July 2024 and downloads were
discontinued; the documentation is CC BY 4.0 and the repository code MIT, but the runtime was never
open, and the Planeverb README (§2.3) warns that the technique is covered by Microsoft patents.
*Bearing:* not usable and not open. Its value to Forge is the *parameterisation* — a handful of
perceptual numbers per (source, listener) is enough to drive a convincing mix — which Forge can compute
by rays instead of waves.

**Google. "Resonance Audio."** [code] [docs] [archived]
<https://github.com/resonance-audio/resonance-audio> ·
<https://resonance-audio.github.io/resonance-audio/discover/concepts.html>

Apache-2.0 C++ library, archived (read-only) on 11 November 2023. Its design is the clean textbook
pipeline: every source is encoded into an ambisonic sound field, room effects are early reflections plus
a late reverb estimated from room dimensions and materials, and the field is decoded binaurally with
HRTFs (the SADIE dataset is in the repository under `matlab/hrtf_data/sadie`). The concept pages remain
the best short explanation of *why* ambisonics is the intermediate format.
*Bearing:* dead as a dependency; read it as reference code for an ambisonic bus with binaural decode,
which is what Forge wants for ambience beds and reverb returns.

**Meta. "Acoustic Ray Tracing" in the Meta XR Audio SDK.** [web] [docs] [recent]
<https://developers.meta.com/horizon/blog/acoustic-ray-tracing-audio-sdk-meta-quest-developer-social-presence/>
(2024-07-30)

Meta's spatialiser gained geometric ray-traced reflections, reverb, occlusion, obstruction and
diffraction from tagged acoustic meshes with material presets, for Unity, Unreal, FMOD and Wwise, tuned
to run on Quest-class mobile CPUs; *Batman: Arkham Shadow* shipped on it. Proprietary SDK, free to use
under Meta's terms.
*Bearing:* proof that geometric acoustics with diffraction fits a mobile budget — a useful bound when
sizing Forge's CPU acoustics LOD. Not a candidate dependency for a Vulkan engine.

**NVIDIA. "VRWorks Audio" (2017) and "VRWorks Audio 2.0 with RTX acceleration" (2019).** [docs] [web]
[historical]
<https://developer.nvidia.com/vrworks-audio-sdk-depth> ·
<https://developer.nvidia.com/blog/vrworks-audio-dials-up-the-immersion-with-rtx-acceleration/>

A GPU geometric-acoustics engine on OptiX: thousands of rays with a dozen bounces per source, reflection,
diffraction and scattering computed live with no baking, a C API and an Unreal 4.15 plugin. Version 2.0
(22 March 2019) added Turing RT-core acceleration, "10× faster than 1.0 on an equivalent Pascal GPU".
The VRWorks page now lists graphics features only; there has been no release since.
*Bearing:* the closest thing to "RTX audio" that ever existed, and it went nowhere — because the
bottleneck in game acoustics is not ray throughput but the perceptual parameterisation, the mix and the
tools. Forge has Vulkan ray queries; spend them on visibility and early reflections *if* profiling shows
the CPU tracer is the limit, not before.

**Microsoft. "Spatial Sound for app developers" (`ISpatialAudioClient`).** [docs] [still-current]
<https://learn.microsoft.com/en-us/windows/win32/coreaudio/spatial-sound>

The platform output path for Dolby Atmos, DTS:X and Windows Sonic on Windows and Xbox: a static channel
bed up to 8.1.4.4 (17 channels) plus *dynamic objects* positioned in 3D, rendered by whichever format
the user picked. Object budgets are small and format-dependent — Dolby Atmos over HDMI allows 32 objects
in total (20 dynamic with a 7.1.4 bed), headphone renderers 128 on current Windows — and the platform
leaves Doppler, distance, occlusion and reverb to the engine. The document recommends "hero" sounds as
objects and everything else in the bed.
*Bearing:* Atmos support is a second output *sink* for the same mix: a 7.1.4 bed decoded from Forge's
ambisonic bus plus a prioritised handful of object voices. Design the bus tree so that binaural, speaker
and object outputs are three decoders of one representation, not three mixes.

### 2.2 HRTFs, SOFA and ambisonics

**Bill Gardner, Keith Martin. "HRTF Measurements of a KEMAR Dummy-Head Microphone." MIT Media Lab,
1994** · **V. R. Algazi, R. O. Duda, D. M. Thompson, C. Avendano. "The CIPIC HRTF Database." IEEE
WASPAA, 2001** · **University of York. "The SADIE II Database" (Armstrong et al., *Applied Sciences*
8(11), 2018).** [data] [paper] [foundational / still-current]
<https://sound.media.mit.edu/resources/KEMAR.html> · <https://escholarship.org/uc/item/3d10j9jw> ·
<https://www.york.ac.uk/sadie-project/database.html>

The three datasets that matter for shipping, with their terms: **MIT KEMAR** — 710 directions at
44.1 kHz, "provided free with no restrictions on use, provided the authors are cited" in research *or
commercial* applications; **CIPIC** — 45 subjects × 1,250 directions, described by its authors as a
public-domain database (the UC Davis host is gone; mirrors exist); **SADIE II** — Apache-2.0, 20
subjects including KU100 and KEMAR mannequins and 18 humans, HRIRs and BRIRs in WAV and AES69 SOFA at
44.1/48/96 kHz with headphone-EQ filters and head scans, citation of the paper required.
*Bearing:* closes the prior project's Q-30 for good. Ship SADIE II's KEMAR or KU100 in SOFA as the
default (Apache-2.0 composes with MIT/Apache code), keep the structural model (Brown & Duda) as the
fallback for exotic media where `c` changes, and let the player pick from the human subjects.

**Piotr Majdak et al. "Spatially Oriented Format for Acoustics" (AES69-2015/2020/2022, SOFA 2.1).**
[docs] [paper] [still-current]
<https://www.sofaconventions.org/mediawiki/index.php/SOFA_(Spatially_Oriented_Format_for_Acoustics)> ·
<https://projects.ari.oeaw.ac.at/research/Publications/Articles/2022/Majdak%202022%20SOFA%203A.pdf>

The AES standard for HRTFs, BRIRs and directivities: a NetCDF container with named *conventions*
(`SimpleFreeFieldHRIR` is what Steam Audio and every renderer read); the 2020/2022 revisions add
spherical-harmonic emitters and receivers for continuous directivity. Pure-Rust reader/renderer:
`sofar` 0.3.0 (March 2026, MIT OR Apache-2.0).
*Bearing:* the asset format for anything spatial Forge imports — HRTFs, measured room responses for
convolution reverb, and later source directivities (a vehicle's engine radiates differently forward
than sideways).

**Franz Zotter, Matthias Frank. *Ambisonics: A Practical 3D Audio Theory for Recording, Studio
Production, Sound Reinforcement, and Virtual Reality.* Springer, 2019 (open access).** [book]
[still-current]
<https://link.springer.com/book/10.1007/978-3-030-17207-7>

The one book on ambisonics: first- and higher-order encoding, decoding to loudspeakers and binaural,
perception of panning, rotation, and the signal processing to make it all work, with free tools and open
data. Explains why a third-order field (16 channels) is the right compromise for beds and reverb —
smooth rotation with head tracking, cheap decode, one representation for every output.
*Bearing:* Forge's spatial bus is an ambisonic bus; this is the design reference for its order, its
normalisation (AmbiX) and its decoders.

### 2.3 Propagation research: wave versus geometry

**Nikunj Raghuvanshi, John Snyder. "Parametric Wave Field Coding for Precomputed Sound Propagation."
*ACM Transactions on Graphics* 33(4), 2014, art. 38.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/2601097.2601184> (PDF:
<https://www.microsoft.com/en-us/research/wp-content/uploads/2016/07/ParametricWaveField.pdf>)

Runs a wave solver offline over a scene, then throws the impulse responses away and keeps a few
*perceptual parameters* per (source probe, listener probe) — direct loudness, early-reflection loudness,
decay time — which vary smoothly in space and compress like a PNG. Diffraction around corners and
through doorways comes for free from the wave solve; the runtime is a table lookup and a parametric
reverb.
*Bearing:* the parameter set is the lesson: whatever Forge computes, reduce it to loudness, early
energy, decay and arrival direction per band before it reaches the mixer.

**Nikunj Raghuvanshi, John Snyder. "Parametric Directional Coding for Precomputed Sound Propagation."
*ACM TOG* 37(4), 2018, art. 108** · **Chakravarty R. Alla Chaitanya, Nikunj Raghuvanshi, Keith W.
Godin, Zechen Zhang, Derek Nowrouzezahrai, John Snyder. "Directional Sources and Listeners in
Interactive Sound Propagation Using Reciprocal Wave Field Coding." *ACM TOG* 39(4), 2020.** [paper]
[still-current / recent]
<https://dl.acm.org/doi/10.1145/3197517.3201339> · <https://dl.acm.org/doi/10.1145/3386569.3392459>

The same line adds *direction*: the 2018 paper encodes where the first arrival comes from (so a voice
through a doorway is heard from the doorway) for scenes of millions of polygons; the 2020 paper makes the
coding reciprocal so both the source's radiation pattern and the listener's HRTF are applied to a wave
solution, with freely rotating sources and listeners.
*Bearing:* "initial arrival direction" is the perceptual parameter after loudness and decay; Forge's
geometric tracer must output it (the direction of the shortest unoccluded or diffracted path), not just a
gain.

**Matthew Rosen, Keith W. Godin, Nikunj Raghuvanshi. "Interactive Sound Propagation for Dynamic Scenes
Using 2D Wave Simulation." *Computer Graphics Forum* (SCA), 2020** · **Nikunj Raghuvanshi. "Dynamic
Portal Occlusion for Precomputed Interactive Sound Propagation." arXiv 2107.11548, 2021.** [paper]
[code] [recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.14099> · <https://github.com/themattrosen/Planeverb> ·
<https://arxiv.org/abs/2107.11548>

Two answers to the baking problem. Planeverb runs a *2D* wave solve on one CPU core around the listener,
fully dynamic (destruction, moving walls), producing diffraction, obstruction and arrival direction; it
is a DigiPen proof of concept under MIT with a patent caveat, and struggles with concave geometry. The
portal paper keeps the precomputed 3D solution and corrects it at runtime for doors that open and close,
searching only the portals along the diffracted shortest path; demonstrated in Unreal 4 with Wwise.
*Bearing:* Forge's caves and buildings will have doors and collapses; the portal correction is the
cheap fix and the one to copy. A 2D wave layer around the listener is a plausible later experiment for
diffraction over terrain.

**Carl Schissler, Dinesh Manocha. "Interactive Sound Propagation and Rendering for Large Multi-Source
Scenes." *ACM Transactions on Graphics*, 2016.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/3072959.2943779> · <http://gamma.cs.unc.edu/MULTISOURCE/>

Backward ray tracing *from the listener* against spherical sources, with distant sources clustered by
relative visibility so cost is sub-linear in their number; late reverb from high-order ray tracing; 200
sources and 50+ reflection orders at interactive rates on a multi-core PC. The paper behind the
architecture Steam Audio's reflection path uses.
*Bearing:* the recipe for an open world with hundreds of emitters: trace from the listener, cluster the
far field, give every cluster one reverb estimate.

**Carl Schissler, Gregor Mückl, Paul Calamia. "Fast Diffraction Pathfinding for Dynamic Sound
Propagation." *ACM Transactions on Graphics* 40(4), 2021.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3450626.3459751>

Diffraction on dynamic scenes without baking: a preprocessing pass keeps only silhouette-like edges that
matter for diffraction, then at runtime bidirectional path tracing plus A* over an edge visibility graph
finds high-order diffraction paths, each weighted by the Uniform Theory of Diffraction. Written at
Facebook Reality Labs by the Steam Audio and Meta lineage.
*Bearing:* the algorithm to implement if Forge wants sound to bend around a cliff or a doorway *without*
probes; it is what would let Steam Audio's baked pathing be replaced for streamed procedural geometry.

**Shiguang Liu, Dinesh Manocha. "Sound Synthesis, Propagation, and Rendering: A Survey." arXiv
2011.05538, 2020 (rev. 2021).** [paper] [recent]
<https://arxiv.org/abs/2011.05538>

The one survey covering all three problems from the graphics side: harmonic, texture, spectral and
physics-based synthesis; wave, geometric and hybrid propagation; rendering; and the first wave of
learning-based methods. Best used as an index into the literature above.
*Bearing:* read after this file to find the paper for any specific effect the demo needs.

### 2.4 Physics outdoors and under water

**ISO. "ISO 9613-1:1993 — Calculation of the absorption of sound by the atmosphere" and "ISO
9613-2:2024 — Engineering method for the prediction of sound pressure levels outdoors."** [docs]
[foundational / still-current]
<https://www.iso.org/standard/17426.html> · <https://www.iso.org/standard/74047.html>

Part 1 is the per-band atmospheric absorption the prior project already implements. Part 2 (new 2024
edition, replacing 1996) is the environmental-noise engineering method: geometric divergence,
absorption, *ground effect*, barrier diffraction, foliage and industrial-site attenuation, with a
meteorological correction; it deliberately assumes *favourable* conditions (downwind, or a night-time
inversion) and a linear temperature gradient. Wind and temperature effects are therefore a single
correction term, not a model.
*Bearing:* Part 2's foliage and ground terms are what a forest and a meadow do to a distant sound and
are cheap per-metre tables; use them as the outdoor attenuation LOD (§9). Do not expect the standard to
give you upwind shadow zones.

**Keith Attenborough. "Sound Propagation in the Atmosphere." In *Springer Handbook of Acoustics*,
Springer, 2007, ch. 4.** [book] [still-current]
<https://link.springer.com/rwe/10.1007/978-0-387-30425-0_4>

The physics behind Part 2: ground waves and surface impedance from porosity, refraction by wind and
temperature gradients (downward at night and downwind, upward by day and upwind, producing *shadow
zones* a few hundred metres out), scattering by turbulence that fills those zones in, and the spreading
and absorption terms. Short-range, community-noise scope — exactly a game's scale.
*Bearing:* the source for a *usable* wind/temperature model: a range factor by wind direction plus a
day/night term reproduces the first-order effect (louder downwind and at night), which is what a
player notices. Refraction proper is not worth simulating.

**Kenneth Sørensen, Jakob Christensen-Dalsgaard, Magnus Wahlberg. "Is Human Underwater Hearing Mediated
by Bone Conduction?" *Hearing Research*, 2022.** [paper] [recent]
<https://www.sciencedirect.com/science/article/pii/S0378595522000557> (open summary:
<https://portal.findresearcher.sdu.dk/en/publications/is-human-underwater-hearing-mediated-by-bone-conduction/>)

Measures divers' thresholds and directional hearing: below 1 kHz humans hear *better* under water than
bone-conduction thresholds predict (the air-filled middle ear resonates), but localisation is very poor —
subjects could place a 700 Hz source only within about 50° of azimuth. Sound travels at ~1,500 m/s, so
interaural time differences shrink ~4.3× and the head is acoustically transparent.
*Bearing:* confirms the prior project's underwater rule and sharpens it: keep low frequencies loud and
present, collapse the stereo image (cap interaural delay by `c`, remove head shadow), and treat the
water surface as a −29 dB interface. Swimming and diving need no separate "muffled" preset if the medium
model does this.

### 2.5 How large open-world games actually do it

**Bradley D. Meyer, Josh Lord (Sucker Punch). "Crafting Ghost of Tsushima's Tremendous Sound." *A
Sound Effect* interview, 2020-07-22** · **Bill Rockenbeck. "Blowing from the West: Simulating Wind in
Ghost of Tsushima." GDC 2021.** [web] [talk] [recent]
<https://www.asoundeffect.com/ghost-of-tsushima-sound/> ·
<https://www.gamedeveloper.com/audio/see-how-i-ghost-of-tsushima-s-i-guiding-wind-came-to-life-at-gdc-2021>

The wind is a gameplay simulation (Rockenbeck: a GPU flow field toward the objective driving cloth,
grass and hundreds of thousands of particles) *and* an audio system: the guiding gust travels on three
splines toward the player's objective and takes the timbre of what it crosses — rustling grass on one
side, clacking bamboo on the other — with flute-like recordings to separate it from ambient wind.
Ambience is a data-driven procedural system (which species plays when, where, how often) rather than
hand-placed emitters, and the small team relied on occlusion and reflection features to cover a
continent.
*Bearing:* the wind Forge already simulates for vegetation should *be* the wind audio's input, sampled
along a few rays around the listener and rendered per biome material; and ambience must be a rule set
over the ecosystem, since nobody will hand-place emitters in a procedural world.

**Mike Niederquell (Sony Santa Monica). "The Sound Design for 'God of War'." GDC 2019.** [talk]
[recent]
<https://gdcvault.com/play/1026054/The-Sound-Design-for-God> (slides:
<https://sms.playstation.com/media/documents/GDC2019_SMS_Mike_Niederquell_The_Sound_Design_of_God_of_War.pdf>)

A workflow talk: aesthetic direction, technical constraints, mixing philosophy, dialogue implementation,
and deconstructions of set pieces. The relevant content for an engine is the *mix hierarchy* and the
priority of dialogue and the player's own actions over the world.
*Bearing:* the mix rules a designer will ask Forge for on day one: player actions and dialogue on top,
HDR culling below, and a state-driven snapshot per gameplay mode.

## 3. Mixer and DSP architecture

**Ross Bencina. "Real-time audio programming 101: time waits for nothing." 2011.** [web]
[foundational]
<http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing> (mirror
discussion: <https://lwn.net/Articles/452630/>)

The rules of the audio callback, stated once and for all: never take a lock, never allocate, never do
I/O, never call anything with unbounded execution time, because one late buffer is an audible glitch and
the callback runs on a real-time thread the OS cannot rescue. Communicate with the rest of the program
only through lock-free structures — atomics, single-producer/single-consumer ring buffers, command
queues — and hand memory across in messages so it is freed off-thread.
*Bearing:* the prior project's mixer obeyed this and should be kept; the new requirement is that *every*
third-party node in the graph (Steam Audio effects, decoders, resamplers) obeys it too, which means
decoding and simulation live on their own threads and only finished blocks and parameters cross into
the callback.

**Microsoft. "Low Latency Audio" (Windows 10+ WASAPI, `IAudioClient3`).** [docs] [still-current]
<https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio>

Windows 10 cut the engine's own latency to ~1.3 ms and lets drivers declare their minimum period;
`IAudioClient3::GetSharedModeEnginePeriod` reports the range and `InitializeSharedAudioStream` requests
it, so a *shared-mode* stream can run at 128 samples (2.67 ms at 48 kHz) on the inbox HDAudio driver
instead of the 10 ms default. Exclusive mode and ASIO remain the low-latency paths for pro drivers.
Audio threads should be MMCSS "Audio"/"Pro Audio" work items so the scheduler protects them.
*Bearing:* a game does not need ASIO; a 5–10 ms shared-mode block (256–512 frames) on WASAPI is the
right default, with the block size and the resulting output latency shown in the profiler. `cpal` does
not expose `IAudioClient3` period negotiation; the `wasapi` crate (§7) does if it ever matters.

**William G. Gardner. "Efficient Convolution without Input-Output Delay." *Journal of the AES* 43(3),
1995, 127–136.** [paper] [foundational]
<https://aes.org/e-lib/browse.cfm?elib=7957> (PDF:
<https://people.montefiore.uliege.be/josmalskyj/files/Gardner1995Efficient.pdf>)

Partitioned convolution: the head of the impulse response is convolved directly (zero delay), the tail in
FFT blocks whose sizes double (N, N, 2N, 2N, 4N …) so each block's result is due exactly when its
input has been collected, giving even CPU load and no added latency. Every real-time convolution reverb
is a descendant.
*Bearing:* the algorithm for Forge's convolution reverb and for applying Steam Audio's or a measured
room's impulse response; implement non-uniform partitions once, in the mixer, and reuse for HRTF
filtering of ambisonic channels.

**Jon Dattorro. "Effect Design, Part 1: Reverberator and Other Filters." *Journal of the AES* 45(9),
1997, 660–684** · **Jean-Marc Jot, Antoine Chaigne. "Digital Delay Networks for Designing Artificial
Reverberators." AES 90th Convention, 1991, preprint 3030.** [paper] [foundational]
<https://ccrma.stanford.edu/~dattorro/EffectDesignPart1.pdf> ·
<https://aes2.org/publications/elibrary-page/?id=5663>

The two algorithmic reverbs worth owning. Dattorro's plate is a fully specified recursive network
(input diffusion, a figure-eight tank of all-passes and delays) with a handful of controls — decay,
diffusion, damping, bandwidth — that sounds good and costs almost nothing. Jot's feedback delay network
generalises Schroeder to any unitary feedback matrix with per-line absorptive filters, so decay time
per frequency band is set analytically from a target RT60.
*Bearing:* the FDN is the runtime reverb for a procedural world because its parameters *are* the
acoustic parameters (RT60 per band from Sabine over the room's materials, or from a ray-traced energy
decay); Dattorro is the fallback preset when there is no geometry to compute from.

**Erik de Castro Lopo. "libsamplerate (Secret Rabbit Code)"** · **Henrik Enquist. `rubato` 5.0.0.**
[code] [still-current]
<https://libsndfile.github.io/libsamplerate/> · <https://crates.io/crates/rubato>

libsamplerate is the quality reference — its best sinc converter reaches 145 dB SNR with a passband to
96 % of Nyquist and supports time-varying ratios — and the number to test a Rust resampler against.
`rubato` (MIT OR Apache-2.0, August 2026) is the mature Rust resampler, synchronous and asynchronous
(varying ratio), sinc-based and FFT-based, designed for real-time use with preallocated buffers.
*Bearing:* one resampler per voice at load or decode time (assets at 48 kHz, device rate whatever it
is), and a *varying-ratio* path for Doppler on streamed sources; measure its SNR against libsamplerate
once in CI.

**Philip Deljanov. "Symphonia" 0.6.1** · **`opus` 0.4.0 (libopus bindings).** [code] [recent]
<https://github.com/pdeljanov/Symphonia> · <https://crates.io/crates/opus>

Symphonia (MPL-2.0, August 2026) is the pure-Rust demuxer/decoder set: Vorbis, FLAC, MP3, AAC-LC, ALAC,
ADPCM and PCM over OGG, MP4, MKV, WAV, AIFF and CAF, within ±15 % of FFmpeg, with gapless playback where
the container allows. **Opus decoding is still not implemented**; the `opus` crate (MIT/Apache-2.0,
August 2026) binds libopus for it, which is also what voice chat (§6) needs.
*Bearing:* stream music and long ambiences as Vorbis through Symphonia on a decode thread; keep short
effects as PCM in memory; use libopus for voice and, if disk size matters, for speech assets. MPL-2.0 is
file-level copyleft and compatible with a proprietary engine, but note it in the licence audit.

## 4. Procedural and synthesised audio

**Andy Farnell. *Designing Sound.* MIT Press, 2010 (ISBN 9780262014410).** [book] [foundational]
<https://mitpress.mit.edu/9780262014410/designing-sound/>

The procedural-audio textbook: sound as a *process* rather than a recording, built from first principles
in Pure Data — wind, rain, fire, water, footsteps, machines, creatures — each chapter an analysis of the
physical mechanism followed by a synthesis model with a few meaningful controls. The prior project's
footstep and wind models come from here.
*Bearing:* the design method for every synthesised source in Forge, and the reference that makes the
"physical parameter in, sound out" contract credible to a designer.

**Kees van den Doel, Paul G. Kry, Dinesh K. Pai. "FoleyAutomatic: Physically-Based Sound Effects for
Interactive Simulation and Animation." SIGGRAPH 2001** · **Nicolas Bonneel, George Drettakis, Nicolas
Tsingos, Isabelle Viaud-Delmon, Doug L. James. "Fast Modal Sounds with Scalable Frequency-Domain
Synthesis." SIGGRAPH 2008.** [paper] [foundational / still-current]
<https://dl.acm.org/doi/10.1145/383259.383322> ·
<https://history.siggraph.org/learning/fast-modal-sounds-with-scalable-frequency-domain-synthesis-by-bonneel-drettakis-tsingos-viaud-delmon-and-james/>

Modal synthesis for games: each object is a bank of damped resonators (frequencies, dampings, gains
from a modal analysis or a material preset) excited by contact forces from the physics engine at audio
rate — impacts, rolling and sliding included. Bonneel et al. make it scale: summing modes in the
short-time Fourier domain gives 5–8× speedups, and auditory masking decides which of hundreds of
colliding bodies are worth synthesising.
*Bearing:* the collision sound system for a physics-heavy world (rockfalls, vehicles, debris): modal
banks per material class, excited by the contact impulse, budgeted by masking. Integrates naturally with
HDR loudness culling.

**James F. O'Brien, Perry R. Cook, Georg Essl. "Synthesizing Sounds from Physically Based Motion."
SIGGRAPH 2001** · **James F. O'Brien, Chen Shen, Christine M. Gatchalian. "Synthesizing Sounds from
Rigid-Body Simulations." SCA 2002.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/383259.383321> ·
<http://graphics.berkeley.edu/papers/Obrien-SSR-2002-07/Obrien-SSR-2002-07.pdf>

The other root of physically based sound: compute the surface vibration of deformable, then rigid,
bodies from the simulation itself and radiate it, with the 2002 paper's precomputed modal decomposition
making rigid bodies cheap enough to be interactive. Together with FoleyAutomatic it established that the
physics engine's contact data is a sufficient audio input.
*Bearing:* justification for the interface Forge needs between physics and audio — contact point,
normal impulse, relative tangential velocity, both materials — and nothing else.

**Zhimin Ren, Hengchin Yeh, Ming C. Lin. "Example-Guided Physically Based Modal Sound Synthesis." *ACM
Transactions on Graphics* 32(1), 2013.** [paper] [still-current]
<https://dl.acm.org/doi/10.1145/2421636.2421637> · <http://gamma.cs.unc.edu/AUDIO_MATERIAL/>

Solves modal synthesis' practical problem: nobody knows the damping parameters of "old oak". From one
recorded hit the method extracts perceptual features and fits the Rayleigh damping and stiffness
parameters that make the modal model sound like the recording, then transfers them to any shape of the
same material.
*Bearing:* the material-authoring workflow — record one hit per material, fit once, synthesise
everything — which is how a small team gets a consistent material library without a sample library.

**Yoshinori Dobashi, Tsuyoshi Yamamoto, Tomoyuki Nishita. "Real-time Rendering of Aerodynamic Sound
Using Sound Textures Based on Computational Fluid Dynamics." *ACM Transactions on Graphics* 22(3),
2003.** [paper] [foundational]
<https://dl.acm.org/doi/10.1145/882262.882339>

Aerodynamic sound (a swung sword, wind past a wire) from CFD: precompute the vortex sound a shape emits
at a set of speeds into *sound textures*, then at runtime blend and pitch them by the object's actual
velocity. The rigorous version of the Strouhal-frequency whistle the prior project used for wind past
thin obstacles.
*Bearing:* the model for vehicle wind noise, rotor wash and weapon swings: a per-shape texture indexed by
airspeed, no CFD at runtime.

**Charles Verron, George Drettakis. "Procedural Audio Modeling for Particle-Based Environmental
Effects." AES 133rd Convention, 2012.** [paper] [still-current]
<http://www-sop.inria.fr/reves/Basilic/2012/VD12/>

Fire, wind and rain from five physically inspired *sound atoms* (impacts, chirps, noise bursts and the
like) distributed stochastically in time and space, with the same parameters driving the particle
system's graphics so the two stay coupled — more particles, more atoms; faster particles, brighter
atoms.
*Bearing:* the design pattern for Forge's weather audio: the weather simulation's rain rate, wind speed
and fire intensity are the atom densities, spatialised per cell around the listener, so sound and visuals
cannot disagree.

**Shiguang Liu, Haonan Cheng, Yiying Tong. "Physically-Based Statistical Simulation of Rain Sound."
*ACM Transactions on Graphics* 38(4), 2019.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3306346.3323045>

Rain as physics plus statistics: each drop is an impact transient followed by the ringing of the bubble
it entrains (the Minnaert mechanism from Zheng & James' *Harmonic Fluids*, SIGGRAPH 2009), and, since
millions of drops cannot be synthesised individually, per-material *sound textures* are built by
decomposition and resynthesis and driven by rain rate — at 44.1 kHz in real time.
*Bearing:* rain on leaves, rock, water and a vehicle roof from one model with a material parameter;
pair with Verron's atoms for the near field and this for the bed.

## 5. Music systems

**Winifred Phillips. "Horizontal Resequencing and Dynamic Transitions for Game Music Composers" and
"Hybrid Horizontal-Vertical Structure for Game Music Composers" (from her GDC 2021 talk *From Spyder to
Sackboy*). *Game Developer*, 2021.** [talk] [web] [recent]
<https://www.gamedeveloper.com/audio/horizontal-resequencing-and-dynamic-transitions-for-game-music-composers-from-spyder-to-sackboy-gdc-2021-> ·
<https://www.gamedeveloper.com/game-platforms/hybrid-horizontal-vertical-structure-for-game-music-composers-from-spyder-to-sackboy-gdc-2021->

The two mechanisms of adaptive music, by a working composer: **horizontal re-sequencing** — the piece is
cut into segments with marked entry/exit points, quantised to beat or bar, and the engine reorders them
(overtly, as in *Spyder*'s randomised 30-segment levels, or seamlessly as in *Sackboy*); **vertical
layering** — stems added and removed over a running segment (a choir on intensity, a success melody on
a pickup). Her *Sackboy* waltz uses seven horizontal segments for level progress with vertical layers
for variety and reward, and the two never conflict because they answer different questions.
*Bearing:* Forge's music system is a segment graph with transition rules (next-beat, next-bar, end of
segment, stinger) and per-segment stems with RTPC-driven gains; this is the Wwise Music Switch/Playlist
model, and the whole thing is a clock, a scheduler and a bus.

**Hello Games / Paul Weir / 65daysofstatic. "No Man's Sky" generative soundtrack.** [web] [recent]
<https://en.wikipedia.org/wiki/No_Man%27s_Sky> (Development, audio)

The one shipped, credible procedural-music system at scale: ambient sound and score are generated at
runtime from a base of samples and loops composed by 65daysofstatic and audio director Paul Weir, and
the 2025 *Journeys* album was assembled from the same loops. Creature vocalisations are likewise
synthesised from parameters.
*Bearing:* generative music in a shipped game is *rule-driven recombination of composed material*, not
note-level generation; that is what a small team can ship and what the segment graph above already
supports if segments can be sampled by rule instead of by sequence.

## 6. Multiplayer voice chat

**Jean-Marc Valin, Koen Vos, Timothy Terriberry. "Definition of the Opus Audio Codec." IETF RFC 6716,
2012.** [docs] [foundational]
<https://www.rfc-editor.org/rfc/rfc6716>

The codec: 6 kbit/s narrowband speech to 510 kbit/s stereo music, frames of 2.5–60 ms, algorithmic delay
5–65 ms, a SILK linear-prediction layer for speech and a CELT MDCT layer for music with in-band
switching, packet-loss concealment and forward error correction. Royalty-free by its contributors'
declarations; libopus is BSD.
*Bearing:* 20 ms frames at 16–24 kbit/s over the game's UDP transport with a small jitter buffer
(2–4 frames), decoded on the decode thread, then fed into the mixer as a normal *voice* so proximity
chat gets the same propagation and HRTF as everything else — nothing in a cave sounds like a radio.

**Valve. "Steam Voice" (Steamworks)** · **Unity. "Vivox"** · **Discord. "Discord Social SDK."** [docs]
[web] [still-current / recent]
<https://partner.steamgames.com/doc/features/voice> · <https://unity.com/products/vivox-voice-chat> ·
<https://docs.discord.com/developers/discord-social-sdk/overview>

Three ways not to write it. Steam Voice captures and compresses the microphone (`GetVoice`,
`DecompressVoice`, `GetVoiceOptimalSampleRate`) but *does not transport* the data — you send it over your
own networking. Vivox is a hosted service with positional 3D voice, team channels, moderation and a Core
SDK for custom engines (Unity, Unreal or your own); pricing is "start for free" with enterprise tiers.
The Discord Social SDK includes voice, rate-limited, tied to Discord accounts.
*Bearing:* for a co-op game Steam Voice plus Opus over the game's own UDP is the least dependency and
keeps voice inside the propagation model; a hosted service only becomes attractive when moderation and
cross-platform accounts matter.

## 7. The Rust audio ecosystem, 2025–2026

**RustAudio. `cpal` 0.18.2.** [code] [still-current]
<https://github.com/RustAudio/cpal> · <https://crates.io/crates/cpal>

Apache-2.0, August 2026. Backends: WASAPI (default), ASIO and JACK (features) on Windows; CoreAudio on
Apple platforms; ALSA default with JACK, PipeWire and PulseAudio features on Linux; AAudio on Android;
Web Audio and AudioWorklet on wasm. ASIO needs LLVM, the ASIO SDK and Visual Studio at build time. It
opens a stream at a requested format and buffer size and calls you back; it does not negotiate
`IAudioClient3` periods, expose MMCSS, or do spatial output. The `wasapi` crate (0.24.0, MIT, August
2026) covers the Windows-specific gaps.
*Bearing:* the device layer, as before. Wrap it so the callback only pulls finished blocks from the
mixer thread's ring buffer; that keeps a backend hiccup from becoming a mixer design constraint.

**Andrew Minnich (tesselode). `kira` 0.12.4.** [code] [still-current]
<https://github.com/tesselode/kira> · <https://crates.io/crates/kira>

MIT OR Apache-2.0, August 2026. A high-level game audio library: tweens on any parameter, a mixer of
tracks with effects (filters, reverb, EQ, compressor…), a clock system for beat-accurate scheduling, and
*spatial tracks* with distance attenuation and left/right panning relative to a listener; Doppler is on
the roadmap and there is no HRTF, occlusion or acoustics. Backend-agnostic (cpal by default), decoding
through Symphonia.
*Bearing:* the best Rust reference for the *ergonomics* of tweens, clocks and tracks; not a spatialiser.
Forge could use it as the mixer under Steam Audio for a first prototype, but its track model would
constrain the bus/send/HDR design within months.

**Billy Messenger (BillyDM). "Firewheel" 0.14.0.** [code] [recent]
<https://github.com/BillyDM/Firewheel> · <https://crates.io/crates/firewheel>

MIT OR Apache-2.0, September 2026. A mid-level audio *graph* engine: nodes with arbitrary channel
counts in a DAG, a public API for custom nodes, a suite of built-in nodes, silence propagation, and hard
real-time constraints ("no mutexes"); backends for desktop, mobile and wasm; explicitly not a DAW engine.
Planned to become Bevy's default audio engine through `bevy_seedling`, so its roadmap now follows Bevy's
needs, but the core stays independent.
*Bearing:* the strongest candidate for the graph substrate under Forge's own nodes (voices, HRTF,
ambisonic bus, FDN, HDR bus). The alternative is a smaller in-house graph tailored to a fixed bus
tree; §9 argues for Firewheel unless its Bevy coupling grows.

**Sami Perttu. `fundsp` 0.23.0, and the long tail: `fyrox-sound` 1.0.1, `oddio` 0.7.4, `glicol`
0.13.5, `dasp` 0.11.0, `sofar` 0.3.0.** [code] [still-current / stale]
<https://crates.io/crates/fundsp> · <https://crates.io/crates/fyrox-sound> · <https://crates.io/crates/oddio> ·
<https://crates.io/crates/glicol> · <https://crates.io/crates/dasp> · <https://crates.io/crates/sofar>

`fundsp` (MIT OR Apache-2.0, January 2026) is a composable DSP/synthesis graph with an inline graph
notation — the natural home for Farnell-style synthesis patches. `fyrox-sound` (MIT, March 2026) is the
Fyrox engine's library with HRTF, streaming and reverb, usable standalone. `oddio` (last release October
2023) and `dasp` (May 2020) are effectively unmaintained; `glicol` (April 2024) is a live-coding language
under a non-standard licence. `sofar` (March 2026) reads and renders SOFA HRTFs in pure Rust.
*Bearing:* `fundsp` for synthesis nodes, `sofar` to load the datasets of §2.2, `fyrox-sound` as a second
reference implementation of HRTF rendering in Rust; nothing else in the tail is worth a dependency.

**Bartosz Taudul. "Tracy Profiler" 0.14.1 and `tracy-client` 0.19.0.** [code] [still-current]
<https://github.com/wolfpld/tracy> · <https://crates.io/crates/tracy-client>

Nanosecond, remote, frame-and-sampling profiler (August 2026 release; the Rust client three days later)
with zones on any thread, named frames, plots of arbitrary values, lock contention and context-switch
tracking — which is what shows a mixer block that ran late and *why* (a page fault, a pre-empting
thread). The prior project already streams CPU and GPU zones to it.
*Bearing:* every mixer block is a Tracy frame on the audio thread; voice count, block time, headroom to
the deadline and dropped commands are plots; the in-game "Sound" overlay reads the same counters. This is
the open substitute for the middleware profilers.

## 8. Comparison

| Option | Licence / cost | Spatialiser quality | Occlusion / reflections | Tooling | Rust integration | Risk |
|---|---|---|---|---|---|---|
| **Wwise** | Free under US$250K budget, no source; paid tiers above (per title) | HRTF plugins, ambisonics, Spatial Audio rooms/portals | Rooms, portals, diffraction, transmission, geometric early reflections (Reflect) | Best in class: authoring app, profiler, live connect | `rrise` 0.2.3 (2022), raw FFI, needs licensed install | Proprietary, per-title licence, binding stale; designer lock-in |
| **FMOD Studio** | Free under US$200K revenue/yr and US$500–600K budget, attribution required; paid tiers above | Built-in panner + plugins (Steam Audio, Resonance, Meta) | Via plugins; no native geometry | DAW-style authoring, Live Update, profiler | `libfmod` 2.222.6 (2024), `fmod-oxide` 0.2.2 (2026) | Proprietary libs distributed outside the crate; licence is per title |
| **Steam Audio + own mixer** | Apache-2.0 (SDK) + MIT/Apache (`audionimbus`), own code | HRTF (built-in or SOFA), ambisonics, directivity, air absorption | Real-time direct occlusion/transmission; real-time or baked reflections; baked pathing | None: events, buses, RTPC, profiler all to be written | `audionimbus` 0.16.0 (Aug 2026), Bevy example exists | Steam Audio quiet since Feb 2025; tooling is a year of work |
| **kira** | MIT OR Apache-2.0 | Distance attenuation + pan; no HRTF | None | None (code-driven) | Native | Would be replaced within months; no acoustics path |
| **Fully custom** | Own code | Whatever you build (structural HRTF exists from the prior project) | Own ray tracer against terrain SDF and BVH; own diffraction | None | Native | Largest scope; spatialiser quality below Steam Audio for a long time |

## 9. Recommendation for Forge

**Architecture.** `cpal` for devices → an in-house *mixer* whose graph substrate is Firewheel (or a
minimal in-house DAG if Firewheel's Bevy coupling becomes a problem; the node API is small enough to
swap) → Steam Audio through `audionimbus` for HRTF, direct occlusion/transmission, real-time reflections
and, where probes exist, pathing → an ambisonic (3rd-order, AmbiX) spatial bus for beds and reverb
returns, decoded to binaural, speakers or an `ISpatialAudioClient` bed → an HDR master bus. Carry over
the prior project's medium model (`c`, `Z`, ISO 9613-1 absorption, interface loss, `c`-scaled interaural
cues) as the *distance/medium* stage that feeds Steam Audio's direct effect, and its Woodworth/Brown–Duda
structural HRTF as the fallback renderer when the medium is not air. Own-code DSP: non-uniform partitioned
convolution (Gardner), an FDN reverb parameterised by RT60 per band (Jot) with a Dattorro preset, biquad
EQ, a compressor/limiter, `rubato` resampling with a varying-ratio path for Doppler. Threads: game thread
(events, RTPCs, listener), *simulation* thread (Steam Audio + own outdoor propagation, 10–30 Hz per
source by LOD), *decode* thread (Symphonia/libopus into ring buffers), *audio* thread (mixer, lock-free
in and out, Tracy-instrumented). Voice chat is Opus over the game's own UDP, arriving in the mixer as a
voice like any other.

**Data model for events.** Steal Wwise's vocabulary and nothing more: `Event { actions: [Play(sound,
bus, priority, loudness_db), Stop, SetRtpc, SetState, Post(event)] }`; `Sound` is either a *container*
(random / sequence / blend / switch over children) or a leaf (`Sample`, `Stream`, `Synth(patch,
params)`); every property may be an RTPC curve over a named game parameter; `Bus` nodes form a tree with
sends to aux buses (reverb, ambisonic) and an HDR window per bus; `State` groups select mix snapshots.
All of it in RON, hot-reloadable, referenced from code by interned name, validated at load (unknown
fields rejected, as the prior project learned with scenes). The profiler is Tracy plus an in-game
overlay of voices, buses, block time, virtual/real voice counts and dropped commands. Voice management is
HDR first (loudness decides), then a per-bus playback limit with priority and "kill / virtualise / resume
from elapsed time" behaviours, so hundreds of emitters cost nothing until they matter.

**Acoustics LOD in a big world.** Near (≤ 50 m, ~32 sources): Steam Audio direct + real-time reflections
against the streamed chunk geometry and object BVHs, one FDN per acoustic cell (cave, building, open),
arrival direction from the shortest path. Mid (50–500 m, ~128 sources): direct occlusion by a few rays
against the terrain SDF and foliage density, ISO 9613-2 ground and foliage attenuation, Attenborough's
wind-direction/day-night range factor, cluster reverb à la Schissler. Far (> 500 m, hundreds): virtual
voices with logical loudness only, promoted when HDR says so. Rooms and portals are derived from the
world model (cave volumes, building interiors) with Raghuvanshi's portal correction for doors; pathing
probes are baked per chunk when a chunk becomes resident and dropped with it. Weather audio is
Verron/Liu atoms and textures driven by the weather simulation's fields; wind audio samples the same wind
field vegetation uses, as Tsushima does; impacts are modal banks fitted per material (Ren) excited by
physics contacts and budgeted by masking (Bonneel).

**The demo that proves it.** A walk at dusk: start in a meadow in wind (wind field → whistle and rustle
by biome; birds from the ecosystem rules; HDR keeps it airy), enter a forest (ISO foliage attenuation
dulls a distant waterfall; reflections thin; footsteps change with ground material), reach a river and
wade in (rain begins: atom density from the weather field; the surface interface and collapsed stereo
image the moment the head goes under), climb out into a cave mouth (direct path occluded around the
rock, arrival direction shifts to the opening, FDN switches to the cave's RT60 with the transition
crossfaded), walk deeper while a friend on voice chat stays outside (their voice diffracts around the
entrance and reverberates with the cave, then goes dry when a rockfall closes the portal), and finally a
vehicle drives past the entrance (modal impacts, aerodynamic noise by airspeed, Doppler through the
varying-ratio resampler). Every one of those transitions is a number changing in the profiler overlay,
and the golden test renders the same walk offline to WAV and checks band energies, delays and
arrival directions against tolerances.

## 10. Checked and left out

- **"The Sound of Horizon Forbidden West" (GDC talk)** — no such session. Guerrilla's GDC 2023 and 2024
  line-ups (living world, cauldrons, tools, packaging, cinematics, relic ruins, UI) contain no audio talk.
  What exists is a Krotos interview with the Horizon audio team
  (<https://sound.krotosaudio.com/horizon-video-game-sound-design/>) on the music/ambience hand-off
  philosophy, and Guerrilla's own "Guerrilla Listens to the Sounds of Horizon Forbidden West" video;
  neither describes the systems, so neither is an entry.
- **Red Dead Redemption 2 audio talk at GDC** — none found. Rockstar's GDC 2020 talk on RDR2 covers
  wildlife animation, not audio; the only audio analysis located is a third-party YouTube essay (Cujo
  Sound). Left out.
- **Battlefield audio propagation and Dolby** — DICE's propagation/occlusion talks for BF3/BF1/BF2042
  could not be located this session (search budget exhausted before a GDC Vault query), and the Frostbite
  HDR article at ea.com returned 404; HDR audio is cited from the Develop 2008 and Frostbite decks
  instead. Dolby Atmos in Battlefield is not cited; the platform path is covered by the Microsoft Spatial
  Sound entry.
- **"RTX audio"** — no such NVIDIA product exists; VRWorks Audio 2.0's RTX acceleration (2019) is the
  real thing and is cited. NVIDIA's later "RTX Voice"/Broadcast is noise removal, unrelated, and was not
  fetched.
- **The Wwise and FMOD documentation pages** (virtual voices, Spatial Audio, interactive music, Live
  Update) — audiokinetic.com returns 403 to automated fetches and fmod.com renders client-side, so the
  feature descriptions in §1 and §8 rest on the reachable third-party pages cited and on the vendor
  URLs as leads. Verify in a browser before quoting them.
- **`lewton` and `rodio`** — not fetched this session; not cited. `rodio` is a playback library rather
  than a game mixer and would not change §7's conclusion.
- **Paul Weir's GDC 2017 "The Sound of No Man's Sky" and the "VocAlien" creature-voice tool** — the
  talk could not be reached (GDC Vault search needs a browser) and the tool name is unverified; the
  system is cited from Wikipedia's development section instead.
- **NVIDIA/AMD GPU convolution ("TrueAudio Next")** — Steam Audio's docs mention Radeon Rays for GPU
  ray tracing; a TrueAudio Next convolution path was not confirmed in the fetched guide and is not
  claimed.
- **Vivox's free-tier PCU threshold** — third-party blogs quote "free up to 5,000 PCU"; Unity's own page
  says only "start for free". Not stated as fact.
- **CIPIC's licence text** — the dataset is described as public domain in its own paper and mirrored as
  such, but the original UC Davis licence page is offline; treat as public domain with attribution.

## 11. Verification notes

- Crate versions and dates come from the crates.io API on 2026-09-23: `cpal` 0.18.2 (2026-08-16),
  `rubato` 5.0.0 (2026-08-10), `symphonia` 0.6.1 (2026-08-13), `fundsp` 0.23.0 (2026-01-07), `kira`
  0.12.4 (2026-08-27), `firewheel` 0.14.0 (2026-09-05), `audionimbus` 0.16.0 (2026-08-08), `opus` 0.4.0
  (2026-08-23), `wasapi` 0.24.0 (2026-08-12), `sofar` 0.3.0 (2026-03-14), `fyrox-sound` 1.0.1
  (2026-03-28), `tracy-client` 0.19.0 (2026-08-25), `fmod-oxide` 0.2.2 (2026-07-14), `libfmod` 2.222.6
  (2024-10-15), `glicol` 0.13.5 (2024-04-23), `oddio` 0.7.4 (2023-10-15), `rrise` 0.2.3 (2022-11-17),
  `dasp` 0.11.0 (2020-05-29), `audiopus` 0.3.0-rc.0 (2021, not cited).
- Steam Audio release dates from the GitHub releases page (4.8.1 on 2025-02-11, 4.8.0 on 2024-11-26,
  4.7.0 on 2024-08-11); Tracy 0.14.1 from the GitHub releases API (2026-08-22); Project Acoustics and
  Resonance Audio archive dates from their repository banners.
- Papers were confirmed on the ACM DL, Microsoft Research, Inria (Basilic), arXiv, SIGGRAPH History
  Archive, AES e-library, CCRMA and author pages; Dobashi 2003 and Liu 2019 authorship via the Crossref
  API after the ACM pages refused the fetcher. Springer pages were reached through their cookie redirect.
- Licensing thresholds: Wwise from GameFromScratch (2023-02-01) with Audiokinetic's pricing page as the
  primary URL (403 to the fetcher); FMOD from *Game Developer* (2020-12-03), GameFromScratch and
  Wikipedia, which disagree on the budget cap (US$500K in 2020, US$600K now) — both are given.
- The web-search budget ran out after the first three batches; the remaining ~35 verifications were
  done by direct fetch of known URLs, which is why a few vendor pages are recorded as unreachable rather
  than as confirmed.
- No browser pane and no YouTube page was used for any citation. Talks were confirmed from GDC Vault
  (God of War, 2019), the *Game Developer* GDC coverage (Ghost of Tsushima wind, 2021; Phillips, 2021),
  SlideShare decks (DICE, 2008–2010) and the Guerrilla and Sucker Punch pages named in each entry.
- Prior-project corrections: Steam Audio 4.8.1 is February **2025** (the earlier file said 2026); HRTF
  dataset licensing (Q-30) is settled — all three canonical datasets are shippable with attribution.
