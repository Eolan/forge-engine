# Research — BRUTAL and Kitten Space Agency: RocketWerkz's framework and game, for ideas

> **Date:** 2026-10-17.
>
> **The owner's request:** "Add the study of the BRUTAL engine https://github.com/ahwootech/BRUTAL, don't steal their source, it's for ideas only, if they have some nice stuff we could borrow we have to do it respecting their license and give credits for inspiration."
>
> **Companion notes:** `large-worlds.md` (precision, D-004, D-037), `planet-terrain.md` (D-056, D-058), `planet-environment.md`, `night-sky.md` (D-046), `galaxy-sky.md` (#223), `worlds-at-every-scale.md`, and the `physics-lab` demo's rocket (#148). D-062 names Kitten Space Agency as the owner's reference for physics and interstellar travel.
>
> **How facts were checked:**
> - WebFetch and WebSearch only. No browser pane, and nothing downloaded, cloned or run.
> - BRUTAL's code was seen only as GitHub folder listings (file names). No source file was opened, on purpose.
> - Kitten Space Agency (KSA) is closed source. It was neither downloaded nor decompiled. What is said about it comes from its developers' forum posts, its changelogs as copied on the community wiki, and the press.
> - Every fact carries its source number and either *read* (the page was fetched and read) or *search-recorded* (seen only in a search result's summary).
> - The KSA wiki (wiki.gg) and areweksayet.com are community sites. Where the wiki relays a developer's Discord message, the note says so.
> - Pages that came back empty or blocked are listed under Sources.
> - The licence section is a reading of the text, not legal advice.
> - Written by a cloud research agent, 2026-10-17.

## The short answer

- **BRUTAL is the plumbing under KSA, not what makes its planets.**
  - It is a set of C# libraries: native bindings, memory, jobs and maths. By its README's own account it has "no editor, no scene graph and no renderer" [2, read].
  - Forge already has its own Rust equivalents: `ash`, `forge-gpu`, `forge-task`, and its Jolt bindings.
  - None of BRUTAL's code is worth taking. Taking any would put the Brutal logo before Forge's games (§1.3).
- **The ideas the owner admires are in KSA.** Its developers publish enough to learn from:
  - celestial frames;
  - patched conics, with two integrators and vessels "on rails";
  - time-warp rules;
  - how space between stars is handed from one system to the next;
  - a body's levels of detail, from a sprite down to terrain;
  - terrain functions that return their own gradients;
  - rings whose rocks move at Kepler speeds.
- **Forge's foundations are level with KSA's, or ahead.**
  - f64 frames with integer cells on the GPU, against KSA's paired floats.
  - Cached cluster-DAG tiles, against terrain rebuilt every frame.
  - Hillaire's sky, and image-based verification.
  - What Forge lacks is the orbital layer. That is the thing to borrow when the planet demo returns.
- **KSA has no generated galaxy, only three nearby stars.** Forge's `galaxy-sky.md` is further along. KSA is still useful on handovers between star systems, barycentres and several suns.

---

## 1. What BRUTAL is

### 1.1 The repository

- **Owner and description:** `ahwootech/BRUTAL`, copyright 2024 Ahwoo Limited [3, read]. It calls itself a free, open-source C# framework for making games or your own engine [1, read].
- **Publication:** made public on 2026-10-15.
  - All ten commits on `main` are dated that day, starting with "Initial commit" [5, read].
  - The press announcement came the same day [10, read]. Gaming.net says the code grew over four years from a small set of libraries. That article says it was AI-written and edited by staff.
- **Size on 2026-10-17:** 1.7k stars, 89 forks, 6 open issues, 2 open pull requests [1, read].
  - The issues are early-adopter bugs: missing Box2D and Jolt native exports, a native list that does not grow its capacity, a clipboard crash.
  - There are also two questions: other platforms, and the logo [6, read].
- **Build state:** there are no project files or build scripts yet (the README says they are coming). It is used through NuGet packages [2, read].
- **Platforms:** C# on .NET 10, Windows or Linux, a Vulkan 1.3 GPU recommended, plus OpenGL bindings [2, read].
  - Contributors say macOS is a desktop priority.
  - WebAssembly (tested with WebGPU), Xbox and Android exist as proofs of concept, with no roadmap [8, read].
- **Philosophy:** a small base that does a few low-level things well: interop, memory, concurrency, maths and strings [2, read].
  - It avoids allocations and uses unmanaged memory and unsafe code where that pays.
  - It leaves architecture to the user, and refuses pull requests that drift towards an engine.
  - Dean Hall argues that frameworks, not engines, are the future [11, read].
- **Projects built on it** [2, read]:
  - RocketWerkz: KSA (the first major project), Torpedia, Art of the Rail and Gruntz.
  - Floating Origin Interactive: Afterskies.
  - Ludeon's Waylith engine for RimWorld. Ludeon reports 2 to 4 times the Unity build's performance in its own tests. RimWorld 1.7 is on Steam's beta branch [10, read]. Ludeon's own announcement could not be read [53].

### 1.2 Modules

The packages are from the README [2, read]. The contents are inferred from folder and file names only [9, read].

| Area | Packages | What the names suggest |
|---|---|---|
| Core | Core, Interop, Memory | handle and string marshallers; a pointer-owner type; memory partitions |
| Data | Collections, Strings, Maths, Numerics | vectors of 2–4 components in bool, byte, short, int, long, float and double; matrices, quaternions, planes, colour, packing |
| Concurrency | Concurrency, Logging, Localization | a job runner and scheduler with job states, metrics and a timer |
| Graphics | Vulkan, Vulkan.Abstractions, Vulkan.Vma, OpenGl, Render.Mesh | staging pool, per-frame resources, sync objects, descriptor helpers, procedural mesh primitives |
| Shaders | ShaderC, Slang | — |
| Windows, input | Glfw, Sdl (+ image, mixer, ttf) | — |
| Debug UI | ImGui (+ FreeType), ImPlot, RenderDoc (+ ImGui panel), Monitor.Common/Host | — |
| Assets | Texture, Ktx, Gli, Stb, Gltf, MeshOptimizer, FreeType | — |
| Audio, network, physics | Fmod, OpenAl, RakNet, Jolt, Box2D | — |
| Other | NativeFileDialog, NotCurses, ZLib | — |

Almost all of these are bindings to libraries Forge already reaches from Rust: Vulkan through `ash`, Slang, Jolt, meshoptimizer and KTX. There is no renderer, no scene, and no orbital or planet code in BRUTAL.

### 1.3 The licence: exact terms, and what Forge may do

**Name and holder.** The licence is called "BRUTAL License v1.0". The holder is Ahwoo Limited, and New Zealand law governs it [3, read].

**The grant.** Free of charge, anyone may use, copy, modify, merge, publish, distribute, sublicense and/or sell copies of the Software, subject to the conditions below [3, read].

**The conditions** [3, read]:
- **1.0:** the copyright and permission notice goes in all copies or substantial portions of the Software.
- **2.0:** these terms apply to a "Product". That is any game, application or other software distributed to end users that incorporates, links to, or is built using the Software or a substantial portion of it. A Product must meet all three:
  - **2.1:** show the copyright notice on its credits screen or equivalent.
  - **2.2:** show the Brutal logo (from ahwoo.com/brutal-branding) in all copies before gameplay or the primary function begins.
    - The logo must be unaltered except for scaling, shown long enough to be legible, and no less prominent than other parties' logos.
    - Exceptions: Ahwoo's written permission, or a product that cannot display graphics.
  - **2.3:** ship `NOTICE.txt` with all copies. Its text says the product is built with BRUTAL, a framework by Ahwoo Limited, and points to ahwoo.com/brutal [4, read].
- **3.0:** provided "as is". No warranty, including of non-infringement, and no liability.
- **4.0:** third-party components (listed in `LICENSES/` folders) stay under their own licences.
- **5.0:** contributors grant Ahwoo and all recipients a perpetual, worldwide, royalty-free, irrevocable licence to their contributions. They also warrant that the contributions infringe no one's rights.
- **6.0:** New Zealand law, with New Zealand courts having exclusive jurisdiction.

**What the licence does not contain** [3, read]:
- No patent clause: no express patent grant and no retaliation term.
- No trademark licence.
- No mention of "derivative works": modification comes under "modify", and products under "built using".
- Nothing about ideas or inspiration.

**Outside the licence**, the README [2, read]:
- offers a separate commercial licence by arrangement;
- waives the logo for non-profit projects, and where it would harm the application's purpose;
- says the licence is not yet OSI-approved.

In issue #14, the maintainer `rocket2guns` said the clause asks for the logo before gameplay, not for a splash screen as such. A logo in the main menu would also comply [7, read]. `rocket2guns` is Dean Hall's handle on X [42, search-recorded].

**What Forge may do:**
- Read BRUTAL's public repository, README and issues, and KSA's public posts.
- Build the ideas in its own design and code.
  - The licence's conditions attach to copies of the Software. They do not attach to methods or designs, and copyright does not protect ideas.
  - The owner's rule still asks for a credit line for inspiration (§8.3).
- Use the third-party libraries BRUTAL wraps, under their own licences. Forge already uses Jolt directly.

**What Forge must not do, or must accept the conditions for:**
- **Copy, translate or port BRUTAL's code.** A Rust port of C# code is still a modified copy of a substantial portion.
  - It would carry condition 1.0.
  - Any Forge game built using it becomes a Product under 2.1–2.3, including the logo before gameplay.
  - That clashes with the owner's rule of no third-party splash screen before the public release, unless Ahwoo waives it in writing.
  - So: no code. If code ever becomes tempting, ask the owner first.
- **Suggest endorsement:** no use of the Brutal logo or name, since there is no trademark licence.
- **KSA in any form:** KSA is a proprietary game with no source licence. Its .NET assemblies run JIT-compiled on the CLR, partly so mods can inject code [11, read], which makes them easy to decompile. Forge must not decompile them, or read decompiled KSA code in mods or elsewhere.

### 1.4 Kitten Space Agency in brief

- **Status:** a free, pre-alpha spaceflight game on Ahwoo since 13 November 2025 [24, read; the wiki cites Ahwoo's pre-alpha release post].
  - Give-what-you-want pricing [23, read].
  - Windows is supported officially, Linux is experimental, and there is no native macOS build [23, read].
- **Team:** Dean Hall; Felipe Falanghe, the creator of Kerbal Space Program, who steers BRUTAL's rendering work; Stefan Moluf, about twelve years in SpaceX flight software; and the KSP modders JPLRepo, blackrack, Linx and Daishi [12, read; 13, read; 15, read].
- **Physics library:** BepuPhysics, a C# library. The wiki's FAQ says so without a citation [23, read], and the changelogs mention Bepu [35, read; 40, search-recorded]. It is not BRUTAL's Jolt binding.
- **Solar system:** the real one, used as a placeholder while the tools are built. A fictional system is planned [23, read; 27, read].

## 2. Large worlds and precision

**The camera never moves.**
- The camera sits at the origin and the world is drawn around it. Positions are made camera-relative as late as possible. Falanghe calls this "first-order floating origin" [11, read].
- On the GPU, pairs of floats emulate doubles, and shaders compute the camera-relative positions [11, read].
- Most of the simulation runs in 64-bit doubles [23, read; the FAQ relays Dean Hall on Discord, 2024].

**Celestial frames** (gravhoek, 2025-12-04) [16, read]. All frames are right-handed.

| Frame | Centre | Kind | Use |
|---|---|---|---|
| ECL | the star | inertial root; the orientation is declared | the whole system |
| ORB | per orbiting body | inertial, rotated by the orbit's elements | orbits |
| CCE | per planet | the ecliptic frame moved to the planet | vehicle attitude is stored here |
| CCI | per planet | inertial: the planet's axis, X at its vernal point | orbital elements |
| CCF | per planet | rotates with the planet: north pole, prime meridian | the surface |

- The prime meridian is chosen by the author. CCF and CCI differ by the hour angle.
- The handover from inertial to rotating happens roughly at the atmosphere or near the ground.
- Doubles in CCI give about a millimetre of precision out to Pluto's orbit. (A check: the step of an `f64` near 6 × 10¹² m is 2⁻¹⁰ m.)
- Vehicles use the aircraft convention: X forward, Y right, Z down.
- gravhoek wrote that nearby vehicles at a system's edge or between stars will need a local frame. A post on vehicle frames was promised; it has not appeared yet.

**Interstellar distances** (JPLRepo, 2026-09-25) [17, read]:
- At one light-year from the origin, a double resolves only about 2 m. Any "absolute position plus small offset" goes wrong.
- This broke a portrait camera near 1 ly and made the map camera jitter 2 ly out.
- The fix was to place those cameras relative to the object they follow.
- Orbit and flight-plan lines are still floats relative to their star. At about 2 ly they draw in the wrong place; this is listed as a known limit.

**Bugs that came from frames** [35, read]:
- Angular velocity was corrupted when a body switched between CCI and CCF.
- Particles updated out of order were offset in CCF.
- Particles drifted away from vehicles at orbital speeds until they had velocity Verlet and the rotating frame's fictitious forces.
- A ray-tracing BVH is now built from origin-shifted vertices [32, read].

**No loading screens.** The ship goes from the atmosphere to orbit to the Moon without one [12, read; 13, read].

**Against Forge:**
- **On the GPU, Forge's scheme is exact.**
  - D-004 keeps CPU positions as `(frame, f64)` in a hierarchy: sector → star system → body → construct.
  - Its amendment sends the GPU `(int3 cell, float3 local)` instead of emulated doubles.
  - D-037's sectors of 2⁴⁰ m keep any `f64` within 0.24 mm, so KSA's light-year problem cannot arise.
- **What Forge lacks:**
  - D-004 names no inertial-versus-rotating pair per body, and no rule for when a vehicle changes frame.
  - It has no test that a follow camera, map lines and UI work at 1 ly.

## 3. Planets

### 3.1 From a dot to the ground

As the camera approaches, a body passes through five stages [27, read; the wiki relays Dean Hall on Discord, 2025-09-25]:
1. A sprite, slightly exaggerated so small bodies stay visible.
2. Above one pixel, a coloured unlit sphere.
3. An emissive sphere, a few pixels wide.
4. A lit sphere.
5. Full terrain: the "spherical billboard" of §3.2.

Bodies are sprites rather than a skybox. Sprites can be shadowed, which shows a gas giant's moons in eclipse [27, read].

### 3.2 Terrain

**The mesh: "spherical billboarding"** (Dean Hall, 2025-10-30) [15, read]:
- A library of sphere subdivisions is generated at start.
- The patch is dense near a reference vertex facing the player, and spreads to uniform quad density at distance.
- The reference vertex is snapped so vertices do not swim. He reports the earlier precision problems as solved.
- The changelogs agree:
  - Mars's mesh snapping ran too rarely, leaving the finest region too far from the camera [31, read].
  - Derivatives near the poles had to be fixed for picking the height mip [29, read].
  - Later, patches were made much smaller to cut sampling cost [35, read].

My reading: this resembles a camera-centred spherical clipmap. Not confirmed.

**Generated every frame:**
- Linx (relayed from Discord) says the terrain is rebuilt from scratch each frame from base heightmaps plus procedural modifiers. That rules out simulated erosion, unless it is very cheap [22, read].
- Terrain and collision data are made only where needed [27, read; relays Linx].
- Linx's real-time erosion reduced the dependence on heightmaps [15, read].

**The modifiers:**
- Kinds: hybrid multifractal, warped hybrid multifractal, Voronoi, simple, complex and eroded craters, and an erosion function [30, read; 31, read; 32, read].
- Each returns its **height and its gradient**:
  - the lookup tables store both;
  - the shader helpers return the gradient;
  - gradients are scaled by the inverse radius;
  - an exporter view rebuilds normals from the gradient, to check them against central differences [30, read].
- Each modifier has a **CPU copy for collisions**. Changelogs fix cases where the CPU and GPU disagreed [32, read].
- Central-difference normals moved from a 5 to a 15 sample spacing to reduce precision error [35, read].

**The Moon** [32, read]:
- small procedural craters with random sizes, blended into the real ones, with the normal map updated;
- subtle low-curvature erosion for slumping regolith;
- dusty textures instead of rocky ones.

Caves were ruled out: too much data, too slow to patch, too complex for collision and rendering [24, read].

**Textures and biomes:**
- A community guide gives an Earth of 16 384 × 8 192 [21, read]:
  - BC1 colour, BC5 normals, and a 16-bit height in KTX2;
  - a biome ID map and a control map, with a ground and a slope material per biome;
  - XML body definitions.
- Biomes now belong to the body, and ground clutter can be limited per biome. Large Moon rocks stay out of the maria [33, read].

**Ground clutter:**
- Each ecotype has its own pipeline [34, read].
- A depth pre-pass with alpha-to-coverage, and a dithered LOD fade [34, read].
- Earth's trees draw to 5.5 km [35, read].
- Collision meshes [24, read].
- A fan tracker says vehicles push rocks and trees, which then move as physics bodies [37, read].

### 3.3 Atmospheres and clouds

- **Scattering** is implemented. **Volumetric clouds** are partly implemented [24, read].
- **Credits:** the press credits the authors of KSP's Scatterer and Parallax mods [12, read]. The wiki credits blackrack for scattering [24, read].
- **Cloud features** [32, read; 33, read]:
  - flow maps for swirls (Jupiter's bands and storms);
  - colour maps per region, as a cubemap or equirectangular;
  - flat 2D layers that use alpha only;
  - one pipeline per layer, with specialisation constants per layer;
  - a per-planet distance to reduce flicker.
- Earth's air and clouds were scaled back to realistic values once its terrain height was realistic [39, search-recorded].
- Mars has dust storms [42, search-recorded].
- The atmosphere method is not published, so whether KSA uses Bruneton's or Hillaire's model is unknown. Forge has Hillaire 2020 (D-023).

### 3.4 Oceans

- Earth has oceans. Bodies define ocean colour and wind maps in XML [21, read; 26, read].
- There is a tessellated ocean and a "screen-space ocean mode" [32, read]. No wave model is described.
- Vessels on the sea far from the camera float "on rails", with no buoyancy simulation [35, read].

### 3.5 Rings

- Saturn's rings are 2D at a distance and rock fragments up close [26, read].
- Ring sections and rocks orbit at speeds set by their radius, so objects drift past one another [41, search-recorded; 24, read, which credits differential rotation to blackrack].
- A 2D ring shader and an upscaling shader exist [34, read].
- Collision with ring rocks is "semi-confirmed" [24, read].

### 3.6 Stars and their light

- The star nearest the camera lights the scene, including flare and bloom [17, read].
- Solar panels follow the star nearest the vessel [17, read].
- Other stars draw as bright points that can be selected [17, read].
- At first, only one star per system lit anything. Proxima b was effectively dark [17, read]. After the addendum, the star the vessel is near lights the scene [18, read].
- Distant craft show sun glints, with the atmosphere's transmittance and roll-aware angles [35, read].
- How the background starfield is made was not found.

### 3.7 From orbit to the ground

- The press describes dynamic LOD and real-time terrain streaming, without a loading screen [12, read].
- Loading the whole solar system needs 6–8 GB of video memory [23, read].

**Against Forge:**
- **Forge's terrain is cached, not rebuilt.** D-056 draws cube-sphere tiles as cluster DAGs, streamed and cached. D-058 makes heights and normal maps on the GPU, with a bit-exact CPU check.
- **What is new to Forge:**
  - terrain functions that carry analytic gradients;
  - craters below the resolution of the real maps;
  - the sprite-to-terrain chain for bodies other than the one being flown;
  - rings with Kepler shear.

## 4. Physics

### 4.1 Orbits

- KSA uses patched conics: each body has a sphere of influence that hands the ship over [17, read]. N-body physics is not planned for the base game [23, read; 24, read].
- One press piece speaks of two-body and multi-body calculations [12, read]. The developers' own posts describe only patched conics.
- Orbit lines of vessels the camera is not following draw grey [29, read].

### 4.2 Integrators

From the addendum, 2026-10-02 [18, read]:
- **Powered flight:** velocity Verlet, accurate up to steps of about 2 s.
- **Free flight with long steps:** an eighth-order Dormand–Prince Runge–Kutta.
  - Over 15 days at 100 000×, it stayed within half a millimetre of the exact orbit.
  - Verlet drifted 2.4 m over the same test.
- **The flight computer** no longer wakes every 0.1 s during long burns. It sleeps until the pointing error passes a threshold, which removed the cap on step length.

### 4.3 Time warp

- **Range:** from 0.1× to 100 years a second [25, read; 18, read]. The first interstellar build had steps up to 30 000 years a second; a week later the top was 100 years [17, read; 18, read].
- **Rules:**
  - Up to 30×, there is full manual control.
  - Above 30×, engines cannot be started, and can only be throttled down. Burns already running continue. RCS works only through the autopilot [25, read].
  - Warp drops to real time when an automatic burn ends [18, read].
  - Warp steps down before the next periapsis or star handover [17, read].
- **Attitude control:** its time step now grows with the attitude error, and the gains are re-solved for each step. A 10 Hz cap was removed, for burns lasting months [36, read].

### 4.4 Many vessels, rigid bodies, collisions

- The physics loop groups nearby vehicles into a shared solver with synchronised time steps [32, read].
- A collision pre-pass between vehicles runs before Bepu. It stops spurious speculative contacts at high warp, which had made vehicles on crossing orbits hitch [35, read].
- Vehicles go on rails when they can. A vehicle with ignition armed but no propellant had been blocking rails, a major slowdown [35, read].
- Performance is still very poor at high interstellar warp when other ships exist [17, read].
- Rigid-body work [35, read; 36, read; 24, read]:
  - rolling resistance computed from inverse inertia;
  - the compound shape of a split vehicle built once, from its final geometry;
  - convex-hull colliders imported from glTF;
  - parts that can be destroyed.

### 4.5 Control and engines

- **Attitude:** the control system fires real jets and resolves their forces, rather than faking rotation [20, read; this is the post's description, and the video was not watched].
- **Thrust vectoring:** the controller moved from LQR to LQI, adding an integral term against steady-state error [36, read].
- **Changing mass:** RCS authority is recomputed when mass, centre of mass or ambient pressure change [33, read].
- **Engine performance** comes from the same thermochemistry and nozzle simulation for every rocket [17, read].

### 4.6 Stability and determinism

- Stability comes from the measures above: two integrators, rails, synchronised clusters, the pre-pass, and adaptive control steps.
- None of the sources says anything about determinism. Multiplayer is planned but not built [24, read].
- Forge's D-016 goes further than anything KSA describes.

**Against Forge:**
- Forge has Jolt in a fixed step and a pad-launched rocket (#148): fixed thrust, manual stick, nose roll jets.
- It has no orbits, no rails, no time warp and no change of mass in flight.

## 5. Rendering in general

- **Whose renderer:** BRUTAL has none. KSA's renderer is its own, built on BRUTAL's Vulkan bindings [2, read; 15, read].
- **Forward+:**
  - clustered shading [23, read; relays Linx and Hall];
  - an MSAA scene, with transparencies (atmosphere, rings, clouds, exhaust) composited by shared code [34, read];
  - a depth pre-pass using dynamic rendering, with alpha-to-coverage [34, read].
- **Bindless resources**, which the team says Felipe uses well [15, read; 32, read; 36, read].
- **Shadows:**
  - a shadow atlas with slots per light, bucketing, and sun shadows for static and skinned meshes [32, read];
  - cascades that include ground clutter [33, read];
  - clustered lights with shadows [15, read].
- **Ray tracing** is used in the cockpit view (IVA) only [24, read; 34, read; 33, read].
- **Smaller pieces:**
  - the Hable tone curve [29, read];
  - thin-film interference [31, read];
  - shell fur with an LOD [32, read];
  - volumetric plumes, with samples concentrated near the nozzle [34, read] and self-shadowed trails [36, read];
  - a user interface drawn from maths rather than textures [11, read].
- **Performance claims:** none for KSA. Ludeon's 2–4× over Unity is the only figure, and it is for RimWorld [2, read].
- **AMD lessons that Forge's cross-vendor rules should note:**
  - a bindless array size misread as negative crashed AMD GPUs at start;
  - `tanh` produced white artefacts on AMD;
  - shaderc's optimisations hung some RX 6000-series cards;
  - a scratch-buffer alignment broke AMD validation [36, read; 32, read; 35, read];
  - users report broken terrain on AMD under Linux, and macOS running through a Vulkan-to-Metal layer [47, 46, search-recorded].

**Against Forge:** Forge has a visibility buffer, TAA/DLAA, ray-traced shadows and GI probes, and has decided clustered lamp lists (D-065, not built yet). It has nothing to take from this list except the AMD notes.

## 6. Interstellar travel and scale

- **Scale:** the real solar system is a placeholder. The fictional system is planned at 1 to 2.5 times KSP's [23, read; relays Hall, 2024].
- **No generated galaxy.** There is no random planet generation [37, read; fan tracker]. Custom star systems come through configuration [37, read].
- **The interstellar build** (2026-09-25) [17, read]:
  - Three nearby stars at their real distances and directions: Alpha Centauri (A, B, Proxima, with Proxima b and d), Barnard's Star and Tau Ceti. Only well-established planets are included.
  - A fusion torch with an Isp of about 49 000 s.
  - Interstellar warp steps.
- **The planner** [17, read; 18, read]:
  - A trip is a speed-up burn, a coast, and a slow-down burn timed backwards from arrival.
  - It corrects for the Sun's pull on the departure heading.
  - There are no porkchop plots, because over millennia the stars are fixed.
  - Burns are re-timed against the real delta-v.
  - Test arrivals fell 156 to 1 700 AU from the target. There is no arrival targeting yet.
- **Handovers between stars** went through two designs:
  - **First** [17, read]: each star owned the region nearer to it than to any other star (a Voronoi split, the boundary being the mid-plane). Stars kept an infinite sphere of influence, because a finite one triggered expensive searches and dropped the game to 1 fps.
  - **A week later** [18, read]: space belongs to whole star systems, and inside a system spheres of influence apply. Alpha Centauri B has a 3 AU sphere. A and B orbit an invisible barycentre with their real 80-year period, and star masses set the orbits.
- **Limits:** one lit star per system at first, long burns modelled as instant, and performance with other ships at high warp [17, read; 18, read].

**Against Forge:**
- `galaxy-sky.md` already plans a galaxy of cells with 64-bit ids per body, and a sky queried by magnitude. KSA adds nothing on generation.
- It does add handover and barycentre designs, and if Forge ever has sublight travel, a planner.
- No Man's Sky and Elite jump between stars rather than fly there, so the planner matters only if the galaxy demo wants real flights.

## 7. Tools and workflow

**BRUTAL:**
- NuGet packages.
- ImGui and ImPlot.
- A RenderDoc binding with an ImGui panel.
- A "monitor" host, whose purpose is not documented.
- No editor [2, read; 9, read].

**KSA:**
- A debug terrain editor [37, read].
- A planet map exporter (height, gradient, and normals from the gradient) and a biome-map exporter in compute shaders [30, read; 31, read].
- Shader hot reload as a developer tool [37, read].
- A Jobs tab in the profiler [36, read].
- GPU markers per pass and per ecotype [35, read].
- Debug windows for the physics step and for long burns [18, read].
- Tests for burn planning and timing [36, read].
- XML data for bodies and parts, and mods injected as C# [21, read; 24, read].

**How the team works:**
- It builds several real cases first, then designs the data architecture and refactors (the "middle outwards" approach) [15, read].
- It uses language models to look up C# and Vulkan documentation, not to write code [11, read].

**Against Forge:** Forge has TOML world files, the F1 overlay, Tracy, runtime shader compiles, and tiered capture verification. Its verification is ahead of anything KSA describes. Worth copying: a view that checks gradients against central differences, and baseline tests for the orbital code when it exists.

## 8. For Forge

### 8.1 Ideas worth considering

Costs: "low" is about one session, "medium" a few sessions, "high" a demo-sized piece of work.

| # | Idea | Where in Forge | Forge today | Cost | Credit if adopted |
|---|---|---|---|---|---|
| 1 | Per body, an inertial frame and a rotating frame. A rule for where a vehicle switches, angular velocity converted at the switch, fictitious forces in the rotating frame | Planet demo (vehicles, the walker on a turning Earth), galaxy | D-004's hierarchy, no such pair | Low | gravhoek, "Celestial Coordinate Frames in KSA" (2025) |
| 2 | Cameras, map lines and UI placed relative to what they follow, never "absolute + offset". A 1 ly test | Galaxy, planet map view | Safe by D-037's sectors; untested at 1 ly | Low | A lesson; cite JPLRepo (2026) in the note only |
| 3 | Patched conics: Kepler solver, spheres of influence with handover and hysteresis, orbit patches | Planet demo (Moon trips), galaxy | None | Medium (a `forge-orbit` crate with analytic tests) | Textbook method (Bate, Mueller & White 1971); the KSA lineage for its use in a game |
| 4 | Two integrators: velocity Verlet under thrust (≤ 2 s steps), 8th-order Dormand–Prince for long coasts, analytic Kepler on rails | Physics lab rockets, planet demo, galaxy | Jolt's fixed step only | Low to medium | JPLRepo, "KSA Goes Interstellar: Addendum" (2026); Dormand & Prince 1980; DOP853 after Hairer, Nørsett & Wanner |
| 5 | Rails and clusters: far or idle vessels (and boats far from the camera) on rails; nearby vessels in one solver with a shared step; a vessel-against-vessel pre-pass before Jolt at high warp | Galaxy and planet ships; city traffic and village boats as simulation LOD | One Jolt world | Medium | KSA changelogs 2025.11.4.2742 and 2026.7.5.4892 |
| 6 | Time-warp rules: thrust up to 30× only, step down before a handover or periapsis, burns that end warp, adaptive attitude-control step | Planet demo, galaxy | None | Low once 3–4 exist | KSA's time-warp design (wiki; JPLRepo 2026) |
| 7 | The body chain: sprite (exaggerated, can be shadowed) → unlit, emissive, then lit sphere → terrain | Planet demo (other bodies in the sky), galaxy | One body flown at a time; stars from NASA's map | Low | Dean Hall's description, via the KSA wiki |
| 8 | Terrain modifiers that return height and gradient; normals from gradients; a gradient-against-differences check view | Planet demo (D-058's GPU heights) | Normals from height samples | Medium | KSA changelog 2025.9.13.2415; analytic-derivative noise after Quilez and Gustavson |
| 9 | Moon craters below the real maps' resolution: sized at random, blended with the real ones, eroded craters, slumping | Planet demo's Moon (the owner's realism feedback) | NASA maps plus noise | Medium | KSA changelog 2025.11.4.2742 (Linx); size distributions from lunar science |
| 10 | Rings: 2D far, rocks near; ring sections and rocks moving at Kepler speed for their radius | Gas giants (planet, galaxy); the ballad's belts now | Belts with no Kepler shear (the belt's motion was not checked) | Low (shear), medium (rings) | blackrack's ring work for KSA (Ahwoo post, 2026) |
| 11 | Several suns: choose the lighting star or stars by irradiance at the camera (better than nearest); panels by the vessel's star; others as selectable points | Galaxy | One sun | Medium | JPLRepo, "KSA Goes Interstellar" (2026) |
| 12 | Star-system territories (Voronoi through Forge's cells), spheres of influence inside a system, binaries around an invisible barycentre | Galaxy | `galaxy-sky.md` cells, no handover | Low with cells | JPLRepo (2026), both posts |
| 13 | Interstellar planner (accelerate, coast, decelerate timed backwards; correct for the star's pull) | Galaxy, only if sublight travel is in the design | None | Low to medium | JPLRepo (2026) |
| 14 | Sun glints from distant craft, through the atmosphere's transmittance | Planet demo (a station, satellites) | None | Low | KSA changelog 2026.7.5.4892 |
| 15 | Cloud layers per planet: flow maps for bands and swirls, one specialised pipeline per layer | Planet demo, gas giants | Researched in `planet-environment.md`, not built (weather is low priority) | High | blackrack's KSA cloud work (changelogs 2025–2026) |
| 16 | Real jets for attitude (allocation across RCS), LQI for thrust vectoring, authority recomputed as mass and centre of mass change | Physics lab rocket (#148), later ships | Manual stick, fixed jets, fixed mass | Medium | KSA's flight computer (Stefan Moluf's video; changelogs 2026); LQR/LQI from control theory |
| 17 | Engine thrust and Isp from chamber chemistry and nozzle flow | Physics lab rocket | Fixed 50 kN | Medium | The KSA approach (JPLRepo 2026); ideal-rocket relations after Sutton |
| 18 | AMD pitfalls: signed bindless sizes, `tanh`, shader-compiler optimisations, alignment | Now: every map (cross-vendor rules, #67) | Rules in CLAUDE.md | Low (add to the rules) | None needed |

**Not worth adopting:**
- BRUTAL's code (licence, §1.3).
- Paired floats on the GPU: Forge's integer cells are exact.
- Rebuilding terrain every frame instead of D-056's cached tiles. At most, a measured experiment against the tiles for procedural bodies visited briefly in the galaxy.
- Bepu: Forge has Jolt.
- MSAA forward+: Forge has a visibility buffer with TAA/DLAA.

### 8.2 Patents

- **Geometry clipmaps:** US 7,436,405 B2, "Terrain rendering using nested regular grids" [38, read].
  - Holder: Microsoft, now Microsoft Technology Licensing LLC. Inventors Losasso and Hoppe.
  - Filed 2004-05-14, granted 2008-10-14, **expired 2024-09-27**.
  - It matters only if Forge builds a camera-centred nested grid like KSA's spherical billboard. That likeness is my reading, not confirmed.
  - Spherical clipmaps (Clasen & Hege 2006) are the related published method. No patent on them is known to this note.
- **No patent known for the rest, but none was searched:**
  - patched conics, Verlet, Dormand–Prince, LQR/LQI;
  - Voronoi territories and barycentres;
  - flow maps, procedural craters, analytic noise derivatives;
  - Kepler-sheared rings, shell fur;
  - double-single arithmetic.
- **Already covered elsewhere:** clustered light lists by D-065 (no patent found there). The atmosphere method by D-023 and `lighting-patents.md`. KSA's own atmosphere method is unpublished.

### 8.3 Credit lines

The owner asked for credit "for inspiration". Almost every idea above comes from KSA's posts, not from BRUTAL's code. So the accurate line names the game and the framework under it. In `CREDITS.md` under "Techniques":

> *<idea>*: inspired by Kitten Space Agency (RocketWerkz, built on Ahwoo's BRUTAL framework), from <author>, "<post>", Ahwoo forums, <date>; <the underlying method's authors>.

For example:

> Orbit propagation by velocity Verlet under thrust and an eighth-order Dormand–Prince integrator in free flight: inspired by Kitten Space Agency (RocketWerkz, built on Ahwoo's BRUTAL framework), JPLRepo, "KSA Goes Interstellar: Addendum", Ahwoo forums, 2026-10-02; after Verlet (1967) and Dormand & Prince (1980).

If only BRUTAL's README or philosophy inspired something, then: "inspired by RocketWerkz's BRUTAL (Ahwoo Limited)". Neither kind of line triggers the licence, because no code is copied.

## Recommendation for Forge

1. **Take no code from BRUTAL.** Credit ideas with the lines of §8.3. Never decompile or read KSA's binaries. If BRUTAL code is ever wanted, ask the owner first: the logo clause conflicts with the no-splash rule unless Ahwoo waives it.
2. **Now (village and city):**
   - Add the AMD pitfalls of §5 to the cross-vendor rules (idea 18).
   - If the ballad is touched, the Kepler shear of its belts (idea 10) is cheap and visible.
   - Otherwise, nothing here should pull work away from the maps (D-062).
3. **Physics lab:** when the rocket grows (fuel burning off, #148's "not yet"), take ideas 16 and 17 and Verlet under thrust (idea 4).
4. **When the planet demo returns:**
   - First, a `forge-orbit` crate: frames (idea 1), patched conics (3), the two integrators and rails (4, 5), time-warp rules (6).
   - Its acceptance test should use KSA's published number: an orbit of the Earth for 15 game days at 100 000× within a millimetre of Kepler's.
   - Then the body chain (7), Moon craters (9), gradients (8) and glints (14).
5. **Galaxy demo:** territories and barycentres (12), several suns by irradiance (11), the 1 ly tests (2). Add the planner (13) only if the design flies between stars rather than jumping.
6. **Revisit in six months.** BRUTAL is two days old, at ten commits. gravhoek's promised post on vehicle frames would be the most useful next read for Forge.

## Sources

**BRUTAL**
1. BRUTAL repository — https://github.com/ahwootech/BRUTAL — *read*
2. README — https://github.com/ahwootech/BRUTAL/blob/main/README.md — *read*
3. LICENSE.txt ("BRUTAL License v1.0") — https://github.com/ahwootech/BRUTAL/blob/main/LICENSE.txt (and the raw file) — *read*
4. NOTICE.txt — https://raw.githubusercontent.com/ahwootech/BRUTAL/main/NOTICE.txt — *read*
5. Commit history — https://github.com/ahwootech/BRUTAL/commits/main — *read*
6. Issues — https://github.com/ahwootech/BRUTAL/issues — *read*
7. Issue #14, "Splash image requirement / Licensing" — https://github.com/ahwootech/BRUTAL/issues/14 — *read*
8. Issue #7, other platforms — https://github.com/ahwootech/BRUTAL/issues/7 — *read*
9. Folder listings (file names only) — https://github.com/ahwootech/BRUTAL/tree/main/ + `Brutal.Numerics/Vector`, `Brutal.Concurrency/Jobs`, `Brutal.Vulkan.Abstractions`, `Brutal.Memory`, `Brutal.Interop`, `Brutal.Render.Mesh`, `Brutal.Monitor.Host`, `Brutal.Maths` — *read*

**Press**

10. Gaming.net, "BRUTAL game framework released as open source…", 2026-10-15/16 — https://www.gaming.net/brutal-game-framework-released-as-open-source-free-for-commercial-use/ — *read*
11. Game Developer, Bryant Francis, "Rocketwerkz CEO says frameworks, not engines…", 2025-11-20 — https://www.gamedeveloper.com/programming/rocketwerkz-ceo-predicts-frameworks-not-engines-will-be-future-of-game-development — *read*
12. Gamerdeal, KSA pre-alpha launch, 2025-11-16 — https://www.gamerdeal.com/2025/11/16/kitten-space-agency-sets-a-new-course-for-spaceflight-simulation-with-a-community-driven-pre-alpha-launch/ — *read*
13. UFO Feed, mirror of Space.com's interview with Stefan Moluf, 2025-11-14 — https://www.ufofeed.com/170160/kitten-space-agency-is-the-spiritual-successor-to-kerbal-space-program-and-they-have-an-ex-spacex-engineer-on-the-team-interview/ — *read*

**Ahwoo forums (developers' posts)**

14. Developer Updates index, pages 1–2 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/ — *read*
15. Rocket (Dean Hall), "Development Recap One Year One", 2025-10-30 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/development-recap-one-year-one.75/ — *read*
16. gravhoek, "Celestial Coordinate Frames in KSA", 2025-12-04 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/celestial-coordinate-frames-in-ksa.619/ — *read*
17. JPLRepo, "KSA Goes Interstellar", 2026-09-25 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/ksa-goes-interstellar.1286/ — *read*
18. JPLRepo, "KSA Goes Interstellar: Addendum", 2026-10-02 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/ksa-goes-interstellar-addendum.1337/ — *read*
19. Daishi, "IVA Update! (Dev Update #18)", 2026-02-08 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/iva-update-daishis-dev-update-18.766/ — *read*
20. Taylor, "Most space games fake how a spacecraft turns…", 2025-11-28 — https://forums.ahwoo.com/forums/kitten-space-agency/developer-updates/most-space-games-fake-how-a-spacecraft-turns-kitten-space-agency-doesnt.584/ — *read* (the video was not watched)
21. UriahGnu (community), "Planet Textures, Materials, and Biomes", 2025-11-26 — https://forums.ahwoo.com/forums/kitten-space-agency/guides-and-help/planet-textures-materials-and-biomes.572/ — *read*
22. "Highly realistic erosion model…" (community thread relaying Linx), 2026-07-06 — https://forums.ahwoo.com/threads/highly-realistic-erosion-model-to-make-terrain-authoring-easier.1037/ — *read*

**KSA wiki (community)**

23. Frequently asked questions — https://kittenspaceagency.wiki.gg/wiki/Frequently_asked_questions — *read*
24. Planned features — https://kittenspaceagency.wiki.gg/wiki/Planned_features — *read*
25. Time warp — https://kittenspaceagency.wiki.gg/wiki/Time_warp — *read*
26. Solar System — https://kittenspaceagency.wiki.gg/wiki/Solar_System — *read*
27. Celestial body — https://kittenspaceagency.wiki.gg/wiki/Celestial_bodies — *read*
28. Version history — https://kittenspaceagency.wiki.gg/wiki/Version_history — *read*
29. Version 2025.8.33.2091 — https://kittenspaceagency.wiki.gg/wiki/Version_2025.8.33.2091 — *read*
30. Version 2025.9.13.2415 — https://kittenspaceagency.wiki.gg/wiki/Version_2025.9.13.2415 — *read*
31. Version 2025.11.3.2634 — https://kittenspaceagency.wiki.gg/wiki/Version_2025.11.3.2634 — *read*
32. Version 2025.11.4.2742 — https://kittenspaceagency.wiki.gg/wiki/Version_2025.11.4.2742 — *read*
33. Version 2026.2.30.3638 — https://kittenspaceagency.wiki.gg/wiki/Version_2026.2.30.3638 — *read*
34. Version 2026.4.2.3942 — https://kittenspaceagency.wiki.gg/wiki/Version_2026.4.2.3942 — *read*
35. Version 2026.7.5.4892 — https://kittenspaceagency.wiki.gg/wiki/Version_2026.7.5.4892 — *read*
36. Version 2026.10.10.5554 — https://kittenspaceagency.wiki.gg/wiki/Version_2026.10.10.5554 — *read*

**Other**

37. Are we KSA yet? (fan tracker, updated 2026-10-13) — https://areweksayet.com/ — *read*
38. Google Patents, US7436405B2 — https://patents.google.com/patent/US7436405B2/en — *read*
39. Version 2025.11.2.2781 (Earth's air and clouds scaled back) — https://kittenspaceagency.wiki.gg/wiki/Version_2025.11.2.2781 — *search-recorded*
40. Version 2026.6.9.4750 (Bepu memory fix) — https://kittenspaceagency.wiki.gg/wiki/Version_2026.6.9.4750 — *search-recorded*
41. Ahwoo on X, blackrack's ring physics — https://x.com/Ahwoo_Official/status/2030373058117050861 — *search-recorded*
42. Dean Hall (rocket2guns) on X, custom engine and Mars dust storm — https://x.com/rocket2guns/status/1942222168085205338 — *search-recorded*
43. Dean Hall on X, the new shadow system with ground clutter, February 2026 — https://x.com/rocket2guns/status/2024727747566993834 — *search-recorded*
44. "Kittens, Kerbals and Kiwis" (video interview) — https://www.youtube.com/watch?v=0isK9z6cyqQ — *search-recorded*, not watched
45. "Kitten Space Agency Early Access – Interstellar 1" (video) — https://www.youtube.com/watch?v=7nM7iIWnkS8 — *search-recorded*, not watched
46. Ahwoo forums, "Running it on a Mac" — https://forums.ahwoo.com/forums/kitten-space-agency/support/running-it-on-a-mac.217/ — *search-recorded*
47. Ahwoo forums, "Linux: AMD GPU: Terrain not Rendering" — https://forums.ahwoo.com/forums/kitten-space-agency/general/linux-amd-gpu-terrain-not-rendering.445/ — *search-recorded*

**Fetched but unreadable**

48. https://ahwoo.com/brutal — script-rendered, empty
49. https://ahwoo.com/app/100000/kitten-space-agency — script-rendered, empty
50. https://modding.kittenspaceagency.wiki/ — script-rendered, title only
51. PC Gamer, Dean Hall GDC interview — https://www.pcgamer.com/games/sim/dean-hall-kitten-space-agency-inspiration-gdc-interview/ — body truncated, headline only
52. Space.com interview — https://www.space.com/entertainment/space-games/kitten-space-agency-is-the-spiritual-successor-to-kerbal-space-program-and-they-have-an-ex-spacex-engineer-on-the-team-interview — body missing (mirror 13 read)
53. Ludeon, "Announcing the Waylith Engine and Update 1.7" — https://steamcommunity.com/games/294100/announcements/detail/672882362547178634 — body missing
54. This Week in Video Games, BRUTAL announcement — https://thisweekinvideogames.com/news/rocketwerkz-founder-and-dayz-creator-dean-hall-announces-open-source-game-development-framework/ — HTTP 403
