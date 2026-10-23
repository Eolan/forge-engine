# Marks by material where bodies strike static surfaces (research note)

*2026-10-24, a research agent, for Phase 7's step after the impacts' sounds by material
(`docs/demos/physics-lab.md`, "Impacts by material"). It builds on `docs/research/decals.md` and
D-074. The proposal is D-079 (🟡). Take-Two's claim 1 was read again by the session (§5.1).*

Tags: **[read]** page opened and read; **[source]** read in public source code; **[search]** only a
search summary was seen; **[local]** Forge's code or docs; **[inferred]** reasoning or an estimate
still to be measured; **[unverified]** recalled, not confirmed today.

> **In short.**
> 1. Shipped engines spawn impact marks as ordinary projected decals with a lifetime or a fade,
>    capped by a count, the oldest dropped first (Unreal, Source, Godot, s&box, HDRP) [1][2][9][16][21][22].
> 2. The decal is chosen by a table keyed on the struck surface's type (Source's game material,
>    Unreal's surface types) or on a surface × surface matrix (CryEngine's `MaterialEffects.xml`),
>    with a random pick among variants and a size range [4][14][18].
> 3. Strength gates are a minimum speed, mass or impulse (Chaos) or an energy `½mv²` (CryEngine's
>    breakage) [5][18]. Forge's `Impact::between` already computes the reduced mass μ, so
>    `E = ½ μ v²` along the normal is one line [local][26].
> 4. **Proposal:** a second, small grid of *marks* beside the static decals' grid: a fixed pool
>    (1 024–4 096 records), one level of 4 m columns with a fixed list per column, updated in place
>    on the CPU and copied up by the few bytes that changed. No hierarchy, no per-frame screen
>    binning. Oldest recycled after a fade the shader computes from a birth time.
> 5. **Patents:** none found on decal pools, recycling, fading, or material-pair selection. The
>    binning patents met (Activision's granted US 12,064,688 B2 and its pending continuation,
>    Apple's US 9,183,664 B2, AMD's US 10,803,655 B2) all require coarse-to-fine levels or image
>    tiles; a single world-space level does neither. Take-Two's US 11,534,688 B2 (trail maps for
>    soft terrain) bears on the deform layer, not on these marks.

## 1. What Forge has

- **Static decals (D-074):** `DecalSet` → `tables()` builds once an open-addressed hash of 4 m
  columns (x, z), each list sorted by (priority, index), at most 16 per column, uploaded as one
  buffer plus a `DecalGrid` record the frame points at [local].
- **The shader:** `apply_decals` hashes the pixel's column, walks its list, moves the position into
  each box, fades by depth and by facing, samples with analytic gradients (`SampleGrad`), and blends
  premultiplied colour; row decals (layered class only) swap in a row's albedo and normal. Colour
  decals change **albedo only** today: a dent or a scratch's roughness needs the "optional normal
  and roughness" D-074 planned but did not build [local].
- **`GpuDecal`** is 64 bytes with three spare words (`pad`), room for a birth time and a strength
  [local].
- **Contacts (Phase 7):** `World::contacts` gives begun contacts: the two bodies, point, normal,
  closing speed along the normal, sorted, the same every run; masses from `World::masses` (none for
  fixed bodies); each body's material row gives a `forge_audio::Surface` (Stone, Gravel, Grass,
  Wood, Metal, Water). A fixed body has one row today ("the floor's"); a city mesh would need the
  struck triangle's row (Jolt's per-triangle materials, D-007) carried in `Contact` [local].
- **Budget lessons from sound:** `drop` begins ~700 meetings a second; sound keeps the hardest per
  frame and re-arms a body after 10 ticks [local]. Marks need the same throttles.

## 2. How shipped engines place run-time marks

