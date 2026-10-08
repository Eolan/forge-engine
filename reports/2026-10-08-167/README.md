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
