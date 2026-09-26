# Research — Render graph, next: transient buffers and parallel recording of pass bodies

> Companion to D-020 (declared accesses, derived barriers, aliased transient images) and to its
> queues entry for #77 (`DECISIONS.md`), to `task-system.md` §F (the four render-graph sources the
> job system's research already carried) and to `gpu-geometry.md`'s "Research for issue #77".
> Written 2026-09-26 for issue #78, "Render graph: transient buffers and parallel recording of
> pass bodies", whose text is the brief: buffers that live for part of a frame, aliased in the
> transient heap the way the images already are (the meshlet scene's per-frame lists and its
> look-back status words are the named candidates); independent passes recorded on several
> threads through `forge-task`; and *measure first*, because the work only pays if recording shows
> in the F1 overlay's CPU zones, which it does not yet. Every citation was checked that day against
> a reachable page or, where the network proxy refused the host, against the search engine's record
> of it; the grade is kept per entry under [Verification notes](#verification-notes), and what was
> looked for and not found is under [Checked and left out](#checked-and-left-out).

The question is what the published render graphs, the Vulkan specification and the vendors say
about the two halves of #78, so that the order of work and the shape of the API are fixed before
any code: how a frame graph aliases *buffers* (every shipped graph aliases images; fewer alias
buffers, and one of the best-known refuses to), what the memory rules of Vulkan add when a buffer
shares a heap with images (`bufferImageGranularity`, the dependency between aliases, the fate of a
buffer's device address), what the packing algorithm can and cannot promise, how the engines that
record in parallel do it (a command buffer per pass or per chunk, a pool per thread per frame in
flight, barriers emitted into the pass's own buffer, one submission per queue in the original
order), what split barriers and events change and what they cannot (queues), and when any of this
pays on a frame whose CPU side is 0.25 ms out of 8.33. The short answer: transient buffers are
the same lifetime analysis and the same first-fit heap Forge already has for images, with two
additions (padding between linear and non-linear neighbours, and a poison mode, because neither
synchronization validation nor GPU-assisted validation can see a stale pointer into aliased
memory); parallel recording is a well-worn shape (Granite's task per physical pass is the closest
published code to Forge's) that Forge should build only behind the ROADMAP's condition, since the
whole of today's recording is 0.08–0.10 ms and a command buffer is not free on the GPU either.

> **State of the art in five sentences.** A frame graph owns its transient resources, computes
> each one's lifetime as the span of passes that touch it, and places the ones whose spans do not
> meet on the same memory, which Frostbite reported as a 40–50 % cut in transient VRAM and which
> Unreal's `IRHITransientResourceAllocator` applies to buffers and textures alike, while Granite
> aliases only identical images and states "Buffers are never transient" and Unity pools by
> descriptor rather than by offset. Vulkan's rules are three: aliases that are not both linear or
> both non-linear overlap only after padding to `bufferImageGranularity`, any overlapping use
> with a write "must be separated by a memory dependency … whether the aliases interpret memory
> consistently or not", and after the other alias writes, the contents are undefined, so the
> device address of an aliased buffer stays valid as an address and becomes garbage as data — a
> hazard the validation layers state they do not detect. Placing intervals of known size at
> offsets is dynamic storage allocation, NP-complete with constant-factor approximations, whose
> lower bound is the peak of live bytes over the frame and whose first-fit heuristics are within
> a small factor of it in practice. Every engine that records in parallel does the same four
> things: a command pool per thread per frame in flight, the frame's plan computed once before
> the jobs start, each job recording one pass or one chunk of consecutive passes with its barriers
> into its own primary command buffer, and the main thread submitting the buffers of a queue in
> the original order in one `vkQueueSubmit2`, with NVIDIA and AMD both warning that submissions
> cost a kernel call and every command buffer carries driver work of its own. Events split a
> barrier into a set and a wait so that unrelated work runs between them, cannot cross queues,
> and are the one primitive Granite uses for aliasing hand-offs, which is why its aliasing, like
> Forge's, stays on a single queue; across queues the timeline semaphore is the answer, and #77
> already has it.

**Contents**

1. [Transient buffers: lifetimes, aliasing, heap packing](#1-transient-buffers-lifetimes-aliasing-heap-packing)
2. [Parallel recording of pass bodies](#2-parallel-recording-of-pass-bodies)
3. [Split barriers, events and the queues](#3-split-barriers-events-and-the-queues)
4. [Measuring, and when it pays](#4-measuring-and-when-it-pays)
5. [What professional engines do](#what-professional-engines-do)
6. [Recommendation for Forge](#recommendation-for-forge)
7. [What the numbers say](#what-the-numbers-say)
8. [Checked and left out](#checked-and-left-out)
9. [Verification notes](#verification-notes)

---

## 1. Transient buffers: lifetimes, aliasing, heap packing

Forge's starting point (`crates/forge-gpu/src/graph.rs`): `FrameGraph::transient(TransientDesc)`
returns an `ImageHandle`; `RenderGraph::execute` computes, per transient image, the first and last
pass that declare it (`Request { desc, size, alignment, memory_type_bits, first, last }`), and
`plan()` sorts the requests largest first and puts each at the lowest offset where it overlaps no
request whose lifetime intersects its own; the heap is one `TransientHeap` (a `gpu-allocator`
allocation, `Device::create_transient_heap`), images are bound into it with
`Device::create_image_in(desc, heap, offset)`, the layout is cached across frames while it is
unchanged, `FORGE_GRAPH_NO_ALIAS=1` gives every transient its own memory and `FORGE_GRAPH_LOG=1`
prints the placement. The first use of a transient in a frame waits for whatever last touched its
memory, this frame or the previous one; an async pass may not use a transient. Buffers exist only
as persistent `GraphBuffer`s imported with `FrameGraph::import_buffer`, and every renderer keeps
`FRAMES_IN_FLIGHT` copies of its per-frame ones (`meshlet.rs`: `indirect`, `clusters`, `lookback`,
`deferred`, `cell_lists`, the `WorkList`s' `work`, `roots` and `lookback`, the visible `lists` and
the `rejects`, 512 KiB each at first and up to 8 MiB and 256 MiB per slot). The sources below say
how far the same machinery carries buffers.

**Yuriy O'Donnell (EA Frostbite). "FrameGraph: Extensible Rendering Architecture in Frostbite."
GDC 2017.** [talk] [foundational] [still-current]
<https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture-in> (slides:
<https://www.slideshare.net/DICEStudio/framegraph-extensible-rendering-architecture-in-frostbite>)

The talk that named the pattern: a graph of every pass and resource of the frame, built each frame,
compiled, executed, with "improved engine extensibility, simplified async compute, automated ESRAM
aliasing" and "tons of GPU memory" saved as its listed results. Its transient resource system
covers resources "alive for no longer than one frame", which the slides list as "buffers, depth and
color targets, and UAVs", and its implementation "depends on platform capabilities": "aliasing in
physical memory, aliasing in virtual memory, object pools, or an atomic linear allocator for
buffers". A third-party summary of the slides puts Battlefield 1's saving at 40–50 % of transient
VRAM; the slide itself was not re-read (the hosts are blocked).
*Bearing:* the first engine to do this treated buffers differently from textures: a bump allocator
reset per frame, not lifetime placement. Forge can do better because its plan is computed offline
per frame before anything records, so a buffer's lifetime is as exact as an image's; the atomic
linear allocator remains the right tool for the *upload* side (host-visible per-frame blocks),
which is not what #78 is about.

**Hans-Kristian Arntzen. "Render graphs and Vulkan — a deep dive." 2017; and Granite,
`renderer/render_graph.cpp`, `vulkan/device.cpp` (MIT).** [web] [code] [still-current]
<https://themaister.net/blog/2017/08/15/render-graphs-and-vulkan-a-deep-dive/> ·
<https://github.com/Themaister/Granite>

The code is explicit about buffers: in `build_physical_resources`, "Buffers are never transient.
Storage images are never transient." (only attachments get the internal transient bit). Aliasing
is decided in `build_aliases()` over pass ranges (`first_write_pass`, `last_write_pass`,
`first_read_pass`, `last_read_pass`): a resource cannot alias "If we read before we have
completely written to a resource" or when a subpass may not execute; two physical images alias
only if `physical_dimensions[i] == physical_dimensions[j]` (identical dimensions, so an alias is
a rename, not an offset), if their lifetimes are disjoint, and "Only … if the resources are used
in the same queue, this way we avoid introducing multi-queue shenanigans. We can only use events to
pass aliasing barriers. Also, only alias if we have one single queue." The hand-off between aliases
is an event (`physical_pass_transfer_ownership`: "Need to wait on this event before we can transfer
ownership to another alias"), with the layout reset to `UNDEFINED` and the note that a resource's
last use is normally a read, so nothing needs flushing. Persistent buffers are recreated only when
their size or usage changes (`setup_physical_buffer`).
*Bearing:* the most-read open render graph does not alias buffers at all and aliases images only
by renaming; Forge's offset heap is already the stronger scheme. Two of Granite's rules are worth
copying verbatim: aliasing only among resources of one queue (Forge's "no transient on an async
pass", D-020), and the read-before-write check (Forge's compile already errors on a transient read
before its write).

**Khronos Group. Vulkan Specification, "Memory Aliasing" (`resources-memory-aliasing`),
`VkImageCreateFlagBits`, `vkGetDeviceBufferMemoryRequirements`.** [spec] [still-current]
<https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#resources-memory-aliasing>
(source read: <https://github.com/KhronosGroup/Vulkan-Docs/blob/main/chapters/resources.adoc>)

The rules, from the source text. Two resources bound to overlapping ranges alias "the memory in
the intersection" if they are "both linear or both non-linear"; "If one resource is linear and the
other is non-linear, then the resources alias the memory in the intersection of paddedRangeA and
paddedRangeB", the ranges "aligned to bufferImageGranularity". "Use of an overlapping range by two
aliases must be separated by a memory dependency using the appropriate access types if at least
one of those uses performs writes, whether the aliases interpret memory consistently or not."
Aliases that do not interpret memory consistently see writes through the other alias "make the
contents of memory partially or completely undefined". `VK_IMAGE_CREATE_ALIAS_BIT` "specifies that
two images created with the same creation parameters and aliased to the same memory can interpret
the contents of the memory consistently with each other"; nothing of the kind exists or is needed
for buffers. Since Vulkan 1.3 (`maintenance4`), `vkGetDeviceBufferMemoryRequirements` answers the
requirements from a create-info alone, as `vkGetDeviceImageMemoryRequirements` does for images
(which Forge already uses in `Device::image_memory_requirements`).
*Bearing:* three consequences for a buffer in the image heap. (1) A buffer is a linear resource
and every transient image is non-linear (`OPTIMAL` tiling), so a buffer next to an image in the heap
needs the boundary padded to `bufferImageGranularity`, a limit read from the device (the plan's
overlap test compares padded ranges, or, simpler, buffer requests get `alignment = max(alignment,
granularity)` and a size rounded up to it). (2) The barrier the graph already records for a
transient's first use (against whatever last touched the memory) is exactly the "memory
dependency" the rule demands; for a buffer it is a `VkMemoryBarrier2`, not an image barrier.
(3) Nothing in Forge reads through two aliases, so no alias bit and no consistency question; the
only new hazard is the address, below.

**Vulkan Memory Allocator (AMD GPUOpen). "Resource aliasing (overlap)"; "Linear allocation
algorithm: ring buffer" (`vk_mem_alloc.h`, MIT).** [docs] [code] [still-current]
<https://gpuopen-librariesandsdks.github.io/VulkanMemoryAllocator/html/resource_aliasing.html> ·
<https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator>

The practitioner's rules, from the header's documentation: one allocation for several resources
has "allocation size = max(size of each image)", "allocation alignment = max(alignment of each
image)", "allocation memoryTypeBits = bitwise AND(memoryTypeBits of each image)"; one must "check
memoryTypeBits returned in memory requirements of each resource to make sure the bits overlap",
since "some GPUs may expose memory types suitable only for specific resource types"; the app must
"treat a resource after aliasing as uninitialized - containing garbage data" and "issue a memory
barrier to make sure commands that use img1 and img2 don't overlap on GPU timeline"; it is worth
doing where "intermediate textures or buffers" are "used only during a small range of render passes,
and … these ranges don't overlap in time". `VMA_ALLOCATION_CREATE_CAN_ALIAS_BIT` exists because a
dedicated allocation "will not be suitable for aliasing resources, resulting in Vulkan Validation
Layer errors". The linear algorithm gives "behavior of a ring buffer / queue" when allocations are
freed "in the same order as you created them (FIFO)".
*Bearing:* Forge's `plan()` already ANDs `memory_type_bits` and falls back to no aliasing when the
AND is empty; with buffers in the set, the fallback should be *two heaps* (images, buffers) rather
than none, since a vendor may segregate types (the AND was never empty on the 5070 Ti). The heap
goes through `gpu-allocator`'s managed scheme, not a dedicated allocation, which is what the CAN_ALIAS
note is about. The ring stays for uploads: a device-local transient is placed, not bumped.

**Epic Games. "Render Dependency Graph in Unreal Engine" (Unreal Engine 5.x documentation).**
[docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/render-dependency-graph-in-unreal-engine>

Epic's graph performs "whole-frame optimization of the render pipeline": "A resource can be
Transient, whereby its lifetime is constrained to the graph and the memory can potentially alias
with other transient resources with disjointed lifetimes"; "the graph will track the resource's
lifetime and can free and reuse the memory when the remaining passes no longer reference it";
transient resources "are allocated through `IRHITransientResourceAllocator`", the interface behind
`FRDGTransientResourceAllocator`, the page's list of pass flags (`Raster`, `Compute`, `AsyncCompute`,
`Copy`, `NeverCull`) and its culling of passes whose outputs nobody reads. (Search-record grade; the
host is blocked.)
*Bearing:* the mainstream engine puts buffers and textures through one transient allocator with
one lifetime analysis, which is the answer to the brief's first question. The name `transient
buffer` and the rule "constrained to the graph" are what Forge's API should say; Forge keeps its
"nothing is culled" rule (D-020), so lifetimes need no culling pass first.

**Unity Technologies. "Render graph system" (Core RP Library; URP manual, Unity 6).** [docs]
[still-current]
<https://docs.unity3d.com/6000.2/Documentation/Manual/urp/render-graph.html> (source read:
<https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.core/Documentation~/render-graph-system.md>)

"During execution, the render graph system allocates memory for resources before each render pass
that uses them, then releases them if later render passes don't use them"; it "optimizes GPU
memory, for example by reusing allocated memory if a texture has the same properties as an
earlier texture", "removes render passes if the final frame doesn't use their output" and "avoids
allocating resources the frame doesn't use".
*Bearing:* Unity's scheme is a pool keyed by descriptor (a released texture is handed to the next
identical request), the weakest of the three; it wastes nothing only when most transients look
alike, which post-processing chains do and a cull's lists (`u64` work items, `u32` counts, a few
words of status) do not. Forge's offset heap is the right tool for buffers of unequal sizes.

**Adam L. Buchsbaum, Howard Karloff, Claire Kenyon, Nick Reingold, Mikkel Thorup. "OPT versus
LOAD in dynamic storage allocation." STOC 2003 (SIAM J. Computing 2004); with H. A. Kierstead,
David A. Smith, W. T. Trotter, "First-fit coloring on interval graphs has performance ratio at
least 5", arXiv:1506.00192 (European Journal of Combinatorics, 2016); and Michael A. Bender, Alex
Conway, Martín Farach-Colton, Hanna Komlós, William Kuszmaul, Nicole Wein, "Tight Bounds for
Memory Allocation With and Without Request Fragmentation", arXiv:2608.28462, August 2026.**
[paper] [foundational] [recent]
<https://dl.acm.org/doi/10.1145/780542.780624> · <https://arxiv.org/abs/1506.00192> ·
<https://arxiv.org/abs/2608.28462>

The problem a transient heap solves has a name and known bounds. Dynamic storage allocation is
"the problem of packing given axis-aligned rectangles into a horizontal strip of minimum height by
sliding the rectangles vertically but not horizontally" (a resource is a rectangle: its lifetime
is the width, its size the height); LOAD is "the maximum sum of heights of rectangles that
intersect any vertical line", the trivial lower bound, and the paper relates the optimum to it.
It is NP-complete "although constant-factor approximations are known, and several heuristics …
have been proposed, including greedy strategies such as First-Fit" (Bender et al.'s framing; their
2026 paper is about the *online* case, where "the optimal competitive ratio for any deterministic
online allocator is Θ(log M)", which a graph that knows the whole frame escapes). For the
unit-height case the greedy in order of left endpoints is exact, but first-fit in an arbitrary
order "has performance ratio at least 5" on interval graphs (Kierstead, Smith & Trotter).
*Bearing:* Forge's `plan()` is first-fit by decreasing size, a sound heuristic and not an optimum;
the number to print next to `heap_bytes` is LOAD (the peak of live transient bytes over the
passes), because `heap_bytes / load_bytes` says how much the heuristic leaves on the table. Below
1.2 nothing is worth doing; above it, try the order by first pass and keep the smaller heap. The
same interval problem is solved for neural-network inference, where Yury Pisarchyk and Juhyun Lee
("Efficient Memory Management for Deep Neural Net Inference", arXiv:2001.03288, 2020,
<https://arxiv.org/abs/2001.03288>) compare greedy-by-size and greedy-by-breadth orders and report
"up to 11% smaller memory footprint than the state of the art": eleven percent is the most a
smarter order is likely to win, and their comparison is the shape of a planner test on
`FORGE_GRAPH_LOG` dumps if the slack ever says the heap is the problem.

**Pavlo Muratov. "GPU Memory Aliasing: overlapping GPU resources and saving memory"; and
"Organizing GPU Work with Directed Acyclic Graphs." Level Up Coding (Medium), 2020–2021.** [web]
[still-current]
<https://levelup.gitconnected.com/gpu-memory-aliasing-45933681a15e> ·
<https://levelup.gitconnected.com/organizing-gpu-work-with-directed-acyclic-graphs-f3fd5f2c2af3>

The two articles the brief names (there is no Kirill Kovalev article; see [Checked and left
out](#checked-and-left-out)). The first explains that "modern graphic APIs such as DirectX 12 or
Vulkan expose the ability to place allocated GPU resources into user-defined memory locations in
manually created heaps", which "allows creating textures and buffers whose memory overlaps
partially or even completely", and gives an algorithm to schedule aliased resources in a render
graph; the second is credited by Godot as the inspiration for its 4.3 render graph. (Search-record
grade; the host is blocked.)
*Bearing:* a readable reference for whoever revisits `plan()`, and evidence that the same
placement is done for buffers and textures together in the DX12 world, where placed resources in
heaps are the norm.

**LunarG. "Vulkan GPU-Assisted Validation" (white paper, buffer device address validation);
Khronos Validation Layers, `docs/syncval_usage.md`.** [docs] [still-current]
<https://www.lunarg.com/news-insights/white-papers/vulkan-gpu-assisted-validation/> ·
<https://github.com/KhronosGroup/Vulkan-ValidationLayers/blob/main/docs/syncval_usage.md>

What the layers can and cannot see. GPU-assisted validation "keeps track of all buffer device
addresses along with the size of the associated buffer, and creates an input buffer listing all
such address/size pairs. Shader code is instrumented to validate buffer_reference addresses and
report any reads or writes that do not fall within the listed address/size regions" (the table is
bounded by `khronos_validation.gpuav_max_buffer_device_addresses`). Synchronization validation
"runs its full set of checks when command buffers are submitted with vkQueueSubmit or
vkQueueSubmit2", covers "pipeline barriers, event operations, and render pass dependencies,
including synchronization2 commands" and "secondary command buffers executed with
vkCmdExecuteCommands", and lists under Known Limitations: "Hazards related to memory aliasing are
not detected properly" and "Host set event not supported".
*Bearing:* the crux of the buffer half. An aliased buffer keeps a perfectly valid address range,
so a shader that dereferences a stale `T*` (a pointer written into a persistent block last frame,
or a pass that reads a transient after its declared last use) is in-range for GPU-AV and invisible
to sync validation. Forge has to build the check itself: a poison mode that fills a transient
buffer with a sentinel after its last pass (and clears a transient image after its last), so a
stale read becomes a visible artefact the capture batch catches at more than 0 pixels. The second
guard is the API: a transient buffer's address exists only inside a pass body (`Resources`), never
before `execute`.

---

## 2. Parallel recording of pass bodies

Forge's starting point: `RenderGraph::execute` runs on the main thread inside the `cpu/record
commands` zone (`forge-app`), which also holds the graph's compile; per batch it takes one primary
command buffer from the slot's pool for that queue (`Frames::command_buffer(slot, kind)`, pools
created `TRANSIENT`, reset per slot in `Frames::begin`, buffers begun `ONE_TIME_SUBMIT`), records
each pass's barriers (`Commands::barriers`), `set_pass`, the body, and the timer mark, then
`Frames::submit` ends the buffers and issues one `vkQueueSubmit2` per batch with the timeline waits
and signals of #77. A body is `Box<dyn FnOnce(&Resources<'_>, &Commands<'_>) -> Result<()> + 'f>`,
not `Send`; `Commands` holds a `Cell<&'static str>`; the timer slot is an `Rc<GpuTimerSlot>` whose
query indices are handed out by a `Cell<u32>` counter in recording order. `forge-task` offers
`TaskPool::scope` (jobs borrow the stack, the caller helps until the counter is zero),
`Scope::spawn_with_priority(Priority::High, FnOnce + Send)`, `par_for`, `join` and a `TaskGraph`;
`Priority::High` is documented as "latency-critical frame work (culling, command recording, …)".
`forge-app` does not depend on `forge-task` today (`forge-procgen` and `tools/genesis` do).

**Khronos Group. Vulkan Specification, "Command Buffers" and "Synchronization" (submission
order); Vulkan Guide, "Threading".** [spec] [docs] [still-current]
<https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#commandbuffers> ·
<https://docs.vulkan.org/guide/latest/threading.html> (sources read on GitHub, `cmdbuffers.adoc`,
`synchronization.adoc`, `Vulkan-Guide/chapters/threading.adoc`)

The constraints. "Command pools are externally synchronized, meaning that a command pool must not
be used concurrently in multiple threads", including recording into buffers allocated from it; the
Guide's pattern is "a separate command pool in each host-thread", with the reminder that threading
gives only "host-side scaling". Secondary command buffers "inherit no state from the primary
command buffer" and "must not be directly submitted to a queue"; `VK_COMMAND_POOL_CREATE_TRANSIENT_BIT`
marks buffers that "will be short-lived", and `vkResetCommandPool` "recycles all of the resources
from all of the command buffers allocated from the command pool back to the command pool".
Submission order within one submit is "the order in which command buffers are specified in the
pCommandBuffers member of VkSubmitInfo or VkSubmitInfo2, from lowest index to highest", after the
order of the `VkSubmitInfo2` structures in `pSubmits`.
*Bearing:* a pool per (frame slot, queue, worker) is mandatory, not a choice; Forge's per-slot
pool reset stays as it is, one reset per pool. Many command buffers in one `VkSubmitInfo2` keep the
pass order the plan fixed, so a batch becomes a `Vec<vk::CommandBuffer>` and nothing about the
waits changes. Secondaries buy nothing here: Forge uses dynamic rendering, no render-pass instance
spans several passes, and a primary per chunk is the simpler object.

**Khronos Group. Vulkan Samples: "Multi-threaded recording with multiple render passes" and
"Command buffer usage and multi-threaded recording" (originally Arm's Vulkan best practice
samples).** [code] [docs] [still-current]
<https://github.com/KhronosGroup/Vulkan-Samples/blob/main/samples/performance/multithreading_render_passes/README.adoc>
· <https://github.com/KhronosGroup/Vulkan-Samples/blob/main/samples/performance/command_buffer_usage/README.adoc>

The only published measurements with both modes. The first sample records a shadow pass and a main
pass on one thread, on two threads into two primaries, and on two threads into secondaries executed
by one primary: on the mobile device used, "two threads perform the same amount of work in 10s as
one thread in more than 15.7 seconds" and, in the profiled debug build, "frame time is decreased
from 531.1ms to 337.7ms using multi-threading (1.57 times decrease)"; its advice is to "Spread the
workload between threads as equally, as possible" and to "Measure CPU time or overall time for each
frame and compare results of using single and multiple threads". The second measures the pool
strategies in one capture: "Reset pool 53.3 ms (0.45 %), Reset buffers 140.29 ms (1.16 %),
Allocate and free 3,319.25 ms (28.8 %)", reports "a 15% improvement in performance when dividing
the workload among 8 buffers across 8 threads" on a scene of about 1 800 draw calls, and says
"Don't … call vkResetCommandBuffer() on a high frequency call path". Arm's text behind these
samples adds the gate: "only go parallel if you measure that draw call recording is taking a
significant portion of your frame time", and "there is no advantage in exceeding the CPU
parallelism level, that is, using more command buffers than threads".
*Bearing:* the gain scales with how much of the frame is recording, which on these samples is
most of it and in Forge is 0.08–0.10 ms of an 8.33 ms budget; the gate is the ROADMAP's sentence
in other words. Pool reset per slot (what Forge does) is the cheap strategy; chunk count per queue
should not exceed the worker count.

**NVIDIA. "Vulkan Do's and Don'ts." NVIDIA Developer Blog (Tips and Tricks), 2019, updated.**
[web] [still-current]
<https://developer.nvidia.com/blog/vulkan-dos-donts/>

The vendor rules for the target GPU: "Try to minimize the number of queue submissions, as each
vkQueueSubmit() has a significant performance cost on CPU"; "Don't record tiny command buffers that
contain only a few small draw calls or small compute dispatches, as each command buffer contains
some additional GPU work inserted by the driver"; record in parallel "by having 1 VkCommandPool and
1 VkCommandBuffer per thread"; `vkAllocateCommandBuffers`, `vkBeginCommandBuffer` and
`vkEndCommandBuffer` "should be called from the thread that fills the command buffer, as these calls
take measurable time on CPU"; use `ONE_TIME_SUBMIT` for buffers submitted once; and "Don't submit a
small amount of GPU work" per submission. (Search-record grade; the host is blocked. The sharing-mode
line of the same page was checked directly for #77.)
*Bearing:* two design rules follow. A pass per command buffer is wrong for Forge's frame, where the
depth pyramid alone is eleven passes of a few microseconds each; the unit is a *chunk* of
consecutive passes of one batch, sized so that its recording time dwarfs the begin/end and the
driver's per-buffer GPU work. And begin and end happen inside the job, never on the main thread
after the fact.

**AMD GPUOpen. "RDNA Performance Guide"; Matthäus Chajdas, "Vulkan Barriers Explained" (2016).**
[web] [still-current]
<https://gpuopen.com/learn/rdna-performance-guide/> ·
<https://gpuopen.com/learn/vulkan-barriers-explained/>

The other vendor says the same: "Submission of command buffers should be kept to a minimum as
submitting requires a call into kernel mode as well as some implicit barriers on the GPU"; on
barriers, specify source and destination stages so as to "maximize the number of 'unblocked'
stages, that is, produce data early and wait late for it". (Search-record grade; the host is
blocked.)
*Bearing:* one submission per batch, as now, whatever the number of command buffers inside it;
and the stage masks the graph derives per access (D-020) are already the "produce early, wait
late" the guide asks for, which a split barrier would refine only where the gap between producer
and consumer holds enough independent work.

**Natalya Tatarchuk (Bungie). "Destiny's Multithreaded Rendering Architecture." GDC 2015.**
[talk] [foundational] [still-current]
<https://www.gdcvault.com/play/1021926/Destiny-s-Multithreaded-Rendering> (materials:
<https://advances.realtimerendering.com/destiny/gdc_2015/>)

"The architecture of a multithreaded renderer that delivers low-latency, efficient execution across
multiple platforms": the engine "designed from the ground up for job-based multithreading", the
renderer as fine-grained jobs over an immutable per-frame packet, submission jobs recording in
parallel and the order restored at submit. (Search-record grade; the Vault entry and its ID were
confirmed by fetch in the job-system session.)
*Bearing:* the canonical shape: the frame's data is frozen, then many jobs record, then one thread
submits in order. Forge's frozen data is the compiled plan plus the resolved resources; the packet
already exists in the form of the `FrameGraph` the renderers build.

**Johan Andersson (DICE). "DirectX 11 Rendering in Battlefield 3." GDC 2011.** [talk]
[foundational]
<https://www.slideshare.net/DICEStudio/directx-11-rendering-in-battlefield-3>

Frostbite's parallel dispatch before graphs: a "DX11 deferred context per hardware thread"; the
renderer builds "a list of all draw calls for each rendering layer", splits it "into chunks of
approximately 256", dispatches the chunks in parallel to deferred contexts that generate command
lists, then executes the lists on the immediate context in order. (Search-record grade.)
*Bearing:* the chunking constant of a shipped engine (a job every few hundred draws, not every
draw) and the invariant (the immediate context executes in the original order). Forge's chunks are
passes, not draws, because its passes are GPU-driven and contain one or two indirect commands each;
the invariant is the same.

**Epic Games. "Parallel Rendering Overview for Unreal Engine" (5.x documentation).** [docs]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/parallel-rendering-overview-for-unreal-engine>

Two levels: the render thread "enqueues platform-agnostic graphical commands into the renderer's
command list", and the RHI thread "translates (executes) them via the appropriate graphics API",
so that "anything generated in parallel on the frontend is translated in parallel on the backend"
where the platform allows (consoles, DX12, Vulkan); "it is guaranteed that, regardless of how
parallelization is configured, the order of submission of commands to the GPU is unchanged from
the order the commands would have been submitted in a single-threaded renderer". (Search-record
grade; the host is blocked.)
*Bearing:* Unreal pays for portability with a translation layer and a dedicated thread; Forge has
one backend and records Vulkan directly from the pass body, which is the right choice to keep (one
level of parallelism, jobs on the pool, no render thread). The order guarantee is the test: the
parallel and serial recordings must produce byte-identical captures.

**Jean Geffroy, Axel Gneiting, Yixin Wang (id Software). "Rendering the Hellscape of Doom
Eternal." SIGGRAPH 2020, *Advances in Real-Time Rendering in Games*; with Gneiting's statements on
id Tech 7's job system (reported, 2020).** [talk] [web] [recent]
<https://advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf> ·
<https://www.dsogaming.com/news/doom-eternal-does-not-have-a-main-or-render-thread-anti-aliasing-uses-up-to-32-temporal-subsamples/>

The talk covers the frame's systems and "optimizations that achieved 60 FPS on all platforms"; id
Tech 7 is "entirely built with a Vulkan backend". On threading, as reported from Gneiting: the
game "does not have a main or render thread", it "has jobs with one worker thread per core",
"around 100 jobs per frame", and "jobs can spawn more jobs, but the engine also has preconstructed
job graphs". (Search-record grade; the hosts are blocked.)
*Bearing:* preconstructed job graphs are what a compiled render-graph plan is: the dependencies are
known before the frame runs, so the recording jobs need no runtime dependency tracking, only a
scope. The order of magnitude (a hundred jobs a frame for the whole game) says a chunk per pass
would be too fine for Forge's forty passes; a handful of chunks per queue is the right grain.

**Dzmitry Malyshau. "Global Pass Barriers Without Per-Resource RHI Tracking: A Cross-Vendor Study
with Blade." arXiv:2607.26506, July 2026.** [paper] [recent]
<https://arxiv.org/abs/2607.26506>

Measured on Forge's own GPU class: Blade "keeps Vulkan images in GENERAL, tracks no per-resource
state, and issues global pass-boundary barriers"; deriving the global barrier's scope from the pass
kinds "saves 5.0% on an NVIDIA graphics chain and 6.7% on an AMD compute chain", and "removing
fifteen redundant barriers from sixteen independent compute passes reduces GPU span by 29.3% on an
RTX 5070 and 32.3% on an RX 7900 XT". (Search-record grade; arxiv.org is blocked.)
*Bearing:* barriers cost real GPU time on the 5070 class, so parallel recording must not add any:
the per-chunk command buffers carry exactly the barriers the plan derived, no "full barrier at the
start of a command buffer for safety". It is also the argument against events for events' sake
(§3): every extra synchronisation command is measurable.

---

## 3. Split barriers, events and the queues

**Khronos Group. Vulkan Specification, "Events" and `vkCmdWaitEvents2`; Vulkan-Docs issue #962,
"I think Event API for split barriers is designed incorrectly" (2019); Hans-Kristian Arntzen,
"Yet another blog explaining Vulkan synchronization" (14 August 2019).** [spec] [web]
[still-current]
<https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#synchronization-events> ·
<https://github.com/KhronosGroup/Vulkan-Docs/issues/962> ·
<https://themaister.net/blog/2019/08/14/yet-another-blog-explaining-vulkan-synchronization/>

Events "can be used to insert a fine-grained dependency between commands submitted to the same
queue, or between the host and a queue", and "must not be used to insert a dependency between
commands submitted to different queues"; `vkCmdWaitEvents2` "is used with vkCmdSetEvent2 to define a
memory dependency between two sets of action commands, roughly in the same way as pipeline
barriers, but split into two commands such that work between the two may execute unhindered". The
2019 issue argued that the original API left the set without "what memory operations and layout
transitions it should start", forcing drivers to defer the work to the wait; `synchronization2`
moved the dependency information into `vkCmdSetEvent2` (a design write-up seen in search results
traces the original split to GCN, where completion was signalled by a memory write and cache
control was a separate command). Arntzen's post explains barriers as splitting the command stream
in two halves and events as the two-command form of the same thing. (The spec source and the issue
were read on GitHub; the blog and the write-up are search-record grade.)
*Bearing:* three facts settle §3 for Forge. Events cannot cross queues, so the cross-queue waits of
#77 stay timeline semaphores whatever happens on the graphics queue. A split barrier only helps when
independent passes sit between producer and consumer, which the plan can measure (the distance in
passes between a write and its first read); in the city that distance is short for the culls and
long only for the sky tables, which already run on the async queue. And Granite, the one open
graph that uses events, uses them only to hand memory between aliases, and pays for it by aliasing
on a single queue; Forge already made the same trade without events (transients never run on
another queue), so nothing forces events into the first step.

**James Jones (NVIDIA, Vulkan WG). "Vulkan Timeline Semaphores." Khronos blog, 15 January 2020;
Vulkan Specification, semaphore wait operations.** [web] [spec] [still-current]
<https://www.khronos.org/blog/vulkan-timeline-semaphores>

Restated from `task-system.md` §F, where it was fetched: a timeline is "a monotonically increasing
64-bit integer value", one primitive for device and host, with waits allowed to be queued before
their signals. The spec's condition (read in `synchronization.adoc`): a batch's wait is valid if
"the signal operation is either earlier in submission order on the same queue, or is submitted by a
command whose host operation happens-before this batch is submitted on the host".
*Bearing:* parallel recording changes nothing on this side: batches, their waits and signals are
computed before recording (`compile_queued`), the jobs only fill command buffers, and
`Frames::submit` submits the batches in the order the plan gave them, which keeps the
happens-before the spec asks for. A batch on the compute or transfer queue usually holds one or two
passes, hence one chunk; the graphics batches are where chunks multiply.

---

## 4. Measuring, and when it pays

**Bartosz Taudul. Tracy Profiler (0.14) with `tracy-client` 0.19; Forge's F1 overlay
(`docs/PROFILE.md`).** [code] [docs] [still-current]
<https://github.com/wolfpld/tracy>

"A real time, nanosecond resolution, remote telemetry, hybrid frame and sampling profiler", with
CPU zones, GPU zones for "OpenGL, Vulkan, Direct3D 11/12, Metal, OpenCL, CUDA, WebGPU", memory,
locks and context switches; Forge already opens a `tracy_client::GpuContext` for its Vulkan
timestamps behind `--features profiling` and spans `record` and `submit and present` on the main
thread. The overlay's CPU zones today: `cpu/update`, `cpu/wait for GPU (frame slot)`,
`cpu/acquire swapchain image`, `cpu/record commands`, `cpu/submit + present`.
*Bearing:* the measurement the issue asks for first is a split of `cpu/record commands` into the
graph's compile (schedule, `compile_queued`, placement), the bodies (summed per pass, so the
overlay can list the five costliest) and the barriers, plus a count of command buffers and
submissions per frame in the COUNTERS block; with parallel recording, a Tracy span per chunk on its
worker, so the timeline shows the fan-out. `docs/PROFILE.md` gets the before/after row the issue's
definition of done names.

**Forge's own numbers (`docs/PROFILE.md`, `docs/demos/city-blocks.md`, `docs/ROADMAP.md`, as of
2026-09-26).** [measured]

`asteroids`: `cpu/record commands` 0.08–0.10 ms for 26–27 graph passes and 49–50 barriers, the
compile included; `cpu/submit + present` 0.10 ms; the GPU 0.33 ms, so the main thread's 0.20 ms
of work finishes first and waits for the slot. `city-blocks` from the south edge: "0.25 ms of work
(record 0.09, submit + present 0.16), the rest waiting for the GPU", against 1.96 ms of GPU at
1600 × 900 and 3.38 ms in the 1440p flight (p99 3.8 ms); the budget at 120 fps is 8.33 ms.
Streamline's DLSS evaluation "adds about 0.07 ms of CPU to recording (0.17 against 0.10)". The
work buffers (`MemoryCategory::Work`, both frame slots) are 84 MiB in the city; the ballad's three
transient images are 25.6 MB and "nothing aliases in that frame yet".
*Bearing:* recording is 1 % of the frame budget and a third of the main thread's own work; the
serial parts of any parallel scheme (compile, submit) are of the same size as the whole. Parallel
recording can save at most about 0.06 ms in the city today, less than the cost of one more command
buffer per chunk on the GPU is likely to be; the ROADMAP's condition ("once CPU recording shows in
the overlay") is right and should be made numeric (below). The buffer half, by contrast, has a
measurable target now: the per-slot copies of every per-frame list.

---

## What professional engines do

| Engine | Transient buffers | Placement | Recording | Barriers between aliases / passes |
|---|---|---|---|---|
| Frostbite (FrameGraph, 2017) | yes, but through "an atomic linear allocator for buffers" | textures aliased in physical or virtual memory, or pooled, by platform; 40–50 % of transient VRAM saved (third-party figure) | jobs over chunks of ~256 draws since BF3 (2011), the immediate context executes in order | derived by the graph |
| Unreal (RDG) | yes: `IRHITransientResourceAllocator` for buffers and textures | lifetimes tracked, memory freed and reused, disjoint lifetimes alias | render thread → parallel RHI translate, order guaranteed identical to single-threaded | derived, batched, async fork/join |
| Unity (render graph, Unity 6) | handles for textures and buffers | pool by identical descriptor, not by offset | native render passes; no published parallel recording | derived; passes culled |
| Granite | **no** ("Buffers are never transient") | images alias by renaming (identical dimensions, disjoint lifetimes, one queue) | a task per physical pass records its own command buffer, per-thread pools per frame context, submit tasks in order | events for alias hand-off; pipeline barriers otherwise |
| Destiny (Bungie, 2015) | not stated | not stated | jobified renderer over a frozen frame packet, submission in order | n/a |
| id Tech 7 (Doom Eternal) | not stated | not stated | no main or render thread; ~100 jobs per frame, preconstructed job graphs | n/a |
| Halcyon (SEED, 2018; search-record grade) | yes, "transient resources" of the graph | not stated | render command lists translated per backend | derived |
| AMD RPS (README read) | yes | "(aliasing) memory scheduler" | not stated in the README | "generally optimal barrier generator" |
| **Forge today (D-020, #77)** | no: `FRAMES_IN_FLIGHT` `GraphBuffer`s per renderer | images: largest-first first-fit offsets in one heap, cached while unchanged, cross-frame safe | one thread, one primary command buffer per batch, one submit per batch | derived per access and mip; timeline waits across queues |

Two patterns hold across the table. Where buffers are transient at all, they go through the same
allocator as textures (Unreal, Halcyon, RPS) or through a bump allocator (Frostbite); nobody pools
buffers by descriptor. And every parallel recorder freezes the plan first, records into per-thread
pools, and restores the single-threaded order at submit; none lets a recording job change the
plan.

---

## Recommendation for Forge

**Order of work.** The buffer half first, because it has a target today (memory, and the
`FRAMES_IN_FLIGHT` copies that every renderer hand-rolls) and no CPU precondition; the CPU zones
second, because they are the measurement the issue asks for and cost an afternoon; parallel
recording third, behind a numeric gate, prototyped on a synthetic frame before it touches the
city.

1. **Transient buffers in the image heap, same analysis, same planner.** `FrameGraph::transient_buffer(TransientBufferDesc { size, usage, name }) -> BufferHandle`
   beside `transient()`; the entry kind `BufferEntry::Transient`. In `execute`, a `Request` per
   transient buffer with `first`/`last` from the passes' `buffers` lists, `size` and `alignment`
   from `vkGetDeviceBufferMemoryRequirements` (cached by desc like `self.requirements`), the
   `memory_type_bits` ANDed into the heap's; `plan()` unchanged except that a buffer's alignment
   becomes `max(alignment, bufferImageGranularity)` and its size rounds up to the granularity (the
   limit read into `Device`), which is the spec's padded-range rule without a linear/non-linear
   overlap test. If the AND of the type bits is empty, two heaps (images, buffers) rather than
   `aliased: false`. `Device::create_buffer_in(desc, heap, offset)` mirrors `create_image_in`,
   with `SHADER_DEVICE_ADDRESS` always in the usage and the address taken after the bind; the
   `TransientCache` holds the buffers next to the images, so addresses are stable while the layout
   is. The first-use barrier for a buffer is a `VkMemoryBarrier2` from whatever last touched the
   range (the existing overlap walk), and a transient buffer read before its write stays a compile
   error. Async passes refuse transient buffers as they refuse transient images. Cost: a day of
   `forge-gpu`; no shader changes.
2. **The address contract.** A body reads a transient buffer through `Resources::buffer(handle)
   -> &Buffer` (a new accessor; today `Resources` has only images) and passes the address in push
   constants or writes it into the slot's per-frame block *at record time*, never before
   `execute`; the doc comment on `transient_buffer` says the address is valid from the buffer's
   first to its last declared pass of this frame and nowhere else. Because two aliased buffers are
   both valid address ranges, neither GPU-AV nor synchronization validation ("hazards related to
   memory aliasing are not detected properly") will catch a violation; step 3 does.
3. **A poison mode: `FORGE_GRAPH_POISON=1`.** After a transient buffer's last declared pass, the
   graph records `vkCmdFillBuffer` over it with a sentinel (`0xDEAD_BEEF`; the `TRANSFER_DST`
   usage is added in this mode); after a transient image's last pass, a clear to a loud colour
   (magenta, or NaN for float targets). A stale read then shows in the capture batch as pixels
   that differ from the run without poison, and the acceptance test is `compare.sh` at 0 pixels
   between `FORGE_GRAPH_POISON=1`, `FORGE_GRAPH_NO_ALIAS=1` and the default, on every demo. The
   fills cost bandwidth only in that mode. The counters gain `load_bytes` (the peak of live
   transient bytes over the passes) next to `heap_bytes` so the planner's slack is a number.
4. **Migrate the meshlet scene's per-frame buffers, one at a time, captures at 0 px after each.**
   In order: the look-back status words (`lookback` in the instance cull and in each `WorkList`,
   cleared before the cull and never read across frames), the cull `work` and `roots` lists, the
   `rejects` list, the `deferred` list, the `cell_lists`, the visible `lists` (their growth rule
   moves into the desc: a bigger desc is a new layout and a heap rebuild, which the cache already
   handles). Anything read back on the host (`stats_readback`, `HostRead`) or read by the next
   frame stays a `GraphBuffer` with its slots. Each migration deletes a `(0..FRAMES_IN_FLIGHT)`
   loop from `meshlet.rs`. Expected: the city's 84 MiB of work buffers shrink toward one frame's
   peak, which the counters will give; the ballad's post-processing images may start aliasing too
   once the buffers' lifetimes interleave with theirs. Then the same for `exposure.rs`,
   `probes.rs`, `sky.rs` and `streaming.rs` where a buffer lives inside one frame.
5. **The CPU zones (the issue's "measure first").** Split `cpu/record commands` into
   `cpu/graph compile`, `cpu/graph barriers`, `cpu/graph bodies` (summed, with the five costliest
   passes listed in the overlay's full mode) and keep `cpu/submit + present`; count command buffers
   and submissions in the COUNTERS block; a Tracy span per pass body under `record`. Numbers into
   `docs/PROFILE.md` for the ballad and the city. This is where the gate for step 7 is read.
6. **A synthetic frame to size the gate.** A `graph-bench` example (or a `--synthetic-passes N
   --synthetic-dispatches M` flag on `meshlets`) that declares N compute passes of M small
   dispatches over a chain of transient buffers, printing compile, bodies, submit and the GPU span
   per configuration. Run N ∈ {40, 100, 400}, M ∈ {1, 32, 256}: this measures Forge's per-pass
   CPU cost, the driver's per-command-buffer and per-submit cost on the 5070 Ti, and, after step 7,
   the scaling from one to six workers.
7. **Parallel recording, behind the gate.** Build when `cpu/graph bodies` exceeds 0.5 ms in a
   shipped demo, or when a frame passes 100 passes; not before. The design, fixed now so nothing in
   steps 1–6 contradicts it:
   - **Pools.** `Frames`' `PerSlot.pools` becomes `[queue][worker]`, `worker_count + 1` pools per
     queue per slot (the last for the main thread), all reset in `Frames::begin`;
     `Frames::command_buffer(slot, kind, worker)`. `forge-task` exposes the worker index it already
     keeps thread-locally (`None` on a foreign thread, which then uses the main thread's pool under
     a check that only the main thread does).
   - **Chunks.** After `compile_queued`, the passes of each batch are cut into chunks: consecutive
     passes until a chunk holds `min_passes` (start at 4) or the batch ends; a pass marked
     `PassBuilder::serial()` (Streamline's DLSS evaluation, anything that touches a non-`Sync`
     renderer) closes its chunk and is recorded by the main thread. The chunk count per queue is
     capped at the worker count.
   - **Jobs.** `pool.scope(|s| …)` spawns one `Priority::High` job per chunk; the job begins its
     own primary command buffer, records for each pass the plan's barriers, `set_pass`, the body
     and the timer mark, and ends the buffer (NVIDIA's rule); the main thread helps until the
     counter is zero. `PassBody` gains `+ Send`, so bodies capture `&T where T: Sync`; `Commands`
     is created per chunk (its `Cell` is then thread-local); the timer slot's `Cell` counter is
     replaced by query indices assigned per pass at compile time, with the batch's `start` stamp on
     its first chunk. `Resources` needs `Device: Sync`, which `ash` gives and `gpu-allocator`'s
     mutex preserves.
   - **Submission.** `Batch.command_buffer` becomes `Vec<vk::CommandBuffer>`; `Frames::submit`
     puts them in one `VkSubmitInfo2` in chunk order with the same waits and signals, one submit
     per batch as today. `FORGE_RECORD_SERIAL=1` records everything on the main thread for the A/B;
     the captures must match at 0 pixels by construction, since chunk contents depend only on the
     plan.
   - **Dependency.** `forge-gpu` takes a `&TaskPool` in `execute` (a new edge to `forge-task`, a
     dependency-free crate) and `forge-app` creates `TaskPool::client()`, which it will need for
     the island's streaming anyway.
8. **Events, later and measured.** A `FORGE_GRAPH_EVENTS=1` experiment on the graphics queue only:
   for a write whose first read is at least `k` passes later, `vkCmdSetEvent2` after the writer
   and `vkCmdWaitEvents2` before the reader instead of the pipeline barrier; keep it only if the
   GPU span shrinks on the city, since each command is measurable on this GPU class (the Blade
   numbers). Nothing cross-queue changes: events cannot cross queues, timelines do.

**API sketch.**

```rust
// forge-gpu: graph.rs
pub struct TransientBufferDesc { pub size: u64, pub usage: vk::BufferUsageFlags, pub name: &'static str }
impl<'f> FrameGraph<'f> {
    pub fn transient_buffer(&mut self, desc: TransientBufferDesc) -> BufferHandle;   // step 1
}
impl Resources<'_> {
    pub fn buffer(&self, handle: BufferHandle) -> &Buffer;                          // step 2: address at record time
}
impl PassBuilder<'_, '_> {
    pub fn serial(self) -> Self;                                                     // step 7: main thread only
    pub fn run(self, body: impl FnOnce(&Resources<'_>, &Commands<'_>) -> Result<()> + Send + 'f);
}
pub struct GraphStats { /* … */ pub transient_buffers: u32, pub load_bytes: u64, pub chunks: u32, pub command_buffers: u32 }
impl RenderGraph {
    pub fn execute(&mut self, frame: FrameGraph<'_>, frames: &mut Frames, slot: FrameSlot,
                   pool: &forge_task::TaskPool) -> Result<GraphStats>;
}
// forge-gpu: frame.rs
impl Frames {
    pub fn command_buffer(&mut self, slot: FrameSlot, kind: QueueKind, worker: usize) -> Result<vk::CommandBuffer>;
}
pub struct Batch { pub queue: QueueKind, pub command_buffers: Vec<vk::CommandBuffer>, pub waits: [(u64, vk::PipelineStageFlags2); 3], pub signal: u64 }
```

**Risks.**
- *A stale address.* The only new class of bug; the poison mode and the `Resources`-only access
  are the two defences, and `compare.sh` with poison on is the test that runs every time.
- *Granularity padding eating the saving.* If `bufferImageGranularity` is large on a vendor, many
  small buffers next to images waste pages; the fix is to sort buffers together in the heap (the
  planner already sorts by size; a second key by linearity keeps buffers adjacent) or the two-heap
  fallback. Read the limit and print it in the log.
- *Heap rebuilds from growing lists.* The visible list and the rejects grow by rule; every growth is
  a new layout and a rebuild (the old heap retires through `destroy_later`). Grow in powers of two,
  as they do now, and the rebuild count in `GraphStats::heap_rebuilds` stays a handful per run.
- *Command buffers are not free.* NVIDIA's "additional GPU work" per buffer and AMD's "implicit
  barriers" per submit mean chunking too fine loses on the GPU what it gains on the CPU; the
  synthetic frame measures the per-buffer cost before the grain is chosen.
- *`Send` bounds ripple.* Every renderer whose `&self` a body captures must be `Sync`; `Rc`,
  `Cell` and `RefCell` in renderers (the timer slot is one) will be found by the compiler in step
  7, not at run time; budget a day for them.
- *Serial parts.* Compile and submit stay on the main thread; by Amdahl, a 6-worker recording of a
  frame whose compile is 0.05 ms cannot go below that, which is why the gate is on the *bodies*.

**Expected numbers.** Step 4: the work-buffer category in the city drops from 84 MiB (two slots)
toward the peak of one frame's live lists, so on the order of 30–45 MiB saved, more if the lists'
lifetimes let them alias each other (the culls' work lists die at the second cluster cull, the
visible list at the resolve); the exact figure is `transient_bytes − heap_bytes` in the counters.
Step 5: no change in time, a split of the 0.09 ms. Step 7 on the synthetic frame: with 100 passes
of 30 µs of recording each (3 ms serial) and 0.1 ms serial overhead, 0.6–0.8 ms on six workers
(4–5×); on today's city, no measurable gain, which is the point of the gate.

**What to measure and report (the issue's definition of done).** Memory: the transient counters
(`transient_images`, `transient_buffers`, `transient_bytes`, `heap_bytes`, `load_bytes`) and the
`Work` category before and after step 4 for the city, the ballad and the bench, in
`docs/PROFILE.md`. CPU: the split zones of step 5 before and after step 7, plus command buffers
and submissions per frame. Correctness: `tools/validate.sh` clean in default, `NO_ALIAS`, `POISON`
and (later) `RECORD_SERIAL` modes; `tools/compare.sh` at 0 pixels between every pair; the flight
p99 unchanged.

---

## What the numbers say

Memory: Frostbite reported 40–50 % of transient VRAM saved by graph-driven aliasing (third-party
summary of the GDC 2017 slides); a smarter placement order buys at most about 11 % over greedy in
Pisarchyk & Lee's study of the same interval problem; the lower bound of any heap is LOAD, the peak
of live bytes; first-fit in an arbitrary order can use five times the optimum on interval graphs
(Kierstead–Smith–Trotter), which is why Forge orders by size and should print the slack. Forge's
work buffers are 84 MiB in the city for two frame slots; its three transient images in the ballad
25.6 MB, with no aliasing yet. Recording: the Khronos sample's two threads gave 1.57× on a mobile
CPU where recording was the whole frame, and eight threads 15 % on 1 800 draw calls; Forge's
recording is 0.08–0.10 ms with the compile, its submit and present 0.10–0.16 ms, its main thread
0.20–0.25 ms of work per frame against a GPU of 0.33–3.4 ms and a 120 fps budget of 8.33 ms.
Command buffers: pool reset costs 0.45 % of a capture against 28.8 % for allocate-and-free; NVIDIA
and AMD both say a submission is a kernel call and a command buffer carries driver work. Barriers:
fifteen redundant barriers among sixteen compute passes cost 29.3 % of GPU span on an RTX 5070.
Jobs: id Tech 7 runs a whole game in about 100 jobs a frame; Battlefield 3 chunked draws by ~256
per job.

---

## Checked and left out

Kept so the bibliography is auditable: things looked for and not above, with the reason.

- **A "Kirill Kovalev" article on memory aliasing** — none found under that name; the searches
  return Muratov's two articles, Riccardo Loggini's "Render Graphs" (2021, logins.github.io,
  blocked) and Pavel Šmejkal's "Aliasing transient textures in DirectX 12" (pavelsmejkal.net,
  blocked). Muratov stands for the genre.
- **Frostbite's exact memory figures and the "atomic linear allocator" slide** — the slide hosts
  (SlideShare, dokumen.tips, ea.com) are blocked; the wording comes from the search engine's
  extracts and the 40–50 % from a third-party summary, so both are stated as such.
- **Unreal's `FRDGTransientResourceAllocator` source** — the engine repository requires an Epic
  account and dev.epicgames.com is blocked; the documentation wording was confirmed through search
  extracts, and a GitHub mirror of a third-party RDG write-up (staticJPL) was read but adds nothing
  primary. `r.RDG.ParallelExecute` and the `NeverParallel` flag were looked for and not confirmed,
  so no Unreal flag name is claimed for parallel pass execution.
- **Godot 4.3's render graph** (godotengine.org, "GPU synchronization in Godot 4.3 is getting a
  major upgrade") — seen in search results, credited to Muratov's DAG article; the host is blocked
  and the article was not read, so it is not an entry.
- **Riccardo Loggini's "Render Graphs" (2021)** — its mirror claims aliasing "can spare no more than
  50 % of the used resource allocation space", a statement without a source; not adopted.
- **Sebastian Aaltonen's threads on one-command-buffer frames** — not citation grade.
- **NVIDIA's "Advanced API Performance: Barriers"** — a D3D12 page; the Vulkan do's and don'ts
  cover the same ground for this file.
- **Halcyon (Wihlidal, 2018) and AMD's RPS SDK** — in the engines table only: Halcyon's hosts
  (wihlidal.com, EA's media server) are blocked and its wording is a search extract; RPS's README
  (read on GitHub, <https://github.com/GPUOpen-LibrariesAndSDKs/RenderPipelineShaders>) says
  nothing about threads, and the header path guessed for its recording API returned 404.
- **`bufferImageGranularity` values per vendor** — vulkan.gpuinfo.org was not reachable; the
  recommendation reads the limit from the device rather than quoting a number.
- **Replies on issue #962 and the Khronos "Understanding Vulkan Synchronization" blog** — the
  issue page's extract carried no replies and khronos.org is blocked; the `synchronization2`
  change is stated from a search extract of a design write-up.

---

## Verification notes

Checked on 2026-09-26 with WebSearch, WebFetch and `curl` through the session's proxy; no browser
pane and no video pages. The proxy served `github.com`, `raw.githubusercontent.com` and the search
engine; it refused themaister.net, dev.epicgames.com and Epic's CloudFront mirror,
developer.nvidia.com, gpuopen.com, logins.github.io, advances.realtimerendering.com,
simoncoenen.com, wihlidal.com, archive.org, gamedeveloper.com, apoorvaj.io, stoleckipawel.dev,
asawicki.info, khronos.org, godotengine.org, levelup.gitconnected.com, pavelsmejkal.net,
dsogaming.com and news.ycombinator.com; api.github.com answered 403 and arxiv.org was not tried
after last session's refusals. Verification therefore has three grades.

- **Read in full or in the relevant section (GitHub):** Granite's `renderer/render_graph.cpp`
  (131 KB, every line quoted above is verbatim: "Buffers are never transient.", `build_aliases`,
  `physical_pass_transfer_ownership`, `physical_pass_handle_gpu_timeline`,
  `enqueue_render_passes`), `renderer/render_graph.hpp` and `vulkan/device.cpp`
  (`request_command_buffer_for_thread`, `frame().cmd_pools[physical_type][thread_index]`,
  `frame().submissions[…]`); the Vulkan specification sources `chapters/resources.adoc` (Memory
  Aliasing, `VK_IMAGE_CREATE_ALIAS_BIT`, `vkGetBufferDeviceAddress`,
  `vkGetDeviceBufferMemoryRequirements`), `chapters/synchronization.adoc` (Events, the
  `vkCmdWaitEvents2` note, submission order, the semaphore wait condition) and
  `chapters/cmdbuffers.adoc` (pools, secondaries, pool flags, `vkResetCommandPool`); the Vulkan
  Guide's `threading.adoc`; the Vulkan Samples' two READMEs (numbers verbatim); VMA's
  `vk_mem_alloc.h` documentation (aliasing rules, `CAN_ALIAS_BIT`, the ring buffer); the
  validation layers' `docs/syncval_usage.md` (coverage and Known Limitations verbatim) and
  `docs/gpu_validation.md` (which no longer describes the address check, hence LunarG below);
  Unity's `render-graph-system.md`; the RPS and Tracy READMEs; Vulkan-Docs issue #962.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  date, and the sentences quoted, which are the search engine's extracts of the page named):
  O'Donnell 2017 (GDC Vault 1024612, SlideShare, the GDC session description); Arntzen's 2017 and
  2019 posts (themaister.net listings, the Khronos news item); Unreal's RDG and Parallel Rendering
  pages (dev.epicgames.com extracts); Unity's URP manual page; Buchsbaum et al. 2003 (ACM DL, dblp,
  SIAM); Kierstead, Smith & Trotter (arXiv listing, ScienceDirect PII S0195669815001328, the
  Arizona and Georgia Tech records); Bender et al. 2026 (arXiv listing with authors and abstract);
  Pisarchyk & Lee 2020 (arXiv listing, ADS); Muratov (Level Up Coding listings, the Godot
  credit); LunarG's GPU-AV white paper and its buffer-device-address update (lunarg.com extracts,
  the `gpuav_max_buffer_device_addresses` setting from a validation-layers issue); NVIDIA's do's
  and don'ts (extracts of the bullets quoted); GPUOpen's RDNA guide and barriers article
  (extracts); Tatarchuk 2015 (GDC Vault 1021926, the Advances page, archive.org listing);
  Andersson 2011 (SlideShare listings and two write-ups of the deferred-context slides); Geffroy,
  Gneiting & Wang 2020 (the Advances 2020 index and PDF listing) and Gneiting's statements (the
  dsogaming article and its HN thread, both by extract); Wihlidal 2018 (wihlidal.com and the EA
  media PDFs by listing); Malyshau 2026 (arXiv listing with the abstract's numbers); Jones 2020
  (khronos.org listing; fetched in full on 2026-09-23 for `task-system.md`); the `synchronization2`
  rationale for events (a Khronos proposal extract).
- **Weaker confirmations, stated plainly.** The 40–50 % Frostbite figure is a third party's
  summary of the slides. The "~100 jobs per frame" and "preconstructed job graphs" for id Tech 7
  are press reports of Gneiting's remarks, not the talk. The Kierstead–Smith–Trotter journal
  venue is inferred from the ScienceDirect PII in the search record. Buchsbaum et al.'s bound is
  described only as relating OPT to LOAD, since the extract's exact constant was not
  cross-checked. The Andersson slide wording ("chunks of approximately 256") is from a write-up
  of the slides. Halcyon's "transient resources" wording is from a search extract of the blog.
  Unreal's `FRDGTransientResourceAllocator` is named from the brief and Epic's API index in the
  search record, not from source. No number in this file for `bufferImageGranularity` or for the
  driver's per-command-buffer cost was found; both are to be measured on the 5070 Ti.
- **Forge's own numbers** are from `docs/PROFILE.md`, `docs/demos/city-blocks.md`, `docs/ROADMAP.md`
  and D-020 as of 2026-09-26, and the code facts from `crates/forge-gpu/src/{graph,frame,commands,
  memory,timers}.rs`, `crates/forge-render/src/meshlet.rs`, `crates/forge-app/src/lib.rs` and
  `crates/forge-task/src/{pool,scope,lib}.rs` on `main` that day.
- **Numbers to re-check before they enter a spec:** every figure in the recommendation's
  "Expected numbers" is an estimate to be replaced by the counters of steps 4–6; the 0.5 ms / 100
  passes gate is a proposal for the owner, not a measurement.
