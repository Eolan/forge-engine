# #107's polish: rings where the splashes' drops land (2026-10-07)

The drops a splash throws up leave rings where they fall back on still water: their impulse goes
into the foam's field, and the wakes turn it into rings (`city-blocks --island 7 --movers N`,
`docs/demos/island.md`, "Rings where the drops land"). The fixed step, 1600 × 900, 2 barrels, the
dropped barrel's logged view (`2160.0,30.55,-1234.0,0.0,-8.5`), cropped.

- `drop-rings.png`: frames 220 (top) and 240 (bottom), without the drop rings
  (`--no-drop-rings`, left) and with them (right).
  - The impact's rings stay concentric; where the spray rained back they are broken and rougher.
  - Frame 220: 20 817 pixels changed, ꟻLIP mean 0.0029, max 0.66.
  - Frame 240: 31 798 pixels changed, ꟻLIP mean 0.0047, max 0.63.
  - Four times higher, the drop rings broke the impact's rings into lumps (ꟻLIP mean 0.0112 at
    240), so they were lowered.

**Cost** (2560 × 1440, 1 000 barrels, two rounds of 1 500 frames): `wakes/rings` 0.030–0.032 ms
on the compute queue; the frame 4.251–4.257 → 4.275–4.279 ms.

**Checks:** the same capture twice, and on the serial frame (`FORGE_ASYNC=0`), is the same to the
pixel.
