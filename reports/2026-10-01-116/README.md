# #116: floors, benches and floodplains in the island's valleys (2026-10-01)

Before (left, `--no-valleys`: the field as eroded, k = 3) and now (right), frame 60 with the
fixed step:
- `valleys-hills.png`:
  - the logged slot (`--view -3083,48.3,-376,123.7,-10`) and from 40 m higher (`-45°`);
  - a bench in the highlands at 284 m (`--view -613,287,2510,144,-10`) and from 40 m.
- `valleys-lowland.png`:
  - down the lowland river (`--view -929,14.8,-4399,83,-10`) and from 40 m;
  - up the steep river (`--view -1112,80.1,-3083,-97,4`).

**What changed.** The valleys are carved into the 8 m field before the rivers are traced for
their water (`forge_procgen::carve_valleys`, `docs/demos/island.md`, "The rivers' valleys"):
- **Reach types.** By the water's fall over 40 m either way:
  - over 4 %, room for the water (its half width plus 2 m);
  - 2–4 %, a bench one and a half widths past it;
  - under 2 %, a floodplain three widths either side.
- **Floors.** A side's floor stops where the ground stands 6 m over it. The floor stands 0.3 m
  plus half the depth over the water.
- **Walls.** They rise to it at most a little steeper than the ground beyond.
- **Protections.** Lakes keep their rims, and the mouths their beaches.

**Numbers (seed 7).**
- **Points by type:** 9 600 floodplain (most on the plain), 1 470 bench, 6 578 room only.
- **The gentler hill reaches** ask for a floor 22.4 m out on average and get 21.4 m. Their
  valleys were open already: the ground at the floor's edge stood 4.4 m over it at most.
- **The logged slot** falls 14 % (5–23 %): a steep reach, which keeps D-041's V with room for its
  water. What darkens it is the sun's shadow in a narrow valley.
- **Lakes and mouths:** 11 lakes, 19 mouths and their falls are unchanged.
- **The water under its banks:** 602 → 125 points more than 2 m under the lower bank.
- **Cost:** the carve takes 0.57 s at start, and the frames move by under 0.05 ms (`docs/PROFILE.md`).

| View | ꟻLIP mean |
|---|---|
| slot, 3 m | 0.088 |
| slot, 40 m | 0.071 |
| bench, 3 m | 0.122 |
| bench, 40 m | 0.118 |
| lowland, 3 m | 0.096 |
| lowland, 40 m | 0.043 |
| up the steep river | 0.102 |

The capture batch changes the island's two views only (`island60` 0.0040, `water60` 0.0052).
The A/B harness and mesh-against-fallback stay at 0 px, and `validate.sh` is clean.
