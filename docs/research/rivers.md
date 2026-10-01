# Research — Rivers: integrating the island's rivers into the terrain, their scale, their drawing, and whether to simulate

> Companion to `water.md` §3 (rivers as flow-mapped ribbons over a carved network; Peytavie 2019,
> Far Cry 5, Emilien 2015, Paris 2023 are verified there and only re-read here where their numbers
> matter) and to `terrain-genesis.md` §1–§2 (the stream-power solver, drainage, lakes; Cordonnier
> 2016, Schott 2023/2024, Tzathas 2024 are verified there). Written 2026-10-01 after the owner's
> look at the island's rivers (step 6 of `docs/demos/island.md`): the rivers "look like small streams
> rather than proper rivers" and "don't feel natural or integrated into the game world" — carved
> channels that read as trenches with hard dark banks on flat ground, a bed layer painted wider
> than the channel, an unnatural lake entry, a mouth that is a cascade (the largest river falls
> 13–17 % over its last 160 m), and water far up a valley that is invisible from a low camera.
> Every source below was fetched this day (page, PDF text, or a publisher/Semantic Scholar record);
> the grade is in [Verification notes](#verification-notes), what could not be verified is in
> [Checked and left out](#checked-and-left-out).

The question is what shipped engines, production talks, the graphics literature and the
geomorphology say about making a generated river read as a river — and whether a fluid simulation
would do what Forge's analytic ribbons do not. The short answer: nobody who shipped a river made
it look like one with the water alone. The engines (Unreal's Water plugin, World Machine, Houdini,
R.A.M) all carve a *valley* — floodplain, terrace, bank, channel — into the heightfield itself,
paint the bed and banks from the same hydraulic fields, and only then lay a thin, flow-mapped
surface in it; the games (Far Cry 5, Horizon Forbidden West, Uncharted 4, LightSpeed's Photon
Water) bake their water motion offline and draw it as advected textures and instanced deformations,
with a real-time shallow-water solver, where one exists at all, confined to a window of about
512 m around the camera. The research since 2013 (Génevaux, Peytavie, the Lyon group's erosion and
amplification papers, McDonald & Cordonnier 2026) agrees on the shape of the pipeline — a network
from drainage, a Rosgen type per reach from slope and area, a template cross-section scaled by
discharge, a graded longitudinal profile, meanders and bars on the plains, braids and deltas at the
coast — and the geomorphologists supply the numbers: bankfull width `≈ 2.7 A^0.35` m and depth
`≈ 0.3 A^0.21` m for `A` in km², a floodplain some six channel widths wide, pools every five to seven
widths, meanders ten to fourteen widths long, and a bed slope that must fall toward the sea, never
rise. Measured against those numbers, Forge's rivers are already two to three times wider and
deeper than nature's for their catchments; what they lack is everything around them.

> **State of the art in five sentences.** A river that reads as a river is a valley first: a
> floodplain several channel widths wide at bankfull level, a bank of about one bankfull depth, a
> channel whose cross-section follows its Rosgen type (parabolic and asymmetric in bends, wide and
> shallow on riffles, braided near the coast) and whose longitudinal profile is concave and graded
> to the sea, all carved into the base heightfield so that every terrain LOD carries it (Unreal's
> landscape brush, World Machine's River device, Génevaux 2013, Peytavie 2019). Its materials come
> from the hydraulics — gravel on riffles, sand on bars and in pools, mud in the backwaters, a wet
> band between low flow and bankfull, a riparian strip of reeds and trees placed by distance to the
> channel and height above the water (Houdini's sediment/debris/flow layers, Unreal's weightmap
> brush, Snowdrop's "near water" scatter rules). Width and depth scale with catchment through the
> regional hydraulic-geometry curves (Leopold & Maddock 1953; Bieger et al. 2015), the floodplain
> and valley width with `A^0.4–0.5` (Turowski et al. 2024), pools, meanders and bars with the width
> (Leopold, Wolman & Miller 1964; Williams 1986). The water surface itself is baked, not simulated,
> in every shipped open world — flow maps generated from the geometry (Far Cry 5), Houdini
> simulations instanced as deformation volumes (Horizon Forbidden West), offline height-and-flow
> maps in virtual textures (Photon Water), Unreal 5.6's baked Shallow Water River — with a real-time
> shallow-water grid of about 512² cells at one metre, 0.5 ms on a PS5, used only where things
> interact. Far rivers are drawn as part of the terrain (the carve is in the heightmap; the flow map
> is coarse far away), never as geometry that can fall below a coarser terrain LOD.

**Contents**

1. [Engines and tools: how rivers are carved, textured and drawn](#1-engines-and-tools-how-rivers-are-carved-textured-and-drawn)
2. [Shipped games and their talks](#2-shipped-games-and-their-talks)
3. [Research 2013–2026: procedural rivers and river–terrain coupling](#3-research-20132026-procedural-rivers-and-riverterrain-coupling)
4. [The geomorphologists' numbers](#4-the-geomorphologists-numbers)
5. [Shallow-water simulation: what it gives, what it costs](#5-shallow-water-simulation-what-it-gives-what-it-costs)
6. [Forge against the numbers](#6-forge-against-the-numbers)
7. [Recommendation for Forge](#recommendation-for-forge)
8. [Checked and left out](#checked-and-left-out)
9. [Verification notes](#verification-notes)
10. [Sources](#sources)

---

## 1. Engines and tools: how rivers are carved, textured and drawn

One pattern underlies this section: every tool that produces a believable river edits the
*terrain* from the river's spline or network — a channel inside a floodplain inside a valley, with
a falloff into the surrounding ground — paints the ground from the same data, and treats the water
surface as a thin layer whose width, depth and velocity are per-point attributes of the spline.

**Epic Games. Unreal Engine 5 Water system — "Water Body Actors", "Water Meshing System and Surface
Rendering", "Water System".** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/water-body-actors-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/water-meshing-system-and-surface-rendering-in-unreal-engine>
· <https://dev.epicgames.com/documentation/en-us/unreal-engine/water-system-in-unreal-engine>

- *Bodies by spline.* Rivers, lakes and oceans are "defined using splines"; a river's spline points
  "can have varying heights" and carry three per-point attributes: **Depth** ("the depth of the
  river at each spline point"), **River Width**, and **Velocity** ("the directional speed along
  the river's spline path", which "is written to a flow map that visually drives the flow of
  water").
- *The carve is in the heightmap.* "Water bodies automatically work with Landscape Terrain to carve
  out the terrain beneath them using a Landscape Brush", which requires **Enable Edit Layers**. The
  brush has **Affect Heightmap**, **Affect Weightmap** and **Affected Weightmap Layers** — the
  same stroke that lowers the ground paints the bed's material layers. The profile is a curve:
  **Use Curve Channel**, an **Elevation Curve Asset** ("how the landscape is carved below the water
  surface"), **Channel Edge Offset** ("an offset from the edge of the water to where the curve
  starts"), **Curve Ramp Width**; outside it a **Falloff** of mode **Angle** ("extends until
  terrain intersection") or **Width** (a fixed distance), and an **Edge Offset** that "applies a
  flat edge around the Water Body" — a flat bank shelf before the slope begins. Because the carve
  edits the landscape's own heightmap, every landscape LOD contains it; there is no separate "river
  mesh" that a coarser terrain can rise above. An **Island** body does the inverse ("terraforming
  controls... to ensure terrain stays above water").
- *Transitions.* "River Water Bodies act as connections between other water bodies"; dedicated
  **River to Lake Transition** and **River to Ocean Transition** materials "automatically blend them
  together seamlessly", otherwise **Overlap Material Priority** decides.
- *The mesh and the far field.* One quadtree water mesh per **Water Zone**: tiles of **Tile Size**
  2400 units by default over **Extent in Tiles** 64, tiles morph between LODs ("four quads collapse
  into one"), **LOD Scale** sets where morphing starts, **Tessellation Factor** the vertex density; a
  **Far Distance Mesh** (default on) "fills gaps between Ocean Water Body extent and horizon" with
  the `Water_FarMesh` material; a **Water LOD Material** is "used by this water body when rendered
  as a Static Mesh" (the non-tessellated LOD). The zone's water info texture ("the
  WaterVelocityTexture is now regenerated at runtime (WaterInfoTexture in AWaterZone)", 5.1 Python
  API) carries height and velocity for the shaders and gameplay.
- *Simulation.* Niagara Fluids lists a **Shallow Water** template beside its 2D/3D gas and liquid
  templates. Unreal 5.6 adds the experimental **Water Advanced** plugin with a **Shallow Water
  River** actor; its component (`ShallowWaterRiverComponent`, already in the 5.5 API) takes a
  `source_river_water_body` and a `sink_river_water_body`, `bottom_contour_actors` captured with a
  `bottom_contour_capture_offset`, `resolution_max_axis`, `num_steps`, `sim_speed`, and *bakes* the
  result: `baked_water_surface_rt`, `baked_water_surface_texture`, `baked_sim`, a
  `water_info_texture` array. Epic's own tutorial series is titled "Baked River Simulations" — the
  simulation is a content-time tool whose output is a flow texture, with the runtime solver optional.
  Users report that World Partition maps need the landscape proxies assigned by hand as bottom
  contours.

*Bearing:* this is the shape Forge should copy, not the renderer: the spline (Forge's polyline)
owns width, depth and speed per point; the carve is a curve profile plus a flat edge shelf plus a
falloff *into the base heightfield*; the same stroke paints the layers; lake and sea joins are a
material blend over an overlap, not a geometric seam; the simulation, when it comes, is baked.

**Unity (HDRP). Water System: "Create a current in the Water System", "Settings and properties
related to the water system".** [docs] [still-current]
<https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@15.0/manual/WaterSystem-currentmap.html>
· <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/settings-and-properties-related-to-the-water-system.html>

- A **River** surface type with geometry as a **Quad**, **Instanced Quads** or a **Custom Mesh**
  (the artist's ribbon); a **Current Map** whose red and green channels are the direction and blue
  the influence ("the water flow can't stop and it always has a direction", unlike a flow map); a
  **Water Mask** to attenuate swell and ripples; two swell bands plus **Ripples** with their own
  wind; **Simulation Foam Amount**; tessellation with a **Max Tessellation Factor** and a fade.
  There is no terrain carving: Unity's rivers are surfaces laid on ground shaped by other tools.

*Bearing:* Forge already has the equivalent of the current map (the D8 direction and the Chézy
speed per ribbon vertex) and the ripple band; Unity confirms the surface alone is not the problem.

**NatureManufacture. R.A.M 3 — River Auto Material (Unity Asset Store; 80.lv release note,
2024-06-18).** [tool] [still-current]
<https://assetstore.unity.com/packages/tools/terrain/r-a-m-3-river-auto-material-3-287456> ·
<https://80.lv/articles/naturemanufacture-s-river-auto-material-3-for-unity-is-out-now>

- The most-used Unity river tool since 2014: river, lake, sea, waterfall and swamp splines that
  "carve terrain and textures on it, automatically under the splines", with flow maps, cascades,
  spline height painting, and a profile system; the 2024 rebuild shipped for HDRP/URP first.
- *Bearing:* the selling point for ten years has been the one operation Forge lacks — the spline
  carves *and* paints, in one tool, with the painted band bound to the carve.

**Procedural Worlds. GeNa Pro — "GeNa Rivers: a specialized spline extension for river networks";
Gaia Pro integrates "GeNa roads and rivers".** [tool] [still-current]
<https://www.procedural-worlds.com/products/professional/gena-pro/>

- Confirms the division of labour in the Unity ecosystem: Gaia generates and textures the terrain,
  GeNa's spline rivers carve and texture under the spline and spawn vegetation along it. Nothing
  more specific is documented on the public page.

**SideFX. Houdini heightfields — "Flow fields", "HeightField Erode", "HeightField Erode Hydro",
"Shallow Water Solver", "Texture layers" (Houdini 22.0 docs).** [docs] [still-current]
<https://www.sidefx.com/docs/houdini/heightfields/flowfields.html> ·
<https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode.html> ·
<https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode_hydro.html> ·
<https://www.sidefx.com/docs/houdini/heightfields/shallowintro.html> ·
<https://www.sidefx.com/docs/houdini/heightfields/texturelayers.html>

- *Flow fields* produce a `flow` layer ("the cumulative material flow"), a `flowdir` layer and a
  `water` layer, used "for erosion channel creation through distortion of terrain", for masks and
  for scattering (the example displaces the terrain by the `flow` layer with a displace scale of 2).
- *Erode* exposes the channel's shape directly: **Bank Angle** "controls the angle of channel walls.
  Higher values will create steeper channels that appear to cut deeper"; **Erosion Rate**,
  **Deposition Rate** ("deposits its excess accumulated sediment"), **Removal Rate**, **Spread
  Iterations** (how far eroded material travels), **Slope Influence** ("lower values... straighter
  channels"). *Erode Hydro* (since 17.0) separates **Erosion Rate Factor (Riverbed)** from
  **Erosion Rate Factor (Riverbank)** — the first deepens the channel, the second widens it — with
  **Sediment Capacity**, **Deposition Rate** and a **Max Bank to Bed Water Ratio** deciding what
  counts as bank. Outputs are layers: `height`, `water`, `sediment`, `debris`, `flow`, `flowdir`,
  `bedrock`.
- *Texture layers:* "a layer is basically a mask"; `sediment`, `debris`, `flow`, `water` and
  `occlusion` are wired into material mixes — the sand is where the sediment layer is, the scree
  where the debris is, the wet rock where the flow is.
- *Shallow Water Solver:* a heightfield solver for "ripples and non-breaking waves, ponds and
  puddles, water running over cracks" and "fast flooding of large areas", fed by `source`/`sink`
  layers, emitting a `water` layer and velocity; "very fast" but "often suffers from stability
  problems, especially at high resolutions or high propagation speeds", mitigated by capping the
  wave speed, adding viscosity and border damping; it "cannot simulate splashes, spray, or breaking
  waves".

*Bearing:* Far Cry 5's and Horizon's rivers were built in this toolset (`terrain-genesis.md` §4,
`water.md` §3). The lesson for genesis is that the bed and bank materials are *outputs of the
erosion* (sediment, debris, flow), not a painted stripe at a fixed distance from the centreline.

**QuadSpinner. Gaea — "Rivers" node.** [docs] [still-current]
<https://docs.quadspinner.com/Reference/Water/Rivers.html>

- "Creates a realistic network of rivers with controllable headwaters"; it "subtly transforms the
  terrain to provide unbroken pathways" (no uphill water); parameters **Headwaters** (count, or a
  mask), **Water** ("the amount of water in headwaters. Higher values can create larger rivers"),
  **Width**, **Depth** ("independent of Downcutting"), **Downcutting** ("how deeply the river
  should cut into rock and soil while moving forward"), **Render water surface**.
- *Bearing:* an artist's river is sized by *water volume at the head*, not by the terrain's own
  drainage — the knob Forge's generated island lacks and may want (an exaggeration factor on the
  discharge, §7 b).

**World Machine. "River Device" (help), "Rivers" (blog, 2015-03-17).** [docs] [still-current]
<https://help.world-machine.com/topic/device-river/> · <https://www.world-machine.com/blog/?p=470>

- The most explicit published river-valley model in a terrain tool. Channel: **Bankfull Width** and
  **Bankfull Depth** "in meters"; **Channel Type** *Curved* ("a roughly parabolic-shaped bottom to
  the channel that migrates back and forth with the thalweg") or *Trapezoidal* (**Trapezoid Aspect**
  0 = box, 1 = triangle); **Flow Speed** automatic from the gradient. Valley: **Create Valley** ("how
  strongly to embed a river valley into the terrain. A value of 0 creates no river valley"),
  **Valley Width**, **Valley Height** ("a minimum depth of the valley below the existing terrain"),
  **Valley Wall Shape** ("low values create steeper-sided walls, while higher values produce broad
  U-shaped valleys"), **Valley Feature Size**, **Valley Breakup Amount** (fractal noise blending the
  valley into the terrain). Floodplain: **Floodplain Width**, **Floodplain Height** (where water
  "overtops its banks"), **Flatten Meander Belt Area**. Networks: drawing from or to mid-river
  extends or feeds a reach; "each defined river segment (reach) can have all of its parameters set
  separately".
- The blog explains the model: a *Geomorphic Covariance Structure* preset "will vary the width of
  the river in a pattern relative to the meanders and thalweg" — wide-shallow riffles and
  narrow-deep pools, cut banks on the outer bends and point bars on the inner — with "River,
  Floodplain, Valley walls" as three nested surfaces and a meander belt at the floodplain's base;
  outputs include a depth map (pools and riffles) and masks for texturing.

*Bearing:* this is the parameter list Forge's carve should grow into: channel (width, depth,
profile, thalweg offset), floodplain (width, height above the water), valley (width, wall shape,
noise), per reach, with the river's own hydraulics (§4) filling the numbers in.

---

## 2. Shipped games and their talks

**Branislav Grujic, Cristian Cutocheras. "Water Rendering in 'Far Cry 5'." GDC 2018.** [talk]
(verified in `water.md` §3)

- Rivers from the Houdini freshwater network; "flow maps are created automatically... based on
  spline and flood-fill routines", "displayed in greater detail near the player than at a greater
  distance"; cost "scaleable with the number of water pixels".
- *Bearing:* the two-resolution flow map is the answer to "far rivers": coarse everywhere, fine
  near the camera, both derived from the geometry.

**Hugh Malan (Guerrilla). "Rendering Water in Horizon Forbidden West." SIGGRAPH 2022, Advances in
Real-Time Rendering in Games.** [talk] [slides read in full]
<https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Water-Malan.pdf>

- The approach: "a Houdini sim of a localized area, bake it down for realtime use. At runtime we'll
  instance that data, it'll be played back and blended to create the rendered water surface" — the
  "realistic 3D movement of the offline Houdini sim" bent "to any shape we need". Rivers and lakes
  needed "standing waves around rocks, eddies and ripples", so beside the breaking-wave
  cross-section they bake *animated 2D regions*: a Houdini water surface expressed as a deformation
  and stacked into a 3D texture ("an actual deformation volume... had 128 layers"), with a
  shrink-wrap (van Overveld & Wyvill 2004, field lines by Coulomb's law) to turn arbitrary sim
  meshes into a deformable sheet. "With this feature we created a library of localized 2D water
  effects. Houdini was used to procedurally place instances throughout the world, which were then
  reviewed and refined by environment artists. They provide all the waterfall impacts."
- The honest limits: "the triangle density wasn't high enough... That's why the deformations on
  rivers are so flat"; foam lives in vertex colour so "can't create features smaller than
  triangles"; late in the project tessellation was raised around the waves; the wish list is "one
  vertex per pixel", "maybe a splat-based solution", and offline-style whitewater.
- *Bearing:* the best-looking shipped rivers are baked, instanced effects on a tessellated sheet,
  placed by a procedural tool from the river geometry. Forge's "potential flow past a cylinder" and
  rapids foam are the analytic version of the same library; what Horizon adds is *vertical* motion
  (standing waves) and plunge pools, which need vertex displacement, not just normals.

**Carlos Gonzalez-Ochoa (Naughty Dog). "Rendering Rapids in Uncharted 4." SIGGRAPH 2016.** [talk]
(verified in `water.md` §3)

- "Offline fluid simulations to inform the overall look of the river as well as to produce data of
  the water surface and flow." *Bearing:* bake, then advect.

**Zhenyu Mao, Kui Wu (LightSpeed Studios). "Open-World Water Rendering and Real-Time Simulation."
GDC 2023, Advanced Graphics Summit; 80.lv interview "Developing a Next-Gen Water Rendering Solution
for Games".** [talk] [interview]
<https://www.gdcvault.com/play/1028829/Advanced-Graphics-Summit-Open-World> ·
<https://80.lv/articles/developing-a-next-gen-water-rendering-solution-for-games>

- The Photon Water System (Mao was "the former water tech lead in Far Cry 5 and 6"): "both
  pre-computing and simultaneous updating of water data (height, velocity, and foams) based on
  physical equations", "converted into an adaptive water mesh on the fly", with CDLOD for the far
  field. Artists define water by splines, procedural tools or meshes; a lattice-Boltzmann
  shallow-water solver *pre-computes* the flow ("high Reynolds number turbulence") into height and
  flow maps stored in "tile-based virtual textures"; a plain shallow-water solver updates a
  **512×512 grid at one square metre per pixel** at run time for **0.5 ms on PS5**; a third,
  surface-wave simulation for players and objects costs **< 0.1 ms on PS5** (about 1 ms on a
  Snapdragon 865); the whole water is "~2 ms when screen-dominant". Foam is derived from the
  velocity field; physics APIs read the same data.
- *Bearing:* the clearest published budget for a runtime shallow-water window in an open world:
  512² cells, 1 m, half a millisecond, over a baked field in a virtual texture. It is also the only
  production system found that does what §5 proposes for later.

**Massive Entertainment. "Crafting Pandora's Breathtaking Landscape With Snowdrop" (blog); Joshua
Simmons, "Upgrading the Snowdrop Engine for the Massive World of 'Avatar: Frontiers of Pandora'",
GDC 2024 (slides read).** [blog] [talk]
<https://www.massive.se/blog/games-technology/snowdrop/crafting-pandoras-breathtaking-landscape-with-snowdrop/>
· <https://media.gdcvault.com/gdc2024/Slides/GDC+slide+presentations/Simmons_Joshua_Upgrading_the_Snowdrop.pdf>

- Not a water talk, but the only verifiable statement of how a shipped open world dresses its
  banks. Carl Leonardsson: "We can scatter in layers... you can have scattering at the bottom of a
  river, at the surface, on the cliffs next to the river"; David Österlind: "a river has a certain
  type of pebbles and rocks on its banks, which in turns have certain flowers that grow around
  them." The GDC deck: scattering is "artist driven, controlled by scripting with graphs", a
  hierarchy where "successfully placed objects chain, attempting to place the next object nearby"
  with "attach conditions – e.g. 'near water'"; 70 M instances in 58,000 sectors of 128 m, 17 bytes
  per instance; the GPU frame lists "Water waves (600us)" on async compute beside "Terrain
  (1.3ms)".
- *Bearing:* the riparian strip is a placement *rule* keyed on the river (distance to water, bank
  rocks → flowers), evaluated by the same scatter system as everything else — Forge's ecosystem
  stage (`game-ai-ecosystems.md`, `vegetation-materials.md`) should take "distance to channel" and
  "height above water" as inputs.

---

## 3. Research 2013–2026: procedural rivers and river–terrain coupling

One idea underlies this section: the river network is the generating structure of the terrain, not
an afterthought laid on it; each reach gets a type from its slope and its flow, the type gives a
cross-section template and a longitudinal rhythm, and the templates are carved into (or the terrain
is grown around) the network.

**Jean-David Génevaux, Éric Galin, Éric Guérin, Adrien Peytavie, Bedřich Beneš. "Terrain Generation
Using Procedural Models Based on Hydrology." *ACM Transactions on Graphics* 32(4), Art. 143
(SIGGRAPH 2013).** [paper] [foundational] [PDF read]
<https://dl.acm.org/doi/10.1145/2461912.2461996> (author PDF
<https://www.cs.purdue.edu/cgvlab/www/resources/papers/Genevaux-ACM_Trans_Graph-2013-Terrain_Generation_Using_Procedural_Models_Based_on_Hydrology.pdf>)

- Rivers first, terrain after: a drainage network grown by Horton–Strahler expansion rules under
  a user's river-slope and terrain-slope maps, then watersheds, then terrain and river *primitives*
  blended and carved (`A = C(B({Tᵢ}), B({Rᵢ}))`). Mean flow from catchment: "Let A [m²] be the
  watershed area. The mean flow φ of the river [m³s⁻¹] is given by φ = 0.42·A^0.69" (after Dunne &
  Leopold 1978; "takes into account evaporation and infiltration"). Rosgen's nine types "(A+, A, B,
  C, D, DA, E, F, or G)" each have "a trajectory type... and a digging profile of the riverbed",
  including "the geological composition of the riverbed (bedrock, rocks, stones, gravel, sand, silt,
  or clay)". Two rules Forge can lift directly: "River nodes that are close to coasts (based on a
  geodesic distance threshold) are labeled as braided rivers (... D or DA)" and "river mouths with a
  flow greater than a fixed value are marked as deltas"; junctions of very different flows meet
  "nearly perpendicular", similar flows "a small angle". A river primitive is a curve skeleton with
  a profile function, `h(p) = u_z(p) + φ(d(p))`, the profile "made of multiple layers that
  correspond to bedrock, water, and sand".
- *What it gives Forge:* the typing (slope, flow, coast distance → A/B/C/D/DA/delta), the layered
  profile (bed material as part of the cross-section), the junction angles. *Cost:* rules on the
  existing polylines; nothing at run time.

**Adrien Peytavie, Thibault Dupont, Éric Guérin, Yann Cortial, Bedřich Beneš, James Gain, Éric
Galin. "Procedural Riverscapes." *Computer Graphics Forum* 38(7) (Pacific Graphics 2019).** [paper]
[recent] [PDF read]
<https://onlinelibrary.wiley.com/doi/10.1111/cgf.13814> (author PDF
<https://perso.liris.cnrs.fr/eric.galin/Articles/2019-riverscapes.pdf>)

- From a bare heightfield (1–30 m per pixel): drainage area (Freeman's D∞), depression filling
  (priority flood), a discrete network above a threshold, a graph with slope `s`, flow `φ`, stream
  power `P = A^½·S` and Strahler number per edge, a Rosgen type per edge; then *amplification*: the
  planar trajectory ("meandering if the local slope is low and the flow moderate (type C, E or G),
  or straight if the slope is steep (type A or A+)"), the longitudinal profile ("randomly sampling
  the river trajectory and flattening the water level and riverbed between certain samples";
  "we check that the river height is monotonically decreasing... If the riverbed slope is greater
  than a threshold, we adjust the height by inserting cascades and leveling the profile"), and the
  cross-section: Rosgen templates "of unit area" scaled so that `a = φ / ‖u‖` with `φ = 0.42·A^0.69`
  (the same formula; see the caveat in §4). Type A: "a succession of waterfalls interspersed by
  stretches of flatter but still turbulent water", basins placed "the steeper the slope, the more
  basins"; type C: "strong curvature... low overall slope (less than 2%)", "the cross-sectional
  profile is asymmetric in sections of high curvature" (scour on the outer bank), symmetric in the
  straights, blended; type D: "wide rivers with little slope... riverbeds with several channels of
  varying width and depth", the channel count from flow and width, the bed "set as the minimum height
  over all channels"; B is A with basins further apart, DA is D wider, E/F/G are C with other
  curvature. Riverbed carving uses the compactly supported blend/carve operators of Génevaux 2015.
- The water: a *blend-flow tree* of primitives (calm, turbulent, wave, cascade, vortex, ripple)
  every 50 cm, each with elevation, amplitude and velocity, blended by `C²` radial weights, with
  merge and replace operators; `f(p,t) = e + a·h(p,t)` where `h` is fBm warped by the flow.
  Numbers: 15 ms to generate small scenes, 130–227 s for 4–193 km of river on 3–24 km terrains;
  22 KB (50 m) to 2.7 MB (4 km) of primitives; a GPU grid of patches with tessellation,
  48–195 fps at 1080p with tens of thousands of primitives; a 30 × 30 m FLIP comparison took 9 h of
  simulation per iteration and 165 h in all against one hour of authoring. Limits stated by the
  authors: no three-dimensional effects ("breaking waves, waterfalls or splashes"), and material
  layers ("bedrock, pebbles, and sand") "to future work".
- *What it gives Forge:* the whole §7 (a) list has a published precedent here — the profile that is
  flattened between samples and never rises, cascades inserted where the slope exceeds a threshold,
  asymmetric bends, braids near the coast, and a primitive library the shader blends. *Cost:* an
  offline pass over the polylines; the primitives are the ribbons' per-vertex data Forge already
  stores.

**Axel Paris, Éric Guérin, Pauline Collon, Éric Galin. "Authoring and Simulating Meandering
Rivers." *ACM TOG* 42(6) (SIGGRAPH Asia 2023); code MIT.** [paper] [code] (verified in `water.md`)

- Curvature-driven migration with cutoffs and oxbows on the lowland reaches. *Gives:* sinuosity on
  the plains, hence point bars, cut banks and oxbow lakes; *cost:* a few hundred iterations per
  lowland reach at genesis.

**Arnaud Emilien, Pierre Poulin, Marie-Paule Cani, Ulysse Vimont. "Interactive Procedural Modelling
of Coherent Waterfall Scenes." *CGF* 34(6), 2015.** [paper] (verified in `water.md`)

- Falls, pools and streams as vector elements with hydraulic consistency. *Gives:* the deliberate
  cascade where a reach's slope breaks — the right answer for a cliff coast, the wrong one for the
  island's largest river (§7 a).

**Guillaume Cordonnier et al. "Large Scale Terrain Generation from Tectonic Uplift and Fluvial
Erosion." *CGF* 35(2), 2016, 165–175; "Authoring Landscapes by Combining Ecosystem and Terrain
Erosion Simulation." *ACM TOG* 36(4), 2017; "Forming Terrains by Glacial Erosion." *ACM TOG* 42(4),
2023.** [papers] (2016 verified in `terrain-genesis.md`; 2017 and 2023 records fetched)

- 2016 is the pipeline Forge runs; 2017 couples it to vegetation (the riparian strip as an
  ecosystem output); 2023 adds glaciers, "ranging from U-shaped and hanging valleys to fjords and
  glacial lakes" — the only published source of *broad flat-floored valleys* in this family, which
  is what a river valley on a plain looks like. *Cost:* 2023 is a deep-learning ice-flow estimate
  plus a multi-scale advection — far beyond the island's needs; the valley shape it produces can be
  imposed by rule (§7 a) instead.

**Hugo Schott, Axel Paris, Lucie Fournier, Éric Guérin, Éric Galin. "Large-scale Terrain Authoring
through Interactive Erosion Simulation." *ACM TOG* 42(5), Art. 162, 2023.** [paper] (record fetched;
also in `terrain-genesis.md`)

- Authoring "in the uplift domain" with "point and curve elevation constraints to precisely sculpt
  ridges or carve river networks" and "hydrologically consistent blending between terrain patches".
  *Gives:* a principled way to make the island's main valleys — constrain the uplift so a few trunk
  rivers exist where the designer wants them, then let the solver grade them. *Cost:* already the
  solver Forge has; the constraints are new inputs.

**Hugo Schott, Éric Galin, Éric Guérin, Axel Paris, Adrien Peytavie. "Terrain Amplification using
Multi-scale Erosion." *ACM TOG* 43(4), Art. 145, 2024.** [paper] [code] (verified in
`terrain-genesis.md`)

- "Thermal, stream power erosion and deposition performed at different scales" to amplify a coarse
  terrain into a hydrologically consistent fine one. *Gives:* the bed and bank detail (gullies,
  deposition in the valley floor) at the 1–2 m scale *without* contradicting the 8 m drainage — the
  owner's "trench on flat ground" is exactly an amplification that did not know about the river.
  *Cost:* seconds per tile on the GPU per the paper; `terrain-genesis.md` §8 already plans it.

**Petros Tzathas, Boris Gailleton, Philippe Steer, Guillaume Cordonnier. "Physically-based
Analytical Erosion for Fast Terrain Generation." *CGF* 43(2) (Eurographics 2024).** [paper]
(record fetched; also in `terrain-genesis.md`)

- Analytic solutions of the stream-power law with time as "a slider that controls the aging of the
  input terrain". *Gives:* a graded river profile in one evaluation — a cheap way to re-grade the
  last reach to the sea (§7 a). *Cost:* an evaluation per cell.

**Cyprien Grenier, Éric Guérin, Éric Galin, Basile Sauvage. "Real-time Terrain Enhancement with
Controlled Procedural Patterns." *CGF* 2023/2024.** [paper] (record fetched)

- Phasor-noise "erosion patterns" that "align with the slope" and the water flow, evaluated on the
  GPU in real time. *Gives:* bank gullies and rills that follow the flow direction at the 1 m scale
  as a shader detail rather than a baked field. *Cost:* a noise evaluation in the terrain shader.

**Oscar Argudo, Éric Galin, Adrien Peytavie, Axel Paris, James Gain, Éric Guérin. "Orometry-based
Terrain Analysis and Synthesis." *ACM TOG* 38(6) (SIGGRAPH Asia 2019); Argudo, Guérin, Schott,
Galin, "Terrain descriptors for landscape synthesis, analysis and simulation", *CGF* 2025.**
[papers] (records fetched)

- 2019 synthesises a mountain range from a peak-and-saddle graph "by deriving a consistent river
  network and shaping the valley slopes"; 2025 is a review of geomorphological metrics (slope,
  drainage, curvature, wetness...) for graphics, with a correlation study. *Gives:* the vocabulary
  (and a checklist) for the fields §7 (a) keys materials and vegetation on. *Cost:* none; it is a
  reading list.

**Éric Guérin, Julie Digne, Éric Galin, Adrien Peytavie, Christian Wolf, Bedřich Beneš, Benoît
Martinez. "Interactive Example-Based Terrain Authoring with Conditional Generative Adversarial
Networks." *ACM TOG* 36(6) (SIGGRAPH Asia 2017).** [paper] [search record only]
<https://dl.acm.org/doi/10.1145/3130800.3130804>

- Sketch rivers and ridges, get a terrain. Not for a seeded island; listed because the task asked.

**Jian Zhang, Changbo Wang, Hong Qin, Yi Chen, Yan Gao. "Procedural modeling of rivers from single
image toward natural scene production." *The Visual Computer*, 2017.** [paper] (record fetched)
<https://link.springer.com/article/10.1007/s00371-017-1465-7>

- "A compact parametric model to represent rivers with features such as tributaries,
  distributaries, tortuosity, and adjacent lakes" fitted from one photograph. *Gives:* a parametric
  description of a mouth with distributaries; *cost:* not applicable to generation from a seed.

**Roland Fischer, Judith Boeckers, Gabriel Zachmann. "Procedural Generation of Landscapes with Water
Bodies Using Artificial Drainage Basins." CGI 2022 (University of Bremen).** [paper] [PDF read]
<https://cgvr.cs.uni-bremen.de/papers/cgi22/CGI22.pdf>

- "After authoring the initial landmass, we first generate rivers and lakes and then create the
  actual terrain by 'growing' it, starting at the water bodies"; "vast landscapes can be created in
  under half a minute"; a Unity prototype. *Gives:* the argument, in a second group's hands, that
  water-first integration is what makes the river "natural-looking"; *cost:* not Forge's pipeline.

**Haoran Feng. "Generating Realistic River Patterns with Space Colonization." WSCG 2023.** [paper]
[search record only] — river *patterns* (branching) from space colonisation; of use only for a
planet-scale network, not the island.

**L. O. Valencia-Rosado, Z. J. Guzman-Zavaleta, O. Starostenko. "A Modular Generative Approach for
Realistic River Deltas: When L-Systems and cGANs Meet." *IEEE Access* 10, 2022, 5753–5767.**
[paper] [search record only] — delta skeletons from a stochastic L-system, elevation and imagery
from cGANs; the only recent paper found on *deltas* as such. A rule (Génevaux's) is enough for one
island.

**Nicholas McDonald, Guillaume Cordonnier. "Stochastic Geomorphological Transport for Terrain
Erosion Simulation." *ACM TOG*, 2026.** [paper] [recent] (record fetched)
<https://dl.acm.org/doi/10.1145/3811336>

- "A novel, parallel, stochastic particle-based method capable of simulating transport over
  geological timescales", relaxing the stream-power law's velocity assumptions; it "captures
  multiscale geomorphological features, producing coherent basin structures and dynamic phenomena
  such as braided rivers, meanders, and deltas". *Gives:* the first erosion model in graphics that
  produces floodplains, braids and deltas *by transport* rather than by rule — the long-term answer
  to "integrated". *Cost:* a new solver; worth a reading session when the rules of §7 (a) stop
  being enough, not before.

---

## 4. The geomorphologists' numbers

One correction underlies this section: Leopold & Maddock's exponents are exponents of
*discharge*, and discharge grows more slowly than drainage area; applied to area, the width
exponent is 0.35–0.42, not 0.5.

**Luna B. Leopold, Thomas Maddock Jr. "The Hydraulic Geometry of Stream Channels and Some
Physiographic Implications." USGS Professional Paper 252, 1953.** [paper] [foundational]
<https://pubs.usgs.gov/publication/pp252>

- Width, depth and velocity "vary with discharge as simple power functions"; the downstream
  exponents commonly quoted from it are about 0.5 (width), 0.4 (depth) and 0.1 (velocity) *of
  discharge*. Since bankfull discharge itself scales roughly as `A^0.7–0.9`, width against *area*
  comes out near `A^0.35–0.45` — which is what the regional curves below measure.

**Katrin Bieger, Hendrik Rathjens, Peter M. Allen, Jeffrey G. Arnold. "Development and Evaluation of
Bankfull Hydraulic Geometry Relationships for the Physiographic Regions of the United States."
*JAWRA* 51(3), 2015, 842–858.** [paper] [PDF read]
<https://swat.tamu.edu/media/114657/bieger_etal_2015.pdf> ·
<https://www.ars.usda.gov/research/publications/publication/?seqNo115=312862>

- Regression on more than 1,200 sites compiled from over 50 publications, `DA` in km², metres:
  nationwide **W = 2.70·DA^0.352**, **D = 0.30·DA^0.213**, cross-section **A = 0.95·DA^0.540**.
  Regional examples: Appalachian Highlands W = 3.12·DA^0.415, D = 0.26·DA^0.287; Pacific Mountain
  System W = 2.76·DA^0.399, D = 0.23·DA^0.294; Rocky Mountains W = 1.24·DA^0.435, D = 0.23·DA^0.225.
  "Regional curves are more reliable than the nationwide curve", and "drainage area is a less
  reliable predictor of bankfull channel dimensions than bankfull discharge."
- The table Forge needs (nationwide curve; width/depth in metres):

  | catchment | 0.5 km² | 1 km² | 8 km² | 50 km² | 200 km² |
  |---|---|---|---|---|---|
  | bankfull width | 2.1 | 2.7 | 5.6 | 10.7 | 17.4 |
  | bankfull depth | 0.26 | 0.30 | 0.47 | 0.69 | 0.93 |
  | width/depth | 8 | 9 | 12 | 16 | 19 |

**David L. Rosgen. "A Classification of Natural Rivers." *Catena* 22(3), 1994, 169–199.** [paper]
[foundational] (thresholds from the NC DEQ fact sheet "Application of the Rosgen Stream
Classification System to North Carolina", PDF read)
<https://www.deq.nc.gov/environmental-assistance-and-customer-service/rbac/grants/river-course-fact-sheet-2-application-rosgen-stream/download>

- Level II criteria: **entrenchment ratio** (flood-prone width at twice the maximum bankfull depth,
  divided by bankfull width) < 1.4 entrenched (A, F, G), 1.4–2.2 moderately (B), > 2.2 slightly
  (C, E, D, DA); **width/depth ratio** with "the break between single channel classifications" at
  12 (A, E, G below; B, C, F above; D above 40); **sinuosity** < 1.2 low (A), 1.2–1.5 moderate (B),
  higher for C and E; **slope** classes: A 4–10 % (Aa+ above 10 %), B and G 2–4 %, C, E, F below 2 %
  (with b/c sub-classes), D and DA braided on "low slope (less than 0.5 percent)". Type A "flow
  through steep V-shaped valleys, do not have a well-developed floodplain, and are fairly
  straight" with step-pools; B has "a broader valley but not a well-developed flood plain", rapids,
  low banks; C are "riffle/pool streams with a well-developed floodplain", W/D > 12, point bars; D
  are braided "in well-defined alluvial valleys"; E narrow, deep, highly sinuous.
- *Bearing:* the types are a lookup on (slope, area, coast distance) Forge already has per polyline
  vertex, and each type says what to carve: V-valley and no floodplain above 4 %, a bench but no
  floodplain at 2–4 %, a floodplain of several widths below 2 %, braids and bars below 0.5 %.

**Vermont DEC. "Stream Geomorphic Assessment, Appendix H: Meander Geometry" (quoting Leopold, Wolman
& Miller 1964 and Williams 1986).** [handbook] [PDF read]
<https://dec.vermont.gov/sites/dec/files/wsm/rivers/docs/assessment-protocol-appendices/H-Appendix-H-04-Meander-Geometry.pdf>

- "Leopold et al. (1964) noted that riffles were spaced 5 to 7 channel widths apart and that
  meander wavelengths measured 10 to 14 channel widths"; Williams (1986), from 153 rivers,
  "B = 3.7·W^1.12" for the meander belt, "approximately equal to six" bankfull widths. Worked
  example: wavelength 13.1 and belt 9.5 widths.

**David R. Montgomery, John M. Buffington, Richard D. Smith, Kevin M. Schmidt, George Pess. "Pool
Spacing in Forest Channels." *Water Resources Research* 31(4), 1995.** [paper] (record fetched)
<https://doi.org/10.1029/94WR03285> — the field test of the five-to-seven-widths rule in wooded
channels; cited for the rule's provenance.

**Jens M. Turowski, Aaron Bufe, Stefanie Tofelde. "A Physics-based Model for Fluvial Valley Width."
*Earth Surface Dynamics* 12(2), 2024, 493–514.** [paper] [recent]
<https://esurf.copernicus.org/articles/12/493/2024/>

- Valley width `W = k_W·A^ω` with "an exponent between 0.03 and 0.9 and the most likely value of
  0.4–0.5"; at low uplift the valley tends to the unconfined channel-belt width `W₀` (the meander
  belt), at high uplift to the channel width `W_C`. *Bearing:* the floodplain's width is the
  meander belt (≈ 6 W, Williams) where uplift is low (the island's plains) and shrinks to the
  channel where it is high (the massif).

**Renato Frasson et al. "Global Relationships Between River Width, Slope, Catchment Area, Meander
Wavelength, Sinuosity, and Discharge." *Geophysical Research Letters* 46, 2019.** [paper]
<https://doi.org/10.1029/2019GL082027> — Landsat/SRTM centrelines worldwide: width tracks
"meander wavelength and catchment area", small rivers vary most in slope and sinuosity. The
check that the relations above hold beyond the United States.

**Kelin X. Whipple, Gregory E. Tucker 1999** (verified in `terrain-genesis.md` §1): the graded
steady-state profile `S = (U/K)^(1/n)·A^(−m/n)` with concavity `m/n ≈ 0.45–0.5` — slope must fall
monotonically with area toward the outlet; a 13–17 % reach at the mouth of the largest catchment is
a knickpoint, not a river.

**A caveat on the graphics formula.** Génevaux 2013 and Peytavie 2019 both print
`φ = 0.42·A^0.69` with `A` in m² and `φ` in m³/s. Taken literally it gives 86,000 m³/s for a 50 km²
catchment (the Amazon is about 200,000); with `A` in km² it gives 6 m³/s, a plausible mean flow for
a wet 50 km² basin. Forge should not route width and depth through this discharge; the regional
curves give them from area directly.

---

## 5. Shallow-water simulation: what it gives, what it costs

One fact underlies this section: no open-world game found simulates its rivers; the ones that
simulate at all bake the result, and keep a runtime grid of a few hundred metres for interaction.

**Nuttapong Chentanez, Matthias Müller. "Real-time Simulation of Large Bodies of Water with Small
Scale Details." SCA 2010, 197–206.** [paper] [foundational] [PDF read]
<https://matthias-research.github.io/pages/publications/hfFluid.pdf> ·
<https://matthias-research.github.io/pages/publications/publications.html>

- A height-field shallow-water solver (semi-Lagrangian advection, explicit height and velocity
  integration with stability guards, PML non-reflecting boundaries, wet–dry tracking, *waterfall
  faces* where the terrain drops) coupled to spray, splash and foam particles, plus an FFT texture
  advected for sub-grid waves. CUDA on a GTX 480 at Δt = 16.66 ms: grids of 128² (waterfall, beach),
  256² (ocean) and 900 × 135 (a boat on "an infinitely long river that flows over an irregular
  terrain", where "the simulation grid is shifted in whole multiples of grid spacing Δx to be
  approximately centered around the boat"); the height-field step costs 0.75–2.19 ms on the GPU,
  whole frames 3.8–18 ms including rendering. Volume is conserved by the height step but not by
  coupling; "our stability enhancements cannot guarantee unconditional stability".
- *Bearing:* the moving window, the waterfall faces and the particle hand-off are the design Forge
  would build; the 2010 numbers scale to a 512²–1024² window at well under a millisecond on the
  5070 Ti.

**Nuttapong Chentanez, Matthias Müller. "Real-time Eulerian Water Simulation Using a Restricted Tall
Cell Grid." *ACM TOG* 30(4) (SIGGRAPH 2011).** [paper] (listed on the authors' page) — a 3D grid near
the surface over tall cells below; the step beyond a height field when splashes must be volumetric.

**François Dagenais, Julián Guzmán, Valentin Vervondel, Alexander Hay, Sébastien Delorme, David
Mould, Eric Paquette. "Extended Virtual Pipes for the Stable and Real-time Simulation of Small-scale
Shallow Water." *Computers & Graphics* (VRIPHYS 2018 section), 2018.** [paper]
<https://www.physicsbasedanimation.com/2018/04/15/real-time-virtual-pipes-simulation-and-modeling-for-small-scale-shallow-water/>

- The pipe model made stable with an implicit viscosity and multi-layer heightmaps, demonstrated at
  10 × 10 cm and 0.5 mm. *Bearing:* the pipe model (the one `terrain-genesis.md` §3 uses for detail
  erosion) is a fine SWE for a window; stability comes from implicit damping, not from small steps.

**Stephen Thompson. "Shallow Water Demo" (2012–2014, page updated 2025).** [code]
<https://www.solarflare.org.uk/shallow_water>

- Kurganov–Petrova 2007 in three HLSL passes per step over a heightfield; real-time "for reasonable
  grid sizes" but "realistic results demand relatively high grid resolution", with occasional
  "spiky" artefacts; the author's conclusion is that games may be better served by "linear wave
  approximations or full 3D simulations". An honest small-scale report of the method's failure
  modes.

**Imaginary Blend (Krystian Komisarek). Fluid Flux — Unreal plugin and documentation.** [tool]
<https://imaginaryblend.com/2021/09/26/fluid-flux/> ·
<https://imaginaryblend.com/2025/01/10/fluid-flux-documentation/>

- Shallow-water equations on render targets (ground, velocity/depth/foam, height/wetness), "based
  on" Müller's 2010 method; power-of-two grids; a river or lake domain up to **1024² over 1 km ×
  1 km**, a coast up to 2048² over 20 km; terrain enters by a top-down heightfield capture; a
  **baked static river** workflow (simulate in the editor, export the state to a data asset, turn
  the runtime solver off); whole-scene frame rates of 320–380 fps at 1440p on an RTX 3080 for the
  river map. Limits: "the accurate open-world setup has not been tested", World Partition
  unsupported ("simulation cannot move or rotate at runtime"), client-side desync in multiplayer,
  a default simulation step of 0.2 (sub-stepped) for speed.

**Epic Games. Unreal 5.6 Water Advanced — Shallow Water River / "Baked River Simulations".** [docs]
(see §1) — the engine-vendor version of the same workflow: simulate against the captured bottom,
bake height, velocity and foam into textures, drive the river material from them.

**LightSpeed Studios. Photon Water (GDC 2023).** (see §2) — lattice-Boltzmann SWE offline into
tiled virtual textures; runtime SWE 512² at 1 m for 0.5 ms on PS5; surface waves for interaction
under 0.1 ms.

**SideFX. Houdini heightfield Shallow Water Solver.** (see §1) — the offline tool's own warning:
fast, but unstable "at high resolutions or high propagation speeds"; no splashes.

**What a simulation would give Forge, against what it has.**

- *Level water across the channel, beds that hold water, pools behind sills:* the SWE gives these
  by construction; Forge's 2026-10-01 fix (water level across, only falling downstream) gives them
  analytically for free.
- *Flow around boulders, eddies, standing waves:* the SWE gives eddies and hydraulic jumps; the
  analytic potential flow gives the deflection but not the standing wave or the recirculation.
  Horizon's answer was a baked library; Peytavie's a primitive library.
- *The mouth and the lake entry:* the SWE mixes the river into still water with a plume and a bar;
  the analytic ribbon needs a blend over a few widths (UE's transition material).
- *Wakes and splashes:* only a simulation (or wave particles) gives them; Phase 3.
- *Rapids' white water, waterfalls:* a height field cannot overturn; both approaches need particles
  or sheets (Chentanez's waterfall particles, Emilien's sheet).

**What it costs.** From Photon Water and Chentanez: a 512² window at 1 m is about 0.5 ms per step on
a PS5-class GPU; a 1024² window at 0.5–1 m with two or three sub-steps should land at 0.3–0.8 ms on
the 5070 Ti, on the async compute queue. Memory: height, two momenta, ground, foam and wetness as
fp16 in ping-pong buffers — 1024² × 6 × 2 B × 2 ≈ 25 MB. The CFL limit `Δt ≤ Δx / (|u| + √(g·h))`
at Δx = 0.5 m, h = 2 m, u = 3 m/s is about 65 ms per step — comfortable at 60 Hz except in rapids,
where sub-steps or a wave-speed cap (Houdini's) are needed. The window re-centres in whole cells
(Chentanez), is initialised at its edge from the analytic ribbon (level, depth, speed) with a
damping band (PML; Fluid Flux's `AreaWorldBlend`), and hands its height, velocity and foam to the
same `water/surface` shader the ribbons use. Failure modes to budget for: drying and wetting at
banks, seams where the window meets the ribbons (velocity and level must match within the damping
band), steep beds (every published solver flags them), streaming the bed under the window, and
determinism — the window is visual only (D-016), gameplay reads the analytic field.

---

## 6. Forge against the numbers

- *Width and depth.* Forge: `w = 0.005·√A` (A in m²), `d = 0.4·(A/km²)^⅜`. At the island's largest
  mouth (14 m wide by the formula, so A ≈ 8 km²) nature's bankfull channel is 5.6 m wide and
  0.47 m deep; Forge's is 14 m and 0.87 m. At 1 km²: 2.7 × 0.30 m against 5 × 0.4 m. At the 0.5 km²
  threshold: 2.1 × 0.26 m against 3.5 × 0.33 m. Forge's rivers are already 1.9–2.5× wider and
  1.3–1.9× deeper than their catchments warrant, and the width exponent (0.5, the *discharge*
  exponent) grows them faster downstream than nature's 0.35–0.42. The owner's "small streams" are
  not undersized channels; an island 16 km across *has* brooks — its largest basin is about 8 km².
  What is missing is the valley, the floodplain, the bars, the sinuosity and the far-field drawing
  that make even a 6 m channel read as a river.
- *Width/depth.* Forge's ratios (12–16) are in Rosgen's B/C range; fine.
- *Banks.* `0.5x + 0.1x²` beyond the water's edge rises indefinitely: a trench. Nature: a bank one
  bankfull depth high (0.3–0.9 m here), then a *flat* floodplain several widths wide, then the
  valley wall. Rosgen C's entrenchment > 2.2 means the flood-prone width at twice the maximum depth
  is more than 2.2 channel widths.
- *Bed material.* Painted "within half a river's width of its course" from the catchment width,
  "8 m at least" (`island.md`) — at least as wide as the channel for small rivers and wider than it
  for the 3.5 m ones. Nature paints by hydraulics (gravel on riffles, sand on bars) and keeps the
  bed inside the bankfull width.
- *The mouth.* 13–17 % over the last 160 m on the largest river is a knickpoint; a graded profile at
  8 km² ends at a fraction of a percent (Whipple & Tucker; Frasson). The base level in the erosion
  (sea cells as fixed receivers at their depth) is the first thing to check.
- *Lake entries and the far field.* No delta, no fan, no distributaries (Génevaux's rule); and the
  ribbon can sit below a coarser terrain LOD because the carve lives in the 1 m refinement, not in
  the 8 m base field every LOD samples (Unreal avoids this by construction).

---

## Recommendation for Forge

In the order the owner asked for. Everything in (a) and (b) is genesis-time work on fields Forge
already has (polylines with area, slope, order; the 8 m height; the layer map); (c) touches the
ribbon builder and the terrain LOD; (d) is a decision to defer.

**(a) Terrain integration — carve a valley, not a trench.**

1. *Type each reach* from its slope `S`, catchment `A` and coast distance (Génevaux, Peytavie,
   Rosgen): `S > 4 %` → A (steps and pools, V-valley, no floodplain); `2–4 %` → B (rapids, a bench
   one to two widths wide, no floodplain); `< 2 %` → C (riffle–pool, floodplain); `< 0.5 %` within
   ~1 km of the coast or in a plain → D/DA (braids and bars); a lake or sea entry with `A` above a
   threshold → delta.
2. *Size the channel from the regional curve, not the discharge formula:* bankfull width
   `W = k·2.7·A^0.37` m and depth `D = k_d·0.3·A^0.21` m (`A` in km²; the exponents between the
   nationwide and the mountain curves), with the exaggeration factors `k`, `k_d` from (b). Keep
   `W/D ≥ 12` on B/C reaches.
3. *Cross-section per type* (World Machine's curved channel, Peytavie's templates): a parabola of
   depth `D` whose thalweg is offset toward the outer bank in bends (asymmetric where the polyline's
   curvature is high, symmetric in straights, linearly blended); a point bar on the inner bend at
   5–10 % slope up to the bank top; a cut bank — near-vertical over the bankfull depth — on the
   outer; low-flow water surface `0.3–0.5·D` below the bank top so that the bank is *visible*.
4. *Floodplain and valley* (Turowski; Williams; Rosgen): on C/D/DA reaches a flat floodplain of
   width `W_fp = max(6·W, 3 heightfield samples)` at the bank-top level plus 0.3–0.5 m, with the
   channel wandering inside it (the meander belt; Paris 2023's migration on these reaches when the
   straight D8 runs bother the owner); on B reaches a bench of `1–2·W`; on A reaches none. Beyond
   the floodplain the valley wall rises with a shape parameter (steep in the massif, broad U on the
   plain), blended into the surrounding ground with low-amplitude noise (World Machine's *Valley
   Breakup*). Carve all of this into the **8 m base field** (so every LOD carries it), and let the
   1 m refinement add only the channel's detail. Pools every 5–7 widths (deeper, slower), riffles
   between (shallower, gravel), meander wavelength 10–14 widths — the flattened-between-samples
   profile of Peytavie 2019 with the spacing from Leopold.
5. *Grade the last reach to the sea.* Fix the base level in the erosion (sea cells as fixed receivers
   at their depth, not the island's edge), then enforce along each river, downstream, `S ≤ S_max(A)`
   with `S_max = S_ref·(A/A_ref)^(−0.45)` clipped so that the trunk's last kilometre falls at no more
   than 0.5 % and its last 200 m at no more than 0.2 %; lower the bed and the floodplain to meet it
   (Tzathas 2024's analytic profile, or a single downstream sweep). Where the coast is a cliff and a
   small river meets it, keep a waterfall deliberately (Emilien 2015) — never on the trunk.
6. *The mouth as an estuary* (Génevaux's braided/delta rules; Rosgen D): over the last `~10·W` the
   channel widens 1.5–2×, the bed drops below sea level (a drowned channel), the water level is the
   sea's, two or three distributaries split around sand bars where `A` is large, a mudflat and a
   bar at the lip, the sea's FFT displacement fading in across `~2·W` while the river's flow fades
   out, foam at the plume front, the river's Jerlov type blending to the sea's.
7. *The lake entry as a fan:* the slope goes to zero over the last `5–10·W`, the channel widens and
   may split, the water level is pinned to the Fill–Spill–Merge level, a sand fan is painted on the
   lake floor, no bank step; the outlet is a sill at the lake level with a riffle below.
8. *Bed and bank materials from hydraulics, inside the bankfull width* (Houdini's layers, Unreal's
   weightmap brush): gravel/cobble where `S > 1 %` and on riffles, sand on bars and in pools, mud
   in backwaters and deltas, bedrock on A reaches; a **wet band** — darker albedo, lower roughness,
   water's F0 (the `water/wetness` recipe of `water.md` §7) — between the low-flow surface and the
   bank top; the "riverbed" layer never wider than `W`.
9. *A riparian strip as a placement rule* (Snowdrop's "near water" conditions, Tsushima's rule
   language in `terrain-genesis.md` §4): reeds and sedges within `1·W` of the water and below the
   bank top, shrubs and willow/alder from the bank top to `~2–3·W`, trees excluded from the channel
   and bars and thinned on the floodplain, scattered bank stones that themselves seed flowers. Two
   new fields drive it: distance to the channel and height above the water surface.

**(b) Scale — fewer, larger rivers, and an honest exaggeration.**

- Nature at the island's catchments gives brooks; three levers exist and they stack.
- *Raise the carved-channel threshold* from 0.5 km² to about 3–5 km²: those reaches (W ≈ 4–5 m real,
  8–10 m with `k = 2`) get the full treatment of (a); reaches between 0.5 and 3 km² become brooks —
  no 1 m carve, a wet gully in the layer map, a riparian strip, a narrow ribbon drawn only near the
  camera. The visible network drops from 43 channels to roughly a dozen, each one reading as a
  river.
- *Shape the uplift so a few trunk basins exist* (Schott 2023's curve constraints on the uplift; the
  island's uplift field in `terrain-genesis.md` §8): an asymmetric or elongated massif with a central
  lowland yields two to four basins of 20–50 km² instead of a radial fan of 8 km² ones. At 40 km² the
  nationwide curve gives W ≈ 10 m, D ≈ 0.66 m — a river by any eye, 20 m wide with `k = 2`.
- *Keep an exaggeration factor, but on the right curve:* games oversize their rivers, and Forge's
  current coefficient is already an unintended `k ≈ 2`. Make it explicit — `W = k·2.7·A^0.37`,
  `D = k_d·0.3·A^0.21` with `k = 2`, `k_d = 1.5` — so that width grows downstream at nature's rate
  and the ratio stays in range. Do *not* grow the depth coefficient further: deep clear water over a
  visible bed reads as a creek; a river's apparent size is its width, its turbidity (depth colour
  saturating at 1–2 m with a silty Jerlov type on the plains) and its valley.

**(c) Drawing — the river is part of the terrain far away.**

- *Carve into the base field* (a.4) so that no terrain LOD can rise above the water: the 8 m field
  under a river cell is at most `water level − D`, and the LOD error metric along the water mask is
  clamped so a coarser mesh never exceeds the water level (Unreal's landscape edit layers give this
  by construction; Peytavie's single elevation function likewise).
- *Two resolutions of surface* (Far Cry 5): near the camera the ribbon with the fine flow map and
  the FFT ripple cascade; beyond ~1–2 km a water layer *in the terrain material* over the water
  mask (a reflective, flat-normal, water-coloured layer with the coarse D8 flow map), so the valley
  water is visible from a low camera without geometry, and the ribbon fades in as the tile refines.
  A small depth bias on the ribbon (or lifting it by the local LOD's maximum error) covers the
  transition.
- *Transitions as material blends over an overlap* (Unreal's River-to-Lake / River-to-Ocean
  materials): the ribbon's last `~2·W` overlaps the lake plane or the sea mesh; level pinned, flow
  faded, ripple band cross-faded, foam at the plume; no end cap.
- *Vertical motion where the owner looks:* standing waves on rapids and plunge pools at cascades as
  displacement primitives (Peytavie's wave/cascade primitives; Horizon's instanced deformations)
  on the tessellated ribbon, not only as normals — this is what separates "painted water" from a
  river in the Horizon slides.

**(d) A shallow-water window — later, and baked first.**

- *Not now.* Nothing in the owner's list needs a solver: level water, carved beds, mouths, banks and
  the far field are terrain and analytic-surface work. Every shipped open world found draws its
  rivers from baked data.
- *Next: bake, don't simulate.* When the rapids and mouths want more than the primitive library,
  run a shallow-water pass **offline in genesis** per river tile over the carved bed (Chentanez's
  solver or the pipe model `terrain-genesis.md` §3 already has, at 0.5–1 m), and bake height
  offset, velocity and foam into the per-tile flow textures the ribbons already read — Uncharted 4,
  Photon Water's LBM pre-compute and Unreal 5.6's "Baked River Simulations" are this exact step.
  Cost: seconds per tile at genesis, zero at run time; stable by construction (the bake is checked
  before it ships).
- *Then, in Phase 3, a runtime window* when boats and characters must disturb the water: 512²–1024²
  cells at 0.5–1 m, 0.3–0.8 ms on the async compute queue (Photon Water: 512² at 1 m, 0.5 ms on
  PS5), ~25 MB, re-centred in whole cells around the camera, initialised at its edge from the
  baked field with a damping band, visual-only under D-016, with particles for splashes and
  waterfall faces (Chentanez 2010). Its F1 zone and `PROFILE.md` line come with it, as every pass
  does.

---

## Checked and left out

- **Red Dead Redemption 2:** no technical talk or document on its rivers was found; only press about
  its water artists. Left out.
- **Ghost of Tsushima:** the GDC 2021 "Samurai Landscapes" description covers placement rules and
  terrain rendering, not water; no water-specific talk found. Left out (the rule language is in
  `terrain-genesis.md` §4).
- **Microsoft Flight Simulator 2024:** Asobo's GDC 2022 terrain talk (MSFS 2020) is verified, but
  its description says nothing about water; no 2024 water talk found. Left out.
- **Assassin's Creed Shadows:** the GDC 2025 "Rendering Assassin's Creed Shadows" breakdown and the
  SIGGRAPH 2025 ray-tracing talk do not mention water or terrain. Left out.
- **Kingdom Come: Deliverance I/II (CryEngine):** interviews cover forests, NPCs and lighting (the
  water shader "is affected by nearby lights and probes"); nothing on river construction. Left out.
- **Dreams (Media Molecule):** no water or fluid talk found. Left out.
- **Hädrich et al.:** no river, riverbed, floodplain or waterfall paper by this author group was
  found (their terrain work is weather and vegetation). Left out.
- **Brodtkorb et al. 2012 (GPU shallow water), Kurganov & Petrova 2007:** found only as search
  records behind paywalls; not cited beyond Thompson's use of the scheme.
- **Keller & Melhorn 1978, Leopold & Wolman 1960, Williams 1986:** the originals were not fetched
  (rate limits, paywalls); their numbers are cited through the Vermont DEC appendix that quotes them.
- **Gaea's "Hydro" node, Unity's HDRP water blog, GeNa's river documentation:** pages fetched but
  with no detail beyond what §1 reports.

---

## Verification notes

- Fetched and read in full as text: Peytavie 2019 (the author PDF, 15.8 MB, fetched to the session
  scratchpad for text extraction since it exceeds the fetch tool's size cap; the extracted text was
  kept, the PDF deleted), Génevaux 2013 (Purdue mirror), Malan 2022 (slides), Chentanez & Müller
  2010, Bieger et al. 2015, the Vermont meander appendix, the NC DEQ Rosgen fact sheet, Fischer et
  al. 2022, the Snowdrop GDC 2024 slides.
- Fetched as pages: all Unreal, Unity, Houdini, Gaea, World Machine, Fluid Flux, GDC Vault, 80.lv,
  Massive, USGS, Copernicus and Procedural Worlds pages listed in Sources.
- Verified by publisher or Semantic Scholar record (title, authors, year, venue, abstract where
  given): Cordonnier 2016/2017/2023, Schott 2023, Tzathas 2024, Grenier 2023/24, Argudo 2019/2025,
  Zhang 2017, Frasson 2019, Montgomery 1995, McDonald & Cordonnier 2026, Hartley et al. 2024
  ("Flexible terrain erosion", not cited), Chentanez & Müller 2011 (authors' page), Dagenais 2018
  (physicsbasedanimation.com).
- Verified by search record only (title, authors, venue from the engine's result text; the page
  itself refused or was not fetched): Guérin et al. 2017, Feng 2023, Valencia-Rosado et al. 2022.
- Re-used from `water.md` and `terrain-genesis.md` without re-fetching: Far Cry 5 (GDC 2018),
  Uncharted 4 (SIGGRAPH 2016), Paris 2023, Emilien 2015, Cordonnier 2016, Schott 2024, Whipple &
  Tucker 1999.
- HAL (Anubis), ACM DL, ScienceDirect, ResearchGate and some USDA hosts refused automated fetches;
  Semantic Scholar rate-limited after about ten requests per minute.

---

## Sources

Engines and tools

- Epic Games, "Water Body Actors in Unreal Engine" — <https://dev.epicgames.com/documentation/en-us/unreal-engine/water-body-actors-in-unreal-engine>
- Epic Games, "Water Meshing System and Surface Rendering in Unreal Engine" — <https://dev.epicgames.com/documentation/en-us/unreal-engine/water-meshing-system-and-surface-rendering-in-unreal-engine>
- Epic Games, "Water System in Unreal Engine" — <https://dev.epicgames.com/documentation/en-us/unreal-engine/water-system-in-unreal-engine>
- Epic Games, Python API: `WaterLandscapeBrush` (5.1) — <https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/WaterLandscapeBrush?application_version=5.1>
- Epic Games, Python API: `ShallowWaterRiverComponent` (5.5) — <https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/ShallowWaterRiverComponent?application_version=5.5>
- Epic Games, "Niagara Fluids Reference in Unreal Engine" — <https://dev.epicgames.com/documentation/en-us/unreal-engine/niagara-fluids-reference-in-unreal-engine>
- Epic Developer Community, "Baked River Simulations — Parameter Reference" (5.6) — <https://dev.epicgames.com/community/learning/tutorials/72Lb/unreal-engine-baked-river-simulations-parameter-reference>; forum overview — <https://forums.unrealengine.com/t/tutorial-baked-river-simulations-overview-and-quick-start/2512102>
- Unity, HDRP 15.0 "Create a current in the Water System" — <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@15.0/manual/WaterSystem-currentmap.html>
- Unity, HDRP 17.0 "Settings and properties related to the water system" — <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/settings-and-properties-related-to-the-water-system.html>
- NatureManufacture, R.A.M 3 — River Auto Material 3 (Unity Asset Store) — <https://assetstore.unity.com/packages/tools/terrain/r-a-m-3-river-auto-material-3-287456>; 80.lv, "NatureManufacture's River Auto Material 3 For Unity Is Out Now" (2024) — <https://80.lv/articles/naturemanufacture-s-river-auto-material-3-for-unity-is-out-now>
- Procedural Worlds, GeNa Pro — <https://www.procedural-worlds.com/products/professional/gena-pro/>
- SideFX, Houdini 22.0 heightfields: "Flow fields" — <https://www.sidefx.com/docs/houdini/heightfields/flowfields.html>; "HeightField Erode" — <https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode.html>; "HeightField Erode Hydro" — <https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_erode_hydro.html>; "Shallow Water Solver: Introduction" — <https://www.sidefx.com/docs/houdini/heightfields/shallowintro.html>; "Texture layers" — <https://www.sidefx.com/docs/houdini/heightfields/texturelayers.html>
- QuadSpinner, Gaea "Rivers" node — <https://docs.quadspinner.com/Reference/Water/Rivers.html>
- World Machine, "River Device" — <https://help.world-machine.com/topic/device-river/>; "Rivers" (blog, 2015-03-17) — <https://www.world-machine.com/blog/?p=470>
- Imaginary Blend, Fluid Flux — <https://imaginaryblend.com/2021/09/26/fluid-flux/>; documentation (2025) — <https://imaginaryblend.com/2025/01/10/fluid-flux-documentation/>

Talks and studio sources

- Branislav Grujic, Cristian Cutocheras, "Water Rendering in 'Far Cry 5'", GDC 2018 — <https://gdcvault.com/play/1025555/Advanced-Graphics-Techniques-Tutorial-Water>
- Hugh Malan (Guerrilla), "Rendering Water in Horizon Forbidden West", SIGGRAPH 2022 Advances in Real-Time Rendering — <https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Water-Malan.pdf>
- Carlos Gonzalez-Ochoa (Naughty Dog), "Rendering Rapids in Uncharted 4", SIGGRAPH 2016 Advances — <https://advances.realtimerendering.com/s2016/>
- Zhenyu Mao, Kui Wu (LightSpeed Studios), "Advanced Graphics Summit: Open-World Water Rendering and Real-Time Simulation", GDC 2023 — <https://www.gdcvault.com/play/1028829/Advanced-Graphics-Summit-Open-World>; 80.lv, "Developing a Next-Gen Water Rendering Solution for Games" — <https://80.lv/articles/developing-a-next-gen-water-rendering-solution-for-games>
- Massive Entertainment, "Crafting Pandora's Breathtaking Landscape With Snowdrop" — <https://www.massive.se/blog/games-technology/snowdrop/crafting-pandoras-breathtaking-landscape-with-snowdrop/>
- Joshua Simmons (Massive), "Upgrading the Snowdrop Engine for the Massive World of 'Avatar: Frontiers of Pandora'", GDC 2024 — <https://gdcvault.com/play/1034412/Upgrading-the-Snowdrop-Engine-for> (slides <https://media.gdcvault.com/gdc2024/Slides/GDC+slide+presentations/Simmons_Joshua_Upgrading_the_Snowdrop.pdf>)

Research papers

- J.-D. Génevaux, É. Galin, É. Guérin, A. Peytavie, B. Beneš, "Terrain Generation Using Procedural Models Based on Hydrology", ACM TOG 32(4):143, 2013 — <https://dl.acm.org/doi/10.1145/2461912.2461996>
- A. Peytavie, T. Dupont, É. Guérin, Y. Cortial, B. Beneš, J. Gain, É. Galin, "Procedural Riverscapes", Computer Graphics Forum 38(7), 2019 (Pacific Graphics) — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.13814>; author PDF <https://perso.liris.cnrs.fr/eric.galin/Articles/2019-riverscapes.pdf>
- A. Paris, É. Guérin, P. Collon, É. Galin, "Authoring and Simulating Meandering Rivers", ACM TOG 42(6), 2023 — <https://dl.acm.org/doi/10.1145/3618350>
- A. Emilien, P. Poulin, M.-P. Cani, U. Vimont, "Interactive Procedural Modelling of Coherent Waterfall Scenes", CGF 34(6), 2015 — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.12515>
- G. Cordonnier, J. Braun, M.-P. Cani, B. Beneš, É. Galin, A. Peytavie, É. Guérin, "Large Scale Terrain Generation from Tectonic Uplift and Fluvial Erosion", CGF 35(2), 2016 — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.12820>
- G. Cordonnier, É. Galin, J. Gain, B. Beneš, É. Guérin, A. Peytavie, M.-P. Cani, "Authoring Landscapes by Combining Ecosystem and Terrain Erosion Simulation", ACM TOG 36(4), 2017 — <https://dl.acm.org/doi/10.1145/3072959.3073667>
- G. Cordonnier, G. Jouvet, A. Peytavie, J. Braun, M.-P. Cani, B. Beneš, É. Galin, É. Guérin, J. Gain, "Forming Terrains by Glacial Erosion", ACM TOG 42(4), 2023 — <https://dl.acm.org/doi/10.1145/3592422>
- H. Schott, A. Paris, L. Fournier, É. Guérin, É. Galin, "Large-scale Terrain Authoring through Interactive Erosion Simulation", ACM TOG 42(5):162, 2023 — <https://dl.acm.org/doi/10.1145/3592787>
- H. Schott, É. Galin, É. Guérin, A. Paris, A. Peytavie, "Terrain Amplification using Multi-scale Erosion", ACM TOG 43(4):145, 2024 — <https://dl.acm.org/doi/10.1145/3658200>
- P. Tzathas, B. Gailleton, P. Steer, G. Cordonnier, "Physically-based Analytical Erosion for Fast Terrain Generation", CGF 43(2), 2024 — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.15033>
- C. Grenier, É. Guérin, É. Galin, B. Sauvage, "Real-time Terrain Enhancement with Controlled Procedural Patterns", CGF, 2023/2024 — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.14992>
- O. Argudo, É. Galin, A. Peytavie, A. Paris, J. Gain, É. Guérin, "Orometry-based Terrain Analysis and Synthesis", ACM TOG 38(6), 2019 — <https://dl.acm.org/doi/10.1145/3355089.3356535>
- O. Argudo, É. Guérin, H. Schott, É. Galin, "Terrain Descriptors for Landscape Synthesis, Analysis and Simulation", CGF, 2025 — <https://onlinelibrary.wiley.com/doi/10.1111/cgf.70080>
- É. Guérin et al., "Interactive Example-Based Terrain Authoring with Conditional Generative Adversarial Networks", ACM TOG 36(6), 2017 — <https://dl.acm.org/doi/10.1145/3130800.3130804>
- J. Zhang, C. Wang, H. Qin, Y. Chen, Y. Gao, "Procedural Modeling of Rivers from Single Image toward Natural Scene Production", The Visual Computer, 2017 — <https://link.springer.com/article/10.1007/s00371-017-1465-7>
- R. Fischer, J. Boeckers, G. Zachmann, "Procedural Generation of Landscapes with Water Bodies Using Artificial Drainage Basins", CGI 2022 — <https://cgvr.cs.uni-bremen.de/papers/cgi22/CGI22.pdf>
- H. Feng, "Generating Realistic River Patterns with Space Colonization", WSCG 2023 — <https://www.researchgate.net/publication/372541112_Generating_Realistic_River_Patterns_with_Space_Colonization>
- L. O. Valencia-Rosado, Z. J. Guzman-Zavaleta, O. Starostenko, "A Modular Generative Approach for Realistic River Deltas: When L-Systems and cGANs Meet", IEEE Access 10, 2022 — <https://www.researchgate.net/publication/357609433_A_Modular_Generative_Approach_for_Realistic_River_Deltas_When_L-Systems_and_cGANs_Meet>
- N. McDonald, G. Cordonnier, "Stochastic Geomorphological Transport for Terrain Erosion Simulation", ACM TOG, 2026 — <https://dl.acm.org/doi/10.1145/3811336>
- N. Chentanez, M. Müller, "Real-time Simulation of Large Bodies of Water with Small Scale Details", SCA 2010 — <https://matthias-research.github.io/pages/publications/hfFluid.pdf>; Eurographics DL <https://diglib.eg.org/items/d0320015-4b07-416b-8f41-047485c9f7f3>
- N. Chentanez, M. Müller, "Real-time Eulerian Water Simulation Using a Restricted Tall Cell Grid", ACM TOG 30(4), 2011 — <https://matthias-research.github.io/pages/publications/tallCells.pdf> (listed at <https://matthias-research.github.io/pages/publications/publications.html>)
- F. Dagenais, J. Guzmán, V. Vervondel, A. Hay, S. Delorme, D. Mould, E. Paquette, "Extended Virtual Pipes for the Stable and Real-time Simulation of Small-scale Shallow Water", Computers & Graphics (VRIPHYS 2018) — <https://www.physicsbasedanimation.com/2018/04/15/real-time-virtual-pipes-simulation-and-modeling-for-small-scale-shallow-water/>; <https://www.sciencedirect.com/science/article/abs/pii/S0097849318301341>
- S. Thompson, "Shallow Water Demo" (Kurganov–Petrova on the GPU, 2012–2014) — <https://www.solarflare.org.uk/shallow_water>

Geomorphology

- L. B. Leopold, T. Maddock Jr., "The Hydraulic Geometry of Stream Channels and Some Physiographic Implications", USGS Professional Paper 252, 1953 — <https://pubs.usgs.gov/publication/pp252>
- K. Bieger, H. Rathjens, P. M. Allen, J. G. Arnold, "Development and Evaluation of Bankfull Hydraulic Geometry Relationships for the Physiographic Regions of the United States", JAWRA 51(3), 2015 — <https://swat.tamu.edu/media/114657/bieger_etal_2015.pdf>; <https://www.ars.usda.gov/research/publications/publication/?seqNo115=312862>
- D. L. Rosgen, "A Classification of Natural Rivers", Catena 22(3), 1994, 169–199 — <https://en.wikipedia.org/wiki/Rosgen_Stream_Classification> (citation); thresholds from NC DEQ, "River Course Fact Sheet 2: Application of the Rosgen Stream Classification System to North Carolina" — <https://www.deq.nc.gov/environmental-assistance-and-customer-service/rbac/grants/river-course-fact-sheet-2-application-rosgen-stream/download>
- Vermont DEC, "Stream Geomorphic Assessment — Appendix H: Meander Geometry" (quoting Leopold, Wolman & Miller 1964; Williams 1986) — <https://dec.vermont.gov/sites/dec/files/wsm/rivers/docs/assessment-protocol-appendices/H-Appendix-H-04-Meander-Geometry.pdf>
- D. R. Montgomery, J. M. Buffington, R. D. Smith, K. M. Schmidt, G. Pess, "Pool Spacing in Forest Channels", Water Resources Research 31(4), 1995 — <https://doi.org/10.1029/94WR03285>
- J. M. Turowski, A. Bufe, S. Tofelde, "A Physics-based Model for Fluvial Valley Width", Earth Surface Dynamics 12(2), 2024 — <https://esurf.copernicus.org/articles/12/493/2024/>
- R. P. M. Frasson et al., "Global Relationships Between River Width, Slope, Catchment Area, Meander Wavelength, Sinuosity, and Discharge", Geophysical Research Letters 46, 2019 — <https://doi.org/10.1029/2019GL082027>
- K. X. Whipple, G. E. Tucker, "Dynamics of the Stream-Power River Incision Model", JGR Solid Earth 104(B8), 1999 — <https://agupubs.onlinelibrary.wiley.com/doi/10.1029/1999JB900120> (verified in `terrain-genesis.md`)