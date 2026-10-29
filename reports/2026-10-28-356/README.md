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

## Built: the far rooms' daylight (2026-10-29)

On the owner's go (2026-10-29) the far rooms take their windows' daylight, following the hour
(D-078's addendum, `ProbeParams::far_room_daylight`, "gi/far rooms"): each room with windows its
own share, K × its windows' sky and sun over its surfaces. Measured as above, against the grids
out to 150 m:

| K | 0.3 | 0.5 (noon) | 0.65 | 0.75 | 0.85 |
|---|---|---|---|---|---|
| fixed share 0.2 | +0.08 EV | +0.19 EV | +0.01 EV | −0.14 EV | 0.00 EV |
| 1 | −0.59 | −0.60 | −0.68 | −0.75 | −0.60 |
| 1.57 | −0.34 | −0.35 | −0.43 | −0.52 | −0.36 |
| **2.7 (the village's)** | **+0.04** | **+0.01** | **−0.07** | **−0.15** | **+0.01** |

- The cost: "gi/far rooms" 0.008 ms; the dolly's frame 5.571 → 5.600 ms at noon and 5.611 →
  5.649 at 0.75.
- The deficit at 0.75 (15:00) is still there: presumably the sun's patch on the floor and its
  bounce, which the grids take and the share, the sun on the windows only, does not.
- **A separate weight for the sun, tried (scratch, not kept):**
  - With the sun's term off (K 2.7), the windows read −0.52, −0.42, −0.58, −0.73 and −0.62 EV at the five hours. The sun is most of the share.
  - Least squares over the five hours, with the sky and the sun weighted apart (sky 2.3, sun 3.2), would take 0.75 (15:00) only from −0.15 to about −0.10 EV.
  - It would make 0.3 and 0.85 (09:36 and 16:12) worse (+0.07 and +0.05).
  - The two terms move together through the day, so the 15:00 deficit is not a matter of their balance.