| Engine | How a mark is placed | Cap, recycling, fade |
|---|---|---|
| Unreal | `SpawnDecalAtLocation(material, size, location, rotation, LifeSpan)`; DBuffer/deferred decal per component [2]; Niagara has a Decal Renderer (community: since 5.2) [7] | `fade_start_delay`, `fade_duration`, `fade_in_*`, `destroy_owner_after_fade`, `fade_screen_size`, `sort_order` ("higher values draw later") [1]; the material must read `DecalLifetimeOpacity` to show the fade [3] |
| Unity HDRP | Decal Projector: size, pivot, draw distance, start fade, angle fade, fade factor, decal layers [8] | `DecalSystem` re-culls every decal each frame through a `CullingGroup`; removal swaps the last entry into the hole; sets grow by 128 up to 2 048 per material [9]; a global draw distance [10] |
| DOOM 2016 / Eternal | decals binned with lights into frustum clusters, 256 per cluster, from an atlas [11]; Eternal bins hexahedra on the GPU, coarse then fine, < 0.5 ms [12] | not documented in what was read |
| Frostbite 2 | destruction masks: spheres placed on a destructible object build a low-resolution signed distance volume (~2 m a texel), drawn as a deferred decal with detail added in the shader [13] | many hits merge into one field, not one decal per hit |
| Source | `TranslateDecalForGameMaterial` turns "Impact.Concrete" into the group named for the surface's game-material character (`TranslationData` in `decals_subrect.txt`); a weighted random pick within the group [14]; `fx_impact.cpp` applies it with no scaling by strength, particles chosen per material [15] | `r_decals` 2 048 dynamic decals; `r_decal_cullsize` 5 px [16] [search]; an overlap rule removing older decals under heavy overdraw [unverified] |
| CryEngine 3 | `CryEngineDecalInfo`: size, lifetime, `fAngle` (roll), normal, hit direction, sort priority [19] | `bSkipOverlappingTest`: "always spawn decals even if there are a lot of other decals in same place", so the default rejects crowded spots [19] |
| Call of Duty (IW) | entities, decals among them, z-binned with tile bitmasks [20] | run-time impact selection not found |
| Godot 4 | Decal node in the same clusters as lights, 512 clustered elements in view by default; "keep the projection length as short as possible" [21] | distance fade, then "culled and not sent to the shader at all"; temporary decals faded by script, then freed [21] |
| s&box, Opsive (Unity), Torque3D | — | transient decals, "the oldest one will be deleted when maxdecals is reached" [22]; an active queue, then a weathered queue whose alpha drops a step per new decal, then a final fade and back to the pool [23]; decals past their `lifeSpan` deleted [24] |

What these give Forge:
- **Pools:** every engine read caps the count; oldest-first is the common rule [16][22][23].
- **Fading:** by time (Unreal, Torque3D), by count (Opsive), by distance or screen size (Godot,
  HDRP, Unreal); Unreal also fades in (`fade_in_duration`) [1][8][21][23][24].
- **Near duplicates:** CryEngine rejects crowded spots by default [19]; Frostbite unions hits into
  one mask [13]. Strengthening the old mark is a middle way [inferred].
- **Lists:** HDRP and Godot cull every frame and bin to screen clusters; DOOM bins per frame on the
  CPU, Eternal on the GPU [9][11][12][21]. None read keeps a world-space list updated per
  insertion: that would be Forge's own extension of D-074 [inferred].
- **Box and orientation:** a size range, a roll angle and the hit normal (CryEngine) [18][19]; a
  short projection depth (Godot) [21].
- **Texture choice:** weighted variants per surface (Source), a random effect (CryEngine's
  `RandEffect`) [14][18].
- **LOD and DLAA:** Source culls decals under 5 px [16]; Unreal fades by screen size [1]. A mark
  appearing in one frame is an albedo step on a still surface; a 0.1–0.2 s fade-in hides it
  [inferred]. Applied in the resolve it keeps the surface's motion vectors, as D-074's decals do
  [local].

## 3. Material-pair tables and strength

- **Unreal:** a physical material per material or per collision body carries a surface type; 62
  are available, named in the project's `DefaultEngine.ini`; complex traces return the hit
  triangle's material [4]. Games map surface type → decal, sound and particles in their own code
  [search].
- **Source:** each `surfaceprop` carries a one-letter `gamematerial` used to pick impact effects
  and decals [17][14]. The table has one axis (the struck surface); the weapon picks the base name.
