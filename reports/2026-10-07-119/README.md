# #119's polish: a bar of sand past each confluence (2026-10-07)

Past a junction's downstream corner the river drops sand in the slack water on the tributary's
side, and a bar of the mouths' teardrop shape now lies there against the bank
(`city-blocks --island 7`, `docs/demos/island.md`, "The bar past the corner"; `--no-confluence-bars`
for the A/B). Frame 60 with the fixed step, 1600 × 900.

- `confluence-bars.png`, without the bars (left) and with them (right):
  - the largest (81 m) from 25 m back up the river, 10 m over the water
    (`4275,10.1,-2860,-78.9,-20`): 45 924 pixels changed, ꟻLIP mean 0.010;
  - the second (38 m) from 25 m back (`26,11.9,4420,-153.6,-20`): 37 058, 0.009;
  - the largest from straight above (`4323,162.8,-2869,0,-89`): 109 643, 0.021.
- Also measured: the second from above (`42,77.5,4453,0,-89`) 27 349 px, ꟻLIP mean 0.011; the
  logged confluence (`4262,8.5,-2834,-134.8,-20`) 16 568, 0.006.

**Tried and changed:**
- The deltas' sand (`LAKE_SAND`) read as a dark brown stain in the sun; the bars take the beach's
  sand, as the mouths' do.
- Painted over its whole outline, a bar's sand cut straight-edged notches into the grass where
  the bank stands higher than the bar; it is painted only where the bar is the ground.
