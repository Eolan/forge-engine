# #179: the slime as a soft body (2026-10-08)

`physics-lab --lab creatures --fixed-step`, 1600 × 900 (`docs/demos/physics-lab.md`, "The
slime"). A Jolt soft body of 258 points before the dogs, hopping every 75 ticks, drawn as a
skinned mesh whose joints are its points.

`slime.png`, left to right, top to bottom: frames 60 (sitting, 0.49 m of its 0.6 m), 91 (the top
of its first hop), 109 (landed) and 166 (its second hop, away from the camera).

**Costs** (600 ticks, balls thrown every 50, two runs to digest `0x71bda706b9022ab9`):
- the tick: 0.180 ms on average (p99 0.27–0.30 ms, max 0.44 ms), against 0.126 ms without it;
- `skin/vertices` 0.006 ms, `skin/blas` 0.105 ms for the six refits, the frame 1.44 ms on the
  GPU against 1.41 ms.

**Checks:** Tier 0 (lab, island, city; the mesh path): every capture 0 px but the four of the
creatures' scene, which now show the slime (ꟻLIP means: `lab-creatures120` and its no-occlusion
twin 0.019, `lab-creatures-throw240` 0.040, `lab-creatures-limp240` 0.026); the occlusion A/B
pair 0 px; 400 tests.
