# #95: double-buffering the probe atlases and the sky tables, measured (2026-09-30)

**Outcome:** no gain on the RTX 5070 Ti, so the change is not in the engine. The patch is
`double-buffer.patch` beside this file; it applies on `1ea31f4` (`git apply`) for a run on the
RTX 3080 (#39) or AMD (#67), where the balance may differ.

## What the patch does

- **Probes (`probes.rs`, `probes.slang`, `probe_update.slang`):**
  - Two irradiance atlases, two distance atlases and two state buffers, swapped at every
    update.
  - The rays read the previous copies, and the state and blend passes write the other ones.
  - The field block holds two records: the rays' (previous maps) and the update's and
    resolve's (current maps).
  - The state pass copies a settled probe's state whole, since the copy it writes is two
    updates old.
  - The atlases then take 2 × 53 MiB (34,560 probes × 1.5 KiB).
- **Sky (`sky.rs`):** the sky-view table, the aerial-perspective volume and the irradiance
  buffer get one copy per frame slot (0.4 MiB).

With both, `FORGE_GRAPH_LOG=1` shows the compute batch of frame N waiting only for frame N−2's
last graphics batch, which has completed when the slot is reused. Before, it waited for frame
N−1's last one, whose resolve reads the atlases and whose compose reads the tables. The probes
and the sky of frame N then run during frame N−1's resolve, TAA and post.

## Numbers

The city, 3000 frames, three runs alternating the two builds. GPU ms per frame; the CPU loop
(the frame including its wait for a slot) agrees within 0.05 ms.

| View | async: before | after | `FORGE_ASYNC=0`: before | after |
|---|---|---|---|---|
| south, 1600 × 900 | 2.06 | 2.13 | 2.24 | 2.24 |
| orbit | 2.53 | 2.53 | 2.66 | 2.63 |
| south, 2560 × 1440 | 3.57 | 3.66 | 3.85 | 3.75 |

- **Probes alone (sky tables still single):** every view of `tools/timings.sh` was flat. The
  sky tables kept the wait, so nothing moved.
- **Where the time went:**
  - With both double-buffered, the probe blend reads 0.34 → 0.72 ms, and `shading/standard`
    (the resolve) 0.35 → 0.55 ms.
  - The work moved from beside the geometry passes (rasterisation, which leaves compute idle)
    to beside the previous frame's resolve (compute and ray queries, the same units).
  - The GPU is already full: the bubble before the resolve was smaller than the new
    contention.
- **Serial frames:** 0.1 ms faster at 1440p, flat at 900p. With one queue the order of the
  passes is the only difference, and it is not the configuration the demos run.

## The frame's measure

The logged GPU time was each frame's span, first timestamp to last on any queue. With the
overlap it read 3.90 ms for a 2.1 ms frame: the span started during the previous frame. The
engine now counts a frame from the previous frame's last timestamp when its first one is
earlier (`GpuTimers::frame_ms`). Summed over frames, that is the GPU's busy time. Without
overlap it is the same number as before (city 2.09–2.11 ms both ways). This part is in the
engine.

## Checks run on the patch

- **Captures:** the probe half against the build before, 0 px on every city and island image.
  The only diffs were #71's TAA flake and the ballad, which has no probes.
- **Validation:** `tools/validate.sh` clean, synchronization validation included.
- **Not run on the sky half:** once the timings showed no gain, captures and validation were
  not repeated for it.

## If it is tried again

- **Keep the overlap away from the resolve:**
  - Let frame N's probes wait for frame N−1's resolve batch, not its last one. They would then
    overlap only TAA and post.
  - Or spread the update over frames (#95's "probe cadence"), which removes work instead of
    moving it.
- **Measure per member:** the vendors' advice is to measure the batch with and without each
  member (`docs/research/dynamic-scenes.md`).

## On the RTX 3080 (2026-10-11, #39)

The server PC: RTX 3080 10 GB, driver 617.42, i7-9700K, Vulkan SDK 1.4.357.
- **Base:** the patch applied on `9b240b7`, the commit after `1ea31f4`, which adds
  `GpuTimers::frame_ms`.
- **Method:** two worktrees, base and patched, each running its own shaders. The city ran 3000
  frames per run, three rounds alternating the builds, in four views (the 1440p flight added).

The frame time is the CPU loop: the frame including its wait for a slot, with unthrottled
frames on a GPU-bound run. Each cell is the mean of the three rounds, in ms.

| View | async: before | after | `FORGE_ASYNC=0`: before | after |
|---|---|---|---|---|
| south, 1600 × 900 | 3.14 | **2.91** | 3.01 | 2.99 |
| orbit | 3.60 | 3.59 | 3.72 | 3.68 |
| south, 2560 × 1440 | 5.19 | 5.15 | 5.10 | 5.11 |
| flight at 300 m/s, 2560 × 1440 | 5.04 | **5.09** | 4.98 | 4.96 |

- **900p south, the one gain:** 0.23 ms, in every round. There the patched async frame is the
  fastest of the four, 0.1 ms under the serial one.
- **1440p, the 3080's target, no gain:**
  - the south view is flat (−0.04; −0.11 to +0.04 by round);
  - the flight costs 0.05 ms (+0.03 to +0.09 by round).
- **The GPU's measure reads high with the overlap.**
  - `frame_ms` averages 0.07 ms above the frame time in the patched async runs, and 0.03 ms
    below it in the base's. On a GPU-bound run it cannot exceed it.
  - By the GPU's measure the patch gains 0.16 ms in the 900p south view and costs 0.08 (the
    1440p south view) and 0.17 (the flight).
  - On the 5070 Ti the two measures agreed within 0.05 ms.
  - On `main` the GPU's measure stays under the frame time: 7.61 against 7.65 ms in the 1440p
    flight below.
- **Where the time goes** in the flight (GPU zones):
  - the resolve (`shading/standard`) goes 1.02 → 1.09 ms and the sky's irradiance 0.07 → 0.21;
  - the probe blend goes 0.68 → 0.53;
  - the same contention as on the 5070 Ti.
- **In the 900p south view** the resolve goes 0.44 → 0.65 ms and the blend 0.33 → 0.96, but
  the frame shrinks. There the overlap fills a bubble.
- **Serial against async at this commit:** serial is faster in three views of four (all but
  the orbit), by 0.06–0.13 ms.
- **Serial against async on main today** (`471efdd`'s binaries, the 1440p flight, three
  rounds of 6000 frames): the frame time is 7.65 ms async against 7.63 serial, level. On the
  3080 the async queue neither gains nor costs in that flight.
- **The patch draws the same frame on this GPU:** the south view, every page resident,
  frame 60, differs in 0 of 1 440 000 pixels (ꟻLIP 0). Synchronization validation, which
  includes the sky half, is clean on the city.

**Outcome:** no gain on the 3080 at 1440p, where its target is, so the patch stays out. It
would no longer apply to `main`: the probes and the sky have changed since, and the clouds
added async passes it does not double-buffer. AMD (#67) is the one card left untried.
