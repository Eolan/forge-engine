# The night of 28–29 October 2026

Your plan for the night:
1. The night check now.
2. Then #356 and #360.
3. Then the FSR 4 module.

All three are done and pushed. The details follow.

## 1. The night check, run early

- **When:** it ran at 00:31 as a Tier 2 on 55db91c3. The scheduled task now starts at 01:30 on 30 October, so it didn't run twice.
- **Result:** it failed, for two expected reasons:
  - **meshoptimizer 1.3** changed images outside the sentinels. They were accepted afterwards.
  - **80 Streamline "Cannot have undefined format" errors** came from the scenes that tag a reactive mask (smoke, flood, island). That tag went in with e3b1c422.
- **The rest held:** every A/B pair was 0 px, with 0 VUID.
- **The fix:** #360 (279468fb) removes the tag.
- **The rerun:** a Tier 1 at 279468fb, expecting the images to change, PASSED in 5,176 s, with 1,101 tests, 0 VUID and 0 Streamline errors. 262 images were accepted.

## 2. #360: DLSS takes no reactive mask (279468fb)

- **Why:** NVIDIA's guide (section 3.15) says only the old Preset F reads the bias hint, and it is not to be given to the current models. That is why the hint changed nothing under DLAA.
- **What changed:** e3b1c422's tag is reverted. FSR 3.1 still reads the mask.
- **Still open:** part (b), the particles' own motion vectors, as you said, "later".

## 3. #356: far rooms follow the hour (5eef68b3, closes #356)

Each far room with windows now takes its windows' sky and sun over its surfaces, times K. This runs on the GPU each frame ("gi/far rooms", 0.008 ms). Rooms without windows keep the fixed 0.2.

The far rooms' windows on #354's dolly, against grids carried out to 150 m:

| | 07:12 | noon | 15:36 | 18:00 | 20:24 |
|---|---|---|---|---|---|
| Fixed share 0.2 (before) | +0.08 EV | +0.19 | +0.01 | −0.14 | 0.00 |
| **K = 2.7 (the village now)** | **+0.04** | **+0.01** | **−0.07** | **−0.15** | **+0.01** |

- **Effect:** noon's excess is gone, and the spread over the day drops from 0.33 to 0.18 EV.
- **Still there:** the 18:00 deficit. Presumably that is the sun's patch on the floor, which the grids see and the share doesn't.
- **Cost:** +0.03 ms on the frame.
- **The map check:** the far windows moved by ꟻLIP ≤ 0.0044; accepted.
- **A bug found on the way:** the first calibration round silently measured the old path. The village hands its rooms over through `set_regions`, which didn't make the new buffers. That is fixed, with a GPU test.

## 4. The FSR 4 module (52039ae2, D-082)

AMD's signed DLLs from the FSR SDK 2.4.0 are now loaded at run time, under `r.aa=fsr`:

| `--fsr-amd` | What it does |
|---|---|
| `auto` (default) | On AMD cards only; takes the DLLs only where they offer FSR 4 |
| `on` | Takes them wherever they load |
| `off` | Never |

Everywhere else, our own FSR 3.1 runs.

**On the 5070 Ti** (`on`, so their FSR 3.1.6):
- 0.288 ms, against our FSR's 0.317.
- ꟻLIP mean 0.015 against our image.
- No VUID. The layer only warns about AMD's own internal images inside the DLLs.

**On the RDNA 2 iGPU:**
- The DLLs offer 3.1.6 and 2.3.4 only, so `auto` keeps our FSR 3.1.
- Forced `on`, theirs takes 14.2 ms against our 18.7.

**FSR 4 itself is untested:** we have no RX 6000–9000 card (#67).

## 5. A review of tonight's code (539bea9e)

A read-only review of #356 and D-082 found 8 issues. All are fixed and checked:
- **A refused frame:** if the DLLs refuse a frame, we now fall back to our FSR instead of failing it.
- **Device features:** the ones AMD's own framework enables for FSR 4 are on where present. The sentinels are unchanged at 0 px.
- **AMD's own version choice** is kept. Overriding it would opt out of driver updates.
- **A switch from the console** to `r.aa fsr` now loads the DLLs too.
- **Rooms of several volumes:** window facing and shared walls are corrected. The village's rooms are single volumes, so its frames are bit-identical.
- **Smaller ones:** the far table is no longer rebuilt every frame, and a missing graph declaration in the particles' pass is fixed.

## Earlier in the evening: dependencies (#361)

- **The crates:** the 35 compatible updates, glam 0.34, and the latest of everything else. ash now comes from its master branch, with gpu-allocator vendored.
- **meshoptimizer 1.3**, which you checked: the cook version is 6, and a test now ties it to the meshopt version.
- **i_overlay 9.1,** with a fix for spikes left in the streets' booleans.
- **Slang 2026.20** through `FORGE_SLANGC`.
- **A weekly check:** `tools/deps-check.sh`, Wednesdays at 22:00.

## Open, and for you

- **FSR 4:**
  - It needs an AMD RX 6000–9000 to test (#67).
  - AMD's ML-upscaling patents need a look before a release ships their DLLs. The SDK is MIT, which grants no patent licence.
- **AMD's DLL warnings:** they write their own 16-bit images through 32-bit format declarations. This could be reported to AMD, but only on your go, since it means posting outside the repository.
- **#334:** waits for your walk. The step at the grid's edge is now 0.01–0.15 EV, depending on the hour.
- **#71's HDR-PQ flake:** about 108k px at up to 44 codes (HDR-FLIP mean 0.0045). That is inside its known signature (about 100k codes in the dark, mean ≤ 0.008), so no action.
- **#362, meshoptimizer 1.3's options:** two more were measured tonight, and neither is taken.
  - **`PreserveFolds`:** 3.6 % more triangles in the walk view and up to 0.09 ms more GPU time. Nothing changed visibly; I compared the moved band side by side at twice the size.
  - **`ErrorClamped`:** no change in the village. It halves the glTF models' reported errors (Sponza's root error 58 → 23), so those would switch LOD earlier: fewer triangles, possibly pops. Testing that needs a scene that shows the models at a distance, in motion. The lab's Sponza view from inside the atrium couldn't show it.
- **The streets' slicing:** its results feed triangulation without despiking. There is no failing case, so I left it.
