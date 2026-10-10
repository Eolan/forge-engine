# Research — Procedural content: how far textures, models and structures can be generated, and what still has to be made, scanned or bought

> Companion to `civilisations-styles.md` (§3 decor, props and objects: Infinigen Indoors,
> ShapeAssembly, furniture layout; §4 materials by mathematics and by example, weathering),
> `procedural.md` (§4 settlements and Infinigen, §5 the texturing canon, signed distance fields and
> material fitting), `vegetation-materials.md` (§1 the tree pipeline, §4–§5 trims, Substance,
> Material Maker and the CC0 libraries), `city-generation.md` (§3 buildings, §4 modules through the
> cluster DAG, §5 professional engines) with D-039 (buildings as grammar-derived assemblies of kit
> modules), `assets-and-licences.md` (written the same day: licence classes 1–4, what free assets
> exist and the gaps they leave) and D-053 (generated content as data, one derived-data cache).
> Written 2026-10-10 for the owner's question of that day: how far procedural techniques can go for
> textures, models and structures, since what Forge has "is very primitive"; Blender is on the
> machine, Forge may build its own tools when they are needed, and other tools and ideas are
> welcome. The motivating example is a city sandbox: a current European city a few years ahead,
> modernised and much greener (green roofs, planted facades, parks), a few landmarks, every building
> enterable with interiors by use, and today's kind of cars, trams and boats. Its sizing is in an
> appendix kept with the sandbox, which is not published. Sources were checked with WebFetch, WebSearch and
> `gh api` only: no browser pane, nothing downloaded, installed or run (the Material Maker 1.7 build
> on the owner's disk was not opened). Each entry says whether its page was read or only seen as a
> search record ([Verification notes](#verification-notes)); patents met on the way are under
> [Patents](#patents).

The short answer: **procedure can take a small team from blockout to a consistent AA look for
everything man-made, and to AAA for the materials of man-made surfaces; it stops short where the eye
reads nature or authorship.** Forge's "primitive" is real but specific. Its terrain and water
generators are research-grade; its objects are displaced boxes, lathed profiles and bevelled
primitives in flat colours, and its twelve tileable textures are two maps (albedo and normal) of
value and Worley noise at 512². Closing the gap needs no new idea. It needs the three layers every
studio pipeline has and Forge lacks:
1. **Materials as full PBR sets with a history:** height, roughness and occlusion maps, wear and dirt
   derived from the geometry, trims and decals. This is the largest visible gain per hour of work.
   Material Maker (MIT, scriptable from the command line again since 1.5) is the free authoring
   tool, and its node shaders are the reference for a Slang evaluator.
2. **Objects as part programs:** furniture, vehicles and facade modules built from bevelled,
   profiled, lofted or SDF-meshed parts that are UV'd for trims, not one function per object.
   Infinigen (BSD-3) is the largest open corpus of such generators, written for Blender.
3. **A cook stage** that cleans, unwraps, bakes masks and hands clusters to the DAG through the
   D-053 cache.

Scans stay the base for bark, leaves, ground and stone, and the CC0 libraries suffice for them. Hand
art stays for landmarks, signage design, hero vehicles and faces. Generative 3D AI is not an engine
path: it breaks D-016's determinism, its output is a triangle soup with baked shading, and it breaks
the one-look rule. Its licences also split. TRELLIS.2 is MIT, with a non-commercial rasteriser in its
tool chain; SPAR3D is free below $1 M of revenue; Hunyuan3D's licence excludes the EU, and so France;
and the services differ on who owns free output and whether they train on it. Live patents sit on
Adobe's (ex-Allegorithmic) Substance engine and particle texturing, on Adobe's PatchMatch-style
correspondence and learned graph generation, and on an Imagination Technologies patch synthesis. The
classic alternatives (image quilting, Carnegie Mellon's shape grammar interpreter, marching cubes)
have expired. The first proofs of concept are a facade module set with green roofs and planted
facades, a furniture family, a near-current vehicle family and a park tree, each with numbers to
measure ([Recommendation for Forge](#recommendation-for-forge)).

---

## Contents

1. [What Forge generates today, and why it looks primitive](#1-what-forge-generates-today-and-why-it-looks-primitive)
2. [Textures and materials](#2-textures-and-materials)
3. [Models](#3-models)
4. [Structures](#4-structures)
5. [Generative AI for 3D and materials](#5-generative-ai-for-3d-and-materials)
6. [A quality ladder](#6-a-quality-ladder)
7. [What professional engines do](#what-professional-engines-do)
8. [Recommendation for Forge](#recommendation-for-forge)
9. [What to measure](#what-to-measure)
10. [Patents](#patents)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

---

## 1. What Forge generates today, and why it looks primitive

Read in the repository on 2026-10-10 (nothing was run):

| Area | What exists | What it lacks |
|---|---|---|
| **Terrain, water, ground** | `forge-procgen`: 19 modules, about 14 100 lines (island fields, drainage, implicit stream-power erosion, amplification, rivers, channels, valleys, lakes, beaches, the sea's spectrum, layer rules, rock sites); `forge-terrain` runs them from a world file (D-053) | nothing comparable: this is the strong part, deterministic and research-grade |
| **Surface textures** | `forge_render::textures`: twelve tileable materials (rock, concrete, brick, grass, gravel, shingle, granite, grus, limestone, karst, scree, scrub), each an albedo and a normal map at 512², periodic value noise and Worley cells, generated on the CPU at start-up (140 ms and 10.7 MiB for the city's four); triplanar projection and hex-tiling; roughness a scalar per material row; glTF models may bring their own maps (D-047) | height, roughness, occlusion and cavity maps; wear, dirt, edge damage; trims, decals, masks; materials as editable records; sizes above 512² |
| **City props** | `forge_geom::city`: twenty props (ten buildings, two towers, boulders, rubble, a lathed column, fountain and lamp post). A building is a closed box of 7–11 cm grid cells displaced into pilasters, a plinth, shop fronts, ledges, recessed bevelled windows and a cornice, with boxy units on a flat roof; 25.8 M triangles in 612 k clusters | parts (balconies, roofs, doors, stairs, railings), interiors, D-039's module grid, UVs, variation beyond proportions |
| **Stones** | `forge_geom::stone`: six shapes by geology (corestone, tor, slab, limestone block, cobble, worn stone), each a cube-sphere pushed to a superellipsoid, roughened by noise and cut by planes | the most considered object generator in the tree; no material variation over the surface |
| **The labs' models** | `assets/blender/*.py`, run headless (`blender --background --factory-startup --python …`): a 3.9 m hatchback, a 5 m boat, an aeroplane, a 16.8 m spaceship and a gull from bevelled boxes, cylinders and lofted stations, with constant Principled colours; the mannequin and the dog voxel-remeshed, decimated, unwrapped by smart UV projection and baked by Cycles into 512² maps | textures on the vehicles, panel lines, trims, interiors, detail on wheels and lights |
| **Vegetation** | none as meshes: the island's grass and scrub are painted ground layers | trees, shrubs, hedges, climbers |
| **Cooking** | the cluster-DAG cook with normal-weighted simplification and material sections (#41); the derived-data cache (D-053); glTF import with UVs (D-047) | an unwrap step, a bake step (curvature, occlusion, thickness), trim snapping |

**Why it looks primitive.** No algorithm is missing. There are three reasons:
1. **The order of work.** The engine was built renderer-first, then terrain and physics. The content
   decision (D-039) is accepted and not yet built.
2. **No shared parts.** Every object generator is one function with its own geometry code, not a
   vocabulary of parts, so each new detail costs code for each object.
3. **Surfaces without a history.** They carry two maps: no roughness variation, no edge wear, no dirt
   in the crevices, no trims, no decals. At walking distance that history is what makes a surface
   read as made of something.

The stones show the way out: shape a thing by what it is made of, then give its surface a history.

---

## 2. Textures and materials

### 2.1 Node-graph tools

**Rodolphe Suescun and contributors, Material Maker: repository, release 1.7 (14 July 2026),
`parse_args.gd`, `LICENSE.md` and README.** [repo] [read through `gh api` and the release page]
<https://github.com/RodZill4/material-maker> ·
<https://github.com/RodZill4/material-maker/releases/tag/1.7>

- **What it is.** An MIT-licensed tool, built on Godot, that makes textures procedurally and paints
  3D models. Textures and brushes are graphs of nodes saved as `.ptex` (JSON).
- **Release 1.7.** Godot moves to 4.7 stable. The release adds a dynamic sky material, 32-bit
  options on several nodes, export nodes usable inside subgraphs, and interface changes. The
  repository has about 6 000 stars and was last pushed on 7 October 2026.
- **The command line, as read in `parse_args.gd`.**
  - `--export` or `--export-material` turns export on; without these flags the application opens
    its window.
  - `-t`/`--target` names the export profile. The default is `Godot/Godot 4 Standard`; an unknown
    name falls back to the most similar profile name.
  - `-o`/`--output-dir` sets the folder. `--output-file` sets a name pattern with placeholders for
    the file name, the material's name and its author.
  - `--size` is parsed into a variable that the export call does not use: in the code read, exports
    are made at 2048 (to check before relying on it).
  - Inputs are `.ptex` files (with `*` wildcards) or `website:N` and `website:N-M` ids, which the
    tool fetches from the project's site.
- **How export runs.** The exporter walks the graph for material nodes and writes each one's maps
  through the chosen profile. It runs inside the Godot application, whose nodes are shaders, so a
  batch export still needs a GPU and a rendering context. This is an inference from the code; it
  was not tested.

*Bearing:* free, scriptable and readable. A material graph can be authored by hand or by a script,
exported in batch to a map set, and versioned as a source file. Its node shaders are the open
reference for a Slang port.

**Material Maker documentation ("Command line arguments"; "Exporting Materials"), the 1.2 devlog,
and Digital Production, "Material Maker 1.5 adds DDS, FBX, CLI, 10 new nodes" (26 January 2026).**
[docs] [web] [documentation and Digital Production read; the devlog a search record]
<https://material-maker.readthedocs.io/en/latest/command_line.html> ·
<https://rodzilla.itch.io/material-maker/devlog/477280/material-maker-12> ·
<https://digitalproduction.com/2026/01/26/material-maker-1-5-adds-dds-fbx-cli-10-new-nodes/>

- **Export targets.** The documentation lists Blender, Godot, Unity and Unreal. Its command template
  is `--export-material --target <engine> -o <path> <files>`.
- **Version 1.5.** It reinstated the command-line interface for scripted batch export without the
  GUI, added DDS image and FBX mesh loading, and keeps the MIT licence.
- **Version 1.2** (search record). Export targets got their own editor. Custom targets are stored in
  the user directory and can be saved to and loaded from files.
- **The community library** (materialmaker.org, read) holds "hundreds" of materials, nodes, brushes
  and environments. No licence for them appears on the pages read.

*Bearing:* a "Forge" export target is a small file the repository can carry. The community library
stays unused until its licence is known.

**Adobe Substance 3D Designer: Steam listing (2022 edition); CG Channel, "Adobe to raise the price
of Substance 3D subscriptions" (21 February 2025); Adobe community answers on the bundled
command-line tools.** [web] [Steam and CG Channel read; community answers search records; Adobe's
help pages returned 403]
<https://store.steampowered.com/app/1775400> ·
<https://www.cgchannel.com/2025/02/adobe-to-raise-the-price-of-substance-3d-subscriptions> ·
<https://helpx.adobe.com/substance-3d-sat/command-line-tools.html>

- **The market claim.** The Steam listing says more than 95 % of AAA projects in development use
  Substance 3D. That is Adobe's own claim, repeated every year.
- **Prices** (CG Channel). From 25 March 2025: Collection $59.99 a month or $599.88 a year,
  Texturing $24.99 a month; perpetual Steam licences of Painter and of Designer $199.99 each.
- **Command-line tools** (community answers). Designer's install includes `sbscooker` (`.sbs` to
  `.sbsar`) and `sbsrender` (outputs to bitmaps); answers disagree on `sbsbaker`. The Python API
  (`pysbs`) belongs to the Automation Toolkit, sold to teams and enterprises.

*Bearing:* the industry's tool, and available as a perpetual licence. Over Material Maker it offers
its node library (FX-Map and Tile Sampler, `civilisations-styles.md` §4) and artists' familiarity.
Optional; its patents are listed under [Patents](#patents).

**ArmorPaint: the `armory3d/armortools` repository (licence and readme) and armorpaint.org.**
[repo] [web] [read]
<https://github.com/armory3d/armortools> · <https://armorpaint.org/>

- **Licence and price.** The source is under the zlib licence; the distributed binaries are paid, to
  fund the project.
- **What it does.** PBR texture painting on desktop and mobile; the page shows version 1.0.
- **Neural nodes.** They make seamless textures from a text prompt or a photo, extract materials from
  photos and upscale up to four times. They run locally once their models are downloaded, and the
  page says those models are under open-source licences. Sculpting is in development.
- **ArmorLab.** The page does not name it; that its features now live in ArmorPaint is this note's
  reading.

*Bearing:* a free-source painter for hero props: paint over a stack of baked masks. Its neural
nodes are placeholder-grade, like §2.3's models.

**Blender: its licence page; Cycles baking as Forge already uses it (`assets/blender/
skinned_creatures.py`).** [web] [repo] [read]
<https://www.blender.org/about/license/>

- **Outputs.** What you create with Blender is your sole property, including `.blend` files.
- **Scripts.** Scripts that use Blender's API must, if published, be shared under a GPL-compatible
  licence. Forge's scripts are MIT OR Apache-2.0. MIT is GPL-compatible, and Apache-2.0 is compatible
  with GPLv3, which Blender's GPL-2.0-or-later allows. That this satisfies the rule is this note's
  reading, not legal advice.
- **Forge's bake path.** The creatures' procedural Blender materials are already baked to base
  colour, normal and packed occlusion-roughness-metal maps at 512² on smart-projected UVs.

*Bearing:* the bake path works and is free. Its costs are Cycles' bake time, and that the material
lives in a `.blend` file and a script rather than in Forge's records.

### 2.2 Wear and weathering from the geometry

**Adobe, Substance 3D Painter documentation: the Dirt generator (and the Metal Edge Wear, Curvature
and Mask Editor generators).** [docs] [Dirt read; the others search records]
<https://experienceleague.adobe.com/en/docs/substance-3d-painter/using/effects/generators/dirt>

The industry's wear masks are functions of baked maps:
- **Dirt** reads the baked curvature, ambient occlusion, world-space normal and position maps, with
  optional grunge and micro height and normal. Its controls are level, contrast, grunge amount and
  scale, seed, triplanar projection and edge masking. It writes a monochrome mask.
- **Metal Edge Wear** puts scratches where the curvature map marks edges.
- **Mask Editor** combines a texture, occlusion, curvature, world normal, gradient, thickness and
  micro detail into one mask.

*Bearing:* Forge can make every input at cook time:
- curvature from the mesh;
- occlusion and thickness by rays against the module's own BLAS (D-029's ray query);
- world normal and position for free.

The masks then drive the material's layer blends (`vegetation-materials.md` §4). The physical models
of ageing (Dorsey's flow, γ-ton exposure) are in `civilisations-styles.md` §4; this is their cheap,
per-module form.

**Wear in three tiers** (a synthesis of the above and `civilisations-styles.md` §4):
1. **Per module, at cook:** curvature, occlusion and thickness masks, stored in a mask texture on the
   module's UVs, or in vertex colours where the mesh is dense.
2. **Per instance, at load:** exposure from rays cast down from the sky over the TLAS (sheltered
   sides stay clean, weather sides streak), one value per instance or per cluster.
3. **Per pixel, at run time:** thresholds on those masks set by the style's age and the site's
   wetness, evaluated in the material resolve.

A "few years ahead, well kept" city keeps tier 3 low. A ruin is the same assets with the thresholds
raised.

### 2.3 Example-based and neural synthesis

The by-example canon is already in Forge's notes: texture bombing, histogram-preserving blending,
hex-tiling, Wang tiles, Efros–Leung, Graphcut and PatchMatch (`procedural.md` §5,
`civilisations-styles.md` §4, `vegetation-materials.md` §4). Three things are new here. First, the
patents on some of these methods ([Patents](#patents)): image quilting is expired, while
PatchMatch-style correspondence and one runtime patch-table method are live. Second, the open
datasets. Third, the state of neural material generation.

**Giuseppe Vecchio, Valentin Deschaintre, "MatSynth: A Modern PBR Materials Dataset", CVPR 2024
(arXiv:2401.06056), Hugging Face dataset card.** [paper] [web] [read]
<https://huggingface.co/datasets/gvecchio/MatSynth>

- **Contents.** 4 069 tileable 4K materials (3 980 for training, 89 for testing). The maps are base
  colour, diffuse, normal, a 16-bit height, roughness, metallic and specular, plus opacity where it
  is useful.
- **Sources and licences.** The CC0 materials come from ambientCG, cgbookcase, Poly Haven,
  ShareTextures and TextureCan; a limited CC-BY set comes from one artist.

*Bearing:* not something to ship, but a reference set: the CC0 part is a class 1 corpus to measure a
procedural material against (histograms of height, roughness and colour per family).

**Giuseppe Vecchio et al., "StableMaterials: Enhancing Diversity in Material Generation via
Semi-Supervised Learning" (arXiv:2406.09293, 13 June 2024), model card.** [paper] [web] [read]
<https://huggingface.co/gvecchio/StableMaterials>

A latent diffusion model that turns text or an image into tileable PBR maps (base colour, normal,
height, roughness, metallic) at 512². It is trained on MatSynth and Deschaintre et al.'s data (6 198
materials) plus 4 000 texture–text pairs made with SDXL. Licence: "openrail", without further text.
*Bearing:* the open state of text-to-material: 512², a generic look, and a licence to read in full
before any use.

**Ubisoft La Forge, "Generative Base Material: an open-source prototype for PBR material estimation
debuting at SIGGRAPH Asia 2025" (9 December 2025); Comfy.org on its release.** [web] [Ubisoft read;
Comfy.org a search record]
<https://www.ubisoft.com/en-us/studio/laforge/news/1i3YOvQX2iArLlScBPqBZs/generative-base-material-an-opensource-prototype-for-pbr-material-estimation-debuting-at-siggraph-asia-2025>
· <https://blog.comfy.org/p/ubisoft-open-sources-the-chord-model>

- **How base materials are made today**, per Ubisoft: photogrammetry, scans, or procedural tools such
  as Substance.
- **The prototype.** Three stages: texture synthesis from text or an image; CHORD, which turns one RGB
  texture into base colour, normal, height, roughness and metalness; then upscaling.
- **Ubisoft's stated limits.**
  - It is not production-ready. An artist quoted in the post says the quality is not yet at AAA
    level.
  - It is optimised for 1024-pixel images, and upscaling accumulates errors.
  - It struggles with strongly specular materials such as metals.
  - Its intended uses are concepts, references and placeholders.
- **Licence.** Comfy.org's record says research-only; Ubisoft's post names no licence.

*Bearing:* a AAA studio's own measure of where neural materials stand in late 2025: placeholders.

**Adobe, "Adobe announces Firefly-powered features for Substance 3D apps" (18 March 2024).** [web]
[read]
<https://blog.adobe.com/en/publish/2024/03/18/adobe-announces-firefly-powered-features-substance-3d-apps>

Sampler's Text to Texture generates texture variations from a prompt, refined by changing the
prompt; Firefly is described as designed for safe commercial use. The post does not state the
training data (Adobe's FAQ does, as a search record: licensed and public-domain content).
*Bearing:* the commercial counterpart, inside a subscription; the same placeholder grade for
engine-quality tileables, by this note's reading of Ubisoft's assessment.

### 2.4 Trims, atlases and decals

Trims are in `vegetation-materials.md` §4: Sunset Overdrive's one fixed trim layout, with UVs
snapped by a script, and the regional set of 2–4 trim sheets, 6–10 tileables and one decal atlas.
What the city adds is decals as a design language: signage, house numbers, plates, road markings,
stains, and seams between paving and plinth.

**Tiago Sousa, Jean Geffroy, "The Devil is in the Details: idTech 666", SIGGRAPH 2016 Advances in
Real-Time Rendering.** [talk] [search record]
<https://www.slideshare.net/slideshow/siggraph2016-the-devil-is-in-the-details-idtech-666/64714659>

DOOM (2016) applies decals in its clustered forward pass. Decals are binned into the same
froxel-like cells as lights; their textures come from an 8 000 × 8 000 atlas, with up to 4 000
decals in view; there is no draw call per decal.
*Bearing:* decals belong in the material resolve, as a per-cell list Forge's visibility buffer can
read. They are the cheapest way to make a generated facade look specific (numbers, signs, a plaque).

**SideFX Labs, `automatic_trim_texture` and `decal_projector`** (in the tool list read in §3.2): an
automatic trim-texture baker and a decal projector exist as free Houdini assets. They are references
for Forge's own trim step, which is: one graph per material family rendered into a fixed row layout,
with grammar-tagged faces snapped onto rows.

### 2.5 Cooked or generated at run time

**Adobe, Substance 3D plugin for Unreal Engine 5 (plugin settings; a community bug report of
12 September 2026).** [docs] [web] [search records]
<https://experienceleague.adobe.com/en/docs/substance-3d/ecosystem/game-engines/unreal-engine/unreal-engine-5/plugin-settings-ue5>

- **Runtime parameters.** The plugin lets `.sbsar` parameters change in the editor or at run time.
- **Engines.** The CPU engine is capped at 2K output and the GPU engine reaches 4K. A memory budget
  and a core limit apply, and synchronous generation blocks the game thread.
- **A user report.** Packaged UE 5.8 builds use the CPU engine even with the GPU engine selected.

*Bearing:* the one shipped runtime material-generation path in a commercial engine is caveated even
there.

**What fits Forge** (this note's judgement):
- **Cook once.** Graph, parameters and code version are the key; the result is BC7, BC5 or BC4 KTX2
  in the D-053 cache, whose mips are made properly. The owner sees shimmer at once, and a cooked mip
  chain is the defence against it.
- **Run time** keeps only what varies continuously per instance or per frame: wear thresholds,
  wetness, tint, seed-driven offsets.
- **Generating on the GPU at load,** as the planet's tiles do (D-058), is for later and for large
  variety; it must follow D-016's GPU rule, and the cache makes it rarely needed.
- **Forge's start-up generation** of 512² maps (140 ms) shows the CPU is fast enough for a small set.
  A city's set (2–4 trims and 6–10 tileables at 2048², five maps each) is a cooking job, not a
  frame-time one.

### 2.6 How close procedural gets to scans, and what the studios do

**Tom Delboo (Guerrilla), "Taking a Procedural Approach to Texturing the Machines of 'Horizon
Forbidden West'", GDC 2023.** [talk] [session page read]
<https://gdcvault.com/play/1029327/Taking-a-Procedural-Approach-to>

The machine and weapons team built a procedural texturing and shading workflow for its complex
machine assets; the session says it saved months of production. No tools are named on the page.
*Bearing:* AAA hard-surface texturing is procedural.

**Other studio evidence, already in Forge's notes or seen as records:**
- Naughty Dog textured Uncharted 4 with procedural Substance tileables and layering, and Helldivers 2
  had one material artist for every biome (`vegetation-materials.md` §4).
- EA DICE presented procedural badge creation for Battlefield 2042 with Designer in Adobe's GDC 2022
  programme (80.lv, a search record).
- Insomniac's GDC 2019 talk "Marvel's Spider-Man, meet Houdini" covers Houdini authoring of the
  ground, buildings, traffic, props and impostors (a search record).
- CD Projekt Red textured Cyberpunk 2077 through Substance Painter (a search record).

**How close, by family** (this note's assessment from the evidence above):
- **Man-made surfaces reach scan quality procedurally.** Plaster, render, concrete, brick, tile,
  metal, painted wood, glass and fabric are regular structures with stochastic wear, which is what
  graphs do well. AAA studios make them this way.
- **Natural, chaotic surfaces are where scans still win.** Bark, soil, forest floor, weathered rock
  faces and leaves are still scanned at AAA, as Ubisoft's own list says. Procedural versions hold at
  mid distance. The standard hybrid is a scanned base with procedural variation, wear and blending
  on top.
- **Forge's 512² noise materials are neither.** They lack the structure (bond patterns, joints,
  panels) and the history (masks) that make up most of the difference.

---

## 3. Models

### 3.1 Blender Geometry Nodes, run headless

**Blender release notes: 4.3 Geometry Nodes; 5.0 Geometry Nodes; CG Channel, "Blender 4.5 LTS is
out: check out its 5 key features" (Jim Thacker, 15 July 2025).** [docs] [web] [read]
<https://developer.blender.org/docs/release_notes/4.3/geometry_nodes/> ·
<https://developer.blender.org/docs/release_notes/5.0/geometry_nodes/> ·
<https://www.cgchannel.com/2025/07/blender-4-5-lts-is-out-check-out-its-5-key-features>

- **4.3** added a For Each Element zone, viewport gizmos on node groups, packed bakes, warning nodes
  and hash nodes.
- **4.5** added a Manifold boolean solver in the modifier and the Mesh Boolean node. It is as fast as
  the float solver and more robust, but works on manifold meshes only. 4.5 also brought a Set Mesh
  Normal node for custom normals, and import nodes for OBJ, PLY, STL, CSV and VDB.
- **5.0** added bundles and closures (a closure injects nodes into a group, which makes node groups
  parameterisable by behaviour). It also added volume grid sockets, with Mesh to SDF Grid, Voxelize
  Grid, SDF Grid Boolean, Mean, Median and Fillet, Advect Grid and Grid to Mesh, plus a UV Tangent
  node and a shape method for packing UV islands.

*Bearing:* Blender 5.2, on the owner's machine, is a complete procedural modeller that Forge already
drives headless. A node group authored once in the UI and saved in a `.blend`, or built from Python,
is instanced by a script with parameters and exported as glTF; the outputs are Forge's.

Its limits for an engine:
- it is a tool, not a library: the cook cannot run it on a player's machine or a server;
- its determinism across Blender versions is not documented (to measure: a digest of the exported
  mesh across two versions);
- it is slow to start (seconds per process), so batching many assets per process matters.

### 3.2 Houdini Indie and SideFX Labs

**SideFX, "Houdini Indie" product page and "Buy" page.** [web] [read]
<https://www.sidefx.com/products/houdini-indie/> · <https://www.sidefx.com/buy/>

- **Price.** Houdini Indie costs $299 for one year or $449 for two. Houdini Engine Indie is free.
- **Eligibility.** For commercial users with under $100 K of annual gross revenue and under $1 M of
  funding in the last 24 months, with at most three licences per studio.
- **Restrictions.** Indie cannot share a pipeline with commercial Houdini, and it saves in its own
  file format. Houdini Engine Indie can run Indie in batch mode, and loads digital assets into
  Unity, Unreal, Maya and 3ds Max. The EULA was not read.

*Bearing:* affordable for one artist under the cap, but every asset made with it stays in Indie's
format. Its value for Forge is SideFX Labs and the TA workflow, not a runtime.

**SideFX, SideFX Labs (GitHub: licence, README and the `otls` listing).** [repo] [read through
`gh api`]
<https://github.com/sideeffects/SideFXLabs>

- **Licence.** BSD-style (redistribution with notice, no endorsement), as a free, open-source set of
  hundreds of digital assets. Running them needs Houdini.
- **The tools relevant here,** from the asset list:
  - building generator, building from patterns, a 2D Wave Function Collapse, a road generator and
    OSM buildings;
  - an automatic trim texture, a decal projector and auto-UV, with bridges to the paid RizomUV and
    Quad Remesher;
  - tree trunk, branch, leaf and controller tools;
  - LOD creation, an impostor baker, a maps baker and a games baker.

*Bearing:* the closest free catalogue of what a game content pipeline needs, and readable as
reference designs. Forge would reimplement what it adopts, since Houdini cannot be in the cook.

### 3.3 Infinigen and the papers infinigen.org lists

**Princeton Vision & Learning Lab, infinigen.org (the project page and its paper list).** [web]
[read]
<https://infinigen.org/>

The site lists four papers:
- Infinigen (CVPR 2023);
- Infinigen Indoors (CVPR 2024);
- Infinigen Articulated (2025);
- ProcFunc (2026).

It says Infinigen is free and open source under BSD-3-Clause and makes everything from randomised
mathematical rules. The repository (`gh api`) is BSD-3-Clause, has about 7 300 stars and was pushed
on 8 October 2026; besides `src/infinigen` its tree now holds a second package, `src/infinigen2`.

**Alexander Raistrick et al., "Infinite Photorealistic Worlds using Procedural Generation", CVPR 2023
(arXiv:2306.09310).** [paper] [read through ar5iv]
<https://ar5iv.labs.arxiv.org/html/2306.09310>

- **Size.** 182 procedural asset generators with 1 070 interpretable parameters, which the authors
  call possibly an overestimate.
- **Cost.** Making a pair of 1080p images takes 3.5 hours of wall time on two Xeon Silver 4114 CPUs
  and one GPU (about an hour of it GPU rendering), with about 24 GB of memory.
- **Geometry.** Assets are meshed so each face is under a pixel from the fixed camera, and the
  average scene has 16 M polygons.
- **Trees.** The skeleton grows by a momentum-weighted random walk, refined by space colonisation;
  the skin is cylinders whose radius grows from the tips to the root. Bark is evaluated in 3D space
  to avoid seams, and five twig templates are instanced per tree.
- **Boulders.** A convex hull of about 32 points, extruded and scaled in large and small passes,
  bevelled and displaced by Voronoi textures.
- **Limits.** The authors note their natural-only data generalise less well to man-made scenes.

*Bearing:* an offline, camera-dependent generator of photoreal stills, not of game assets. Its rules
(trees, boulders, materials as Blender node graphs) are a corpus to read and port.

**Alexander Raistrick et al., "Infinigen Indoors: Photorealistic Indoor Scenes using Procedural
Generation", CVPR 2024 (arXiv:2406.11824); Infinigen documentation, "Hello Room" and "Exporting to
External File Formats".** [paper] [docs] [read]
<https://arxiv.org/html/2406.11824> ·
<https://infinigen.cs.princeton.edu/docs/latest/HelloRoom.html> ·
<https://infinigen.cs.princeton.edu/docs/latest/ExportingToExternalFileFormats.html>

- **Generators.** 79 object generators: 10 appliances; 14 windows, doors and staircases; 17
  furniture; 15 decorations; 19 small objects. Also 30 material generators with 120 parameters.
- **Floor plans.**
  1. A room adjacency graph is sampled from a probabilistic grammar, or given.
  2. A random outline is cut by a Mondrian process.
  3. Simulated annealing moves walls and swaps rooms.
  4. Floors, walls, ceilings, doors, windows and stairs are added.
- **Arrangement.**
  - Constraints are written in a Python API that builds compute graphs. The full home programme has
    105 constraints; one residential example has 1 058 nodes, 11 hard constraints and 25 score
    terms.
  - Simulated annealing with Metropolis–Hastings acceptance, cooling from 0.25 to 0.001, adds,
    deletes, re-poses and resamples objects in three passes: large, then medium, then small objects.
  - The optimised solver runs about three times faster than the original (2 280 against 6 309 in
    the paper's table, units not stated).
- **Export.** USD for whole scenes; OBJ, FBX or STL per object, with procedural materials baked to
  base colour, roughness, metal and normal maps (`-r` sets their size), plus URDF. Unreal 5 shows
  the scenes at 60 fps on an RTX 4090.
- **Run times** (documentation, CPU only, at a coarse setting with a faster solver): a dining room
  about 8 minutes, a bathroom 13, a bedroom 10, a kitchen 10, a living room 11; a floor layout with no
  objects about 34 seconds; a whole home "a long time".
- **Export limits.**
  - Material features beyond the four maps are dropped, and transparent materials may come out
    wrong.
  - Exported meshes are densely subdivided without their displacement.
  - Whole scenes are practical only in USD.
  - Whole trees export as OBJ only at very low resolution.

*Bearing:* the best open evidence that furniture, fittings and rooms can be generated from rules at
an AA-to-photoreal look offline. The generators are readable Blender Python under BSD-3, and their
part structure is what Forge should copy (§3.4). The minutes per room and the export limits say it
is a reference, not a cook step Forge can run per room at load.

**Abhishek Joshi et al., "Procedural Generation of Articulated Simulation-Ready Assets" (Infinigen
Articulated), arXiv:2505.10755 (May 2025, v3 October 2025).** [paper] [abstract read]
<https://arxiv.org/abs/2505.10755>

Generators for 18 common articulated object categories, Blender utilities for custom ones, and an
export of physical properties to robotics simulators.
*Bearing:* doors, drawers, cabinets and appliances that open are what "every building enterable"
needs next; joints exported with the mesh map onto Jolt's constraints.

**Alexander Raistrick et al., "ProcFunc: Function-Oriented Abstractions for Procedural 3D Generation
in Python", arXiv:2604.26943 (April 2026, v2 September 2026); `princeton-vl/procfunc`.** [paper]
[repo] [abstract read; repository metadata through `gh api`]
<https://arxiv.org/abs/2604.26943> · <https://github.com/princeton-vl/procfunc>

- **What it is.** A Python library over Blender for making, combining, analysing and running
  procedural generators.
- **Its finding.** Vision-language models can edit procedural material and geometry code, and write
  new generators, with significantly fewer errors.
- **Demonstration.** A new indoor room generator with compositional procedural materials.
- **Licence.** BSD-3-Clause.

*Bearing:* the idea worth the owner's attention. A language model writing **generators** (code that
is deterministic, reviewable and licence-clean) rather than **assets** (opaque meshes with unclear
rights) is how AI can speed up Forge's content without D-016 or licence problems.

### 3.4 Furniture, props and vehicles as part programs

**Infinigen's furniture generators (`src/infinigen/assets/objects/` in the repository).** [repo]
[directory listing read through `gh api`]
<https://github.com/princeton-vl/infinigen>

- **Tables** are assembled from separate modules for legs, stretchers, a lofted top and the dining
  and cocktail variants.
- **Seating** covers beds, bed frames, chairs, mattresses, pillows and sofas.
- **Shelves** cover cabinets, kitchen cabinets and kitchen spaces, bookcases, desks, drawers and
  doors.
- **Other folders** hold appliances, bathroom fittings, lamps, decorations, tableware, windows and
  clothes.

*Bearing:* the part structure ShapeAssembly formalises (`civilisations-styles.md` §3: cuboid proxies
with attach, reflect and repeat), here with real leaves (lofts, sweeps, bevels).

**Forge's form.** A part program is a small tree of placed parts, each a leaf generator:
- a bevelled box;
- a profile swept along a path;
- a lathe (already in `forge_geom::city`);
- a loft through sections;
- an SDF part meshed with fillets (§3.5).

Attachments, symmetry and repetition are the same operators, and every leaf names its material row
and its trim row. A family is one program with ranges per parameter and rules between them (seat
height against table height, leg count against top size).

**Vehicles have no research generator of standing.** The search found commercial parametric kits (a
"Procedural Vehicles" set on Unreal's marketplace and CGTrader with about 100 exposed parameters, and
a Geometry Nodes stylised car generator on Blender Artists), all search records; the 2026 arXiv work
found is part-level Gaussian vehicles for simulation, not game meshes. The part-family design that
follows is this note's:
- **The body:** a spine with 8–12 cross-sections, lofted, its proportions set by a class (city car,
  hatchback, saloon, estate, van, minibus, tram module, small boat) with ranges taken from real
  dimensions.
- **Openings:** the glasshouse inset from the body, the wheel arches cut by a Manifold boolean (§3.5).
- **Separate parts:** wheels, lights, handles, mirrors, wipers, plates.
- **Shared look:** materials (paint, glass, rubber, chrome, plastic) and a decal atlas (plates,
  badges without brands) across the family.
- **Collision:** convex parts derived from the section loft, as `car-shell` is today.

### 3.5 SDFs, booleans and meshing

The SDF canon (Hart, Frisken, Quilez, Evans's Dreams, Aaltonen's Claybook) is in `procedural.md` §5.
For modelling, three things count:
- **Blender 5.0's volume grid nodes** (§3.1) bring SDF meshing into the headless path: mesh to SDF,
  SDF boolean and fillet, grid to mesh.
- **Manifold**, Apache-2.0 (repository metadata read through `gh api`; adopted by Blender 4.5,
  §3.1), is a robust mesh boolean library usable outside Blender.
- **OpenVDB** (Apache-2.0, metadata read) is the sparse volume library under both.

Marching cubes' patent expired in 2005 ([Patents](#patents)).
*Bearing:* SDFs are for the smooth blends that make furniture, planters, car bodies and street
furniture look made rather than assembled. Booleans are for openings (windows in wall modules,
arches in bodies). The DAG cook then simplifies whatever the mesher makes.

### 3.6 Trees and rocks

Forge's tree plan is in `vegetation-materials.md` §1: a Weber–Penn species file, branches grown by
space colonisation with shadow competition, and scanned bark and leaves. What this note adds:

**The Grove (Wybren van Keulen), home page.** [web] [read]
<https://www.thegrove3d.com/>

- **Growth model.** A simulation of light and gravity, year by year. Leaves shade the tree's own
  older growth, branches sag under their weight and the tree answers by growing up. Obstacles and
  attractors, a building's shade and pruning steer it.
- **Output.** Twig instances at the branch ends; wind by shape keys or bones.
- **Packaging.** A compiled core with a Python API under the Blender and Houdini add-ons. Its licence
  and prices are in `assets-and-licences.md` §6.

*Bearing:* the reference for what "grown" adds over "generated": crowns that answer to neighbours
and buildings. A city tree is defined by pruning, crown lifting and pollarding, which are rules on
the same growth.

**Infinigen's trees** (§3.3, read): a momentum random walk, then space colonisation, cylinder skins,
bark evaluated in 3D and five twig templates per tree; too heavy to export whole except at low
resolution.

**SideFX Labs' tree tools** (§3.2): trunk, branch, leaf and controller assets.

**Mtree / modular_tree** (`gh api`, read): GPL-3.0 add-on; last release December 2021, last push
September 2024.

*Bearing:* Forge's own grower stays the plan. Infinigen's and The Grove's descriptions are second and
third designs to check it against, and Sapling (GPL tool, outputs ours) is the quickest offline
reference tree.

**Rocks:**
- Forge's stones (§1) are already geology-shaped.
- Infinigen's boulders add the extrude-and-bevel pass for blocky rock.
- Peytavie's procedural rock piles are in `vegetation-materials.md` §6.

Close-up hero rocks are the one place scans clearly win (§6).

### 3.7 Humans and clothes

Covered in `assets-and-licences.md` §2–§3. In short: MetaHuman for the sandbox; MPFB, Anny and
GarmentCode as class 1 seeds for a generator later. New and only as search records here: ChatGarment
(CVPR 2025) fine-tunes a vision-language model to write GarmentCode programs from images or text, and
Design2GarmentCode (2025) turns sketches, images or text into GarmentCode programs. That is the
"model writes the program" pattern of §3.3, applied to clothing. Their licences were not found.

### 3.8 Cleanup, remeshing, UVs, and the hand-off to the cluster DAG

**xatlas (`jpcy/xatlas`).** [repo] [read through `gh api`]
<https://github.com/jpcy/xatlas>

An MIT C++11 library with no dependencies that makes unique UVs for lightmaps or painting; a fork of
thekla_atlas, as used by The Witness. Last pushed June 2024.
*Bearing:* the unwrap for every part that is not trim-snapped. It is C++, so a dependency to agree on
(an FFI build, or a port).

**Wenzel Jakob et al., Instant Meshes (`wjakob/instant-meshes`).** [repo] [read through `gh api`]
<https://github.com/wjakob/instant-meshes>

An interactive field-aligned remesher under a BSD-3-style licence, last pushed in 2022.
*Bearing:* retopology of scans and SDF outputs into clean quads when a part needs good topology for
skinning or subdivision. The DAG cook itself does not need quads.

**Forge's own:** `meshopt` is already in the build (Cargo.lock), and the cook's normal-weighted
simplifier and material sections are proven on the city (#41).

**The pipeline from generator to DAG** (a synthesis):
1. **Generate:** a part program (Rust) or a headless Blender script emits parts with material rows,
   trim rows and semantic tags.
2. **Clean:** weld and remove degenerate faces (the lathe's fan lesson, `docs/demos/city-blocks.md`),
   and run a manifold check before booleans.
3. **UV:** snap tagged faces to their trim rows; xatlas for the rest; texel density checked against a
   target per class.
4. **Bake:** curvature, occlusion and thickness masks into the module's mask texture (§2.2).
5. **Cook:** the DAG as today, with the module's normal weight, and its sections by material.
6. **Cache:** everything keyed by generator version, parameters and inputs (D-053), so a style tweak
   re-cooks only what it touches.

---

## 4. Structures

### 4.1 Buildings with interiors: grammars, kits, solvers

D-039 settles the architecture: a CGA-style split grammar whose terminals are kit modules on a 0.5 m
grid, rooms and portals as records. `city-generation.md` §3 has the canon (Wonka 2003, Müller 2006,
Merrell's layouts, Lopes, Marson & Musse, interior mapping, SideFX's building generator). What this
note adds:

**Michael Schwarz, Pascal Müller, "Advanced Procedural Modeling of Architecture" (CGA++), SIGGRAPH
2015, DOI 10.1145/2766956.** [paper] [SIGGRAPH history record read]
<https://history.siggraph.org/?p=112748>

CGA++ makes shapes first-class: a rule can read other shapes and shape trees, act on several at once,
rewrite subtrees and spawn alternatives. "Events" give dynamic grouping and synchronisation, for
context-sensitive tasks plain CGA cannot express.
*Bearing:* what D-039's grammar needs for a perimeter block: aligned floor lines across neighbours,
a corner treated once, balconies stacked consistently. Read it before writing the interpreter, and
take queries over sibling shapes as a first-class operation.

**Epic Games, "Using Shape Grammar with PCG in Unreal Engine" (UE 5.8 documentation).** [docs]
[read]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/using-shape-grammar-with-pcg-in-unreal-engine>

Modules are symbols chained into grammar strings:
- `A,B` places A then B;
- `*` repeats as often as fits, `+` at least once;
- `[A,B]2` repeats a group;
- `{[A,P]:2,[B,P]:1}` is a weighted choice;
- `<A,B,C>` places the first that fits.

Nodes subdivide splines and segments, and duplicate cross-sections along an extrusion; only the
Primitive Cross-Section node is marked experimental.
*Bearing:* the shipped engine's grammar is one-dimensional (along splines and segments), the facade
row and fence case. It is a good syntax for D-039's `repeat` and `split` within a floor.

**Tim Tutenel, Ruben Smelik, Ricardo Lopes, Klaas Jan de Kraker, Rafael Bidarra, "Generating
Consistent Buildings: A Semantic Approach for Integrating Procedural Techniques", IEEE Transactions
on Computational Intelligence and AI in Games 3 (2011), 274–288, DOI 10.1109/TCIAIG.2011.2162842.**
[paper] [record read]
<https://publications.graphics.tudelft.nl/papers/491>

A moderator negotiates element placement through semantic classes and constraints, so that a facade
grammar, two floor-plan methods and furniture placement agree. Windows fall where rooms want them,
and doors where corridors meet.
*Bearing:* the coordination layer between D-039's facade and its rooms. In Forge the module grid
plays the moderator: a window module and a room's wall share a cell.

**Interiors at engine rates.** Infinigen Indoors' solver (§3.3) is the best open design, at minutes a
room. Forge's form (`civilisations-styles.md` §3, Germer & Schwarz) is the same constraint language
with:
- a fixed iteration count per room;
- a pinned seed;
- dressing when a building's cell opens, in a low-priority job.

A first budget to measure is 5–20 ms a room for a few dozen objects. That is plausible for a
two-stage solver (rule placement, then a few hundred annealing steps), but unmeasured.

**Yue Yang et al., "Holodeck: Language Guided Generation of 3D Embodied AI Environments", CVPR 2024
(arXiv:2312.09067); `allenai/Holodeck`.** [paper] [repo] [abstract read; repository metadata
through `gh api`]
<https://arxiv.org/abs/2312.09067> · <https://github.com/allenai/Holodeck>

- **Method.** GPT-4 turns a prompt (an arcade, a spa, a museum, a researcher's flat with a cat) into
  spatial constraints between objects; a solver lays them out with Objaverse assets.
- **Results.** Annotators preferred it to manual procedural baselines for homes.
- **Licence.** The code is Apache-2.0; the Objaverse assets carry their own licences (not checked).

*Bearing:* a language model is useful at the brief, turning "a ramen bar" into the room's object list
and rules. The assets must come from Forge's own generators and class 1 sources (the room types the
city needs are listed in `assets-and-licences.md` §5).

**Xiang Tang, Ruotong Li, Xiaopeng Fan, "ProxyBuild: Text-Guided Structured 3D Building Generation
with Mesh-Anchored Procedural Proxies", arXiv:2609.23386 (20 September 2026).** [paper] [abstract
read]
<https://arxiv.org/abs/2609.23386>

The method predicts semantic roles for the faces and edges of a building shell, has a language model
read style attributes from text, and assembles assets under placement rules with hard constraints.
No code is listed.
*Bearing:* the current research direction is hybrids where learning labels a shell and procedure
places modules. That is D-039 with a learned front end, so nothing to wait for.

**Microsoft Flight Simulator's buildings: TechCrunch, "Meet the startup that helped Microsoft build
the world of Flight Simulator" (17 August 2020).** [web] [read]
<https://techcrunch.com/2020/08/17/meet-the-startup-that-helped-microsoft-build-the-world-of-flight-simulator>

Blackshark reconstructed 1.5 billion buildings from 2D satellite images. Footprints come from
computer vision, heights from shadows and learned cues, and windows and lights are placed for night
scenes. Its founder expected perhaps 20 % of buildings to be off. Forum records (search records)
describe footprints and the height tags used, and roofs and facades generic at street level.
*Bearing:* the quality floor of footprint-plus-generic-facade generation: convincing from the air,
blockout at street level. Forge's city is seen from the street, so modules carry the look.

**Townscaper: Tommy Thompson, "How Townscaper Works: A Story Four Games in the Making", Game
Developer (6 April 2022).** [web] [read]
<https://www.gamedeveloper.com/game-platforms/how-townscaper-works-a-story-four-games-in-the-making>

An irregular grid, with Wave Function Collapse choosing the valid building pieces per cell in real
time, and pieces as corner segments placed marching-cubes style.
*Bearing:* WFC on an irregular grid is a shipped, charming indie result. D-039 keeps it for
irregular ground (a village) and for paving, not for the city's blocks.

### 4.2 Streets, bridges, quays

These are profile sweeps and span rules more than research topics. No paper of standing on
procedural bridges or quays was found (Checked and left out). The forms, from the sources already
cited (SideFX Labs' road generator, Cyberpunk 2077's spline road tool in `city-generation.md` §1,
D-039's road profiles), with the bridge typology this note's own:
- **Streets** sweep a road profile (carriageway, tram track, cycle lane, kerb, pavement, tree pits,
  drainage) along the road graph's polylines. Crossings are their own generator (corner radii,
  pedestrian islands), and paving is a Wang set of modules (`civilisations-styles.md` §4).
- **Bridges** sweep a deck along the crossing, with a structure chosen by span: a beam bridge on
  piers under about 30 m per span, an arch for 30–150 m, cables beyond. Piers are placed by rule from
  the riverbed's profile, and parapets, lamps and stairs down to the quay are modules.
- **Quays** extrude a wall section along the water's offset of the bank, with stairs, ramps,
  bollards, ladders and mooring rings as modules at rule spacing. Planted terraces belong to the
  green rules of §4.3.

### 4.3 Parks, green roofs and planted facades

**Michael Vasiljevs, "Procedural Modelling of Park Layouts", master's thesis, TU Wien (2018;
supervisors Martin Ilčík, Michael Wimmer).** [thesis] [record read]
<https://www.cg.tuwien.ac.at/research/publications/2018/VASILJEVS-2018-PMPL/>

- **Rules from real parks.** Recurring layout patterns from real parks and park models become a shape
  grammar: a block is split into paths, lawns and beds, and vegetation is placed by Poisson sampling.
- **New operators.** A rule for curved regions, boundary-subset indexing and an insetting operation;
  CityEngine is the intended host.

*Bearing:* parks as a grammar beside the building grammar, sharing its scopes and lot records.

**J. H. Liu, S. K. Zhang, Q. Liu et al., "Automatic Planning of Urban Green Spaces", Computational Visual Media 12(3)
(2026), 701–720, DOI 10.26599/CVM.2025.9450466.** [paper] [record read]
<https://researchportal.bath.ac.uk/en/publications/automatic-planning-of-urban-green-spaces/>

Three stages: graph-based roads that divide the site into regions; a genetic algorithm choosing
surfaces and vegetation types per region; then models placed by patterns, with decoration.
*Bearing:* a recent, complete pipeline for parks. Its genetic search is not deterministic in time,
but with a pinned seed and a fixed generation count it is in result. A WFC park design (a 2024
University of Twente thesis, a search record) is the lighter alternative.

**Wikipedia, "Green roof" and "Green wall"; "Bosco Verticale".** [web] [read]
<https://en.wikipedia.org/wiki/Green_roof> · <https://en.wikipedia.org/wiki/Green_wall> ·
<https://en.wikipedia.org/wiki/Bosco_Verticale>

- **Green roofs.**
  - *Extensive:* 2–12.7 cm of substrate, sedums and mosses, 50–120 kg/m², nearly self-sustaining.
  - *Intensive:* at least 12.8 cm, from herbs to shrubs and small trees, 390–730 kg/m², irrigated and
    maintained.
  - *Mandates:* Toronto since 2009 (residential buildings of six storeys or more, industrial ones a
    share of their roofs) and Denver since 2018; Germany's cities since the early 2000s.
- **Green walls.** A green facade keeps its growing medium at the base and climbers cover the wall;
  a living wall carries the medium on the face, in engineered layers (support, waterproofing and
  drainage, medium, plants, irrigation). Patrick Blanc's walls in Paris (the Cité des Sciences in
  1986, the Musée du quai Branly) are the reference.
- **Bosco Verticale** (Milan, 2014). 730–800 trees, 5 000 shrubs and about 11 000 perennials and
  ground covers on cantilevered concrete balconies 28 cm thick and about 3.3 m deep. Tree basins are
  about 1.10 m across, shrub containers at least 0.5 m, with drip irrigation monitored remotely.

*Bearing:* the green layer is rules over modules, with real numbers behind them:
- a roof's type follows the building's structure class (extensive on light roofs, intensive on
  podiums and courtyards);
- a facade's planting follows its orientation and its balcony depth (climbers on trellis modules,
  living walls as panels, trees only where a balcony is 3 m deep or more);
- species follow climate and exposure.

---

## 5. Generative AI for 3D and materials

**Microsoft, TRELLIS.2 ("Native and Compact Structured Latents for 3D Generation",
arXiv:2512.14692; repository README; Hugging Face model card `microsoft/TRELLIS.2-4B`) and TRELLIS
(CVPR 2025).** [repo] [paper] [web] [README and model card read; TRELLIS metadata through `gh api`]
<https://github.com/microsoft/TRELLIS.2> · <https://huggingface.co/microsoft/TRELLIS.2-4B> ·
<https://github.com/microsoft/TRELLIS>

- **The model.** 4 B parameters, image to 3D, with base colour, roughness, metallic and opacity.
- **Speed** (on an H100). About 3 s at 512³, 17 s at 1024³ and 60 s at 1536³.
- **Requirements.** An NVIDIA GPU with at least 24 GB, tested on Linux only.
- **Export.** The README's GLB export decimates to a target such as 1 M faces, bakes 4 096 textures
  and can remesh.
- **Model card.** Meshes may have small holes; the model is not aligned with aesthetic preferences,
  and outputs reflect the training distribution. The card names no training data; the README's
  training example uses an Objaverse-XL Sketchfab subset.
- **Licences.** Code and weights are MIT; TRELLIS (v1) is MIT too.
- **The tool chain.** The setup script installs `nvdiffrast`, whose licence
  (`NVlabs/nvdiffrast`, read through `gh api`) limits use to non-commercial research or evaluation
  except by NVIDIA. The README's example renders with it; whether the GLB export path needs it was not
  checked.

*Bearing:* the open state of the art is MIT on paper. Check the tool chain before any commercial use,
and expect triangle soups at a million faces with baked maps.

**Tencent, "Tencent Hunyuan 3D 2.1 Community License Agreement" (release date 13 June 2025),
`Tencent-Hunyuan/Hunyuan3D-2.1`.** [repo] [licence read through `gh api`]
<https://github.com/Tencent-Hunyuan/Hunyuan3D-2.1>

- **Territory.** The licence states that it does not apply in the European Union, the United Kingdom
  or South Korea. The Territory is the world minus those, and any use of the works or their outputs
  outside it is unlicensed.
- **Other terms.** Above 1 M monthly active users a separate licence is needed. Outputs may not be
  used to improve other AI models. Tencent claims no rights in outputs.
- **Version 3.0.** Announced on 16 September 2025 as a service with an open release planned; no open
  3.0 weights were found (search records).

*Bearing:* **not usable from France**, whatever its quality. The owner works in the EU.

**Stability AI, Community License Agreement (last updated 5 July 2024); `Stability-AI/
stable-point-aware-3d` (SPAR3D) and Stable Fast 3D.** [web] [repo] [licence and README read]
<https://stability.ai/community-license-agreement> ·
<https://github.com/Stability-AI/stable-point-aware-3d>

- **Licence.** Free for research and commercial use until the licensee and affiliates pass $1 M of
  annual revenue; the licensee owns the outputs; worldwide, subject to trade controls; outputs may
  not be used to create or improve a foundational generative model.
- **SPAR3D.** Built on Stable Fast 3D; single image to mesh with a point-cloud stage for the back,
  plus UV unwrapping, texture baking and optional remeshing modules. The weights are gated on Hugging
  Face.

*Bearing:* the cleanest licence for an EU user below the revenue cap; a fast, mid-quality object
reconstructor.

**MeshAnything V2: "Artist-Created Mesh Generation With Adjacent Mesh Tokenization" (ICCV 2025),
`buaacyw/MeshAnythingV2`.** [repo] [licence read
through `gh api`]
<https://github.com/buaacyw/MeshAnythingV2>

"Artist-like" mesh generation with adjacent mesh tokenisation, under the S-Lab License 1.0, which
allows use for non-commercial purposes only.
*Bearing:* the artist-topology line exists, but in class 4 (not for a release).

**Meshy, Terms of Service (last updated 19 September 2026).** [web] [read]
<https://www.meshy.ai/terms-of-use>

- **Free plans.** Meshy owns the output and licenses it under CC BY 4.0 with credit to Meshy.
- **Paid plans.** The customer owns the output, as far as law allows.
- **Community posts.** Outputs posted to the community page are CC0.
- **Training.** Meshy may use non-Enterprise inputs and outputs to train and improve its services
  unless an order says otherwise.

*Bearing:* free outputs are class 1 with credit (CC BY 4.0) and paid ones are owned. Anything
uploaded, such as a reference photo, may train their models.

**Tripo (Tripo3D), terms and help pages.** [web] [search records; the terms and help pages returned
403]
<https://www.tripo3d.ai/help/privacy-policy/how-to-use-tripo-models-commercially>

According to the records, free-plan generations are public under CC BY 4.0, while Tripo's help page
says Tripo keeps the rights to free outputs, grants no commercial use and may train on them; paid
users hold the rights to inputs and outputs and are not used for training.
*Bearing:* contradictory records; read the terms in force before any use.

**Deemos (Hyper3D Rodin and ChatAvatar), Terms.** [web] [read]
<https://hyper3d.ai/legal/terms>

- **Rodin outputs.** Deemos will not limit their use, subject to the agreements and the law. No
  clause says the user owns them, and Deemos makes no representation on their copyrightability.
- **ChatAvatar.** Free outputs are licensed for personal use only.
- **Training and use.** Deemos may use prompts and outputs to provide and improve its services and
  for research. Users may not use the service to build competing models.
- **Law and date.** Delaware law; no revision date on the page.

*Bearing:* usable but with no ownership statement; record the terms' date at each use.

**What the output is worth to an engine** (this note's assessment from the sources above):
- **Topology.** Remeshed triangle soups at up to a million faces. Forge's cook would simplify them,
  but parts are not separated: wheels, doors and drawers are one surface with the body. Holes
  happen.
- **UVs.** Generated atlases of many small islands; no trim, no tiling texel density, no shared
  materials. Each asset is its own material set, which breaks "one look per scene" and the texture
  budget.
- **PBR.** Lighting and the model's style are baked into the base colour. Metalness and roughness
  are estimated, and Ubisoft found metals the hardest.
- **Determinism.** A network's output is not a pure function of a seed across GPUs and drivers, so
  under D-016 any use is offline: generate once, review, store the result as a source asset under
  its licence class, never regenerate in the cook.
- **Verdict.** For blockout and concept (a stand-in prop during layout, a reference shape for a
  procedural family), TRELLIS.2 or SPAR3D offline are acceptable, with licences recorded. For
  anything the owner will see up close, no. The money and time are better spent on generators,
  whose output can be regenerated, varied and kept in one style.

---

## 6. A quality ladder

What each technique reaches, from blockout to AAA, in a real-time game at the distance it is seen;
the levels are this note's assessment from the cited examples.

| Technique | Reaches | Shipped or published example | Where hand art or scans stay |
|---|---|---|---|
| Node-graph materials, man-made surfaces | **AAA** | Uncharted 4 (Substance tileables), Horizon Forbidden West's machines, Helldivers 2 (one material artist) | references and hero surfaces |
| Node-graph materials, natural surfaces | **AA** at mid distance | the hybrids studios use; Ubisoft names scans first | bark, leaves, soil and rock faces up close: scans (CC0 suffices) |
| Wear masks from curvature, occlusion, position | **AAA** (the standard) | Substance Painter's generators | a few hand-painted hero stories (a scorch, a repair) |
| Neural text- or photo-to-material | **placeholder** | Ubisoft's CHORD (not production-ready, by Ubisoft), StableMaterials at 512² | everything final |
| Buildings: grammar over hand-made kits | **AAA** | The City Sample (Houdini plus kits), The Division, Watch Dogs: Legion (250+ module industrial kits) | the kit modules, landmarks |
| Buildings: grammar with generated modules | **AA** offline, **indie–AA** in a game | Infinigen Indoors' windows, doors and stairs; Townscaper (stylised, WFC) | ornament, landmarks, entrances that tell a story |
| Buildings from footprints and generic facades | **blockout** at street level, fine from the air | Microsoft Flight Simulator (Blackshark; about 20 % off, its founder said) | everything at street level |
| Interiors by a constraint solver | **AA** offline, **indie** at game rates today | Infinigen Indoors (minutes a room, photoreal renders), Holodeck | hero rooms; signature shops and bars |
| Furniture and props by part programs | **AA** with good leaves and materials | Infinigen's 79 indoor generators | hero props (Poly Haven scans) |
| Vehicles by part families | **indie–AA** | commercial parametric kits (search records); no research generator | hero vehicles, the design language itself |
| Trees by growth or colonisation | **AAA** with scanned bark and leaves | SpeedTree and The Grove (growth-based tools used in production, `vegetation-materials.md` §1); Infinigen offline | leaf and bark scans; a few hero trees tuned by hand |
| Rocks by noise and geology | **AA** | Forge's stones, Infinigen's boulders | close-up hero rocks: scans |
| People by parametric bodies | **AAA** (over scans) / **indie** (open) | MetaHuman / MPFB (`assets-and-licences.md` §2) | faces, skin, hair: scans and artists |
| Clothes by sewing programs | **AA** cut, needs simulation and textures | GarmentCode | hero outfits (Marvelous Designer) |
| Image-to-3D AI | **blockout–indie** props | TRELLIS.2, SPAR3D | anything seen up close |

**Where Forge stands on it today:** blockout for buildings, props and vehicles; indie for stones and
terrain materials; AA–AAA for terrain shape and water (not part of this question).

---

## What professional engines do

| Engine or pipeline | Materials | Buildings and structures | Props and vegetation | Generative AI |
|---|---|---|---|---|
| **Unreal Engine 5.8** | material layers; the Substance plugin (`.sbsar`, runtime parameters, CPU engine to 2K) | PCG with Shape Grammar nodes along splines; the City Sample generated in Houdini over kits | PCG placement; Nanite foliage (`vegetation-materials.md` §1) | none built in |
| **Unity 6** | the Substance plugin; Unity Labs' stochastic texturing (Heitz–Neyret) | no built-in generator (`city-generation.md` §5) | SpeedTree import | none built in |
| **Insomniac** (Spider-Man) | Substance (industry practice) | Houdini procedural systems for the ground, buildings, traffic, props, impostors (talk records) | Houdini | — |
| **Guerrilla** (Decima) | procedural texturing and shading of the machines (GDC 2023) | hand-built, with GPU placement rules | GPU placement at run time (`procedural.md` §4) | — |
| **Ubisoft** (Anvil, Snowdrop) | photogrammetry, scans and Substance; CHORD as a placeholder prototype | footprint plus kit per building (The Division), module kits (Watch Dogs: Legion) | — | a research prototype, not production |
| **Microsoft Flight Simulator** | — | machine-learned footprints and heights, procedural buildings | procedural vegetation | learned reconstruction at planet scale |
| **Infinigen** (research, Blender) | 30+ procedural material generators baked to four maps | floor plans by grammar plus annealing; 14 window, door and stair generators | 182 nature and 79 indoor generators | VLMs writing generators (ProcFunc) |
| **Forge today** | twelve noise tileables, two maps, 512² | displaced boxes; D-039 decided, not built | stones by geology; no trees | none |

---

## Recommendation for Forge

**Verdict.** Build the three missing layers (materials with history, part programs, the cook stage)
before any new generator of whole objects. With them, the existing city props and lab models can be
remade at an AA look, and D-039's modules become cheap to author. Keep scans for natural surfaces,
leaves and bark (CC0 libraries); keep hand work for landmarks, signage design and hero vehicles; keep
generative AI out of the engine and out of anything seen up close.

**Build, in the engine** (names to be argued in the code, as the sibling notes say):
1. **Material graphs as records** (`forge-material`, or a module of `forge-render` for the cook).
   - *Format.* A graph in TOML (D-053's format): nodes from a vocabulary of about 30 (gradient, value,
     Perlin and Worley noise; domain warp; recursive stamp, as Designer's FX-Map; a map-driven tile
     sampler; bevel and shape; blend modes; levels and histogram ops; height to normal; curvature
     and occlusion from height; gradient map; phasor stripes from `civilisations-styles.md` §4).
   - *Evaluation.* A Slang compute evaluator at cook time with a CPU reference for tests.
   - *Output.* Full map sets (base colour, normal, height, roughness, metal, occlusion), BC-compressed
     KTX2 in `cache/`.
   - *Variants.* Seeds and named parameters.
   - *First step.* Before the evaluator exists, the same graphs authored in Material Maker and
     exported by its command line, so the look improves now and the graphs port later.
2. **Bake and UV tools in `forge-geom`.** Curvature, occlusion and thickness masks by rays (the
   module's BLAS, D-029); trim snapping for tagged faces; xatlas for the rest (a C++ dependency:
   owner's agreement first); a texel-density check.
3. **A part-program interpreter** (`forge-parts`). ShapeAssembly-style placement (attach, reflect,
   repeat) over leaf generators: bevelled box, sweep, lathe (move it from `city.rs`), loft, and an SDF
   part meshed with fillets. Each leaf carries material and trim rows and tags. Families are
   programs with parameter ranges and rules. Furniture, fittings, vehicles and facade modules all go
   through it.
4. **D-039's grammar and modules** (`forge-building`). CGA's operators with CGA++'s sibling queries;
   the shape grammar string syntax for rows; rooms and portals as records. Interiors dressed by a
   constraint program (Infinigen Indoors' terms: against wall, on surface, facing, clearance, count
   ranges) with a fixed iteration budget and a pinned seed. Parks as a sibling grammar (Vasiljevs);
   green roofs and facades as module families with rules (§4.3).
5. **The tree grower** from `vegetation-materials.md` §1, with The Grove's light and gravity response
   and urban pruning rules; Infinigen's tree description as the second design to check against.

**Use offline** (tools, not dependencies; outputs Forge's):
- **Blender 5.2, headless,** for parts faster to make visually than in code: Geometry Nodes groups
  saved in `.blend` sources, instanced and exported by scripts as today. Scripts stay MIT OR
  Apache-2.0 (GPL-compatible).
- **Material Maker 1.7 (MIT)** for authoring and previewing material graphs. Batch export through
  `--export-material` with a Forge export target. Do not use the `website:` inputs (a download, and
  the library's licence is unknown).
- **Infinigen (BSD-3)** as a corpus to read and port: tables, seating, shelves, windows, doors,
  trees. Installing and running it is a download and needs the owner's go. Its value is in the code,
  which `gh api` can read without installing anything.
- **Instant Meshes** (BSD-3) for retopology of a scan or an SDF part when quads matter.

**Other tools and ideas worth the owner's attention:**
1. **Language models write generators, not assets** (ProcFunc's finding). A session can turn a
   reference photo or a brief into a Rust part program or a Geometry Nodes script, which the owner
   reviews like any code. It keeps determinism, licences and one style, and is the fastest way to
   grow the vocabulary.
2. **Fit generators to photos.** MATch and node-graph optimisation (`procedural.md` §5) fit a
   material graph to a CC0 photo, and Šťava's inverse tree modelling (`vegetation-materials.md` §1)
   fits a tree. Both are offline tools whose output is parameters.
3. **MatSynth's CC0 part as a yardstick** for the material generator: compare map statistics per
   family.
4. **Houdini Indie** ($299 a year, under $100 K of revenue) only if a technical artist joins; SideFX
   Labs is the free reference catalogue either way.
5. **Substance Designer, perpetual** ($199.99 on Steam at the 2025 price) only if Material Maker's
   nodes fall short for trims.
6. **ArmorPaint** (zlib source) for hand-painting the few hero props.

**First proofs of concept,** each with free inputs and a visible, measured result. Sizes for one city
belong in that city's own appendix.
1. **A facade module set for a modernised European perimeter block with green roofs and planted
   facades.**
   - *Modules.* About 24 on D-039's grid: wall bays, three window types, a French window with
     railing, a balcony with a planter, a loggia, a shopfront bay, an entrance, a corner, a plinth, a
     parapet, an extensive green-roof tile, an intensive roof-garden tile, a solar-panel module, a
     climber trellis module and a living-wall panel.
   - *Materials.* Three trims, six tileables and a decal atlas from Material Maker graphs. Planting
     uses CC0 leaf scans.
   - *The result.* One block of six to eight buildings around a courtyard, from a grammar.
   - *Measure:* modules, triangles and clusters per module; texture MiB; cold and warm cook times;
     distinct facades across 20 seeds; the block's GPU time at 1440p; a street shot and a courtyard
     shot against today's `city-blocks` buildings.
   - *Inputs:* Poly Haven and ambientCG (CC0), Material Maker (MIT), Blender (outputs ours).
2. **A furniture family.**
   - *Programs.* Chair, table, shelf and cabinet, sofa, in `forge-parts`, read against Infinigen's
     tables (legs, stretchers, lofted top) and seating.
   - *Variants.* 50 seeded per family.
   - *Measure:* milliseconds per variant; triangles; zero failures of the validity checks (no
     interpenetration, feet on the floor, stable under Jolt); trim coverage; a showroom line-up
     capture; one room dressed by a 20-rule solver and its milliseconds.
   - *Inputs:* CC0 wood, fabric and metal materials.
3. **A near-current vehicle family.**
   - *Classes.* City car, hatchback, saloon or estate, small van, a low-floor tram of three to five
     modules (2.40–2.65 m wide, the range of a common European family), a harbour shuttle boat.
   - *Grammar.* One part grammar (spine and lofted sections, glasshouse, boolean arches, separate
     wheels and lights), shared paint, glass, rubber and trim materials, and an unbranded decal
     atlas.
   - *Measure:* dimensions within each class's real ranges; triangles; cook time; collision shells
     derived automatically and driven in the physics lab in place of `car.glb`; a line-up capture
     beside today's Blender car.
   - *Inputs:* public dimension ranges, CC0 materials.
4. **A park tree.**
   - *Species.* A London plane (and a lime) from the Forge grower, with CC0 bark and leaf scans; a
     Sapling tree exported headless as the reference.
   - *Measure:* triangles per band; leaf-card count; cook and impostor bake milliseconds; wind cost;
     an avenue of 20 trees with no two neighbours alike; ꟻLIP between seeds as a variety measure; a
     close and a far capture.

**For a decision later** (a D-0xx after the owner's review):
1. The material graph format and node vocabulary; whether Material Maker's `.ptex` is a source
   format Forge imports, or only an authoring aid whose exports are the sources.
2. New native dependencies for the cook (xatlas, Manifold, possibly OpenVDB): each a C++ build and a
   download, to agree first.
3. Whether offline tools (Blender, Material Maker) are part of the documented cook, with versions
   pinned and outputs cached, or only used by hand.
4. Generative AI: none in the engine; offline use for blockout only, with which models and services,
   and the licence record each needs.
5. Purchases (Houdini Indie, Substance Designer): only if a proof of concept shows the free tools
   fall short.

---

## What to measure

- **Materials:** cook time per graph at 2048², per map and per material set; GPU against the CPU
  reference (max difference); KTX2 bytes; cache hit time; map statistics against MatSynth's CC0
  families; ꟻLIP between a Material Maker export and Forge's evaluator for the same graph (the
  port's error).
- **Wear:** mask bake time per module (curvature, occlusion, thickness); the material resolve's cost
  with wear thresholds on and off (F1 overlay); a capture pair with and without wear.
- **Part programs:** milliseconds per variant; triangles and clusters; validity failures per
  thousand variants; trim coverage (share of faces snapped); UV stretch on the rest.
- **Buildings:** modules per building; instances per block; cook time per block, cold and warm;
  distinct facade sequences per 20 seeds; GPU time of a block at 1440p; memory of modules against
  unique meshes (`city-generation.md` §4's numbers as the baseline).
- **Interiors:** milliseconds per room by type, constraint violations left, objects per room; the
  frame cost of dressing as a building streams in.
- **Trees:** triangles per band, cook and impostor times, wind pass cost, variety across seeds.
- **Pipeline:** per-asset determinism (digest of the cooked DAG across two runs and two machines);
  Blender batch throughput (assets per process); total cook time of the city's content set.

---

## Patents

Found with WebSearch and Google Patents pages on 2026-10-10. The statuses and expiry dates are Google
Patents' own estimates (it labels them assumptions, not legal conclusions). This is a record of what
was met, not a freedom-to-operate review; nothing was checked in the EPO or national registers
directly.

| Patent | Holder | Jurisdiction | Status (as listed) | Expiry (estimate) | Technique | Alternative |
|---|---|---|---|---|---|---|
| US 9,218,672 B2 | Allegorithmic → Adobe | US | granted 2015-12-22, active | 2032-09-04 | an engine rendering procedural graphs that, when memory runs short, deletes the lowest-weight intermediate results (read) | cook offline into the D-053 cache keyed by inputs, with no runtime graph engine; if one is ever built, read the claims first |
| EP 2 599 057 B1 | Allegorithmic → Adobe | EP: lapsed in FR (2021-07-31) and several states; DE and GB fees paid in 2025 | granted 2017-05-31 | 2031-07-29 | a procedural texture editor whose optimiser linearises the graph into an ordered list to minimise memory, and a render engine walking it (read); the US counterpart US 2013/0187940 A1 was abandoned in 2015; CA 2 806 802 C and KR 10-1893034 B1 granted | free in France and the US as listed; for DE and GB, the same alternative as above |
| US 9,959,663 B2 (and US 10,229,514 B2) | Allegorithmic → Adobe | US | granted 2018-05-01 (and 2019) | 2034-07-15 | procedural textures on an object from physical ink and applicator data, painting with simulated media (first read; second a search record) | masks from geometry (§2.2); hand painting in ArmorPaint or Material Maker |
| US 11,176,714 B2; US 11,688,108 B2 (continuation) | Adobe (from Allegorithmic) | US | granted 2021-11-16; 2023-06-27; active | 2034-07-15 | a design tool's particle emitter fires particles at an object; their traces (cracks merging where they meet) generate procedural textures (read) | exposure by sky rays and curvature masks (§2.2; γ-ton tracing, 2005, in `civilisations-styles.md` §4). **Ask the owner before building particle-trace wear.** |
| US 11,875,446 B2 | Adobe | US | granted 2024-01-16, active | 2042-08-05 | generating procedural material graphs with a node-generation network (the MatFormer line) (read) | a hand-authored node vocabulary; parameter fitting (MATch's status not checked) |
| US 8,285,055 B1 | Adobe (Barnes, Goldman, Shechtman) | US | granted 2012-10-09, active | 2031-04-09 | correspondence between image regions by propagation from neighbours and randomly perturbed candidates: PatchMatch's method, though the text does not name it (read) | image quilting (expired, below), Efros–Leung at cook time, hex-tiling at run time |
| US 10,043,291 B2 | Imagination Technologies | US | granted 2018-08-07, active | 2036-05-13 | patch-based synthesis with a table of best-matching blocks for combinations of blocks, for real-time texture generation (read) | Cohen et al.'s Wang tiles (2003; no patent found), hex-tiling |
| US 6,919,903 B2 | MERL (Freeman, Efros); now listed under Mind Fusion LLC | US | **expired** 2021-07-23 | — | image quilting: overlapping blocks with minimum-error seams, and texture transfer (read) | free to use |
| US 7,050,051; 7,415,156; 7,502,511 | Carnegie Mellon University | US | **expired** (7,502,511: fee-related, 2020-01-28) | — | a parametric shape grammar interpreter (7,502,511 read; the others search records) | free to use |
| US 4,710,876 | General Electric (Cline, Lorensen) | US | **expired** 2005-06-05 | — | marching cubes (read) | free to use |
| US 2009/0174703 A1 | Disney | US | **abandoned** application | — | growing 3D trees from a seed particle steered by light and obstacles (read) | free to use |
| US 2026/0099985 A1; GB 2 702 122 A | NVIDIA | US, GB | published applications, pending | — | 3D scene generation by sparse-voxel latent diffusion (search records and a patent digest read) | relevant only to building such a model, which this note does not recommend |

**Looked for and not found** (searches only): a SpeedTree or IDV patent on tree modelling, wind or
LOD; a patent on space colonisation; on Wave Function Collapse; on interior mapping; on hex-tiling or
histogram-preserving blending (Unity); on Wang tiles held by Microsoft; on CGA by Procedural Inc. or
Esri; on curvature- or occlusion-driven wear masks as such. Their absence from search results is not
proof that none exists.

---

## Checked and left out

- **Material Maker:**
  - its community library's licence: the "terms" link served the privacy page, and no licence
    statement was found;
  - whether a command-line export needs a visible window: not documented, inferred from the code;
  - `--size` being unused in the export call: read in `parse_args.gd` on 2026-10-10, not tested;
  - its custom export target format: only in the 1.2 devlog (a search record).
- **Substance:** the Automation Toolkit's availability per plan (Adobe's help pages returned 403;
  community answers disagree); the 2026 Steam price of Designer in US dollars (only an aggregator's
  Australian price); whether the UE5 plugin's GPU engine runs in packaged builds (one user report).
- **ArmorPaint:** the price of its binaries (not on the page read).
- **Houdini:** the EULA, and the name of Indie's file format.
- **Guerrilla's and EA's tools:** Guerrilla's talk page names no software; DICE's Battlefield 2042
  badge talk is only a session listing (a search record). Insomniac's talks are session records.
- **Vehicles:** no research paper of standing on procedural game vehicles; the commercial parametric
  kits were search records (the CGTrader page returned empty).
- **Bridges and quays:** no procedural generation paper found; §4.2's typology is this note's.
- **France's 2015 green-roof requirement for new commercial buildings:** not in the Wikipedia article
  read and not checked further.
- **Infinigen Indoors' solver time units** (Table 3): not stated in the text read; the documentation's
  room times were used instead. Its perceptual study's numbers were not extracted.
- **Unreal's Shape Grammar:** the engine version that introduced it is not on the page.
- **Hunyuan3D 3.0 open weights:** not found; a KuCoin item about an Apache-2.0 "HunYuan 3.0" concerns
  a language model.
- **Tripo's terms:** 403 on every Tripo page; only search records.
- **Objaverse's per-object licences** (Holodeck's assets, TRELLIS.2's training data): not checked.
- **libfive's licence:** the GitHub API returned none; left out.
- **ChatGarment's and Design2GarmentCode's licences:** not found.
- **A measured, published comparison of procedural against scanned materials:** none found; §2.6 is
  this note's assessment.
- **Microsoft Flight Simulator 2024's building pipeline:** only the 2020 TechCrunch article and forum
  records.
- **A Microsoft Wang-tile patent, SpeedTree patents, a WFC patent:** none found (see Patents).

---

## Verification notes

Checked on 2026-10-10 with WebFetch, WebSearch and `gh api` (reading GitHub files and metadata)
only. No browser pane was used; nothing was downloaded into the repository, installed or run. The
Material Maker zip on the owner's disk was not opened. One licence text (Hunyuan3D 2.1's) was read
through `gh api` into the session's scratch directory to search it, and deleted afterwards. "Read"
means the page or file was returned and the facts are from it; "search record" means only a search
engine's extract was seen.

- **Read:**
  - *Material Maker:* release 1.7; its documentation's command-line page; Digital Production on 1.5
    (26 January 2026); materialmaker.org.
  - *Adobe and Substance:* Steam's Substance 3D Designer 2022 listing; CG Channel on Substance prices
    (21 February 2025); Adobe's Painter "Dirt" generator page; Adobe's blog on Firefly in Substance
    (18 March 2024).
  - *Other tools:* armorpaint.org; Blender's licence page; Blender's 4.3 and 5.0 Geometry Nodes
    release notes; CG Channel on Blender 4.5 (15 July 2025); SideFX's Houdini Indie and Buy pages;
    The Grove's home page.
  - *Infinigen and its papers:* infinigen.org; ar5iv's Infinigen paper; arXiv's HTML of Infinigen
    Indoors; Infinigen's "Hello Room" and "Exporting to External File Formats" documentation; the
    arXiv abstracts of Infinigen Articulated, ProcFunc, Holodeck and ProxyBuild.
  - *Datasets and models:* Hugging Face's MatSynth dataset card and StableMaterials and TRELLIS.2-4B
    model cards; the Stability AI Community License; Meshy's Terms of Service (19 September 2026);
    Deemos's terms.
  - *Studios and games:* Ubisoft La Forge's Generative Base Material post (9 December 2025); GDC
    Vault's Horizon Forbidden West texturing session; TechCrunch on Blackshark (17 August 2020); Game
    Developer on Townscaper (6 April 2022).
  - *Structures and the green layer:* the SIGGRAPH history record of CGA++; Epic's "Using Shape
    Grammar with PCG"; TU Delft's record of Tutenel et al. (2011); TU Wien's record of Vasiljevs
    (2018); Bath's record of Liu et al. (2026); paris.fr on the bioclimatic PLU (used in the
    appendix); Wikipedia's "Green roof", "Green wall", "Bosco Verticale", "Alstom Citadis" and
    "Platanus × hispanica" (the last three for a city appendix).
  - *Google Patents:* US 9,218,672; US 9,959,663; US 11,176,714; US 11,688,108; US 2013/0187940 A1;
    EP 2 599 057 B1; US 11,875,446; US 8,285,055; US 10,043,291; US 6,919,903; US 7,502,511;
    US 4,710,876; US 2009/0174703 A1. Also patent.nweon.com's digest of US 2026/0099985 A1.
- **Read through `gh api`:**
  - *Material Maker:* `parse_args.gd`, `LICENSE.md`, README, repository metadata and releases.
  - *Other tool repositories:* `armory3d/armortools` (licence, readme); `sideeffects/SideFXLabs`
    (licence, README, the `otls` listing); `MaximeHerpin/modular_tree` (metadata, releases).
  - *Infinigen:* `princeton-vl/infinigen` (metadata, the `src` tree, the seating, tables and shelves
    folders); `princeton-vl/procfunc` (metadata).
  - *Generative models:* `allenai/Holodeck` (metadata); `microsoft/TRELLIS` and `microsoft/TRELLIS.2`
    (metadata, README); `NVlabs/nvdiffrast` (licence); `Tencent-Hunyuan/Hunyuan3D-2.1` (metadata,
    licence); `Stability-AI/stable-point-aware-3d` and `stable-fast-3d` (metadata, licence,
    README); `buaacyw/MeshAnythingV2` (metadata, licence).
  - *Geometry libraries:* `jpcy/xatlas` (metadata, README); `wjakob/instant-meshes` (metadata,
    licence); `elalish/manifold`, `AcademySoftwareFoundation/openvdb` (metadata).
- **Search record only:** Material Maker's 1.2 devlog (custom export targets); Adobe's community
  answers on Designer's command-line tools; Adobe's Firefly FAQ on training data; Comfy.org on
  CHORD's licence; the UE5 Substance plugin settings and the September 2026 bug report; Substance
  Painter's Metal Edge Wear, Curvature and Mask Editor pages; the idTech 666 talk; EA DICE's GDC 2022
  session; Insomniac's GDC 2019 and 2024 talks; CD Projekt Red's use of Painter; the commercial
  "Procedural Vehicles" kits and the Geometry Nodes car generator; ChatGarment and Design2GarmentCode;
  the University of Twente WFC park thesis; Tripo's licence terms; Hunyuan 3D 3.0's announcement;
  MSFS forum threads on OSM data; US 10,229,514; US 7,050,051 and 7,415,156; GB 2 702 122 A.
- **Refused or empty:**
  - *Adobe help:* helpx.adobe.com (Substance Automation Toolkit pages: 403).
  - *Tripo:* tripo3d.ai's terms, help and blog pages (403).
  - *Patents:* patents.justia.com (403).
  - *Material Maker:* readthedocs' "sharing" and "export" pages (404); materialmaker.org/terms
    (served the privacy page).
  - *Other:* gdconf.com's schedule (403); the CGTrader vehicle listing (empty); arXiv's Infinigen PDF
    (over the size limit; ar5iv used instead); Blender's command-line manual page (navigation only).
- **From Forge, read in the repository:**
  - *Rules and notes:* `CLAUDE.md`; `docs/research/galaxy-sky.md` (format);
    `assets-and-licences.md`; `civilisations-styles.md` (§3–§4); `procedural.md` (§4–§5);
    `vegetation-materials.md` (§1, §4–§5, Recommendation); `city-generation.md` (§3, §5,
    Recommendation).
  - *Decisions and demos:* `docs/DECISIONS.md` (D-039, D-053); `docs/demos/city-blocks.md`
    (materials, props); `island.md` (stages); `physics-lab.md` (models).
  - *Code and assets:* `crates/forge-procgen/src/lib.rs` and its line counts;
    `crates/forge-geom/src/city.rs`, `stone.rs` and `procedural.rs`;
    `crates/forge-render/src/textures.rs`; `assets/blender/*.py` (car, ship, boat,
    skinned_creatures); `CREDITS.md` (Blender's line); `Cargo.toml` (licence); `Cargo.lock`
    (`meshopt`).
- **Numbers to re-check before a spec:**
  - every price (Houdini, Substance, Steam);
  - Material Maker's `--size` behaviour on the 1.7 build;
  - Infinigen's per-room times on the owner's CPU;
  - TRELLIS.2's export path's dependency on `nvdiffrast`;
  - the interior dressing budget (5–20 ms a room is an estimate);
  - the bridge span thresholds (rules of thumb, not sourced).
