# Research — Water: the open ocean, shores, rivers and lakes, shading, and the hand-off from genesis

> Companion to `physics-fluids.md` §5 (the surface tier of D-009's water: "far = analytic spectrum
> evaluated identically on CPU and GPU") and to `terrain-genesis.md` §2 (the hydrology the island
> already computes: drainage, rivers above 0.5 km² of catchment, lakes as flooded depressions).
> Written 2026-09-25 for Phase 2's third item, "Water surface: FFT ocean far, flow-mapped rivers,
> shore handling", for the `island` demo (`docs/demos/island.md`), where today the sea at 0 m is flat
> ground under the cluster-DAG island. Every citation was checked that day against a reachable page
> or, where the network proxy refused the host, against the search engine's record of it; the grade
> is kept per entry under [Verification notes](#verification-notes), and what could not be found is
> under [Checked and left out](#checked-and-left-out). Entries `physics-fluids.md` already carries
> (Tessendorf, Finch, Sea of Thieves, Malan, Vlachos, Chentanez & Müller) are re-verified and placed
> where the island needs them rather than repeated at length.

The question is what the published work says about drawing the water of a 16 km island — an open
sea to the horizon, a coast with beaches and cliffs, rivers traced from a drainage network, lakes at
known levels — so that Forge's water can be built in order, on the renderer it has (a visibility
buffer, a render graph with a compute queue, bindless images, Slang, TAA), without the shimmer and
the tiling the owner sees at once. The short answer: the open ocean has been one technique since
1999 — a directional spectrum (Phillips, then the oceanographers' JONSWAP and its finite-depth TMA
form) synthesised by inverse FFT into a tiling displacement, run today as three or four cascades of
different patch sizes so that no tile repeats, with foam where the Jacobian of the displacement goes
negative — and its GPU cost is a small fraction of a millisecond; what separates a shipped ocean from
a demo is the LOD of the grid (clipmap rings or a quadtree, never a projected grid alone), the
anti-aliasing of the sun's glitter (unresolved waves become roughness), and the shore. Shores are
where every studio spends its years: depth-based colour is a Beer–Lambert integral, but breaking
waves are baked animation (Horizon Forbidden West, Skull and Bones) or boundary-aware procedural
waves (Jeschke 2020), both driven by one field — the distance to the shore and the depth of the
floor — which a generated island gets for free from genesis. Rivers are flow maps (Valve 2010)
over ribbons traced from the polyline network, with widths and depths from the drainage area
(Peytavie 2019) and flow directions from the same D8 field the erosion used; Far Cry 5 generated
exactly these from its splines. Lakes are flat planes at the Fill–Spill–Merge level. The engines
(Unreal's Water plugin, Unity's HDRP water) converge on the same shape: bodies defined by splines
or polygons, a quadtree or cascade mesh, a per-zone "water info" texture the shaders and gameplay
read, Gerstner or FFT bands per body type.

> **State of the art in five sentences.** An open ocean is a Fourier synthesis of a directional
> wave spectrum (Tessendorf 1999–2004 over Phillips 1958; Horvath 2015 for JONSWAP/TMA with a
> swell parameter), run as three or four cascades of 256²–512² FFTs per frame on the GPU, whose
> displacement, slopes and Jacobian feed the surface, its normals and its whitecaps (Dupuy &
> Bruneton 2012). The surface is a viewer-centred mesh — clipmap rings or a quadtree with the data
> in cascaded textures (Crest 2017–2019, Unreal's water mesh) — whose sub-pixel waves are folded
> into the BRDF's roughness so the glitter does not alias (Bruneton, Neyret & Holzschuch 2010, over
> Cox & Munk 1954 and Ross 2005). Near the coast the spectrum's depth term (TMA) damps the swell,
> colour and transparency follow Beer–Lambert through the depth to the floor (Pope & Fry 1997;
> Jerlov types), and breaking waves are either baked shapes streamed per tile (Guerrilla 2022,
> Ubisoft Singapore 2024) or boundary-aware procedural waves with a particle system for the crest
> (Jeschke et al. 2018, 2020), all keyed on the distance to shore. Rivers are flow-mapped normals
> and foam (Vlachos 2010; Far Cry 5's automatic flow maps from splines) on ribbons carved from a
> hydrological network with widths and depths from drainage area (Génevaux 2013, Peytavie 2019),
> waterfalls where the slope breaks (Emilien 2015), lakes as planes at their fill level. Shading is
> Fresnel, a sun glitter from a slope distribution, the sky and the scene reflected (screen-space or
> ray-traced), and a scattering colour that brightens thin wave crests against the sun (Sea of
> Thieves 2018); everything the water needs from the terrain — shore distance, floor depth, river
> polylines, flow direction, lake levels, a water-body mask — is a by-product of genesis and is
> baked at the heightfield's own resolution.

**Contents**

1. [The open ocean: spectra, the FFT, cascades, foam, the mesh](#1-the-open-ocean-spectra-the-fft-cascades-foam-the-mesh)
2. [Shores and shallow water](#2-shores-and-shallow-water)
3. [Rivers, lakes and waterfalls](#3-rivers-lakes-and-waterfalls)
4. [Shading the surface](#4-shading-the-surface)
5. [What genesis hands to the water](#5-what-genesis-hands-to-the-water)
6. [Water in professional engines and shipped games](#6-water-in-professional-engines-and-shipped-games)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [What the numbers say](#what-the-numbers-say)
9. [Checked and left out](#checked-and-left-out)
10. [Verification notes](#verification-notes)

---

## 1. The open ocean: spectra, the FFT, cascades, foam, the mesh

One idea underlies this section. A wind-driven sea is a random superposition of linear waves whose
amplitudes follow a measured spectrum `S(k)` and whose phases advance by the dispersion relation
(`ω² = g·k` in deep water, `ω² = g·k·tanh(k·h)` over a floor at depth `h`); sampling that spectrum on
a grid of wave vectors and inverse-transforming it gives a height field that tiles, animates without
state, and can be evaluated at any time from the seed alone. Everything below is a spectrum to
sample, a way to hide the tile, a by-product of the transform (foam, slopes) or a mesh to draw it
on.

**Owen M. Phillips. "The equilibrium range in the spectrum of wind-generated waves." *Journal of
Fluid Mechanics* 4(4), 1958, 426–434.** [paper] [foundational]
<https://www.cambridge.org/core/journals/journal-of-fluid-mechanics> (the article record; DOI
10.1017/S0022112058000550 from memory, see §10)

The spectrum every graphics ocean started from: on dimensional grounds alone, a fully developed
sea's spectrum falls as `k⁻⁴` in the equilibrium range because wave slopes are capped by breaking,
which is the origin of the `A · exp(−1/(kL)²) / k⁴` form Tessendorf popularised.
*Bearing:* the Phillips spectrum is the one to implement first (three parameters: amplitude, wind
speed through `L = V²/g`, wind direction) and the one to replace, since it has no peak enhancement
and no fetch: its seas look the same at every distance from shore.

**Klaus Hasselmann et al. "Measurements of wind-wave growth and swell decay during the Joint North
Sea Wave Project (JONSWAP)." *Deutsche Hydrographische Zeitschrift*, Ergänzungsheft A8(12), 1973,
1–95; with Evert Bouws, Heinz Günther, Wolfgang Rosenthal, C. Linwood Vincent, "Similarity of the
wind wave spectrum in finite depth water: 1. Spectral form", *Journal of Geophysical Research*
90(C1), 1985, 975–986.** [paper] [foundational]
<https://epic.awi.de/10163/> · <https://agupubs.onlinelibrary.wiley.com/doi/10.1029/JC090iC01p00975>
(DOI 10.1029/JC090iC01p00975)

The oceanographers' spectra. JONSWAP came from "wave spectra measurements along a profile extending
160 kilometers into the North Sea westward from Sylt", with attention to "the attenuation of swell in
water of finite depth"; it adds to Pierson–Moskowitz a peak-enhancement factor and a dependence on
fetch, so a young sea near a windward coast has a sharper, shorter-wavelength peak than an old one.
The TMA (Texel–MARSEN–ARSLOE) form is "a self-similar spectral shape … to describe wind waves in
water of finite depth", "depth dependent and an extension of the deep water JONSWAP spectrum": a
multiplicative depth function that removes energy from long waves where the floor is shallow.
*Bearing:* JONSWAP with a fetch is what makes the sea on the lee and windward coasts of the island
differ; TMA is the shore's spectrum, and its depth factor, evaluated per cascade from the baked floor
depth, is the cheapest wave damping in shallow water (§2). Both are three or four scalars the
designer sets per sea; the seed sets nothing here but the phases.

**Jerry Tessendorf. "Simulating Ocean Water." SIGGRAPH course notes, 1999–2004 (the 2001 and 2004
editions are the ones cited).** [paper] [foundational] [still-current]
<https://people.computing.clemson.edu/~jtessen/reports/papers_files/coursenotes2004.pdf> (the 2002
edition: <https://jtessen.people.clemson.edu/reports/papers_files/coursenotes2002.pdf>)

The method itself, and still the reference every open implementation names: Gerstner waves as
the analytic case, then the statistical ocean — Gaussian random amplitudes `h̃₀(k)` from the
Phillips spectrum, the dispersion relation animating them, an inverse FFT giving the height,
the slope and the horizontal "choppy" displacement `λ·D(x,t)` that sharpens the crests. On
breaking: "The Jacobian of the transformation from x to x + λD(x, t) is used as a test for
deciding when the effect is taking place, and it is a measure of the uniqueness of the
transformation" — where it goes negative the surface folds over itself, which is where foam and
spray go. The notes also cover the practical choices: grid resolution, patch size, the repeat
period that makes the animation loop, and the cost.
*Bearing:* the algorithm Forge writes, in one compute pipeline per step (spectrum → time evolution
→ FFT rows → FFT columns → displacement, normals, Jacobian). The random amplitudes must come from
`pcg3d(kx, ky, seed)` rather than a generator so the same sea exists on the CPU (D-016); the
Jacobian is the foam signal; the choppiness parameter is where the owner tunes the look.

**Christopher J. Horvath. "Empirical directional wave spectra for computer graphics." *DigiPro
2015* (Digital Production Symposium, Los Angeles, 8 August 2015), 29–39; with EncinoWaves (C++,
Apache-2.0).** [paper] [code] [still-current]
<https://dl.acm.org/doi/10.1145/2791261.2791267> (DOI 10.1145/2791261.2791267; code
<https://github.com/blackencino/EncinoWaves>)

The paper that moved games from Phillips to the measured spectra: "the practical application of
several empirically-based, directional ocean wave spectra for use in the Fourier synthesis of
animated ocean height fields, using the Texel MARSEN ARSLOE (TMA) empirical model for the
non-directional component of the wave spectrum", with directional spreading functions from Ochi's
*Ocean Waves: The Stochastic Approach*, and "a novel, normalized parameter called 'swell' which
modifies the directional spreading to produce wavelength-dependent elongation of waves into parallel
wave trains". EncinoWaves implements "TMA, JONSWAP, and Pierson Moskowitz" with the spreading
functions and is "Licensed under the Apache License, Version 2.0".
*Bearing:* the spectrum module to port: TMA with fetch, depth, wind speed and direction, spreading,
and Horvath's swell parameter, which is the single most visible control on an open sea (long
parallel swell from a distant storm against short local chop). Apache-2.0, so the code can be read
line by line and credited in `CREDITS.md`.

**Jonathan Dupuy, Éric Bruneton. "Real-time Animation and Rendering of Ocean Whitecaps." *SIGGRAPH
Asia 2012 Technical Briefs*, Article 15; code on GitHub.** [paper] [code] [still-current]
<https://dl.acm.org/doi/10.1145/2407746.2407761> (DOI 10.1145/2407746.2407761; open copy
<https://hal.science/hal-00967078>; code <https://github.com/jdupuy/whitecaps>)

Foam done so it filters: "a scalable method to procedurally animate and render vast ocean scenes with
whitecaps on the GPU", where "whitecap coverage is determined using a wave deformation criterion that
can be pre-filtered linearly", giving "anti-aliased images for scales ranging from centimeter to
planetary in real time". The criterion is the Jacobian of Tessendorf's choppy displacement, but
carried as a mean and a variance per texel so the coverage at a distance is the expectation over the
pixel footprint rather than a thresholded sample.
*Bearing:* the foam the owner will accept under TAA: the Jacobian's mean and variance stored in the
cascade textures with mips, the coverage evaluated from them per pixel, then accumulated and decayed
in a persistent foam texture. The thresholded-per-frame foam of most demos flickers at distance.

**Éric Bruneton, Fabrice Neyret, Nicolas Holzschuch. "Real-time Realistic Ocean Lighting using
Seamless Transitions from Geometry to BRDF." *Computer Graphics Forum* 29(2) (Eurographics 2010),
487–496.** [paper] [still-current]
<https://diglib.eg.org/handle/10.2312/CGF.v29i2pp487-496> (open copy <https://hal.science/inria-00443630>)

"An algorithm for modelling, animation, illumination and rendering of the ocean in real-time, at all
scales and for all viewing distances, based on a hierarchical representation combining geometry,
normals and BRDF": waves larger than a pixel are geometry, waves near a pixel are normals, and
smaller waves become the slope variance of a Gaussian (Cox–Munk-like) BRDF, so the sun's glitter is
correct and stable from the deck to the horizon.
*Bearing:* the cure for the shimmer the owner sees first on any ocean: the high cascades are faded
into roughness by distance instead of being sampled and aliased, with the variance from the
spectrum itself. This is the one entry of the section to implement together with the FFT, not
after it.

**Claes Johanson. "Real-time water rendering: Introducing the projected grid concept." Master's
thesis, Lund University (with Saab Bofors Dynamics), March 2004.** [paper] [foundational]
<https://fileadmin.cs.lth.se/graphics/theses/projects/projgrid/projgrid-lq.pdf>

The other mesh: "a grid mesh whose vertices are even-spaced in post-perspective camera space rather
than in world-space", projected onto the base plane and displaced by the height field, so screen
density is uniform by construction. Its known costs are swimming vertices as the camera moves and
aliasing of the displacement where the projected cells are large; later systems keep it for the far
horizon only.
*Bearing:* not the primary mesh (TAA needs stable, world-anchored vertices for its motion vectors);
worth knowing as the reference the clipmap and quadtree schemes below are measured against, and as
a fallback for the last kilometres to the horizon on the planet variant.

**Huw Bowles et al. (Studio Gobo, later Electric Square / Wave Harmonic). "Crest: Novel Ocean
Rendering Techniques in an Open Source Framework", SIGGRAPH 2017 *Advances in Real-Time Rendering
in Games*; Huw Bowles, Tom Read-Cutting, "Multi-resolution Ocean Rendering in Crest Ocean System",
SIGGRAPH 2019 *Advances*; Crest (Unity, MIT).** [talk] [code] [still-current]
<https://advances.realtimerendering.com/s2017/index.html> ·
<https://advances.realtimerendering.com/s2019/index.htm> · <https://github.com/wave-harmonic/crest>
(documentation <https://crest.readthedocs.io/>)

The open-source ocean that shipped in games and documents its LOD: Crest "uses an efficient Level
Of Detail (LOD) representation for data like surface shape/displacements, foam values, shadowing
data, and water depth, stored in a multi-resolution format using cascaded textures centered at the
viewer, which is then sampled when the ocean surface geometry is rendered" — a clipmap of data
rings with a matching ring mesh, the same structure as Losasso & Hoppe's geometry clipmap
(`terrain-genesis.md` §5) with the height coming from the FFT and the simulation layers. The
repository calls itself "A class-leading water system implemented in Unity", MIT-licensed, "Unity:
2022.3.62f3 or later".
*Bearing:* the LOD to copy: the FFT cascades, the foam, the shore depth and the flow live in
viewer-centred cascaded textures, and one ring mesh reads them all. It removes the choice between
"FFT for the sea" and "flow maps for the river" as separate systems: they are layers of the same
cascade. The MIT code is the reference implementation to read for the ring geometry and the
transitions between rings.

**Tim Tcheblokov (NVIDIA). "Ocean simulation and rendering in War Thunder." CGDC 2015 (also GDC
2015), with NVIDIA WaveWorks in Gaijin's Dagor engine.** [talk] [still-current]
<https://developer.download.nvidia.com/assets/gameworks/downloads/regular/events/cgdc15/CGDC2015_ocean_simulation_en.pdf>
· <https://developer.nvidia.com/waveworks>

The talk the open implementations cite for cascades: several FFT simulations of the same spectrum
at different patch sizes, stored as a texture array and run through one pipeline per layer, so
that the visible tiling of a single patch disappears and detail exists from the horizon to the
bow; WaveWorks packaged it (waves and foam "factoring in wind strength and direction", for
oceans, seas, rivers and lakes) and War Thunder shipped it.
*Bearing:* the number and the sizes of the cascades are the first knobs of the sea's look; the
usual production choice is three to four patches spanning roughly 10 m to 1 km. WaveWorks itself
is vendor SDK territory (D-024's rule: optional and only for its vendor); the technique is not.

**Mark Mihelich (Studio Wildcard), Tim Tcheblokov (NVIDIA). "Wakes, Explosions and Lighting:
Interactive Water Simulation in 'Atlas'." GDC 2019, Advanced Graphics Techniques Tutorial, 18 March
2019.** [talk] [still-current]
<https://www.gdcvault.com/play/1025819/Advanced-Graphics-Techniques-Tutorial-Wakes>

"The challenges faced during implementation of water surface simulation for Atlas, a massive
multiplayer online game", and "the solutions designed, including simulating and rendering ocean
surfaces, interactive effects (gameplay physics and graphics effects), and synchronizing multiple
servers and players": an FFT ocean with cascades, a local interactive layer for wakes and
explosions on top, and the shading (the light through wave crests) that most open
implementations reproduce from this talk.
*Bearing:* the closest published analogue to Forge's brief (a large shared ocean, many servers,
ships): the layering of a global deterministic spectrum with local, visual-only interaction, and
the reminder that the water's height must be agreed on by servers and clients, which a spectrum
from a seed gives and a fluid solver does not. The slides were not readable this session (§10);
the video is on the Vault and YouTube.

**Open FFT-ocean implementations: 2Retr0, GodotOceanWaves (Godot, MIT); Ivan Pensionerov, FFT-Ocean
(Unity); Mozobo, Ocean-Simulation (Unity); tessarakkt, godot4-oceanfft (Godot).** [code] [recent]
<https://github.com/2Retr0/GodotOceanWaves> · <https://github.com/gasgiant/FFT-Ocean> ·
<https://github.com/Mozobo/Ocean-Simulation> · <https://github.com/tessarakkt/godot4-oceanfft>

The state of the practice, readable. GodotOceanWaves: "Given the wind speed (U), depth (D), and
fetch length (i.e., distance from shoreline) (F), the TMA spectrum combines its preceding JONSWAP
spectrum with a depth attenuation function"; "Each cascade has its own tiling size and set of
parameters. Cascades can be added/removed from the generation system dynamically in real-time";
"The Stockham FFT algorithm was used over the Cooley-Tukey algorithm to avoid the initial
bit-reversal permutation"; foam follows Tessendorf ("when the Jacobian of the displacement is
negative") and "accumulates linearly and dissipates exponentially on a texture over multiple wave
updates". Mozobo: "The TMA spectrum extends the JONSWAP spectrum, which models wind-driven waves in
deep water, and adjusts it for the effects of shallow water", "a maximum of 4 cascades", after
Flügge's 2017 GPGPU FFT thesis. godot4-oceanfft pairs the FFT with "Integrated LOD for surface
(CDLOD)" and "Basic buoyancy system".
*Bearing:* four independent implementations converge on TMA, three or four cascades, a Stockham
FFT in compute, Jacobian foam with linear growth and exponential decay, and a clipmap or CDLOD
mesh; that is the design, and these repositories are the ones to diff a first `ocean.slang`
against. None publishes GPU timings, which is why the cost below is an estimate to measure.

**David Algis, Bérenger Bramas, Emmanuelle Darles, Lilian Aveneau. "Arc Blanc: a real time ocean
simulation framework." arXiv:2503.03326, March 2025; and "Real-Time Interactive Hybrid Ocean:
Spectrum-Consistent Wave Particle-FFT Coupling", arXiv:2511.02852, October 2025.** [paper] [recent]
<https://arxiv.org/abs/2503.03326> · <https://arxiv.org/abs/2511.02852>

The recent word. Arc Blanc is "a comprehensive and fully described real-time framework that
simulates the free ocean surface and the coupling between solids and fluid" on the "Tessendorf
method", adding "the real-time velocity of ocean fluids at any depth" for buoyancy. The hybrid
paper states the limit of the FFT plainly — spectral oceans "assume global stationarity and spatial
homogeneity, making it difficult to represent non-uniform seas and near-field interactions" — and
couples "a global FFT background with local wave-particle (WP) patch regions around interactive
objects, jointly driven under a unified set of spectral parameters and dispersion".
*Bearing:* the FFT is the background and stays so; local interaction (wakes, splashes, the shore's
non-uniform sea) is a second layer that must share the spectrum's parameters or it reads as pasted
on. Arc Blanc is the paper to take the buoyancy equations from when D-009's boats arrive.

---

## 2. Shores and shallow water

The open sea is solved; the coast is where the years go (Guerrilla's own account). Four things
change near the shore: the colour (light is absorbed and scattered through the water column to the
floor), the spectrum (long waves feel the floor and slow, steepen and turn toward the beach), the
crest (waves break, foam and run up the sand) and the sand (it is wet where the last wave reached).
All four are functions of two fields — the depth of the floor and the distance to the shore — which
is why a generated island is the easy case.

**Robin M. Pope, Edward S. Fry. "Absorption spectrum (380–700 nm) of pure water. II. Integrating
cavity measurements." *Applied Optics* 36(33), 1997, 8710–8723; with Michael G. Solonenko, Curtis
D. Mobley, "Inherent optical properties of Jerlov water types", *Applied Optics* 54(17), 2015,
5392–5401.** [paper] [foundational] [still-current]
<https://opg.optica.org/ao/abstract.cfm?uri=ao-36-33-8710> (DOI 10.1364/AO.36.008710) ·
<https://opg.optica.org/ao/abstract.cfm?uri=ao-54-17-5392> (DOI 10.1364/AO.54.005392)

The coefficients behind depth colour. Pope & Fry measured pure water's absorption and found "the
absorption in the blue is significantly lower than had previously been believed", with "the
absorption minimum … at 0.0044 ± 0.0006 m⁻¹ at 418 nm" and red absorbed tens of times faster; that
asymmetry is why deep water is blue and why a Beer–Lambert term `exp(−σ(λ)·d)` over the optical
path `d` (down to the floor and back to the eye) gives the colour ramp from the beach outward.
Solonenko & Mobley fitted "absorption a(λ) and scattering b(λ) coefficients … for all Jerlov water
types over the wavelength range 300–700 nm", the standard classification from clear oceanic water
to turbid coastal water with its higher, greener attenuation.
*Bearing:* the depth colour is not a gradient texture but two RGB coefficient triples (absorption,
scattering) per water body, chosen by Jerlov type — clear oceanic for the outer sea, a coastal type
in bays and river mouths, a lake type inland — applied along the path through the depth that the
opaque depth buffer and the sea level give per pixel. The same term, per wavelength, dims the sun
in the caustics and the underwater view.

**Carlos Gonzalez-Ochoa, Doug Holder (Naughty Dog). "Water Technology of Uncharted." GDC 2012 (7
March 2012).** [talk] [foundational] [still-current]
<https://www.gdcvault.com/play/1015517/Water-Technology-of>

The pre-FFT production water most shore techniques descend from: "mesh generation and the flow
shader", a "new ocean system … created specifically for the cruise ship sequence" of Uncharted 3
with "Gerstner waves, b-spline waves, and wave particles", and a shader stack of "normal mapping,
bump mapping, refraction, reflection, depth-based coloring, and foam simulation", with flow maps
to displace and animate the surface.
*Bearing:* the shore-wave recipe in its simplest shipped form: Gerstner trains whose direction and
phase come from a field over the beach (the gradient and the value of the shore distance), so the
crests arrive parallel to the coast and slow down in the shallows without any simulation; depth
colour and foam from the same depth. This is step 2 of the build order.

**Cem Yuksel, Donald H. House, John Keyser. "Wave Particles." *ACM Transactions on Graphics* 26(3)
(SIGGRAPH 2007).** [paper] [foundational] [still-current]
<https://www.cemyuksel.com/research/waveparticles/> (DOI 10.1145/1276377.1276501)

"A simple, fast, and unconditionally stable approach to wave simulation": waves as particles that
carry a small displacement kernel, subdivide as they spread, and are splatted into a height field
"which is warped horizontally to account for local wave-induced flow"; the method behind Uncharted's
local waves and the "WP patches" of the 2025 hybrid paper.
*Bearing:* the local, interactive layer of the water — a bow wave, a splash, a stone — as a
splatted particle system that dies into the FFT background; visual only, never gameplay truth
(`physics-fluids.md`), and a candidate for the compute queue.

**Stefan Jeschke, Tomáš Skřivan, Matthias Müller-Fischer, Nuttapong Chentanez, Miles Macklin, Chris
Wojtan. "Water Surface Wavelets." *ACM Transactions on Graphics* 37(4) (SIGGRAPH 2018).** [paper]
[recent]
<https://visualcomputing.ist.ac.at/publications/2018/WSW/> (DOI 10.1145/3197517.3201336)

The middle ground between Fourier and finite differences: a method that can "faithfully simulate
wave interactions with moving obstacles in real time while simultaneously preserving minute details
and accommodating very large simulation domains", by advecting wave energy in a position–direction–
wavenumber space where reflection and refraction at boundaries are natural.
*Bearing:* the principled way to make waves bend around an island's headlands and reflect off
cliffs; more machinery than the first shore needs, and the reference to return to if the
distance-field Gerstner waves of step 2 look wrong in bays.

**Stefan Jeschke, Christian Hafner, Nuttapong Chentanez, Miles Macklin, Matthias Müller-Fischer,
Chris Wojtan. "Making Procedural Water Waves Boundary-aware." *Computer Graphics Forum* 39(8)
(SCA 2020), 47–54.** [paper] [recent]
<https://visualcomputing.ist.ac.at/publications/2020/MPWWBa/> (DOI 10.1111/cgf.14100)

Procedural waves that respect the beach: an extension "that guarantees the satisfaction of boundary
conditions imposed by terrain while still approximating physical wave behavior", and "in combination
with a particle system that models wave breaking, foam, and spray, this allows naturally modeling
waves interacting with beaches and rocks", animating "waves at large scales at interactive frame
rates on a commodity PC".
*Bearing:* the paper for the second shore pass: Gerstner or spectral waves whose amplitude and
direction are corrected by the terrain's boundary, with the breaking crest as particles. It keeps
the statelessness Forge wants (a function of position and time) while removing the tell-tale of
waves passing through rocks.

**Hugh Malan (Guerrilla). "Rendering Water in Horizon Forbidden West." SIGGRAPH 2022, *Advances in
Real-Time Rendering in Games*.** [talk] [recent]
<https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Water-Malan.pdf>

The shore at the top of the market. "The big new feature is breaking waves … waves with an
overhanging shape"; they are baked because "art directability is a priority", "baked out, stored
with the world tile data, and streamed in at runtime" (`physics-fluids.md`); "waves enter the bay
and spread out, with only waves starting to break close to the shore"; the breaking-wave system
"uses quadrilateral shapes defining the four corner positions and animation parameters, with quads
extending from the wavefront shape", and "the vertex buffer for the water surface is generated by a
compute shader"; waterfalls and underwater effects are the other headline features.
*Bearing:* the overhanging crest is a mesh with a baked animation, not a height field, placed along
the wavefront the shore distance defines; a generated island can bake the same thing at cook time
from the coast polyline and the floor slope. Also the model for how the water mesh is built: in
compute, per frame, on the GPU.

**Vladimir Lopatin, Elco Vossers, Hugh Malan-Revell (Ubisoft Singapore). "Simulation and
Representation of Topology-Changing Rolling Waves for Massive Open Ocean Games." *ACM SIGGRAPH 2024
Talks*.** [talk] [recent]
<https://dl.acm.org/doi/10.1145/3641233.3664308> (DOI 10.1145/3641233.3664308)

The same problem for Skull and Bones: rendering "close-shore water oceanic phenomena such as rolling
waves" in a massive open world, by "capturing wave simulation data with a set of approximation
curves that can deal with overhangs, changing topology and strict budget requirements", against the
animated height-displacement maps of Assassin's Creed Odyssey and Death Stranding that cannot fold
over.
*Bearing:* confirmation from a second studio that the breaking shore wave is a baked, curve-based
representation streamed with the world, and the note that height displacement alone caps the look;
Forge's first shore stays height-based (cheap, stateless) and the curve representation is the later
upgrade for the golden shots.

**Sébastien Lagarde. "Water drop 3a/3b – Physically based wet surfaces." Blog, 19 March and 14
April 2013.** [web] [still-current]
<https://seblagarde.wordpress.com/2013/03/19/water-drop-3a-physically-based-wet-surfaces/> ·
<https://seblagarde.wordpress.com/2013/04/14/water-drop-3b-physically-based-wet-surfaces/>

Wet sand is a material change, not a decal: measured dry and wet albedos show "the darkening effect
is color channel dependent with no real pattern", and the mechanism — "a rough surface will scatter
light more diffusely, and when a thin layer of water is present on top, there are more hits of the
water-air interface at larger angles causing more internal reflection and increasing the darkening
effect" — with the specular going toward water's (smooth, F0 ≈ 0.02) as the film forms. Already the
reference for D-034's wetness field (`planet-environment.md`).
*Bearing:* the beach's wet band is the terrain's layered material reading a wetness value (darker
albedo, lower roughness, a water F0) from a small clip texture the water writes: the maximum height
the shore waves reached recently, decaying with time. One texture, no second material.

---

## 3. Rivers, lakes and waterfalls

The island already knows where its rivers are (cells above 0.5 km² of catchment), how much water
each carries (the integer drainage area), which way it flows (the D8 receiver) and where its lakes
stand (the flood). This section is what the rendering literature does with exactly those inputs.
Génevaux 2013 (rivers as the generating structure), Bruneton & Neyret 2008 (rivers as vectors
rasterised into tiles) and Fill–Spill–Merge (lake levels) are in `terrain-genesis.md` §2 and §5 and
are not repeated.

**Alex Vlachos (Valve). "Water Flow in Portal 2." SIGGRAPH 2010, *Advances in Real-Time Rendering
in 3D Graphics and Games*.** [talk] [foundational] [still-current]
<https://advances.realtimerendering.com/s2010/Vlachos-Waterflow(SIGGRAPH%202010%20Advanced%20RealTime%20Rendering%20Course).pdf>

Flow maps: artists (or a tool) create "flow maps that define vector fields to distort normal maps
over time"; two phase-offset samples of the normal map advected along the local flow vector are
cross-faded so the texture never stretches beyond a fraction of a tile, giving eddies, acceleration
around obstacles and a direction of travel from one texture lookup and no simulation.
*Bearing:* the river's surface for the island in its entirety: the flow vector is the D8 (or D∞)
direction scaled by a speed from slope and area, baked per river tile or per ribbon vertex; the
normal map is the river's ripple cascade; the same trick advects foam and debris textures.

**Branislav Grujic (Ubisoft), Cristian Cutocheras (AMD). "Water Rendering in 'Far Cry 5'." GDC
2018, Advanced Graphics Techniques Tutorial.** [talk] [still-current]
<https://gdcvault.com/play/1025555/Advanced-Graphics-Techniques-Tutorial-Water> (slides
<https://media.gdcvault.com/gdc2018/presentations/Grujic_Branislav_WaterRenderingFarCry5.pdf>)

The production river system closest to Forge's inputs: "material structure buffers, tessellation,
normal mapping, smoothness, foam generation, and flow mapping" for "waterfalls and lakes" and
rivers; "flow maps are created automatically but calculated dynamically, based on spline and
flood-fill routines, which are displayed in greater detail near the player than at a greater
distance"; the several water systems are composited, "performance is scaleable with the number of
water pixels", and the shaders were moved to "half precision" with attention to "register pressure".
Far Cry 5's rivers came out of the same Houdini pipeline that generated its "freshwater networks"
(`terrain-genesis.md` §4).
*Bearing:* the flow map is derived from the river geometry, not painted, and at two resolutions
(coarse everywhere, fine near the camera); the cost model is pixels of water, so a river seen from
a ridge is nearly free and a lake filling the frame costs a full-screen pass. Half precision is
worth it on the water's per-pixel work on both vendors, and Slang's `half` makes it one type
change.

**Carlos Gonzalez-Ochoa (Naughty Dog). "Rendering Rapids in Uncharted 4." SIGGRAPH 2016, *Advances
in Real-Time Rendering in Games*.** [talk] [still-current]
<https://advances.realtimerendering.com/s2016/> (slides
<http://advances.realtimerendering.com/s2016/Rendering%20rapids%20in%20Uncharted%204.pptx>)

"The latest developments of the water engine and solutions to produce raging river rapids in
Uncharted 4, including how they advanced techniques to simulate oceans and created a new integrated
system to handle rivers. For rivers, they use offline fluid simulations to inform the overall look
of the river as well as to produce data of the water surface and flow."
*Bearing:* the high end of the river: an offline simulation baked to surface and flow textures per
reach. For a seeded island the offline step is the genesis (a shallow-water pass over the carved
bed, `terrain-genesis.md` §3's pipe model, run once per river tile) and its outputs are the same
textures; the rapids are where the bed slope exceeds a threshold and the foam texture saturates.

**Adrien Peytavie, Thibault Dupont, Éric Guérin, Yann Cortial, Bedřich Beneš, James Gain, Éric Galin.
"Procedural Riverscapes." *Computer Graphics Forum* 38(7) (Pacific Graphics 2019).** [paper]
[recent]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.13814> (DOI 10.1111/cgf.13814; author PDF
<https://perso.liris.cnrs.fr/eric.galin/Articles/2019-riverscapes.pdf>)

The paper that turns a network into geometry and animation: it "generates the inscribing geometry of
a river network and then synthesizes matching real-time water movement animation", taking
"bare-earth heightfields as input", deriving "hydrologically-inspired river network trajectories",
carving "riverbeds into the terrain", and generating "a corresponding blend-flow tree for the water
surface"; "characteristics, such as the riverbed width, depth and shape, as well as elevation and
flow of the fluid surface, are procedurally derived from the terrain and river type".
*Bearing:* stage 4 of genesis and step 3 of the water in one paper: river types by slope and order,
width and depth from the drainage area, a carved bed profile per reach, and a tree of flow
primitives (laminar, rapids, pools, falls) that the shader blends. Its rules for width and depth
are the ones to encode (the values were not re-read this session, §10).

**Axel Paris, Éric Guérin, Pauline Collon, Éric Galin. "Authoring and Simulating Meandering Rivers."
*ACM Transactions on Graphics* 42(6) (SIGGRAPH Asia 2023); code (MIT).** [paper] [code] [recent]
<https://dl.acm.org/doi/10.1145/3618350> (DOI 10.1145/3618350; open copy
<https://hal.science/hal-04227965>; code <https://github.com/aparis69/Meandering-rivers>)

Rivers that bend the way rivers do: "starting from a terrain with an initial low-resolution network
encoded as a directed graph", a curvature-driven migration model reproduces "downstream migration of
bends" and "abrupt events like cutoffs forming oxbow lakes and avulsions", under user constraints.
The repository is the "source code for the paper", MIT.
*Bearing:* the D8 network's straight runs on gentle slopes (`docs/demos/island.md`, "what the
pictures say") are exactly what this fixes: a few hundred iterations of meander migration on the
lowland reaches before the bed is carved, then the ribbons follow the curved polyline. Later than
the first river, and worth it on the plains.

**Arnaud Emilien, Pierre Poulin, Marie-Paule Cani, Ulysse Vimont. "Interactive Procedural Modelling
of Coherent Waterfall Scenes." *Computer Graphics Forum* 34(6), 2015, 22–35.** [paper]
[still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.12515> (DOI 10.1111/cgf.12515; open copy
<https://hal.science/hal-01095858>)

"The first solution for the interactive procedural design of coherent waterfall scenes": vector
elements (falls, pools, streams) assembled over a terrain, "a procedural model that parametrizes
these elements from hydraulic exchanges; enforces consistency between the terrain and the flow; and
generates detailed geometry, animated textures and shaders for the waterfalls and their
surroundings".
*Bearing:* where a river polyline's bed slope exceeds a threshold, split the reach into a fall
element (a sheet mesh from lip to plunge pool with an advected texture and spray particles) and a
pool; the flow rate is the drainage area, so the sheet's width and the pool's size follow. The
first island needs the split and the sheet; the particles wait for Phase 3.

**Lakes as planes** (a synthesis; the sources are in `terrain-genesis.md` §2). Fill–Spill–Merge
gives each lake a level, a shoreline polygon and an outlet; the lake is a flat plane at that level
clipped to the polygon, shaded with the same surface shader as the sea but a different spectrum
(a single short-wave cascade from the local wind, no swell, Unity's "ripples" band in §6), its
colour from the flooded depth (level minus floor) with a lake Jerlov type, its inflow and outflow
the river ribbons' end caps, whose water elevation is pinned to the level. Nothing here needs a
new technique; what it needs is that the genesis writes the level and the polygon.

---

## 4. Shading the surface

Water's BRDF is a dielectric's: Fresnel between about 2 % at normal incidence and total at grazing,
a specular lobe from the slopes of the unresolved waves, no diffuse, and underneath it the light
that entered the volume and comes back out (the transmitted, absorbed and scattered term the shore
colour and the crest glow both belong to). The sources below are the pieces Forge does not already
have; Schlick's Fresnel, the sky-view reflection (D-031) and the mirror ray through the TLAS (#50)
exist.

**Charles Cox, Walter Munk. "Measurement of the Roughness of the Sea Surface from Photographs of the
Sun's Glitter." *Journal of the Optical Society of America* 44(11), 1954, 838–850.** [paper]
[foundational]
<https://opg.optica.org/josa/abstract.cfm?uri=josa-44-11-838> (DOI 10.1364/JOSA.44.000838)

The slope distribution of the sea, measured from the sun's glitter: a method "identifying surface
points with particular slopes required for reflecting the sun's rays toward the observer and
interpreting average brightness in terms of the frequency of particular slopes", yielding a
near-Gaussian slope distribution whose variance grows linearly with wind speed, wider along the wind
than across it.
*Bearing:* the microfacet distribution for water is a Gaussian (or Beckmann) in slope space with an
anisotropic variance from the wind, not GGX with a roughness; the variance is what the far cascades
fold into (Bruneton 2010), and Cox–Munk's wind law is the default when no spectrum is resolved.

**Vincent Ross, Denis Dion, Guy Potvin. "Detailed analytical approach to the Gaussian surface
bidirectional reflectance distribution function specular component applied to the sea surface."
*Journal of the Optical Society of America A* 22(11), 2005, 2442–2453.** [paper] [still-current]
<https://opg.optica.org/josaa/abstract.cfm?uri=josaa-22-11-2442> (DOI 10.1364/JOSAA.22.002442)

The sea BRDF in closed form: a model that "includes mutual shadowing by waves, wave facet hiding, and
projection weighting", reduced from its integral form "to an analytical form, allowing computation
of sea reflected radiance more than 100 times faster than traditional numerical solutions"; the BRDF
Bruneton 2010 adopts for the far field.
*Bearing:* the specular term of `water.slang`: Gaussian slopes with the Ross shadowing–masking, the
variance per pixel being the sum of the unresolved cascades' slope variances. It replaces the GGX
of the standard material class for the water class only.

**Nigel Ang, Andrew Catling, Francesco Cifariello Ciardi, Valentine Kozin (Rare). "The Technical Art
of Sea of Thieves." *ACM SIGGRAPH 2018 Talks*; with Ryan Stevenson (Rare), "Visual Adventures on
'Sea of Thieves'", GDC 2018.** [talk] [still-current]
<https://dl.acm.org/doi/10.1145/3214745.3214820> (DOI 10.1145/3214745.3214820; slides
<https://history.siggraph.org/wp-content/uploads/2022/09/2018-Talks-Ang_The-Technical-Art-of-Sea-of-Thieves.pdf>)
· <https://gdcvault.com/play/1025015/Visual-Adventures-on-Sea-of>

The benchmark look, in Unreal 4 on hardware "ranging from integrated GPUs on a laptop to the most
powerful modern gaming PCs": the talk covers "techniques used to stylise and supplement the look of
their FFT water implementation", where "the water colour is based on scattering approximations,
blending between deep water colour and sub-surface water colour based on view angle, sun direction
and a wave peak mask" (the mask from the FFT's choppiness offsets), plus foam and detail normals on
top, and "real-time surface fluid simulations to model water behavior on the GPU" for the decks.
Stevenson's GDC talk is the art direction of the same water.
*Bearing:* the scattering term as shipped: a sub-surface colour weighted by how much wave is between
the eye and the sun (the wave-peak mask, i.e. the displacement height and the Jacobian), the view
angle and the sun's direction, over the Beer–Lambert deep colour. Cheap, stateless, and the reason
Rare's crests glow green against the light. Atlas (§1) does the same with a height-and-angle term.

**Morgan McGuire, Michael Mara. "Efficient GPU Screen-Space Ray Tracing." *Journal of Computer
Graphics Techniques* 3(4), 2014, 73–85; with Tomasz Stachowiak (EA DICE/Frostbite), "Stochastic
Screen-Space Reflections", SIGGRAPH 2015 *Advances in Real-Time Rendering in Games*.** [paper] [talk]
[still-current]
<https://jcgt.org/published/0003/04/04/> · <https://www.ea.com/frostbite/news/stochastic-screen-space-reflections>

The two screen-space reflection references. McGuire & Mara give "full implementation details of a
method that has been proven in production", "adapting the perspective-correct DDA line rasterization
algorithm to support multiple depth layers for robustness". Stachowiak's stochastic version "robustly
handles spatially-varying material properties, such as roughness and normals", traces "at
half-resolution" with rays "reused from adjacent pixels in Monte Carlo integration", and "first
shipped in Mirror's Edge and Need for Speed" — Frostbite's one public water-adjacent technique.
*Bearing:* the island's reflections on water in order: the sky-view table (D-031, exists), then
the mirror ray against the TLAS (#50, exists: the island and its props are in it), then stochastic
SSR for the rough, wind-blown surface where a single mirror ray under-samples. On calm lakes the
mirror ray alone is right; the perturbation by the normal is what sells it.

**Juan Guardado, Daniel Sánchez-Crespo. "Rendering Water Caustics." *GPU Gems* ch. 2, 2004; Tiago
Sousa (Crytek), "Generic Refraction Simulation", *GPU Gems 2* ch. 19, 2005.** [book] [foundational]
<https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-2-rendering-water-caustics>
· <https://developer.nvidia.com/gpugems/gpugems2/part-ii-shading-lighting-and-shadows/chapter-19-generic-refraction-simulation>

The two underwater staples, still what engines ship. Guardado: "an aesthetics-driven method for
rendering underwater caustics in real time", a projected texture of light focused by the surface's
curvature onto the floor. Sousa: refraction "based on perturbing the texture coordinates used in a
texture lookup of an image of the nonrefractive objects in the scene", with the mask that keeps
objects above the surface from bleeding into the refraction.
*Bearing:* refraction is a perturbed read of the opaque HDR image, which the render graph
expresses as a transient copy before the water pass; caustics come later, from the same FFT
normals projected along the sun onto the floor and attenuated by the depth term, inside the
terrain's shading where the water mask says "under water". Neither costs a new pass beyond the
copy.

---

## 5. What genesis hands to the water

A synthesis of the entries above and of `terrain-genesis.md`; no new source. Every technique in
§§1–4 keys on a small set of fields, and every one of them is either already computed by
`forge_procgen` or a cheap derivative of what is:

| Field | From | Resolution and size | Consumers |
|---|---|---|---|
| Sea level | the mask (0 m) | a constant | everything |
| Floor depth below the sea | the eroded height (negative where sea) | the height field itself, 4 m (2 m after amplification) | Beer–Lambert colour, TMA damping, foam threshold, breaking position |
| Signed distance to the coast | the mask, a distance transform at genesis | 4 m: 4097² × 2 B (f16 metres) = 34 MB; or 16 m (2 MB) island-wide plus 4 m per coastal tile | shore fade of the FFT, shore-wave direction (−∇d) and phase (d), foam line, wet band, placement |
| Water-body mask and id | mask + rivers + lakes | 4 m u16: 34 MB, or per tile | which shader path a pixel takes, terrain wetness, sound, physics |
| River polylines | stage 4 tracing of cells above the catchment threshold | a vertex every 4–8 m: `x, y, z_bed, z_water, width, depth, order, speed`; ~15 k river samples at 4 m → tens of KB | ribbons, flow direction, junction caps, waterfall split |
| Flow direction and speed | D8/D∞ receivers, slope and area | per ribbon vertex (KBs), or a 4 m RG8 field per river tile for wide reaches | flow maps (Vlachos), foam advection |
| Lake polygons, levels, outlets | Fill–Spill–Merge | per lake: a polygon and one level; KBs | lake planes, river end caps, depth colour |
| Wetness clip | run time, written by the shore waves | 1–2 k² texels around cameras, 1–4 m | the terrain's layered material (D-028) |
| Wave parameters | the seed's climate (wind, fetch per coast) and design | bytes per sea | the spectrum |

Three consequences. First, the shore fields are the same fields the materials read (sand where the
coast is near and low, `terrain-genesis.md` stage 6), so the beach and its water agree by
construction. Second, nothing water-specific needs the 2 m amplification: the shore distance and
the floor at 4 m suffice for waves whose shortest resolved length is metres, and the wetness clip
is the only thing at the terrain's fine resolution. Third, the whole set is deterministic and
seed-derived (D-016), so a server evaluating a boat's draught (D-009) reads the same shore distance
and the same spectrum as the client without a byte on the wire. Unreal's "water info texture" (§6)
is the same list packed into one per-zone texture; Forge's split — static fields in the genesis
cache, one small dynamic clip — matches how the terrain's other fields already travel.

---

## 6. Water in professional engines and shipped games

The engines' documentation is the best record of what a production water system must contain;
the studio talks (Rare, Guerrilla, Ubisoft, Naughty Dog, Studio Wildcard) are cited in §§1–4 where
their technique belongs and only summarised in the table below.

**Epic Games. "Water System", "Water Body Actors", "Water Meshing System and Surface Rendering",
Unreal Engine 5.x documentation; the `WaterZone` API.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/water-system-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/water-body-actors-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/water-meshing-system-and-surface-rendering-in-unreal-engine>

Bodies and a zone: "Water Bodies use splines to define areas within the Level that represent rivers,
lakes, and oceans. The splines define where water mesh tiles are drawn and rendered by the Water
Zone"; "by itself, the Water Zone doesn't render a surface"; "each water body contains information
about its depth and flow. You can query these for gameplay usage", rivers having "extra properties
to define depth and width". The mesh: "the Water Mesh Component contains a quadtree which defines
where there are water tiles, and the level of detail (LOD) of the water mesh tiles is handled by
traversing a quadtree each frame to generate an optimized set of tiles that are visible on screen";
"each level of detail is made up of a concentric circle around the camera view based on distance,
where each lower level of detail is farther from the camera and contains half the number of
vertices as the level that precedes it"; "the Far Distance Mesh is enabled by default in the Water
Zone Actor". The zone owns a `WaterInfoTexture` (a read-only "Water Velocity Texture" in the API;
Epic's bug tracker describes water "missing, in strange blocks, or in wrong places due to
incorrectly rendered WaterInfoTexture, which is part of the WaterZone actor", updated by a render
pass) that carries the bodies' surface height, depth and velocity to the materials. Waves are
Gerstner sums per body, indexed from the material.
*Bearing:* the architecture Forge should mirror with its own parts: bodies from genesis (a sea, N
rivers, M lakes) instead of splines, one viewer-centred quadtree or ring mesh instead of per-body
meshes, and the per-zone info texture split as §5 says. Epic's quadtree-per-frame is a compute job
Forge would run as a graph pass that emits the tile list for a mesh-shader draw.

**Unity Technologies. "Capabilities of the water system", "Water system simulation", HDRP 14–17
documentation; the Unity-Technologies/WaterScenes samples.** [docs] [code] [still-current]
<https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.3/manual/water-capabilities-of-the-water-system.html>
· <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.1/manual/water-water-system-simulation.html>
· <https://github.com/Unity-Technologies/WaterScenes>

An FFT water with bands per body type: "three water surface types: Pool, River, and
Ocean/Sea/Lake"; "a Simulation Band is a specific range of wave frequencies"; "Ocean, Sea, or Lake
water surfaces have three simulation bands, two for Swell waves and one for Ripples. River surfaces
have two simulation bands, one for Agitation (the equivalent of Swell) and one for Ripples. Pool
surfaces only have one band, for Ripples"; "local wind produces Ripples … distant wind produces
Swells"; "the CPU simulation can be evaluated at full or half resolution" for gameplay queries;
foam, deformers, foam generators, current maps and water masks are the authoring layer. The
samples' island scene "uses water deformers and foam generators to improve the visual around the
shoreline" and the river scene "use[s] instanced quads for the water surface in addition to a
current map to simulate the flow".
*Bearing:* Unity's bands are the cascades with a body-type preset, which is the right user-facing
model for Forge's three bodies (sea: swell + swell + ripples; river: agitation + ripples; lake:
ripples); the CPU half-resolution simulation for queries is the same choice D-009's "evaluated
identically on CPU and GPU" implies, and the current map is Vlachos's flow map under another name.

**Jean-François St-Amour (Ubisoft Montréal). "Rendering Assassin's Creed III." GDC 2013; Bartłomiej
Wroński (Ubisoft Montréal), "Assassin's Creed 4: Black Flag — Road to Next-Gen Graphics", GDC 2014;
and the Ubisoft Singapore ocean lineage to Skull and Bones and Black Flag Resynced (2026).** [talk]
[still-current]
<https://gdcvault.com/play/1017710/Rendering-Assassin-s-Creed> ·
<https://www.gdcvault.com/play/1020397/Assassin-s-Creed-IV-Black> (slides
<https://bartwronski.com/wp-content/uploads/2014/03/ac4_gdc.pdf>)

The other long-running ocean. St-Amour's talk covers "the game's weather system, lighting solution,
ocean rendering, and material system" for a world "in both summer and winter"; the ocean itself was
Ubisoft Singapore's, whose team (an ocean-simulation group under Georges Torres, ocean rendering by
Andrew Ellem, per press coverage) says "the ocean technology breakthrough started with Assassin's
Creed 3" and has carried it through Black Flag, Odyssey, Skull and Bones (the 2024 talk in §2) and
the 2026 Black Flag Resynced with "fully modernized water rendering and simulation". Wroński's talk
is the next-gen port: "novel techniques and effects that contribute to the next-gen look" and
"porting various GPU effects to next-gen consoles".
*Bearing:* twelve years of one studio's ocean say the pieces are stable (spectrum, cascades, shading,
shore) and the investment goes into the shore and the weather coupling; the AC3/AC4 slides were not
readable this session (§10), and the owner's question about a GDC 2020 "Breaking Down Barriers"
water talk has a negative answer (§9).

**Who does what** (from the entries above only):

| | Open sea | Cascades / bands | Mesh LOD | Shore | Rivers | Foam |
|---|---|---|---|---|---|---|
| Sea of Thieves (Rare) | FFT (Tessendorf) | — | UE4 | stylised, depth colour | — | choppiness mask + textures |
| Horizon Forbidden West (Guerrilla) | authored, baked per tile | — | compute-built vertex buffer | baked breaking waves | waterfalls | — |
| Skull and Bones (Ubisoft Singapore) | the Singapore ocean since AC3 | — | — | curve-represented rolling waves | — | — |
| Atlas (Wildcard/NVIDIA) | FFT | yes (video, from memory) | — | — | — | — |
| War Thunder (Gaijin/NVIDIA) | FFT (WaveWorks) | texture-array cascades | — | — | — | yes |
| Far Cry 5 (Ubisoft) | — | — | tessellation | — | automatic flow maps from splines, waterfalls | yes |
| Uncharted 3/4 (Naughty Dog) | Gerstner + b-spline + wave particles | — | own mesh LOD | depth colour, foam | offline sim → flow data | yes |
| Unreal Water plugin | Gerstner per body | wave sets | quadtree + far mesh | water info texture | spline bodies | material |
| Unity HDRP water | FFT | 3 / 2 / 1 bands | — | deformers, foam generators | current maps | simulation foam |
| Crest (open source) | FFT (Gerstner earlier) | cascaded LOD data | clipmap rings | depth cache | flow | simulated |

---

## Recommendation for Forge

**Where the water sits in the frame.** The water is a forward pass after the opaque resolve and
before the TAA resolve, not a visibility-buffer material class: it needs the opaque depth (depth
colour, shore foam, intersection fade) and the opaque HDR image (refraction) already shaded, and it
writes HDR colour, depth and motion vectors so that TAA and the aerial perspective treat it as a
surface. Concretely, in the island's graph: `shading/*` → `sky/compose` (the sky behind the scene,
the haze) → `water/scene-copy` (a transient copy of HDR for refraction; the graph derives the
barriers) → `water/surface` (a graphics pass: the ring mesh through the mesh-shader path with the
indirect fallback, P1; depth test against the opaque depth, depth write on; the fragment reads the
cascades, the shore fields, the sky-view table, the aerial-perspective volume of D-023, the shadow by
ray query as the standard class does) → TAA. The FFT itself is a chain of compute passes on the
async compute queue, since it depends on nothing in this frame's geometry (`.queue(QueueKind::
Compute)`, #77): `water/spectrum` (once per parameter change), `water/evolve`, `water/fft-rows`,
`water/fft-cols`, `water/derive` (displacement, slopes, Jacobian mean and variance, foam accumulate)
per cascade, into persistent `GraphImage`s with mips. Every pass gets its F1 zone from the graph.

**The build order, for the look first.**

1. **The sea: FFT cascades, depth colour, glitter without shimmer.** The TMA/JONSWAP spectrum with
   Horvath's swell and spreading, three cascades of 256² (patches near 1 km, 100 m, 10 m; a fourth
   of 512² at 4 m when the camera is on the deck), Stockham FFT in compute (a 256-point row is
   256 × 8 B = 2 KB of groupshared, far under the 32 KB cross-vendor limit; no assumption on the
   subgroup size), phases from `pcg3d(kx, ky, seed)`, time from the simulation clock. The surface:
   a clipmap of rings (Crest) or the quadtree tile list (Unreal), built in compute each frame,
   drawn through the mesh path, displaced by the cascades with the high cascades faded by distance
   and their slope variance added to the BRDF (Bruneton 2010 over Ross 2005), Schlick Fresnel, the
   sky-view reflection (D-031) with the normal, Beer–Lambert with a Jerlov type through the depth to
   the opaque floor (which already fades the surface into the beach), the Sea of Thieves scattering
   term from the wave height and the sun. Foam from the Jacobian's filtered coverage, accumulated
   and decayed. Passes added: the compute chain above, `water/scene-copy`, `water/surface`.
   *Cost at 1440p, estimated:* the FFT chain is arithmetic- and bandwidth-trivial (three 256²
   complex transforms of four packed fields are a few tens of megabytes of traffic), 0.1–0.3 ms on
   the compute queue and hidden behind the geometry; the surface pass is a full-screen forward
   shade when the camera is at sea, 0.3–0.8 ms including the shadow ray, plus 0.1–0.2 ms of raster
   for a ring mesh of half a million to a million triangles. *Baked:* nothing but the sea level
   and the shore distance for the fade. *Measure:* the chain's zones per cascade size; the
   surface pass filling the frame; the ꟻLIP between consecutive frames of a still camera (the
   shimmer metric the owner will otherwise judge by eye); triangles per pixel at the horizon;
   `--no-occlusion` and the fallback path at 0 pixels of difference as always.

2. **The shore: damping, waves, foam line, wet sand.** The TMA depth factor per cascade from the
   floor depth (long waves die in the shallows), a shore-fade of the displacement inside the last
   metres of shore distance, two or three Gerstner trains whose direction is `−∇d` and phase `d`
   of the coast distance (Uncharted 3's recipe), their amplitude rising then collapsing to foam
   where the floor is shallower than a fraction of their wavelength (the breaking position), a
   foam line from `(depth < threshold) ∨ (Jacobian < 0)` advected shoreward, and a wetness clip
   texture written by `water/wetness` (compute: the maximum run-up height reached, decaying) and
   read by the terrain's layered material (Lagarde: darker albedo, lower roughness, water's F0) —
   the beach is wet where the last wave reached, dry above. Passes added: `water/wetness` (and the
   terrain shading reads one more image). *Cost:* 0.1–0.2 ms; the shore work is per water pixel
   near the coast only. *Baked:* the signed coast distance at 4 m (34 MB as f16, or 16 m island-
   wide plus 4 m in coastal tiles), the water-body mask; the floor depth is the height. *Measure:*
   foam-line stability under TAA and camera motion; the wet band's edge against the sand's
   texture; frames per second on a beach view against step 1's sea view. Then, for the golden
   shots, Jeschke 2020's boundary correction and Guerrilla's baked breaking crest as a mesh along
   the wavefront.

3. **Rivers from the network.** Stage 4 of genesis (rivers traced to polylines, Strahler order,
   width and depth from the drainage area by Peytavie 2019's rules, the bed carved into the
   height, water elevation per vertex) becomes: a ribbon mesh per reach (a strip of quads across the
   width, following the polyline, its edges tucked under the carved bank), flow direction and speed
   per vertex from the D8 receiver, the slope and the area; the surface pass's river variant with
   Vlachos flow-mapped ripple normals, foam where the bed slope and the speed are high, depth colour
   from the carved depth with a river Jerlov type, junction caps at the sea and the lakes; a reach
   whose slope exceeds a threshold is split into a waterfall sheet and a plunge pool (Emilien 2015)
   drawn as a vertical ribbon with an advected sheet texture. Meander migration (Paris 2023) on the
   lowland reaches before carving, once the straight D8 runs bother the owner. Passes added: none;
   the ribbons are instances in `water/surface`. *Cost:* proportional to the river pixels (Far Cry
   5), 0.1–0.3 ms in a valley view, near zero from a ridge. *Baked:* the polylines with per-vertex
   width, depth, water elevation, order and speed (tens of KB); a 4 m flow field only for reaches
   wider than a few cells. *Measure:* seams between reaches and at junctions; the flow map's
   cross-fade at the speed of the fastest reach; the count of ribbons drawn.

4. **Lakes at their level.** A plane per lake at the Fill–Spill–Merge level, clipped to the
   polygon, the ripples cascade only, the lake Jerlov type, the depth colour from level minus floor,
   the inflow and outflow ribbons pinned to the level. Passes added: none. *Cost:* pixels.
   *Baked:* polygon, level, outlet per lake. *Measure:* that no lake plane pokes through its bank
   (the flood's 0.5 m margin against the amplified 2 m height is the risk; the amplification must
   respect the lake level as it respects the drainage).

5. **Afterwards** (not in this issue): the scene mirrored in the water — the mirror ray through the
   TLAS (#50) perturbed by the water normal, then stochastic SSR for the rough surface; caustics
   from the FFT normals projected along the sun onto the floor inside the terrain's shading where
   the mask says under water; the underwater view (Sousa's refraction, the same Beer–Lambert term as
   fog, the surface seen from below); wakes and splashes as wave particles on the compute queue;
   and the CPU side of D-009: the spectrum "evaluated identically on CPU and GPU" is not free with
   an FFT, so the honest options are the lowest cascade's FFT re-run on the CPU with `dmath` (a
   256² transform is a millisecond on one core, once per tick, deterministic by construction), a
   readback of the GPU displacement for the client's prediction only, or a matched band-limited
   Gerstner sum for physics; this is a decision the owner should take before the boats and belongs
   in a 🟡 entry of `DECISIONS.md` when Phase 3 starts, not now.

**What is deterministic, and what is only visual.** The spectrum's amplitudes and phases (seed and
wave-vector hash), the shore fields, the river polylines and the lake levels are seed-derived and
identical on every machine (D-016); the sea's height at any point and time is therefore a pure
function the server can evaluate. The FFT on the GPU, the foam accumulation, the wetness clip, the
wave particles and the reflections are visual: they may differ per GPU and never feed gameplay.

**Sizes and resolutions.** Three cascades of 256² with height, two horizontal displacements, two
slopes, the Jacobian mean and variance as fp16 with mips: about 4 MB per cascade, 12–16 MB in all,
persistent. The shore distance at 4 m, 34 MB as f16 island-wide (or 2 MB at 16 m plus tiles); the
water-body mask 34 MB as u16 or 17 MB as u8 without ids. The wetness clip 2048² × 1 B = 4 MB. The
ribbons: tens of KB of vertices. All of it beside the terrain's 67 MB fields, none of it needing
the 2 m amplification.

**What to build first, without a GPU.** The CPU side is the useful start (this session's
environment): the spectrum module (TMA, JONSWAP, Phillips, spreading, swell) as a pure function of
`Seed` and parameters with a golden digest; a CPU FFT with `dmath` producing a 256² displacement
PNG for the flow preview; the coast distance transform, the water-body mask, the river polylines
with widths and the lake polygons as stage 4 of `genesis`, drawn on `overview.png`; then, on the
dev PC, `ocean.slang` diffed against the CPU transform (the same bytes within fp16), the ring mesh,
the surface pass under TAA, and the numbers above replaced by the F1 overlay's.

---

## What the numbers say

The spectra are settled science: Phillips's `k⁻⁴` equilibrium range from 1958, JONSWAP's peak
enhancement from ten weeks of North Sea measurements in 1968–69, TMA's depth function from 1985,
Horvath's 2015 games-and-film formulation with its swell parameter, all with public code
(EncinoWaves, Apache-2.0). Water's absorption minimum is 0.0044 m⁻¹ at 418 nm (Pope & Fry 1997)
and the Jerlov types span clear ocean to turbid coast (Solonenko & Mobley 2015), which sets the
depth colour's parameters rather than a painted ramp. Production oceans run three or four FFT
cascades of 256²–512² per frame (War Thunder 2015, Atlas 2019, Unity HDRP's three bands for a sea,
two for a river, one for a pool; the open implementations cap at four); whitecap coverage is a
linearly pre-filterable Jacobian statistic (Dupuy & Bruneton 2012); the far field's glitter is a
Gaussian slope BRDF whose variance absorbs the unresolved cascades (Cox & Munk 1954, Ross 2005,
Bruneton 2010). Meshes: Unreal traverses a quadtree per frame with each LOD ring holding half the
vertices of the one before it and a far mesh to the world's edge; Crest keeps the data in cascaded
viewer-centred textures under ring geometry; Guerrilla builds the vertex buffer in compute. Shores:
Forbidden West's and Skull and Bones's breaking waves are baked representations streamed with the
world tiles; Jeschke 2020 animates boundary-aware procedural waves with breaking particles "at
interactive frame rates on a commodity PC". Rivers: Far Cry 5's flow maps are generated from splines
and flood fills at two resolutions and its cost "is scaleable with the number of water pixels";
Peytavie 2019 derives width, depth, shape, elevation and flow from terrain and river type. For
Forge: three 256² cascades are 12–16 MB of persistent images and an estimated 0.1–0.3 ms on the
compute queue; the surface pass an estimated 0.3–0.8 ms when the sea fills a 1440p frame; the
shore fields 34 MB at 4 m or 2 MB at 16 m; the island's 14 809 river samples at 4 m are tens of
kilobytes of polyline. Every Forge figure here is an estimate to be replaced by the F1 overlay.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A GDC 2020 "Breaking Down Barriers" water talk (Ubisoft)** — none exists. "Breaking Down
  Barriers" is Matt Pettineo's (Ready at Dawn) GDC 2019 Advanced Graphics Techniques tutorial on GPU
  synchronisation and barriers, and a 2023 Ubisoft Toronto mentoring talk of the same name; neither
  is about water. Ubisoft's water talks are St-Amour 2013, Wroński 2014, Grujic 2018 and the
  Singapore SIGGRAPH 2024 talk, all cited.
- **A Frostbite / Battlefield ocean talk** — none found. Battlefield 4's Paracel Storm and its
  dam-flood are documented only in press and wikis; EA's Frostbite pages list volumetrics (Hillaire
  2015), terrain (Keable 2023), shader authoring (2024) and Stachowiak's stochastic SSR (2015), which
  is the one Frostbite water-adjacent technique and is cited. Left out rather than cited from memory.
- **A Horizon Zero Dawn (2017) water or shore talk** — none found; Guerrilla's water publication is
  Malan's 2022 Forbidden West talk, which stands for both games.
- **A Sea of Thieves GDC 2018 *technical* water talk** — Rare's GDC 2018 session is Stevenson's art
  talk ("Visual Adventures", cited as such); the technical description is the SIGGRAPH 2018 talk.
  The SIGGRAPH talk's author list is given here as the ACM record shows it (Ang, Catling, Cifariello
  Ciardi, Kozin); `physics-fluids.md` spells two names differently and should be reconciled.
- **Yuri Kryachko, "Using Vertex Texture Displacement for Realistic Water Rendering", GPU Gems 2
  ch. 18 (Pacific Fighters)** — found (O'Reilly, NVIDIA PDF, Gamasutra reprint) but not read; the
  displaced-grid idea is covered by Johanson and Crest, so it is not cited.
- **Waylon Brinck, Andrew Maximov, "The Technical Art of Uncharted 4", SIGGRAPH 2016** — confirmed to
  exist (ACM, history.siggraph.org PDF); its beach and wet-sand content could not be verified, so
  wet sand rests on Lagarde and the talk is not cited for it.
- **Frank Vitalone, "Creating Real-Time Oceans for Call of Duty: WWII", 2018** — a Vimeo video
  listed in Wave Harmonic's resource list; video only, not citation grade.
- **Disney's "Moana: Crashing Waves" (Byun & Stomakhin, 2017)** — film simulation, listed in the
  same resource list; out of scope.
- **Qizhi Yu, Fabrice Neyret, Éric Bruneton, Nicolas Holzschuch, "Scalable real-time animation of
  rivers" (Eurographics 2009) and "Lagrangian texture advection" (TVCG 2011)** — not searched
  within the budget; Vlachos's flow maps and Peytavie's blend-flow tree cover the island's need.
- **Leopold & Maddock's hydraulic geometry (width and depth as powers of discharge, 1953)** — not
  searched; Peytavie 2019's rules are the cited source for width and depth from area, and the
  classical exponents should be looked up when stage 4 is written.
- **Fynn-Jorin Flügge, "Realtime GPGPU FFT Ocean Water Simulation", TUHH 2017** — the thesis the
  open implementations follow for the Cooley–Tukey walkthrough; its timing tables were not readable
  (tore.tuhh.de blocked), so no FFT millisecond figure is cited from it.
- **Naty Hoffman, "Fresnel Equations (in RGB) Considered Harmful" (2019)** — seen in a resource
  list; water's Fresnel is a single dielectric and Schlick suffices.
- **Robert Ryan's and Barth Paléologue's 2025 FFT-ocean blog posts** — seen in search results, hosts
  blocked; they would be the place to look for measured FFT timings on current GPUs.
- **The Star Citizen / Elite / No Man's Sky planet oceans** — not searched; the planet variant's
  water is a later note.

---

## Verification notes

Checked on 2026-09-25 with WebSearch and WebFetch only; no browser pane and no video pages watched.
The session's egress proxy refused every host tried except `github.com` and
`raw.githubusercontent.com`: advances.realtimerendering.com, media.gdcvault.com, gdcvault.com,
history.siggraph.org, dl.acm.org, developer.download.nvidia.com, download.nvidia.com,
developer.nvidia.com (not retried), dev.epicgames.com, issues.unrealengine.com, docs.unity3d.com,
unity.com, crest.readthedocs.io, jtessen.people.clemson.edu, people.computing.clemson.edu,
perso.liris.cnrs.fr, fileadmin.cs.lth.se, visualcomputing.ist.ac.at, tore.tuhh.de, hal.science,
arxiv.org, bartwronski.com, 80.lv, fxguide.com, gamedeveloper.com, cemyuksel.com,
seblagarde.wordpress.com and the github.io blog hosts all returned "blocked by the network egress
proxy"; schedule2019.gdconf.com and codeandcache.com did not resolve. Verification therefore has
two grades, as in `terrain-genesis.md`.

- **Fetched and read (GitHub):** GodotOceanWaves (README, verbatim quotes), Mozobo/Ocean-Simulation
  (README), tessarakkt/godot4-oceanfft (README), blackencino/EncinoWaves (README and licence),
  wave-harmonic/water-resources (the resource list, used to confirm titles and URLs),
  wave-harmonic/crest (the marketing README only), Unity-Technologies/WaterScenes (README, verbatim),
  jdupuy/whitecaps (README header only), aparis69/Meandering-rivers (README, licence).
  gasgiant/FFT-Ocean returned no README text.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  volume, pages, DOI, and the sentences quoted, which are the search engine's extracts of the page
  named) — "(verified through search results)" applies to every one of these, for issue #99:
  Phillips 1958 (JFM records and citing papers); Hasselmann 1973 (AWI EPIC, MPG, TU Delft records);
  Bouws 1985 (AGU/Wiley, ADS, MPG); Tessendorf 2002/2004 (the Clemson PDF listings and the extract
  on the Jacobian); Horvath 2015 (ACM DL, Semantic Scholar); Dupuy & Bruneton 2012 (ACM DL, HAL,
  the SIGGRAPH Asia fact sheet); Bruneton, Neyret & Holzschuch 2010 (EG diglib, HAL, Kesen's list);
  Johanson 2004 (the Lund listing, Google Books); Bowles 2017 and 2019 (the Advances 2017 and 2019
  course pages, the Crest technical documentation extract); Tcheblokov 2015 (the NVIDIA PDF listing,
  cgmeetup); Mihelich & Tcheblokov 2019 (GDC Vault 1025819, the GDC 2019 schedule, Class Central);
  Arc Blanc and the hybrid ocean 2025 (arXiv listings and abstracts); Pope & Fry 1997 (Optica, ADS,
  PubMed); Solonenko & Mobley 2015 (Optica, GEOMAR, PubMed); Gonzalez-Ochoa & Holder 2012 (GDC Vault
  1015517, Naughty Dog's blog, the slide mirrors); Yuksel 2007 (cemyuksel.com listing, ACM);
  Jeschke 2018 (IST Austria, history.siggraph.org, EurekAlert); Jeschke 2020 (IST Austria, Wiley,
  EG diglib); Malan 2022 (the Advances 2022 PDF listing and extracts, press coverage; two sentences
  carried from `physics-fluids.md`); Lopatin, Vossers & Malan-Revell 2024 (ACM DL, dblp's SIGGRAPH
  2024 Talks listing); Lagarde 2013 (the blog's listings with dates and the extracts); Vlachos 2010
  (the Advances 2010 page and PDF listing, Vlachos's site); Grujic & Cutocheras 2018 (GDC Vault
  1025555, the media.gdcvault.com PDF listing, 80.lv, Class Central); Gonzalez-Ochoa 2016 (the
  Advances 2016 page, ResearchGate); Peytavie 2019 (Semantic Scholar, the LIRIS PDF listing, dblp);
  Paris 2023 (ACM DL, HAL, the author's page); Emilien 2015 (Wiley, HAL); Cox & Munk 1954 (Optica,
  ADS); Ross, Dion & Potvin 2005 (Optica); Ang et al. 2018 (ACM DL, history.siggraph.org PDF
  listing, Semantic Scholar) and Stevenson 2018 (GDC Vault 1025015); McGuire & Mara 2014 (jcgt.org);
  Stachowiak 2015 (ea.com, the Advances 2015 page); Guardado & Sánchez-Crespo 2004 and Sousa 2005
  (NVIDIA GPU Gems listings, O'Reilly); Epic's Water System, Water Body Actors and Water Meshing
  pages (dev.epicgames.com listings and extracts, the WaterZone Python API page, two entries of
  Epic's issue tracker for `WaterInfoTexture`); Unity's HDRP water pages (docs.unity3d.com extracts,
  the Unity blog listing); St-Amour 2013 (GDC Vault 1017710, archive.org, gamedeveloper.com);
  Wroński 2014 (GDC Vault 1020397, the bartwronski.com PDF listing); the Ubisoft Singapore lineage
  (fxguide, gamedeveloper.com and Ubisoft news extracts).
- **Weaker confirmations, stated plainly.** Phillips 1958's DOI and Yuksel 2007's DOI are from
  memory. Peytavie 2019's DOI (10.1111/cgf.13814), its Wiley URL built from it, and the issue
  number (7) are from memory of the Pacific Graphics volume, not from a record seen this session.
  Emilien 2015's issue (6) is inferred; the search records give only the volume and pages. The
  Optica papers' DOIs (Cox & Munk, Ross) and the `opg.optica.org` URLs of Pope & Fry and Solonenko &
  Mobley follow the publisher's URI pattern from the DOIs and URIs the search returned; the DOIs of
  Pope & Fry and Solonenko & Mobley themselves were seen. Jeschke 2018's article number was not
  confirmed. Unreal's waves being Gerstner sets (the `GerstnerWaterWaves` asset) is from memory; the
  documentation extract seen says only that a body's "GPU-driven wave data" is indexed from the
  material. The War Thunder description of cascades as a
  texture array with the pipeline run per layer comes from a secondary summary in the search
  results, not from the slides. The Atlas talk's content beyond its session description (the
  cascades, the interactive layer, the crest lighting) is from memory of the video and from the open
  implementations that cite it, and is written as such. The Unreal water info texture's channels
  (height, depth, velocity) are inferred from the API's velocity texture and the bodies' documented
  depth and flow, not from a page naming the channels. The Sea of Thieves author names follow the
  ACM record as the search engine returned it. Guerrilla's "years" on the water and Ubisoft
  Singapore's team names come from press coverage. Fill–Spill–Merge, Génevaux 2013, Bruneton &
  Neyret 2008 and Losasso & Hoppe 2004 are not re-verified here; they were verified for
  `terrain-genesis.md` the same day.
- **Forge's own numbers** (the island's 14 809 river and 181 377 lake samples at 4 m, 4097² fields
  of 67 MB, the sea at 0 m as flat ground, the probes' 1.05 ms and their lookup's 0.11 ms at 1440p)
  are from `docs/demos/island.md` and `docs/ROADMAP.md` as of 2026-09-25.
- **Numbers to re-check before they enter a spec:** every millisecond in the recommendation is an
  estimate from arithmetic and from the size of comparable Forge passes, marked as such, to be
  replaced by the F1 overlay's zones; no source reachable this session published an FFT-ocean GPU
  timing; the cascade sizes and patch lengths are the open implementations' common choice, not a
  measured optimum; the 0.5 km² river threshold is the genesis preview's starting value; the Jerlov
  coefficients must be read from Solonenko & Mobley's tables, not from memory.
