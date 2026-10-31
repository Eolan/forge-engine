# The calibration, night of 2026-10-30 to 31 (D-083)

The owner's brief: "calibrate the engine first with those tools (Falcor, pbrt-v4, RTXGI, the
material test...)", then update the demos. No day check before the calibration baseline, then
targeted maps only until 14:00. Every number is the mean |log2| of 16 × 9 tiles of luminance
(`tools/gi-compare.sh`, `tools/gi-measure.py`); 0.1 is about 7 %, 1.0 a stop.

## In short

- **The baseline is in** (D-083's table): twelve views, three techniques (the probes, the port,
  SHaRC) against Forge's path tracer and Falcor, with their GPU ms.
- **The references agree indoors.** Falcor, pbrt-v4 and Forge's path on the Cornell box are within
  0.025 to 0.055 of each other, in the classroom's and the kitchen's clay within 0.04 to 0.10 (with
  clay made diffuse alone on both sides this morning). Outdoors by day Forge's path reads 0.15 to
  0.19 stops over Falcor's in clay (#373).
- **The lamps were the night's fault, and are fixed for flat lights** (#369, #370). A panel or a
  quad of light is now its own rectangle, with its exact diffuse light. Forge's path with the
  lamps on is within 0.1 of itself with them off (the glow traced) in all three rooms.
- **SHaRC is the nearest indoors, by day and at night;** the probes are as good outdoors by day.
  That is D-090's recommendation, taken (the owner, 2026-10-31).
- **Your decisions, the same morning:** D-090, SHaRC indoors (as recommended); #372, ReSTIR DI for
  every lamp with a denoiser. What #372 became by midday is under "#372, the same day" below.

## The table

From Falcor (Forge's own path first, then the probes, the port, SHaRC), and the frame's GPU ms
(1600 × 900, the RTX 5070 Ti). Day rows from the baseline (637abc2c, three runs); night rows
after the lamps' fix (9d72b245, one run; the Bistro at night's ms from before it).

