# #94: the HDR output (2026-10-02)

The ballad's HDR output through ACES 2.0 at 1000 nits, P3-D65 limited, with the Academy's
look (no paper-white offset: the owner's pick). Drawn off-screen in HDR10 and previewed
(`asteroids --tonemap aces2 --hdr offscreen`), frame 600 with the fixed step, 1600 × 900.

`hdr.png`, left to right:
- **SDR**: ACES 2.0's 100-nit output, as before.
- **The HDR preview**: what an SDR display at the UI's white (203 nits here, BT.2408's, since
  Windows has this monitor in SDR) shows of the HDR10 image. Below paper white it is the HDR
  image. It is darker than the SDR frame: at 1000 nits ACES 2.0 puts grey at 14.5 nits and a
  scene white at 107, under the 203-nit white. This is the Academy's look against the desktop
  that the decision accepted (the paper-white offset, 0 stops by default, is what would lift
  it). Against SDR: ꟻLIP mean 0.148, as expected of a darker image.
- **False colours** (F4, `FORGE_HDR_FALSE_COLOURS=1`): grey below paper white, yellow above it,
  red at the peak, blue outside Rec.709. Only the sun's glow goes above 203 nits, and its disc
  reaches the 1000-nit peak. The lit ice and rock stay under paper white.

**Accuracy:**
- **The HDR table against the per-pixel transform** (CPU, 400 000 colours, 10-bit PQ codes):

  | Peak | p50 | p99 | p99.9 | max |
  |---|---|---|---|---|
  | 500 nits | 0.18 | 1.17 | 10.5 | 26.9 |
  | 1000 nits | 0.21 | 1.29 | 15.2 | 38.0 |
  | 2000 nits | 0.23 | 1.56 | 9.3 | 29.2 |
  | 4000 nits | 0.25 | 1.84 | 13.8 | 65.5 |

  The SDR table is at p99 1.5 and max 15 in 8-bit sRGB codes. A 129³ table would bring the
  1000-nit p99 to 0.48, for 7 times the bake and 17 MB; 65³ stays.
- **`meshlets --tone-check`**: the GPU's per-pixel HDR transform is within 0.013 10-bit codes
  of the CPU's, its table within 0.10 of the CPU's reading of the same table (SDR: 0.002 and
  0.14 8-bit codes, as before).

**Cost** (the ballad's flight, 600 frames, three alternating rounds; RTX 5070 Ti):

| | 1600 × 900 | 2560 × 1440 |
|---|---|---|
| `temporal/TAA resolve`, SDR | 0.056–0.057 ms | 0.173–0.177 ms |
| `temporal/TAA resolve`, HDR10 | 0.060 ms | 0.182–0.187 ms |
| `post/hdr preview` (off-screen only) | 0.014 ms | 0.038–0.042 ms |
| the frame, SDR | 1.264–1.274 ms | 2.616–2.648 ms |
| the frame, off-screen HDR10 | 1.271–1.282 ms | 2.659–2.714 ms |

On an HDR display the cost is the resolve's 0.004–0.010 ms: the PQ encoding and its dither.
The preview exists only in the off-screen mode. The table is baked once per process,
10–15 ms.

**Checks:**
- The capture batch is unchanged (0 px on every line). It now holds the ballad's HDR output at
  frames 240 and 600 on both paths, preview and PQ codes (`-pq.png`, 16 bits, compared to the
  code). The mesh path and the fallback give the same codes.
- `validate.sh` is clean on 28 runs, among them the HDR output from the TAA resolve and from
  the display pass. With synchronization validation, so is HDR switched on and off every 25
  frames (`FORGE_HDR_CYCLE`, as F2 does), now one of `validate.sh`'s runs.
- Tests, clippy and fmt pass.

**Not verified here:** the present on an HDR display. Windows shows the demos' monitor (the
secondary) in SDR; it reports HDR support, a 270-nit peak and an 80-nit SDR white. With "Use HDR" on,
`asteroids --tonemap aces2 --hdr hdr10` (or F2) presents HDR10. Without it, the demo says so
and stays SDR.
