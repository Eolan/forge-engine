# #167: the creatures' clips through their motors (2026-10-08)

`physics-lab --lab creatures --fixed-step`, 1600 × 900 (`docs/demos/physics-lab.md`, "The clips
through the motors"). Each creature idles 6 s and walks 6 s in turns, at its own phase; each
switch dies away over 0.4 s (inertialized).

`walk.png`: the first mannequin on its stand from the side (`--view=-5.2,1.0,0.0,-90,-3`) at
frames 555, 570, 585 and 600 (9.25 to 10 s, a quarter of its 1 s walk apart): the thighs swing,
the knees bend, the arms swing against the legs.

`dogs.png`: the lab at frames 120 (2 s, idling), 555 (9.25 s), 1 300 (21.7 s) and 2 100 (35 s).
The dogs walk circles about their spots (their torsos held upright and turned at 0.4 rad a
second while they walk), and the mannequins walk and idle on their stands.

**Tick:** 0.13 ms on average over 2 100 ticks, p99 0.22 ms (55 bodies awake, the clips sampled
and the targets made included).

## The dogs' feet on uneven ground: `--lab course`

`physics-lab --lab course --fixed-step` (`docs/demos/physics-lab.md`, "The dogs' feet on uneven
ground"). Two lanes: three steps of 5 cm up, a landing at 15 cm and three down; a 10° ramp up, a
landing and down. Each dog walks its lane, turns about in its idle, and walks it back.

`course.png`: ticks 450, 600, 750, 1 200, 1 350 and 1 500 from the default view: the ramp's dog
climbing and on its landing, turning about on it; the steps' dog on the top step, coming down;
both back on the floor turning to go again.

What it took (the first take, dropped earlier, had the paws catching on 8 cm edges and the dogs
thrown on 11° slopes):
- **The joint frames were wrong:** laid out as twist, plane, normal where Jolt's constraints use
  twist, plane × twist, plane. A leg's swing fore and aft came out sideways: the dogs walked
  crabwise, which their circles on the floor had hidden.
- **The walk clip moonwalked:** each paw was lifted while it went back and planted while it went
  forward (`assets/blender/skinned_creatures.py`, re-exported).
- **A knee flipped** past its hinge when the leg was nearly straight: `two_bone_toward` bends
  it towards a pole.
- **Swinging paws** (told by the clip moving them forward) are put over the rise ahead, 5 cm
  higher before it; the torso is guided over its lane at the walk's pace by springs (root motion,
  as games carry a ragdoll). On their legs alone the dogs stalled at the first step and the ramp.

On the floor the dogs now walk 0.7 m/s on their legs; in `--lab creatures` each is held on its
spot as it walks and turns.

**Ticks** (600, two runs each to the same digest): the course 0.095 ms (p99 0.16–0.17); the
creatures 0.285–0.291 ms (0.324 before).

**Tier 1** (`captures/verify/20261008-093954-4a4515e`, 406 tests): the creatures' images change
as expected (ꟻLIP mean 0.0098 at 120, 0.0101 thrown, 0.0303 limp, both paths); the course's
four are new, mesh against fallback 0 px; everything else 0 px but the #71 flake.