- **CryEngine:** `SurfaceTypes.xml` lists the types; `MaterialEffects.xml` is a matrix whose rows
  and columns are surface types (in the same order), each cell naming an effect such as
  `bulletimpacts:hit_mat_soil`; an effect holds sounds, particles and decals (`minscale`,
  `maxscale`, a texture and a material), with an optional `delay` and `maxdist` [18]. Its debug
  output prints `impact-speed`, `fx-threshold` and `mass` [18].
- **Thresholds:** Chaos filters collision events by a minimum mass (the lighter body), minimum
  speed (the slower) and minimum impulse [5][6]. CryEngine's breakage counts hits whose energy
  `m v²/2` exceeds `break_energy` [18]. The energy two bodies can spend on each other is
  `½ μ v_rel²`, μ the reduced mass [26]; for a body on fixed ground μ is its own mass.
  How games tune the numbers was not found; the lab can measure it (§6.4).

## 4. Decals and deformation

- Soft ground is usually a height or trail field, not decals: Batman: Arkham Origins renders feet,
  hands and thrown bodies into a height map from under the snow [25]; Take-Two's patent draws
  decals where a foot lands *into* a trail map, read by a terrain-type shader [31].
- Forge already splits this way: D-007's `deform` layer takes presses from feet, wheels and
  impacts [local]. **Rule:** a struck row with a `deform` block gets a press, not a mark; a hard
  row gets a mark; a soft row without a layer (lawn, gravel today) gets a faint dark print decal.

## 5. Patents

Status and expiry as Google Patents shows them, not a legal opinion; US and CN only were checked.
"None found" means none surfaced in about a dozen searches (decal pools, ring buffers, fading,
impact decals by material, bullet holes in English and Chinese, damage textures by force).

| Technique | Patent met | Holder, status, expiry | What the claim requires | The proposal |
|---|---|---|---|---|
| Hierarchical decal binning | US 12,064,688 B2 [27] | Activision, active, 2042-07-23 | coarse-to-fine groups of spatial units over the field of view, finer tested only against the coarser's decals | one world-space level; does not do the coarse-to-fine step |
| Same, continuation | US 2024/0408486 A1 [28] | Activision, **pending** | claim 1: "progressively dividing the portion of the game space" in a field of view into spatial units of two resolutions, the finer evaluated against "a portion of the first set" of projections, then shading | columns are not derived from a field of view and have one resolution; **do not add a coarse super-cell level** to speed up the marks' table |
| Tiled light/volume lists | US 9,183,664 B2 [29] | Apple, active, 2033-12-27 | claim 1: "dividing the image into a plurality of tiles", a depth range and volume list per tile | the grid divides the world, not the image |
| Per-tile light lists | US 10,803,655 B2 [30] | AMD, active, 2036 | lights culled per screen tile | not lights, not screen tiles |
| Trail maps for soft terrain | US 11,534,688 B2 [31] | Take-Two, active, 2039-04-02 | a world map of dynamic terrain types; marks at places *not near the camera* stored, then written into a trail map when the camera comes near; a surface shader chosen by terrain type | marks are inserted at once, wherever the camera is, with no trail map; **check the deform layer** (D-007) against this claim before it replays prints made outside its window |
| Decal sheet before the camera | CN 112884874 B [32] | Tencent, granted (CN), 2041-03-18 | (summary; claims not read) a decal sheet placed between camera and model, rendered and composited over the model | not done: marks apply in the resolve |
| Also met, unrelated | US 9,367,135 B2 [33]; US 11,666,825 B2 [34] | Nikon, active, 2032-10-04; an individual, active, 2038 | (claims not read) a handheld housing with a vibration unit that picks sounds by collision and material; mixed-reality laser tag with decals at beam hits | neither a device nor mixed reality |
| Pools, oldest recycled, fades, overlap rejection, material tables, energy thresholds | none found | — | — | prior art: Source SDK 2013, CryEngine 3, Unreal, Godot, Opsive, Torque3D [1][14][16]–[24] |

