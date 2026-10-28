# #356: the far rooms' light by the hour, measured (2026-10-28)

The village's rooms past the room grids' reach (20 m) take a fixed share of the open sky
(`far_room_sky` 0.2). Walking towards a house, its windows step as its grids come within reach.

**How measured.**
- The #354 dolly: `--view=16.81,21.86,-14.47,11.3,-24.1 --dolly=5`, frame 372.
- The reference: grids out to 150 m (`--probe-rooms 150 --probe-room-pool 32768`).
- Over the far rooms' openings only: the pixels where `--probe-far-sky 0` differs from today's
  frame by more than 0.15 EV.
- Mean log2 luminance, display-referred.

**The fixed share against the reference, by hour** (positive: too bright):

| Hour | 0.3 | 0.5 (noon) | 0.65 | 0.75 | 0.85 |
|---|---|---|---|---|---|
| Error | +0.06 EV | +0.18 / +0.19 EV | 0.00 EV | −0.15 EV | −0.01 EV |

**Options measured** (the error at noon / at 0.75; the cost against today):

| Option | Error | Cost |
|---|---|---|
| Today: grids to 20 m, pool 4,096 | +0.19 / −0.15 EV | — |
| Grids to 40 m (`--probe-rooms 40 --probe-room-pool 8192`) | not measured | +0.2–0.7 ms over the map check's 13 views |
| Grids to 60 m (pool 16,384) | +0.01 / +0.005 EV | +0.5–1.2 ms over the 13 views |
| Grids to 100 m (pool 16,384) | +0.01 / +0.004 EV | +1.6–1.7 ms at the dolly; the pool overflows |
| Coarse grids for every room to 150 m (`--probe-room-spacing 10,10`) | −0.97 / −0.22 EV | +0.4 ms: too few probes for a room's light |
| A far tier (`far-room-grids.patch`) | see below | +0.45 ms at the dolly |

The far tier (`far-room-grids.patch`):
- Rooms from 20 to 60 m take grids at the same 1 m spacing.
- Those grids update one frame in 4 or in 3 (`probe_due`), at 64 or 128 rays.
- Its error: −0.11 / −0.13 EV (4 turns, 64 rays), −0.10 / −0.08 EV (3 turns, 64 rays) and −0.11 / −0.12 EV (3 turns, 128 rays).
- The darkness follows the slower updates, not the rays; not found why.
- Its cost at the dolly is about +0.45 ms: probe rays +0.20, blend +0.09, and the resolve's grid lookups +0.15.

**For the owner:** keep 20 m, or pay 0.2–1.2 ms for the step to move to 40–60 m (exact at 60).
`--probe-room-pool N` is kept to try them.
