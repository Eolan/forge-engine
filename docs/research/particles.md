# Particles: smoke, dust, fire, rain and sparks for the mission maps

The research behind D-062's next core step after the flora: a particle system for the effects
the first maps need (a chimney's smoke at dusk, dust kicked up on a gravel path, leaves falling,
sparks and fire for the first scenario, rain when the weather comes). It starts from what Forge
already has, the splashes, the dust's froxel volume and the weather decision, and looks at how
current engines simulate, light and draw particles, what an upscaler needs from them, where the
textures come from, and the patents met. Companion to [water.md](water.md) §7 (the splashes),
[particle-fluids.md](particle-fluids.md) (the lab's liquid, D-044), D-032 (participating media)
and D-034 (the weather's rendering). Labels follow the house convention: **[paper] [web] [docs]
[patent]** and **foundational / still-current / recent**. Sources read on a reachable page on
2026-10-16 are marked *read*; the others were seen in search results only (*search-recorded*).

> **In five sentences.** Forge already runs a GPU particle system: the splashes are emitted and
> advanced on the async compute queue from deterministic streams, drawn as soft sprites streaked
> along their velocity, and write the reactive mask TAA reads. Current engines do the same at
> larger scale (Niagara's GPU emitters with fixed bounds and indirect draws, simulation stages
> for grids), add flipbooks blended by motion vectors so a few dozen frames last, and light smoke
> with six baked directional responses ("six-way lighting", used in several AAA games under
> many names). In an RT-first engine a particle can take its sunlight from one ray per particle
> and its ambient from the GI probes, as Killzone took its particles' light at a few reference
> points each, and dense smoke can also feed the froxel volume that already lights the dust.
> Upscalers need the transparent pixels marked (a reactive or transparency mask), which the
> splashes already do for TAA. No patent was found on GPU particle simulation, sorting, six-way
> lighting or weighted blended transparency; the stable-fluids patent (Stam, US 6,266,071)
> expired in 2018, so the smoke flipbooks can be baked by our own solver.

**Contents**

1. [What Forge has](#1-what-forge-has)
2. [Simulating on the GPU](#2-simulating-on-the-gpu)
3. [Drawing: sprites, streaks, flipbooks, volumes](#3-drawing-sprites-streaks-flipbooks-volumes)
4. [Lighting particles in an RT-first renderer](#4-lighting-particles-in-an-rt-first-renderer)
5. [Upscalers and temporal anti-aliasing](#5-upscalers-and-temporal-anti-aliasing)
6. [Where the textures come from](#6-where-the-textures-come-from)
7. [The village's first effects](#7-the-villages-first-effects)
8. [Patents](#patents)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. What Forge has

- **The splashes** (`forge-render::splashes`, #107, `docs/research/water.md` §7). Ballistic drops
  with drag in a ring of slots handed out in blocks, in order, so the draw's order holds from
  frame to frame. A stream's drops are born at fixed times and take their random numbers from
  the source's seed and their index, so a stream is the same at any frame rate. `splashes/emit`
  and `splashes/advance` run on the async compute queue; `splashes/draw` draws soft sprites
  streaked along their velocity, lit and hazed, and writes the reactive mask TAA reads.
- **The dust** (`forge-render::dust`, #58, D-032). A froxel volume in front of the camera
  (160 × 90 × 64) whose extinction and in-scattered sunlight are integrated per column, with a
  shadow ray per froxel against the scene's TLAS.
- **The weather's plan** (D-034 ✅). Precipitation particles shaded from a streak array, rain as
  extinction in the froxel volume, and one depth-from-above map shared by the rain's occlusion and
  the splashes. Weather is low on the owner's list.
- **The light a particle can read.** The sun's ray queries against the TLAS (RT-first, D-062's
  point 5), the GI probe cascades (finest at 4 m, `forge-render::probes`), the sky's irradiance.

What is missing is the general part: emitters as data, more forces and collisions, more ways to
draw, smoke's lighting, sorting for alpha-blended effects, and textures.

## 2. Simulating on the GPU

- **Niagara's GPU emitters** *[docs, still-current; search-recorded]*. The simulation runs in
  compute; the CPU cannot read how big the effect is, so a GPU emitter declares fixed bounds for
  culling. A count manager keeps each emitter's live count on the GPU and generates the draws'
  indirect arguments from it (five 32-bit words a draw). **Simulation stages** run several spawn
  and update passes in sequence, which is how grids (fluids) live beside particles.
- **AMD's GPU particle sample** (GPUOpen) *[web, still-current; search-recorded]*: emission and
  simulation in compute, collisions against the depth buffer, and a GPU sort so that alpha-blended
  billboards blend in order; also a tiled compute rasteriser as an alternative to the hardware.
- **Collisions, three ways.** The depth buffer (cheap, screen-space: particles behind what the
  camera sees cannot collide); a depth map from above (D-034's: roofs and canopies stop rain and
  dust); and, RT-first, a ray query against the TLAS for the few particles that need it (sparks,
  debris), which also sees what the camera does not.
- **Forces** in common use: gravity, drag, a wind field (Forge's flora already reads a
  prevailing wind), and curl noise for turbulence (divergence-free, so smoke swirls without
  sources or sinks).

## 3. Drawing: sprites, streaks, flipbooks, volumes

- **Soft sprites** fade where they meet the depth behind them, so a sprite crossing a wall does
  not show a hard line (NVIDIA's "Soft Particles", 2007 *[web, foundational; not re-checked
  today]*). The splashes do it.
- **Streaks** stretched along the velocity, for drops, sparks and rain (the splashes; Tariq's GPU
  rain, NVIDIA 2007, expands each drop into a billboard and textures it from a library of
  streaks *[paper, foundational; search-recorded]*).
- **Flipbooks with motion vectors** *[web, still-current; search-recorded]*. A flipbook's frames
  blend along a baked flow map from one frame to the next, so a flipbook of 50–70 frames plays
  smoothly over a long life (Klemen Lozar's write-up is the reference the realtimevfx threads
  point to; Unreal blends frames this way). The vectors come best from the simulation that
  rendered the frames, or from optical flow on footage.
- **Six-way lighting for smoke** *[web, still-current; read: Unity's blog, January 2023]*. Each
  frame stores the smoke's response to light from six directions relative to the camera (right,
  top, back in one RGBA texture with the alpha; left, bottom, front in a second, its alpha free
  for an emissive mask). The shader blends the six by each light's direction and adds the
  probes' ambient; smoke is treated as fully diffuse. Unity says it is used in "multiple AAA
  games" under names such as 6-point lighting, 6D lightmaps and smoke lighting. Its limit: a
  texture sheet still looks flat, so it suits steam, background smoke and explosions, and wants
  more particles or a volume behind it for depth.
- **Into the volume.** Particles can write their density into a froxel volume that is then lit
  and integrated with the rest of the participating media (Killzone Shadow Fall fed a 3D light
  buffer from its particles, GDC 2014 *[web, still-current; search-recorded]*). Forge's dust volume
  is such a volume: chimney smoke and kicked-up dust could thicken it locally and be lit and
  shadowed by the same rays.
- **Order.** Additive and soft additive particles (sparks, embers, mist) need no order.
  Alpha-blended smoke does: per emitter, back to front (a GPU sort within an emitter, emitters
  sorted by distance on the CPU), or weighted blended order-independent transparency (McGuire and
  Bavoil, JCGT 2013 *[paper, still-current; search-recorded]*), one pass with additive blends and
  weights falling with depth, approximate but without popping.

## 4. Lighting particles in an RT-first renderer

- **Per particle, not per pixel.** A sprite's lighting changes little across it. Killzone lit its
  particles at a few reference points each, inside its deferred lighting, and found shadow maps
  for them costly *[search-recorded]*. In Forge one ray query a particle a frame against the TLAS
  gives the sun's visibility, far cheaper than a ray per pixel, and the GI probes sampled at the
  particle's centre give the ambient: the light a chimney's smoke gets from the street under it.
- **Shadows cast by particles.** Through the volume: smoke written into the froxels darkens what
  lies behind it from the sun's side when the volume is lit. Particles are not put in the TLAS.
- **Six-way smoke** takes the sun's direction against its six responses, the visibility from its
  ray, and the probes' ambient, as Unity's does with its probes.

## 5. Upscalers and temporal anti-aliasing

Transparent pixels carry no reliable history: their colour changes while the depth and the
motion vectors describe what lies behind them. Upscalers ask for them to be marked
*[docs, still-current; search-recorded]*:
- FSR's **reactive mask** lowers the history's weight where alpha-blended effects were drawn, and
  its transparency-and-composition mask protects other pixels' history (Unigine's documentation;
  AMD's FSR 2.1 notes name particles).
- DLSS takes an optional **transparency mask** of the input's size for the same ghosting (Unity's
  DLSS texture table).

Forge's TAA already reads the splashes' reactive mask (#107). A particle pass writes the same mask,
and the DLSS path passes it on as the transparency mask.

## 6. Where the textures come from

Smoke, fire and dust flipbooks, with their motion vectors and six-way responses, are baked from
simulations. The options:
1. **Our own offline solver.** A small smoke solver after Stam's "Stable Fluids" (SIGGRAPH 1999):
   its patent, US 6,266,071 B1 (Stam and Brinsmead, Silicon Graphics, then Alias), expired in
   November 2018 *[patent; read]*. Rendered by our own ray march from six light directions, with
   the velocity written out as the motion vectors: the same code bakes every texture the maps need,
   nothing to download, the textures generated as the leaf atlas is.
2. **EmberGen** (JangaFX), commercial: exports flipbooks, motion vectors and a "six point" map,
   which Unity's article calls unsupported because the light's intensity is not controlled. A
   purchase, so the owner's call.
3. **Unity's Blender add-on and VFXToolbox** bake six-way maps from a Blender smoke simulation.
   A download, so the owner's go.
4. **CC0 flipbooks** from asset sites. A download, so the owner's go.

## 7. The village's first effects

What the first map would show first, before any fight:
- **Chimney smoke** from the homes whose fire is lit at dusk (the population's routines already
  light the windows): six-way smoke drifting with the wind, thickening the dust volume a little.
- **Dust** kicked up by steps on the garrigue's paths and the gravel, a puff a step from the
  walker's foot-down events (D-012's procedural layer emits them).
- **Falling leaves and petals** from the forest's trees (the flora's species give the colours).
- **The fountains' spray**, already the splashes'.
- Later: sparks and fire for the first scenario, rain with D-034, insects with the fauna (#269).

## Patents

| What | Found | Status |
|---|---|---|
| GPU particle simulation, emission, indirect draws | none | — |
| GPU sorting of billboards | none | — |
| Six-way lighting for smoke | none; used in several AAA games under many names (Unity) | — |
| Weighted blended OIT (McGuire and Bavoil 2013) | none | — |
| Flipbooks blended by motion vectors | none on flipbooks; met US 11,941,744 B2 (Pixelworks), per-pixel motion vectors turned into block vectors for frame interpolation, where particles appear only as objects without motion vectors | active to 2042; nothing recommended here uses it |
| Stable fluids (Stam, US 6,266,071 B1, SGI then Alias) | the solver for our own bakes | expired 2018 |

The searches were general web searches plus the two Google Patents pages read; a professional
search would go further before shipping.

## Recommendation for Forge

1. **Grow the splashes into the particle system**, in `forge-render` beside them: emitters as
   data (a TOML record each, as the flora's species: spawn, life, size and colour over life,
   forces, collision, how it is drawn), simulated on the async compute queue from deterministic
   streams (each particle's birth time and random numbers from its emitter's seed and index), so
   the map check's captures stay exact.
2. **Collide** against the depth buffer, D-034's depth from above, and for sparks and debris
   against the TLAS.
3. **Draw** soft sprites, velocity streaks, flipbooks blended by motion vectors, six-way lit
   smoke; additive effects unsorted, alpha-blended ones sorted per emitter (weighted blended OIT
   as the fallback where emitters overlap heavily); smoke and dust also into the dust's froxel
   volume.
4. **Light per particle:** one sun ray a particle a frame and the GI probes at its centre.
5. **Mark the pixels** in the reactive mask TAA reads, passed to DLSS as its transparency mask.
6. **Bake the textures ourselves** with a small stable-fluids solver (expired patent) and our own
   six-direction renders and motion vectors; EmberGen or downloaded sets only on the owner's go.
7. **First effects:** chimney smoke, footstep dust, falling leaves, measured in the F1 overlay and
   PROFILE.md, each with a still-camera and a dolly shimmer check (the leaf cards' scripts).

## Checked and left out

- **Particles in the TLAS** (procedural AABBs or quads): ray-traced reflections of smoke are
  rarely seen and the cost grows with every particle; the volume carries their shadows.
- **A per-pixel ray for particles' shadows**: one a particle is enough for sprites.
- **Per-pixel linked lists or moment-based OIT** (Münstermann et al. 2018): exact or nearly, at a
  memory and bandwidth cost the village's few emitters do not need.
- **Simulating smoke at run time** (Niagara's grid stages): the flipbooks and the volume give
  the look; a live grid is for a hero effect later.

## Verification notes

- *Read on 2026-10-16:* Unity's blog "Realistic smoke lighting with 6-way lighting in VFX Graph"
  (January 2023); Google Patents US 6,266,071 B1 and US 11,941,744 B2.
- *Search-recorded:* Unreal's Niagara GPU sprite guide, the `FNiagaraGPUInstanceCountManager` and
  simulation stage API pages; AMD GPUOpen's GPU particle sample; the realtimevfx threads on
  motion-vector flipbooks; Tariq's GPU rain (NVIDIA 2007) through later papers; Killzone Shadow
  Fall's GDC 2014 talk; McGuire and Bavoil's JCGT paper; Unigine's FSR documentation, AMD's FSR
  2.1 notes and Unity's DLSS texture table.
- *Not re-checked today:* NVIDIA's "Soft Particles" whitepaper (2007), Stam's "Stable Fluids"
  paper (SIGGRAPH 1999).
