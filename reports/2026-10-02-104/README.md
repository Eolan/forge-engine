# #104: the transfer wait of frames without streaming uploads (2026-10-02)

Carried over from #95. A frame without streaming copies still made a graphics batch wait for
the transfer queue's last copy, which the frames before it had waited for already. The wait
cost nothing on the GPU's side, since that value had long been signalled, but it split a
batch: one more submission per frame. D-020, "Waits made once", has the rule.

## What the frame did

The city's flight (`FORGE_GRAPH_LOG=1 city-blocks --fly`). The streaming copies run every
few hundred frames, on the transfer queue, and the culls read the pool and the page table
they fill. A frame without a copy, before:

```
batch 0 on compute  (signals 2020, waits for graphics 6057 at COMPUTE_SHADER)
batch 1 on graphics (signals 6058)
  geometry/cull clears, geometry/cell cull, geometry/instance cull
batch 2 on graphics (signals 6059, waits for transfer 37 at COMPUTE_SHADER | MESH_SHADER_EXT)
  geometry/cluster cull 1 (visible last frame) … shading/standard
batch 3 on graphics (signals 6060, waits for compute 2020 at COMPUTE_SHADER,
                     waits for transfer 37 at COMPUTE_SHADER)
  shading/standard … the end of the frame
```

Transfer 37 was the copy of an earlier frame. That frame's graphics batches had waited for
it, and so had every frame since. The pool remembers its last writer from frame to frame
(D-020), so each read of it asked for the wait again.

## The rule

With `vkQueueSubmit2`, a semaphore wait's second synchronization scope is every command of
its batch "and all commands that occur later in submission order", limited to the wait's
stages ([vkQueueSubmit2](https://docs.vulkan.org/refpages/latest/refpages/source/vkQueueSubmit2.html)).
So a wait that an earlier batch of the same queue made at the same stages covers the later
batch.

The graph now keeps, per queue, per queue waited for and per stage, the largest value waited
for (`Waited` in `crates/forge-gpu/src/graph.rs`), from frame to frame. It drops an access's
wait when every one of its stages, or `ALL_COMMANDS`, already waited for that value or a
later one. The record is kept only once the frame is recorded, like the resources' states.
The frame's last batch still waits for every other queue's last batch of the frame, so a
frame slot is free when its graphics work is.

The same frame, after:

```
batch 0 on compute  (signals 2016, waits for graphics 4070 at COMPUTE_SHADER)
batch 1 on graphics (signals 4071)
  geometry/cull clears … shading/standard
batch 2 on graphics (signals 4072, waits for compute 2016 at COMPUTE_SHADER)
  shading/standard … the end of the frame
```

A frame with a copy keeps its five batches. Its last batch now waits for the copy at every
stage, as the frame's end. Before, it waited at the compute stage only, for the shading's
reads of the pool, which the cull's batch had already waited for.

## Cost

The flight, five alternating runs of 6 000 frames per build (the exit log's `gpu:` and
`cpu:` lines):

| | before | after |
|---|---|---|
| GPU frame | 1.873–1.881 ms | 1.870–1.873 ms (run 1: 1.890) |
| CPU submit + present | 0.158–0.162 ms | 0.149–0.150 ms (run 1: 0.164) |
| CPU graph compile | 0.040–0.044 ms | 0.043–0.044 ms |

The first run of the new build was an outlier on both lines. A submission saves about
0.01 ms of CPU, as the issue expected. On the GPU the frame moves by less than the noise.
That fits: the wait was long signalled, so the GPU never stalled on it.

`tools/timings.sh`, three runs each, alternating:

| View | before | after |
|---|---|---|
| city | 1.798–1.805 ms | 1.799–1.806 ms |
| city orbit | 2.316–2.333 | 2.324–2.331 |
| city fly | 1.876–1.882 | 1.872 (×3) |
| city resident | 1.740–1.745 | 1.742–1.747 |
| island | 1.602–1.605 | 1.603–1.606 |
| meshlets | 0.223–0.225 | 0.225–0.226 |
| meshlets orbit | 0.141–0.143 | 0.141–0.142 |
| meshlets side 700 | 1.327–1.334 | 1.331–1.333 |
| ballad 900p | 1.281–1.287 | 1.272–1.283 |
| ballad 1440p | 2.620–2.627 | 2.610–2.631 |

Only the flight streams after its start, and only it moves.

## Checks

- **Tests:** 248, two new. One is the issue's case over two frames: the copy, then a frame
  without one, where the cull joins the batch before it and nothing waits for the copy. The
  other checks that a wait at another stage is still made: a fragment shader's read after
  a wait at the compute stage. Clippy with `-D warnings` and fmt pass.
- **Synchronization validation:** the flight, 600 frames under the layer with
  synchronization validation, on the mesh path (two copies) and the fallback (three), and
  with `FORGE_ASYNC=0`: no messages. `tools/validate.sh` now has that run (`city-fly`, 3 s
  per path), the first of its runs to cross the transfer queue after the start. The rest of
  `validate.sh` is silent.
- **Captures:** the batch against #125's final one (the same code but this change): 0 px on
  every line, the culling harness and the mesh path against the fallback included.
