# Foliage reference scenes for the calibration (D-083, 2026-11-01)

**The ask** (the owner, 2026-11-01): "It would be nice to find a falcor or pbrt-v4 or other source
for forest / plant / foliage scene we could test against later on." The calibration so far has the
Bistro (a few trees and planters), Bitterli's rooms and the material chart: nothing that measures
light under a canopy, through leaves or over grass, where Forge's foliage (D-075, D-091) and its
lighting under trees (D-093) are judged by eye. This note lists free scenes a path tracer renders
as provided and Forge can load, for a later step. **Nothing has been downloaded:** the download
waits for the owner's word (CLAUDE.md).

Found by a research agent over the web on 2026-11-01 (sources at the foot); the licences are as the
pages state them, to read again in each archive's own notice once downloaded.

## The candidates, ranked for Forge

| # | Scene | Holder, licence | Where | Formats; what renders it | Size | Vegetation |
|---|---|---|---|---|---|---|
| 1 | **Landscape** | Laubwerk (Schliep, Kahraman, Dapper), CC BY 4.0 (commercial use and renders allowed) | `landscape/` in github.com/mmp/pbrt-v4-scenes | pbrt-v4 text + PLY meshes; pbrt-v4 as is; Falcor 9 likely through its pbrt-v4 importer; Cycles after a conversion | 24 M unique triangles, 3.1 billion instanced; 23,241 plant models | White fir, silver birch, rowan, silver linden, horse chestnut; meadow grass, daisy, plantain, vetch, nettle. Leaves with alpha maps and `diffusetransmission` (coated diffuse in front, transmission behind); lit by a sky map alone. Ships `view-0.png` and five views (pbrt's third edition's cover, 3840 × 2160 at 512 spp) |
| 2 | **Jungle Ruins** | Intel, "Creative Commons Attribution" (version not stated on the page) | `JungleRuins_1_0_1b.zip`, 4.0 GB, Intel's graphics research samples page | Blender source + USD; Cycles as is; Falcor's USD importer untried | Over a trillion instanced triangles; an 8K × 8K terrain in tiles (~64 km²) | A rainforest in size classes (canopy, medium, ...), modelled leaves and opacity-mapped cards, built procedurally; made for real-time path tracing (Intel quotes 1 spp at 30 fps at 1440p on its B580) |
| 3 | **Sponza's Trees and Ivy add-ons** | Intel, Creative Commons Attribution (version to check) | Intel's samples page: Trees 907 MB, Ivy 1.29 GB, over Sponza's base (3.71 GB) | glTF, FBX, USD; Falcor as is (glTF/FBX); Cycles after import; pbrt-v4 after a conversion | not stated | Italian cypresses on alpha cards beside ivy with modelled leaves: a clean cards-against-geometry test |
| 4 | **Poly Haven's nature models** | CC0 | api.polyhaven.com (models, category nature) | single assets to place (Blender/Cycles; glTF, FBX, blend) | e.g. pine_tree_01 17.4 M triangles, fir_tree_01 7.9 M, grass_medium_01 1.6 M | 35+ plants with modelled leaves: pines and firs, jacaranda, island trees, fern, moss, nettle, grasses, flowers |
| 5 | **Blender Studio's geometry-nodes demos** | CC0 | blender.org demo files: `tree_leaves_moss_geo-nodes-demo.blend` (1 MB), `flower_scattering.blend` (6 MB) | .blend; Cycles | small | one procedural tree with leaves and moss; scattered flowers |
| 6 | **Infinigen** (Princeton) | code BSD-3; the licence of its generated scenes not stated | github.com/princeton-vl/infinigen | Blender; whole scenes export as USDC, the materials cut to albedo, roughness, normal and metal (a leaf's translucency lost) | as large as asked | procedural natural scenes without end |

**The recommendation:** pbrt-v4's **Landscape** first, the reference for light through foliage
(the same renderer as the rooms' third reference, CC BY 4.0, leaves as alpha cards and diffuse
transmission, a sky alone). Loading it in Forge needs a pbrt-v4 reader that takes `ObjectBegin` and
`ObjectInstance` (its `geometry.pbrt` is about 9.4 MB of text and numbered PLY meshes): the step
past `tools/pbrt-to-gltf.py`, which writes every shape flat. Its 3.1 billion instanced triangles are
a test of Forge's instancing too. Then **Sponza's Trees and Ivy** for the cards-against-geometry
A/B, and **Jungle Ruins** as the forest a game would hold. Before trusting a Falcor-against-pbrt
comparison on Landscape, check how Falcor's importer reads `diffusetransmission`.

## Not usable, or flagged

- **Disney's Moana Island:** a custom licence, "research or software development (including
  benchmarking) purposes" only, passed on with the same restriction; renders named "Moana Island
  Scene", no endorsement implied. pbrt-v4's package 5.6 GB (30 GB unpacked), USD 15 GB, base
  45 GB; 15 billion primitives instanced; pbrt took 41-81 GB of memory in Pharr's 2018 tests. A
  calibration may count as software development: the owner's call.
- **NVIDIA ORCA's SpeedTree pack** (eight species with a Falcor scene) and **Emerald Square**
  (10 M triangles, those trees): CC BY-NC-SA 3.0, non-commercial. Sun Temple: BY-NC-SA 4.0, no
  plants.
- **pbrt-v3's "ecosys"** (Deussen et al.): no licence stated; old and simple. **San Miguel**:
  no licence stated, no plants named.
- **Blender's splash scenes** (2.91's red autumn forest, 3.1's deer): CC BY-SA, art scenes.
- **Unity's Book of the Dead:** the Asset Store's EULA (usable in another engine, never
  extracted or passed on), a Unity 2018.2 project: private renders at most.
- **Megascans / Fab:** free for everyone until the end of 2024 only; the Epic Content License
  binds the assets to Unreal Engine. Epic's own samples were not read; assumed the same.
- **Nothing** in Mitsuba 3's gallery, Bitterli's resources or Falcor's release notes.

## Sources

github.com/mmp/pbrt-v4-scenes (README, `landscape/view-0.pbrt`, `landscape/geometry/geometry.pbrt`);
pbrt.org/scenes-v3; developer.nvidia.com/orca (speedtree, nvidia-emerald-square, ue4-sun-temple);
disneyanimation.com/resources/moana-island-scene/ and its License.txt;
pharr.org/matt/blog/2018/07/15/moana-island-pbrt-4; intel.com graphics-research samples.html and
content-details/844047 (Jungle Ruins); blender.org/download/demo-files/;
benedikt-bitterli.me/resources/; mitsuba.readthedocs.io (gallery);
github.com/NVIDIAGameWorks/Falcor/releases; github.com/princeton-vl/infinigen;
api.polyhaven.com/assets?t=models&c=nature; support.unity.com (article 34387186019988);
cgchannel.com (2024/10 Megascans, 2018/06 Book of the Dead).
