# #180: the tropical island's slime, jelly seen through (2026-10-08)

`physics-lab --lab creatures --fixed-step`, 1600 × 900 (`docs/demos/physics-lab.md`, "The
tropical island's slime"; D-051 for the jelly class). The owner asked for the slime "more
jelly-like, translucent with a face", then for the tropical island's slime with its eyes: a squat
drop of mint jelly, a darker nucleus inside, two tall glossy eyes.

`slime.png`, left to right, top to bottom: frames 60 (sitting), 91 (the top of its first hop),
109 (landed), and frame 60 from 1 m before it (`--view=-0.2,0.5,3.2,0,-12`; `close.png`
enlarged): the floor and the dogs' legs seen through it, the nucleus inside, the eyes set into
the surface.

**Costs** (600 ticks, balls every 50):
- the creatures' tick: 0.237 ms on average (p99 0.35–0.40 ms), against 0.180 ms with the ball;
  two runs to digest `0xd1b63c0b319cf978`;
- `shading/jelly`: 0.043 ms in the lab's view, 0.066 ms from 1 m; the frame 1.51 ms on the GPU.

**Left for later:** its shadow is a solid body's (the sun's shadow rays and SIGMA treat it as
opaque); what is seen through it is shaded plainly (its rows' colours, the textures' averages).

**Checks:** Tier 1: every image 0 px but #71's flake and the creatures' scene's eight on both
paths, which show the new slime (ꟻLIP means: `lab-creatures120` and its no-occlusion twin
0.015, `lab-creatures-limp240` 0.037, `lab-creatures-throw240` 0.058); the A/B pairs 0 px;
validation clean; 401 tests.
