# Demo: city-blocks

The Phase 1 closing demo (issue #13): a 16 km² procedural terrain patch holding a million
GPU-placed instances of twenty props of 0.5–3 M triangles each, flown at 300 m/s with
streaming on, 120 fps at 1440p on the RTX 5070 Ti. It is built in steps:

| Step | Issue | State |
|---|---|---|
| The meshlet renderer at a million instances | #33 | ✅ 197 MiB for 980 k instances (`docs/demos/meshlets.md`) |
| Twenty props, cooked once and cached on disk | #34 | ✅ this page: the prop gallery |
| Terrain patch and GPU placement | #35 | next |
| Culling at a million instances | #37 | the culls are 5 ms of a million-instance frame |
| Streaming of cluster pages | #36 | |
| The flight, 1440p, numbers | #13 | |

```
cargo run --release -p city-blocks
```

Keys: WASD/QE move, Shift fast, right mouse look, **L** cluster LOD, **K** LOD colours,
**M** cluster colours, **O** occlusion, **R** software rasteriser (auto → on → off), **H**
what it drew, **[** / **]** LOD threshold, **Tab** wireframe, **G** tone curve.

Options:
- `--focus NAME` frames one prop (`fountain`, `tower-wide`, …).
- `--recook` cooks every prop again.
- `--orbit` gives a scripted camera.
- `--no-lod`, `--no-occlusion`, `--lod-error PX`, `--sw-raster auto|on|off`,
  `--sw-raster-area PX`, `--ev100 EV`, `--tonemap agx|aces|neutral`, `--force-fallback`,
  `--frames N`, `--capture file.png`, `--capture-frame N`.

## The props (issue #34, 2026-09-24)

`forge_geom::city` generates twenty props: ten buildings, two towers, three boulders, two
rubble piles, and a column, a fountain and a lamp post turned on a lathe. Together they hold
**25.8 M triangles in 612 k clusters** (both at full detail).

- **Buildings.** A building is a closed box whose faces are dense grids, 7 to 11 cm per
  cell, cut to the triangle target. The grids are displaced along the face normal into a
  facade:
  - flush corner pilasters and a plinth;
  - shop fronts on the ground floor;
  - a ledge at every floor line;
  - recessed windows with bevelled reveals in regular bays;
  - a cornice under the roof line.

  The face edges never move, so the box stays closed (a test checks every edge has its
  twin). Boxy units on the roof are placed from the building's seed.
- **Boulders and rubble.** The asteroid generator, flattened and sunk into the ground.
- **Lathed props.** A profile polyline resampled evenly and revolved, with optional
  flutes. Where the profile meets the axis there is one vertex and a fan of triangles: a
  ring of copies there made degenerate triangles, which stalled the simplifier and left
  the column and the fountain with 25 and 35 roots.

**Cooking with the normals.** With geometric error alone, the coarse levels of a facade
dropped its windows while they were still several pixels wide. A recess is 25 cm deep, and
that is all the error the simplifier saw. The vertices left behind kept normals that no
longer matched the surface, so the facades smeared at a distance. Hard-surface props
therefore add their normals to the simplification error (`CookOptions::normal_weight`):
- buildings count a 90° turn of the normal like about a metre;
- lathes count half that;
- rocks count geometry alone, like the ballad's asteroids.

At 1 px, the gallery's LOD view and its full-detail view now differ by 1 % of their pixels,
all of them sub-pixel window edges.

| prop | triangles | clusters | levels | roots | fill | cook ms | cached ms | file MB |
|---|---|---|---|---|---|---|---|---|
| house-narrow | 517 k | 12 096 | 14 | 2 | 0.69 | 890 | 9 | 15.7 |
| house-wide | 632 k | 14 778 | 14 | 2 | 0.69 | 1 055 | 14 | 19.2 |
| corner-block | 1 219 k | 28 584 | 15 | 2 | 0.69 | 2 043 | 35 | 36.9 |
| terrace | 828 k | 19 331 | 16 | 2 | 0.69 | 1 640 | 27 | 25.1 |
| apartments | 1 515 k | 35 549 | 15 | 1 | 0.69 | 2 887 | 32 | 45.9 |
| office | 2 032 k | 47 570 | 15 | 3 | 0.69 | 3 975 | 44 | 61.6 |
| hotel | 2 225 k | 52 203 | 16 | 2 | 0.69 | 4 449 | 48 | 67.5 |
| warehouse | 1 005 k | 23 431 | 14 | 2 | 0.69 | 1 930 | 31 | 30.4 |
| school | 926 k | 21 649 | 14 | 3 | 0.69 | 1 807 | 29 | 28.0 |
| clinic | 614 k | 14 370 | 14 | 2 | 0.69 | 1 014 | 15 | 18.6 |
| tower-slim | 2 832 k | 66 410 | 16 | 2 | 0.69 | 5 444 | 79 | 85.9 |
| tower-wide | 3 021 k | 70 975 | 16 | 2 | 0.69 | 5 771 | 88 | 91.6 |
| boulder-1 | 529 k | 12 638 | 14 | 1 | 0.67 | 1 043 | 15 | 16.1 |
| boulder-2 | 750 k | 17 887 | 14 | 1 | 0.68 | 1 386 | 21 | 22.8 |
| boulder-3 | 504 k | 12 020 | 13 | 1 | 0.68 | 812 | 14 | 15.4 |
| rubble-1 | 560 k | 13 337 | 16 | 3 | 0.68 | 873 | 15 | 17.0 |
| rubble-2 | 871 k | 20 792 | 16 | 2 | 0.68 | 1 422 | 21 | 26.5 |
| column | 1 839 k | 44 750 | 15 | 1 | 0.66 | 3 130 | 53 | 56.3 |
| fountain | 2 794 k | 68 656 | 16 | 1 | 0.66 | 4 530 | 76 | 85.8 |
| lamp-post | 612 k | 15 143 | 14 | 1 | 0.65 | 950 | 12 | 18.8 |

Reading the table:
- **Roots** are clusters no coarser level replaces: 1 when the DAG simplified the prop down
  to a single cluster; 2–3 when a group stalled near the top, usually one per disconnected
  part.
- **Fill** is the mean triangles per cluster over the maximum of 124: meshoptimizer's
  64-vertex limit keeps clusters near 85 triangles.
- **Cooking** takes 47 s of CPU, **9.2 s** on the job system. Before issue #34's DAG fix
  (`docs/demos/meshlets.md`, "Cooking a DAG in seconds"), the 3 M-triangle props alone would
  have taken over eight minutes each.
- **From the cache**, the twenty load in **0.54 s** (0.70 s of CPU, about 1.4 GB/s).

**The cache.**
- **Location:** `mesh-cache/<name>-<key>.fmesh`, git-ignored, 749 MB for the set.
- **Key:** a hash of the prop's parameters, its cook options and `COOK_VERSION`.
- **Contents:** a small header, then the cooked arrays as they lie in memory.
- **Writing:** into a temporary file, renamed into place. A new cook removes that prop's
  files under older keys.
- **Mismatches:** a file whose key, version or size does not match is cooked again.

This is the demo's cache, not the engine's asset format. That arrives with streaming (#36):
fixed-size cluster pages, D-018's container, compressed vertices.

**GPU:**
- The gallery overview draws in **0.12 ms** at 1 px LOD, against 0.71 ms at full detail
  (`--no-lod`).
- Close-ups (`--focus corner-block`, `--focus fountain`) take 0.12–0.13 ms.
- 749 MiB of geometry in VRAM.

**Materials.** The props are shaded with the ballad's rock shading: per-instance tints, and
a fifth of the instances icy blue, which is why the fountain looks frozen. Materials come
with #20.