No expired patent specific to impact marks was found; the techniques above rest on published,
unpatented practice.

### 5.1 Take-Two's US 11,534,688 B2 and the deform layer (read again, 2026-10-24)

Claim 1 is the only independent claim. Its steps, paraphrased:
1. A world map ties locations to dynamic terrain types.
2. A location's type is read from it.
3. **While the location is not near the virtual camera, a record of the marks characters leave
   there is stored.**
4. **When it comes near, its trail map (a top-down map round the camera) is updated from that
   record**, guided by the terrain type.
5. A surface shader is chosen by the terrain type.
6. The location is drawn with it, showing the trail map.

The dependent claims tie deep snow and shallow mud to resolutions, projection sizes and
tessellation or parallax shaders, add a blur, and name scree, snow, quicksand and mud.

**Forge's deform layer (D-007, #197) does not do steps 3 and 4.** `Layer::move_to` moves its
window with the walker, keeps what lies where the old and new places overlap, and gives the
points it takes on their plain depth, no relief (`crates/forge-physics/src/deform.rs`). Nothing
pressed outside the window is kept, and only the walker's feet press it on the island. Forge
has the world map (`island_layer`) and draws soft rows by their own material row, so steps 1, 2,
5 and 6 are close. **The rule that keeps it clear:** never store presses made away from the window
or the camera, to write them in when it comes near. Tracks kept far away would live in a layer
of their own, written at once wherever they are made and drawn as they are, never replayed into
a near map. The impact marks proposed here have no trail map at all.

## 6. Proposal for Forge (a sketch, consistent with D-074)

### 6.1 Data
- **`MarkPool`** in `forge_render::decals`: `N` `GpuDecal` records (1 024 by default, 4 096 at
  most) in a ring; the three spare words hold the birth time, the strength and the fade-out start.
- **The marks' grid:** a second `DecalGrid`-shaped record, `f[0].marks`, with the same `column_run`
  hash, but slots that hold their list inline: (x, z, count, 8 indices) = 48 bytes, so an
  insertion changes one record and at most four slots (a mark ≤ 4 m spans ≤ 2 × 2 columns).
  Emptied slots stay as tombstones; the CPU mirror rebuilds the hash when they pass a quarter.
- **Per column at most 8 marks:** a ninth recycles that column's oldest, which also caps overdraw
  where a pile keeps falling.
- **Order:** the static column first, then the marks' column by age, so marks paint over grime.