| View | Forge's path | probes | port | SHaRC | ms: probes, port, SHaRC |
|---|---|---|---|---|---|
| Bistro Exterior_1, clay | 0.149 (#373) | 0.158 | 0.117 | 0.104 | 6.72, 6.50, 7.68 |
| Bistro Exterior_2, clay | 0.192 (#373) | 0.315 | 0.155 | 0.137 | 5.59, 5.46, 6.37 |
| Bistro Exterior_1 | 0.080 | 0.089 | 0.096 | 0.096 | 6.89, 6.60, 7.87 |
| Bistro Exterior_2 | 0.387 | 0.443 | 0.480 | 0.484 | 5.90, 5.71, 6.70 |
| Classroom, clay | 0.098 | 0.434 | 0.097 | 0.100 | 3.84, 3.73, 4.10 |
| Classroom | 0.151 | 0.405 | 0.150 | 0.149 | 4.08, 3.98, 4.34 |
| Material chart | 0.066 | 0.088 | 0.084 | 0.085 | 3.14, 3.14, 3.15 |
| Cornell box, night | 0.025 | 0.207 | 0.092 | 0.096 | 2.90, 2.99, 2.96 |
| Kitchen, night, clay | 0.064 | 0.128 | 0.097 | 0.092 | 5.12, 4.95, 5.72 |
| Kitchen, night | 0.206 | 0.354 | 0.191 | 0.232 | 5.44, 5.28, 6.04 |
| Bedroom, night, clay | 0.114 | 0.849 | 0.274 (#371) | 0.237 | 4.90, 4.73, 5.41 |
| Bedroom, night | 0.392 | 1.191 | 0.596 (#371) | 0.445 | 5.18, 5.06, 5.73 |
| Bistro at night | 0.317 | 0.544 | 0.543 | 0.543 | 13.16, 13.19, 16.04 |

The village has no outside reference; against Forge's path, the landing is 0.507 (the probes),
0.230 (the port) and 0.110 (SHaRC), the room 0.147, 0.052, 0.052, the street 0.116, 0.115, 0.116.

- **Clay is diffuse alone on both sides** since this morning: Falcor's clay had kept a dielectric
  layer (0.09 to 0.14 stops of light in every clay view), Forge's its sky reflection. With both
  diffuse, the three renderers agree in the rooms within 0.05 to 0.10 (the kitchen: pbrt 0.052
  from Falcor, Forge's path 0.044 from pbrt; the classroom 0.066 and 0.048). **The Bistro by day
  does not:** Forge's path is 0.15 to 0.19 stops over Falcor's there (#373, next to split by sun
  and sky).
- **Textured rows carry the converter's materials** (Exterior_2 and the bedroom: Forge's own path
  is 0.39 from Falcor). The clay rows measure the light. Exterior_2's diffuse albedos match
  Falcor's (0.031): its gap is in the textured bounce and the reflections.
- **The port is unstable in the bedroom:** two runs of the same build are 0.16 to 0.27 apart
  (#371). SHaRC gives the same image each run there.
- **The Bistro at night** is its lamps', not the techniques' (the three alike): see #372 below.

## What the night found and fixed

| Commit | What |
|---|---|
| 59205665 | `tools/gi-compare.sh`'s baseline, the Cornell box (`tools/cornell-box.py`), Falcor's renders as a tool (`tools/falcor-render.sh`) |
| 8b9522cf | A lamp's glow was counted twice in the bounce, the port and SHaRC: the Cornell box's floor at twice Falcor's |
| 748c19b1 | D-090 proposed; pbrt-v4 built (the CPU renderer) and agreeing on the Cornell box (0.055 from Falcor) |
| 8f319824 | The resolve's reference path leaves a lamp's glow out; the asteroids' shadows back on NVIDIA (#355: the AMD iGPU as the oracle) |
| e7592513 | Bitterli's bedroom and kitchen as lab scenes and views, at night |
| 7d4a0993 | A flat piece's lamp stood half its radius in front of it, past the bedroom's curtains: ×6 too bright (#370) |
| 9d72b245 | A flat piece's lamp is its rectangle, its diffuse light exact (#369): the bedroom's lamp model from a stop dark to 0.097 |

Each step was checked with the gate, the sentinels (0 px, the #71 flake aside; the asteroids 0 px
for #355), the lab's lamp scenes (0 px: sphere lamps untouched), and a read-only review. The
reviews found real bugs both times (a test that did not compile, a lamp blocked by its own
surface).

## Decisions for you

1. **D-090: which light under the bounce.** Recommended: SHaRC indoors (the village's room grids),
   the probes outdoors, with NRC measured beside it before it becomes the default. Tonight's rooms
   add the night's evidence: SHaRC is the nearest or level in every room, the probes a stop dark in
   the bedroom (its light comes from behind curtains after many bounces).
2. **#372: the Bistro at night.** Two faults half cancel. The frame keeps the 64 nearest lamps, so
   the far street goes black (−0.21). The lanterns send their side's whole light straight down
   (+0.17 with all 132 lamps). Fixing the lanterns alone darkens the default, so it waits for a
   many-lights choice (D-065: clustered lists, or ReSTIR DI for every lamp with a denoiser).

## Filed tonight

- #367: rays started inside a mesh's coarser traced cut (the bounce's and SHaRC's first rays fixed
  in d98c1425 and 637abc2c; every mesh up to 250,000 triangles traced as drawn, 52909422; open for
  the rest).
- #368: the GI's half floats overflow when a night room is as bright as daylight (to decide; not
  met by realistic content).
- #369 ✅, #370 ✅: the flat lamps.
- #371: the port unstable in the bedroom.
- #372: the Bistro at night's lamps.
- #373: the Bistro by day in clay, Forge's path over Falcor's once both are diffuse alone.

## Next, by the plan

- #373: split by sun and sky this morning, Forge's path is 8 to 10 % over Falcor's under each
  alone (the sun +0.19, the sky +0.13 in Exterior_1). Ruled out: the sun's cut, the file's header,
  Falcor's half floats, the unit, the leaves, the clay albedo (0.496). The material chart under the
  Bistro's sun alone matches (0.058): it is the Bistro scene itself, its walls +10 % under the sun.
  Next: the walls' normals and geometry as Falcor's FBX and Forge's glTF hold them.
- pbrt-v4's bedroom lets its panels through the curtains (its folds' shading normals): the
  kitchen and the classroom are its fair rooms.
- After your D-090 choice: SHaRC as the default where chosen, then NRC on the same hook, then the
  demos updated and tested.
- The materials' gap (Exterior_2, the textured rooms): Falcor's G-buffer albedos against
  `r.show.input=1`, view by view.

The sheets (each technique beside Falcor and Forge's path, with ratio maps) stay out of git:
they are renders of the Bistro, Bitterli's rooms and Falcor's images.

## #372, the same day (ReSTIR DI for every lamp, denoised)

Measured on the Bistro's night view as each tile's light over Forge's path tracer with every lamp
on (the plain mean of 144 tiles' ratios: the mean |log2| reads one sample's noise as darkness),
and a still camera's shimmer (frames 280 and 290, pixels differing by more than 2 levels):

| | shimmer | light | frame |
|---|---|---|---|
| the light lists, the 64 nearest (the default) | 4,892 px | 0.797 | 15.3 ms |
| the light lists, every lamp | 6,884 px | 0.987 | 21.9 ms |
| ReSTIR, undenoised, as it stood in the morning | (sparkling) | 0.775 | 15.1 ms |
| ReSTIR, both lights denoised, reuse with visibility (now) | 24,660 px | 0.905 | 19.0 ms |

- **Commits:** 055e927f (ReSTIR's light fixed, its diffuse light denoised), 45b7ab80 (the lanterns'
  profile, `--moonless`), b2e9eaec and e8561c4e (the highlight denoised, the resolve's surface code
  made a function, a review's fixes), 0521a6f5 and d9482af2 (REBLUR's history, the reuse with
  visibility). All behind `--lamp-restir --all-lamps`; the default is unchanged.
- **Faults found in ReSTIR's bridge:** the flat lamps not sampled as rectangles; RTXDI's permutation
  sampling (0.2 stops dark); shadow rays running into the lanterns' glass; RTXDI's pairwise reuse
  reading an off-screen reservoir (the device lost); and RTXDI's naive discount, which kept every
  pixel's history at one sample and halved the light under `Basic`.
- **Not the default yet:** it shimmers four times the lists' (the denoised light still moves at the
  pixel on the cobbles, the basket's leaves, the wires). Turning, it keeps its light (0.906 of the
  lists). Next: the lists for the nearest lamps and ReSTIR for the rest.
- **The lanterns' profile** (`Emission::Prism`, their glass's shares up and down) was right but
  small: the lamp model stays 13 % over the traced glow, its cause still open (not the lamps'
  intensities: each kind's lumens match its glass's).
- **The Moon** is 0.4 % of the night view's light; `--moonless` exists, and gi-compare's nights use
  it.

