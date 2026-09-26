# #93 on the RTX 5070 Ti (2026-09-26): what the batch found

The branch `claude/keen-sagan-vk91st` at `6c4a270`, plus `7e13674` (the fix below; `ce318a8` in `env.txt`, before a rebase onto the research commit `0986b79`), against
`main` at `f915145`. The toolchain here: rustc 1.98.1, slangc 2026.13.1 (the Vulkan SDK's; the
cloud had 2026.18.3). Not merged: three of the owner's pass conditions do not hold as written
(below). The directories hold each run's summary, logs and contact sheet; `crops/` the details.

## What passes

- `cargo build --release`, `cargo test --release`, clippy with `-D warnings` as in CI, `fmt --check`.
- Within the new batch (`batch-after/compare.txt`): the occlusion and cone A/B, `--show-culled`
  (no red) and mesh against fallback on all eight views, **0 px** on every line.
- `tools/validate.sh` (`validate/`): the ten runs with the validation layer and synchronization
  validation, no VUID and no hazard; the mip check and the ACES 2.0 check pass.
- The placement's checksum: `4e10743a3499dc0e` on every city run and on both paths (the old
  record's was `ed6454c65dd1e823`).
- Timed serially (`FORGE_ASYNC=0`), the streamed city takes the same time on both builds (2.11
  against 2.12 ms), `cluster cull 1` a little less (0.254 → 0.248 ms).

## Found and fixed on the branch (`7e13674`)

`sun_light`'s highlight took `legacy_camera_world(f)`, the camera's **world** position: with
`--origin` it moved every highlight. The first `origins.sh` (`origins-first/`) differed by the
same amount at every offset, which gave it away:

| Offset | City | Ballad |
|---|---|---|
| 10⁴ m | 63 139 px, ꟻLIP mean 0.0159, max 0.42 | 41 951 px, 0.0208, max 0.69 |
| 10⁷ m | 65 049 px, 0.0162, max 0.41 | 42 162 px, 0.0208, max 0.69 |

The fix hands it the camera in the scene frame (`legacy_camera_position`), which is what the world
position was when every scene stood at the origin: the origin's captures are unchanged (0 px
against the unfixed build), and the highlights no longer depend on the offset.

## What does not pass as written

**1. `origins.sh` is not 0 px** (`origins-fixed/`, `origins-cells/`):

| Offset | City | Ballad |
|---|---|---|
| 1 024 m, 10 240 m (whole cells) | **0 px** | **0 px** |
| 10⁴ m | 2 755 px (max 21 levels), ꟻLIP mean 0.0017, max 0.096 | 1 666 px (max 131), 0.0005, max 0.29 |
| 10⁵ m | 2 579 px, 0.0016, max 0.096 | 1 666 px, 0.0005, max 0.29 |
| 10⁶ m | 2 640 px, 0.0016, max 0.096 | 1 665 px, 0.0005, max 0.29 |
| 10⁷ m | 2 548 px, 0.0015, max 0.096 | 1 666 px, 0.0005, max 0.29 |

For comparison, the record before (`origins-old-record/`, built from `998d0f8`), city and ballad:
10⁴ m 139 809 and 46 828 px (ꟻLIP mean 0.031 and 0.023), 10⁵ m 344 622 and 111 939 px
(0.041, 0.033), 10⁶ m 1.12 M and 1.30 M px (0.53, 0.35: blocks of the frame black, the sky grey),
10⁷ m 1.34 M and 1.18 M px (0.40, 0.28).

What the rest is:
- **Nothing uses the world position any more.** Offsets of whole cells give 0 px in both demos:
  the offsets inside the cells are then the same bits, and so is every pixel.
- **It does not grow with the distance.** The far offsets agree with each other: in the ballad,
  10⁴ against 10⁵, 10⁶ and 10⁷ m differ by 12, 13 and 12 px; in the city by 698–848 px, 11
  levels at most. Their offsets inside a cell (784, 672, 576 and 640 m) are all multiples of 16,
  so they round the positions onto the same 0.06 mm grid. The origin, whose positions keep
  finer bits, is the odd one out.