### 6.2 From a contact to a mark (game thread, after the step, beside the sound)
1. Keep contacts where exactly one body is fixed (D-074's static scope) and the tick is not a
   re-simulated one (as the sound does).
2. `E = ½ μ v²` with μ from `World::masses`; `v` the closing speed.
3. Look up `(struck surface, striker surface)` → `MarkKind` with `E_min`, `E_full`, a size range and
   variants. Strength `s = saturate((E − E_min)/(E_full − E_min))`; nothing under `E_min`.
4. **Near duplicate:** a live mark of the same kind within half its size: raise its strength and
   birth time instead of adding.
5. **Box:** `y` along the contact normal, half depth 3–5 cm (no wrap through thin walls), size from
   the striker's radius × `lerp(0.6, 1.2, s)`, roll and variant from a hash of (tick, body ids):
   the same marks every run and in replays.
6. **Throttle:** at most 8 marks a tick; a body marks again only 10 ticks later.

### 6.3 Table and textures (first cut)
| Struck \ striker | Stone, Metal | Wood | Rubber (balls) |
|---|---|---|---|
| Stone | pale scuff | faint scuff | dark rubber smudge (high `E` only) |
| Wood | dent + dark crush (needs normals) | faint scuff | none |
| Metal (painted) | scratch: bright metal, lower roughness (needs roughness) | scuff | none |
| Grass, Gravel | dark print | dark print | faint print |
| Water | none (out of scope) | | |

Textures generated in code, as `decal_stain` and `decal_cracks` are: R8 or premultiplied RGBA,
streaked along `u`. Keep them faint and warm (the 2026-10-19 lesson: round dark stains read as
shadows).

### 6.4 Shader and frame
- `apply_decals` called a second time with `f[0].marks`; alpha × the fade from `frame time −
  birth`: a 0.15 s fade-in, a 2 s fade-out once the ring is 90 % full (Opsive's weathering, done
  in the shader so fading uploads nothing).
- A graph copy pass uploads the changed records and slots before the resolve: about
  64 + 4 × 48 ≈ 256 bytes per mark [inferred].
- Probes and mirror rays see bare surfaces, as for static decals.

## 7. For the owner to decide
1. **Scope:** D-074 says "static surfaces only for now: nothing on movers, no bullet holes", and
   its grid is "built once". Marks on static surfaces added at run time change the second part. Go
   for run-time marks on static surfaces? Bullet holes and scorches later through the same pool?
2. **Lifetime:** permanent until recycled, or a time limit as well?
3. **State:** presentation only (not saved, not in replay digests, each client its own), as
   proposed, or simulation state?
4. **Normals and roughness** for colour decals (D-074's plan), needed for dents and scratches.
5. **Contacts carry the struck triangle's row** (for city meshes with several rows per body).
6. **The deform layer and Take-Two's US 11,534,688 B2:** checked (§5.1). It stays clear while it
   never stores presses made away from its window to write in later.

## 8. Numbers to measure
- `shading/standard` and `shading/layered` at 1600 × 900 with 0, 256 and a full pool, and with one
  column at 8 marks; `FORGE_SHADER_STATS` for spills from the second loop.
- Upload bytes and CPU µs per insertion; hash rebuild time at 4 096 marks.
- `drop` and `materials` (600 fixed frames): contacts seen, marks made, merged, throttled; the
  `E` distribution, to set each `E_min` so resting and woken contacts never mark and a 2 m crate
  fall does.
- Steadiness: still camera under DLAA while marks fade in and out; `--no-marks` A/B (0 px apart on
  the other demos).
- Memory: 64 B a record (64 KB at 1 024 marks, 256 KB at 4 096) plus the slots at half load
  (8 192 × 48 B ≈ 384 KB at 4 096 marks) [inferred].

## Sources
1. [docs] Epic, `DecalComponent` (Python API). <https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/DecalComponent> [read]
2. [docs] Epic, `UGameplayStatics::SpawnDecalAtLocation` (4.26). <https://docs.unrealengine.com/4.26/en-US/API/Runtime/Engine/Kismet/UGameplayStatics/SpawnDecalAtLocation/index.html> [search]
3. [blog] "Fix Unreal decal component fade" (DecalLifetimeOpacity). <https://bugnet.io/blog/fix-unreal-decal-component-fade-screen-distance-ignored> [search]
4. [docs] Epic, "Physical Materials User Guide". <https://dev.epicgames.com/documentation/en-us/unreal-engine/physical-materials-user-guide-for-unreal-engine> [read]
5. [docs] Epic, `FSolverCollisionFilterSettings`. <https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Chaos/FSolverCollisionFilterSettings> [read]
6. [docs] Epic, `FChaosCollisionEventRequestSettings`. <https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/GeometryCollectionEngine/FChaosCollisionEventRequestSetti-> [search]
7. [docs] Epic, Niagara render module reference; forum on the 5.2 Decal Renderer. <https://dev.epicgames.com/documentation/unreal-engine/render-module-reference-for-niagara-effects-in-unreal-engine>, <https://realtimevfx.com/t/help-with-decal-renderer-component-on-niagara/20907> [search]
8. [docs] Unity HDRP 17, Decal Projector reference. <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/decal-projector-reference.html> [read]
9. [code] Unity HDRP, `DecalSystem.cs`. <https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.high-definition/Runtime/Material/Decal/DecalSystem.cs> [source]
10. [docs] Unity HDRP 15, HDRP Asset (Decals). <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@15.0/manual/HDRP-Asset.html> [read]
11. [blog] Courrèges, "DOOM (2016) – Graphics Study". <https://www.adriancourreges.com/blog/2016/09/09/doom-2016-graphics-study/> [read]
12. [talk] Geffroy, Gneiting, Wang, "Rendering the Hellscape of Doom Eternal", SIGGRAPH 2020. <https://advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf> [search]
13. [talk] Kihl, "Destruction Masking in Frostbite 2 using Volume Distance Fields", SIGGRAPH 2010. <https://www.advances.realtimerendering.com/s2010/Kihl-Destruction%20in%20Frostbite(SIGGRAPH%202010%20Advanced%20RealTime%20Rendering%20Course).pdf> [search]
14. [code] Valve, Source SDK 2013 `decals.cpp`. <https://github.com/ValveSoftware/source-sdk-2013/blob/master/src/game/shared/decals.cpp> [source]
15. [code] Valve, Source SDK 2013 `fx_impact.cpp`. <https://github.com/ValveSoftware/source-sdk-2013/blob/master/src/game/client/fx_impact.cpp> [source]
16. [wiki] Valve Developer Community, "Console Command List/R". <https://developer.valvesoftware.com/wiki/Console_Command_List/R> [search: 403 to fetch]
17. [wiki] Valve Developer Community, "Material surface properties"; Garry's Mod wiki, `MAT` enums. <https://developer.valvesoftware.com/wiki/Surfaceprop>, <https://wiki.facepunch.com/gmod/Enums/MAT> [search]
18. [docs] CRYENGINE 3 manual, "Creating New Material Effects". <https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1310761> [read]
19. [docs] CRYENGINE 3 manual, `CryEngineDecalInfo`. <https://cryengine.com/docs/static/engines/cryengine-3/categories/17399809/pages/17074592> [read]
20. [talk] Drobot, "Improved Culling for Tiled and Clustered Rendering in Call of Duty: Infinite Warfare", SIGGRAPH 2017. <https://www.activision.com/cdn/research/2017_Sig_Improved_Culling_final.pdf> [search, via decals.md]
21. [docs] Godot, "Using decals". <https://docs.godotengine.org/en/stable/tutorials/3d/using_decals.html> [read]
22. [docs] s&box, decal lifetime. <https://sbox.game/dev/doc/rendering/effects/decals/lifetime> [read]
23. [docs] Opsive, Surface System "Decal Manager". <https://opsive.com/support/documentation/ultimate-character-controller/surface-system/decal-manager/> [read]
24. [docs] Torque3D, game decals. <https://torque-3d.readthedocs.io/en/latest/script/gamedecals.html> [read]
25. [talk] Barré-Brisebois, "Deformable Snow Rendering in Batman: Arkham Origins", GDC 2014. <https://www.slideshare.net/colinbb/gdc2014-deformable-snow-rendering-in-batman-arkham-origins> [search]
26. [course] Oregon State Paradigms, two-body kinetic energy with reduced mass. <https://paradigms.physics.oregonstate.edu/act/3528.pdf> [search]
27. [patent] US 12,064,688 B2, Activision. <https://patents.google.com/patent/US12064688B2/en> [read 2026-10-19, decals.md]
28. [patent] US 2024/0408486 A1, Activision. <https://patents.google.com/patent/US20240408486A1/en> [read: claims 1 and 11]
29. [patent] US 9,183,664 B2, Apple. <https://patents.google.com/patent/US9183664B2/en> [read: claim 1]
30. [patent] US 10,803,655 B2, AMD. <https://patents.google.com/patent/US10803655B2/en> [read 2026-10-19, decals.md]
31. [patent] US 11,534,688 B2 (published as US 2019/0299097 A1), Take-Two Interactive. <https://patents.google.com/patent/US11534688B2/en> [read: claim 1, the only independent claim]
32. [patent] CN 112884874 B (CN 112884874 A), Tencent. <https://patents.google.com/patent/CN112884874A/en> [read: summary; claims not read]
33. [patent] US 9,367,135 B2, Nikon. <https://patents.google.com/patent/US9367135B2/en> [read: summary; claims not read]
34. [patent] US 11,666,825 B2, D. L. Delamont. <https://patents.google.com/patent/US11666825B2/en> [read: description; claims not read]
