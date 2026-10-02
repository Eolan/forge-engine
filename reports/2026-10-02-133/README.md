# #133: the rivers' worn stones (2026-10-02)

The owner, on #132's cobbles: a river's stones are broken chunks of varied shapes and sizes,
their edges worn round, mostly pebbles and cobbles, none giant beside the water. Before (left,
#132's round cobbles) and now (right), `island` (seed 7):
- **`zoom.png`:** a crop of a river's stones from close, gliding down to the water (`--view
  672,199.5,-1957,-88.7,-20 --dolly 10`).
- **`worn.png`**, by rows:
  - 3 m above the water (`--view 672,200,-1957,-88.7,-25`), ꟻLIP mean 0.0108;
  - just above it (`--view 672,198.2,-1957,-88.7,-12`), 0.0114;
  - the `valley` shot, 0.0438: its stones and the water over and around them. The large stone
    in the middle holds up a step (#122); those keep their size.

**Shapes** (`StoneShape::Worn`): a rounded box cut by four to seven planes, the cuts' edges
rounded by a smooth minimum (Inigo Quilez's), a faint noise on the faces. Five granite and three
limestone chunks, flat, blocky or long, and a round pebble of each rock.

**Sizes:** in the water, one stone in four is a boulder breaking the surface (0.25–0.85 m in
radius) and the rest pebbles and cobbles of 0.08–0.38 m, mostly small; beside the water,
0.08–0.33 m instead of 0.3–1.1. 4 552 stones in the water (3 866 breaking it) against 4 216
(4 059); 991 beside it.

**Checks:** the first change checked by tiers. Tier 0, as it is a CPU-only change to the
island's props and procgen:
- **The gate:** fmt, clippy `-D warnings`, 261 tests; the credits are up to date.
- **The sentinels** (meshlets, the ballad with its HDR output, the city and the gallery, mesh
  path), against #132's batch: 0 px.
- **The island's captures**, recooked, against #132's: the views by 175–197 px (ꟻLIP mean
  ≤ 0.0003), the shots by 33–842 px (≤ 0.0009) apart from the `valley` (232 577 px, 0.0438).
  Only the rivers change.
- **The A/B harness** (occlusion off) on the meshlets, the ballad, the city and the island,
  and the island streamed against resident: 0 px.
- **Left for Tier 2:** the fallback path (the change feeds both paths the same data), the
  validation layers (no GPU code changed), timings (no pass changed; 336 more stones, small ones).
