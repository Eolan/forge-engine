# #120: where rivers meet lakes (2026-10-01)

Before (left, 191a295) and now (right), frame 60 with the fixed step, `--island 7` drawn at 2 m:
- `into-lakes.png`:
  - the largest river into the plain's lake (`--view -1655,23.4,-3082,94.1,-20`), and from
    70 m straight down (`--view -1695,81,-3080,0,-89`);
  - a hill river into its lake (`--view -465,308.2,1917,-127.4,-20`), and from above
    (`--view -434,366,1941,0,-89`).
- `out-of-lakes.png`:
  - the plain's lake out into its river (`--view -2272,23.4,-3474,179.5,-20`), and from above
    (`--view -2272,81,-3434,0,-89`);
  - a hill lake out into its river (`--view 934,366.5,-1741,162.7,-20`), and from above
    (`--view 922,425,-1703,0,-89`);
  - a second hill outlet from above (`--view -324,366,2269,0,-89`).

The demo logs these junctions as `rivers into and out of the lakes (--view)`.

**What it was.** Every junction was measured by the lake's 8 m samples, not by its water:
- the channel stopped at the first segment with both ends under the lake's mask, which reaches a
  sample past the depression, so it ended in a round cap on the shore, short of the water;
- the river's water faded out three points before the lake's deeper cells, and on the shallow
  margin its level went under the lake's, a freeboard under its banks;
- out of a lake the lowest ground beside the river was the lake's own bed, so the river fell 0.3
  to 0.8 m at once, and the mask carried the lake's plane a sample past the lip, over it.

**What changed** (`docs/demos/island.md`, "Where rivers meet lakes"):
- **The lake's edge is where its water stands** over a point's nearest sample: there the river
  is at the lake's level (a centimetre over it, so it draws over the lake's water as it fades),
  never under it upstream, its freeboard gone towards it, and beside a lake its level, not its
  bed, holds the river's water up.
- **The lake takes the river on where its water stands half as deep as the river:** in a shallower
  flat the river runs on in its channel at the lake's level. Its water is whole to there and
  fades over three points; out of a lake, the reverse.
- **Out of a lake** the river keeps the lake's level a sample past the lip, then falls 5 % a metre
  at most, never over the lowest ground across it.
- **The channel** shoals and flattens its banks into the lake's edge, runs on into the lake as far
  and fades out: no hollow.
- **The lake's mask** grows only where the ground rises through the level, not past the outlet.
- **An outlet is a mouth too:** the lake's water in a cone back from it takes the river's colour,
  bed and ripples, drawn towards the outlet.

| View | ꟻLIP mean |
|---|---|
| into the plain's lake | 0.034 |
| into the plain's lake, from above | 0.040 |
| into a hill lake | 0.119 |
| into a hill lake, from above | 0.051 |
| out of the plain's lake | 0.128 |
| out of the plain's lake, from above | 0.065 |
| out of a hill lake | 0.044 |
| out of a hill lake, from above | 0.018 |
| a second hill outlet, from above | 0.067 |

**Numbers:** 12 runs in lakes (11 entries before); 20 lake mouths with the outlets; 86 875 cells of
8 m refined (86 662), 1 390 000 at 2 m (1 386 592). The valleys' carve (#116) keeps 1 084 samples by
the lakes' guard (2 449): its rivers stand at their lakes' levels.

**Checks:** the capture batch changes the island's images only (`island60` ꟻLIP mean 0.0025,
`water60` 0.0033, `island8-60` 0.0026) and #71's flake; the A/B harness, the streamed island against
resident and mesh against fallback stay at 0 px; `validate.sh` is clean; 213 tests (a river handed
through a bowl's lake: its levels, its fade and its channel), clippy, fmt.

**Left for later:**
- On the plain's flat lake, centimetres deep over a kilometre, the lake's mask still ends across the
  outlet's channel in a soft straight edge along its samples; the outlet's mouth softens it.
- Past a hill lake's lip, a pale patch of thin water on one bank: the river's water over a bank lower
  than it at the mask's soft edge (a similar patch was there before, by the old cap).
- D-041's lake entry also widens the channel and paints a fan on the lake's floor: not done.