- **It is the split's rounding, amplified.** Even 1 m moves the city by 5 564 px. The traced sun
  shadows make most of it: 910 px with `--no-shadows`, and 979 px with shadows, AO, probes and
  reflections all off. A shadow ray is a hit or a miss, so a 0.06 mm change flips pixels along the
  occluders' edges. TAA then blends 60 jittered frames of such flips into many pixels that differ
  a little, mostly 4 levels or less.

To get 0 px at every whole-metre offset, the positions would have to be exact. That means putting
the instances' offsets and the camera's on a fixed grid, for example 2⁻¹³ m (0.12 mm): below 1 024
m such a value is an exact `f32`, and so is the difference of two of them. That is a design change
(it also helps D-016's determinism) and the owner's to decide. It was not built.

**2. Against `main`, the largest ꟻLIP values exceed D-017's 0.15** (`batch-after/compare.txt`).
All 26 images differ. Every ꟻLIP mean is under 0.02 (at most 0.0047, `mesh-cityorbit120`). The
largest single value is above 0.15 in 22 pairs:

| Image | Pixels | Mean | Largest |
|---|---|---|---|
| static60 | 51 | 0.0004 | 0.26 |
| orbit120, noocc120 | 205 | 0.0010 | 0.31 |
| nolod120 | 383 | 0.0014 | 0.36 |
| ast240 (and its A/B variants) | 3 371 | 0.0016 | 0.39 |
| ast-notaa600 | 2 505 | 0.0030 | 0.43 |
| ast-taa600 | 763 | 0.0032 | 0.39 |
| city60 | 7 547 | 0.0039 | 0.14 |
| cityorbit120 | 4 535 | 0.0047 | 0.16 |
| gallery60 | 210 | 0.0009 | 0.05 |

The largest values are single pixels on a silhouette, and a shadow edge on a rock moved by less
than a pixel (`crops/meshlets-static60-…`, `crops/ballad-notaa600-…`). By eye, the two batches
look the same. This is the same rounding as in (1), from a quaternion instead of a matrix and
positions relative to the camera instead of the world. PROCESS.md's table puts a single white
pixel at 0.38, so D-017's max < 0.15 rules out any change that moves one silhouette pixel.

**3. The streamed city is slower with async compute** (`timings/summary.txt`, 3 runs each,
alternating):

| View | main | branch | `cluster cull 1`, main → branch |
|---|---|---|---|
| city south (streamed) | 1.99 ms | 2.18 ms | 0.351 → 0.481 ms |
| city fly (streamed) | 1.98 ms | 2.04 ms | 0.258 → 0.318 ms |
| city orbit | 2.47 ms | 2.42 ms | 0.546 → 0.509 ms |
| city resident | 2.02 ms | 1.97 ms | 0.302 → 0.298 ms |
| meshlets, orbit, side 700 | flat | flat or −0.01 ms | flat |
| ballad 900p, 1440p | 1.29, 2.62 ms | 1.27, 2.62 ms | flat |

Serially, the two builds take the same time, and `FORGE_GRAPH_LOG=1` prints the same plan for
both. So no pass got slower on its own. What changed is how the graphics queue's cull overlaps the
compute queue's probe rays: on `main` the overlap saves 0.13 ms on the south view, on the branch
it costs about 0.03 ms (probe rays 0.79 → 0.95 ms, cull 1 0.355 → 0.443 ms in one async run).

Where to look: the cull now builds the rotation matrix from the quaternion at every
`instance_point` (the cluster's centre, the LOD's self and parent centres, the cone's apex and
axis). Each call also reads `camera_cell` and `camera_local` through the frame pointer. Computing
the instance's frame and its camera-relative origin once per cluster would trade that arithmetic
back. Only a measurement will say.

## What the owner decides

- Whether the whole-cell 0 px plus the rounding residual (1) counts as `origins.sh`'s pass, or
  whether the positions go on a fixed grid first.
- Whether D-017's largest-value threshold should allow single-pixel edge moves (2): the means all
  pass.
- Whether (3) blocks the merge or becomes its own issue.
- D-004's amendment itself (keep, amend, drop), with the old record's numbers above.
