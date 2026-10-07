# #177: barrels afloat (2026-10-08)

`city-blocks --island 7 --movers N`: the barrels are Jolt bodies on the island's water
(`docs/demos/island.md`, "Barrels afloat"). Seed 7, `--movers 40`, the fixed step, 1600 × 900,
the log's views (`barrels afloat on the rivers (--movers)`).

`barrels.png`, by rows:
- the first barrel at frame 60 by a step (`245.8,277.34,-2033.9,-29.5,-20.6`), and the moored
  barrel at frame 60, still carried with the stream (`181.4,16.64,3911.5,-14.0,-48.5`);
- the moored barrel at frame 1200, its rope holding it and the stream parting round it, and the
  towed barrel at frame 300 (`2168.1,37.05,-1234.1,-104.0,-36.6`; `towed.png` enlarged);
- the dropped barrel meeting the lake at frame 164, and bobbing in its rings at frame 200
  (`2160.0,30.55,-1234.0,0.0,-8.5`).

**The tick** (the CPU, the fixed step):

| Barrels | Mean | p99 | Started again |
|---|---|---|---|
| 41 | 0.17 ms | 0.26 ms | 0 in 10 s |
| 101 | 0.23 ms | 0.37 ms | 0 in 60 s |
| 1 001 | 1.10 ms | 1.49 ms | 22 in 60 s |
| 10 001 | 13.6 ms | 28.9 ms | 2 in 10 s |

The ground: 71 height-field tiles of 64 × 64 samples, made in 54 ms.
