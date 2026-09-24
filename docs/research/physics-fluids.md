# Research — Rigid-body physics, motion, destruction and fluids

An annotated bibliography for the physical side of Forge: which rigid-body engine to bind, the solver
theory needed to judge one, character and vehicle motion, buoyancy and boats, destruction, and the
water and fluid simulation that has to match or beat Enshrouded. Organised by problem. The
constraints it is written against: physics must be deterministic enough for a server-authoritative
tick with client prediction, live in an `f64` world through local `f32` islands, run on 8 server
threads and many client threads, carry characters, vehicles, ragdolls and destruction, and sit next to
a GPU fluid simulation that never becomes gameplay truth. Labels are the ones used in
[RESEARCH.md](../RESEARCH.md): **[paper] [book] [talk] [web] [code]** for the kind of source, and
**foundational / still-current / recent** for how it has aged. Every entry was checked against at
least one reachable page (§10 says how, and §9 lists what could not be checked).

> **State of the art in five sentences.** Rigid-body solvers converged in 2019–2024 on *sub-stepped
> soft-constraint sequential impulses* ("soft step" / TGS-soft): Macklin et al. showed that small steps
> beat more iterations, Catto's Solver2D measured eight solvers head to head and picked TGS_Soft for
> Box2D v3, and Avian, Jolt (sequential impulses with warm starting) and Box3D all sit on that family.
> Jolt Physics is the only permissively licensed engine that has shipped two AAA open worlds, has a
> documented cross-platform deterministic mode and a double-precision build, and has a character
> controller, vehicles, ragdolls and soft bodies in one library; Box3D (June 2026, v0.1.0) brings
> Catto's default-on, worker-count-independent determinism to 3D with a C17 API but is three months
> old and explicitly alpha. Destruction in shipped games is either pre-fractured pieces on a support
> graph (Blast, Chaos, Red Faction) or voxels edited by deterministic commands (Teardown, Noita,
> Enshrouded), and the networking lesson from Teardown's 2026 multiplayer is to replicate destruction
> as commands and everything else as prioritised state. Game water is tiered: an analytic or FFT
> spectrum far away, a heightfield shallow-water or column-automaton solver where the player digs and
> swims, and GPU particles (PBF/FLIP/MPM) only as visual splashes seeded from the heightfield, which
> is exactly how Chentanez & Müller coupled them in 2010. Nothing volumetric is gameplay-authoritative
> at 60 fps in a large world; the authoritative water is a coarse, deterministic column model on the
> CPU, and Enshrouded's own "voxel water" (November 2025) is the public proof that this is enough.

**Contents**

1. [Rigid-body engine candidates](#1-rigid-body-engine-candidates)
2. [Solver theory needed to judge them](#2-solver-theory-needed-to-judge-them)
3. [Characters, vehicles, ragdolls and boats](#3-characters-vehicles-ragdolls-and-boats)
4. [Destruction](#4-destruction)
5. [Fluids and water](#5-fluids-and-water)
6. [Large worlds and networking](#6-large-worlds-and-networking)
7. [Rust ecosystem](#7-rust-ecosystem)
8. [Recommendation for Forge](#8-recommendation-for-forge)
9. [Checked and left out](#9-checked-and-left-out)
10. [Verification notes](#10-verification-notes)

---

## 1. Rigid-body engine candidates

### Box3D — what it actually is today

**Erin Catto. "Box3D: a 3D physics engine for games." GitHub repository, 2026.** [code] [recent]
<https://github.com/erincatto/box3d>

The repository was created on 10 May 2026, has a single tag, `v0.1.0`, and on 20 September 2026
showed 6,430 stars, 342 forks and 18 open issues; pull requests are disabled and feedback goes through
issues and Discord. It is portable C17 (C++20 only for the samples), MIT, builds on Windows, Linux and
macOS, and the README lists: continuous collision, convex hulls / capsules / spheres / triangle meshes
/ height fields, multiple shapes per body, sensors, ray, shape and overlap queries, a "Character
mover"; a "Robust Soft Step rigid body solver", continuous physics for fast rotation, island-based
sleep, revolute / prismatic / distance / motor / weld / wheel joints with limits, motors, springs and
friction; "Extensive multithreading and SIMD", "Optimized for large piles of bodies", "Cross platform
determinism" and "Recording and replay". There is no spherical or 6-DoF joint, no ragdoll or soft-body
layer, no vehicle beyond the wheel joint, and no GPU path.
*Bearing:* the feature list is a rigid-body core, not a game-physics library like Jolt; everything Forge
needs above that (ragdolls, vehicles, a real character controller) would be Forge's own code.

**Erin Catto. "Announcing Box3D." Box2D blog, 30 June 2026.** [web] [recent]
<https://box2d.org/posts/2026/06/announcing-box3d/> (press summary:
<https://80.lv/articles/box3d-new-open-source-3d-physics-engine>)

Catto wrote Box3D for *The Legend of California*, an Unreal survival game, after Chaos gave him
falling trees that "moved erratically, teleporting around the screen", no gyroscopic torque, and no
control over a broadphase for hundreds of thousands of server entities. It started as a fork of
Rubikon-Lite (Valve's Half-Life: Alyx physics) in which he "replaced almost all the APIs, data
structures, and algorithms in Rubikon-Lite with Box2D code", keeping Rubikon code in convex-hull
generation and some collision routines. The post lists a sub-stepping solver, a wide-SIMD contact
solver, graph colouring for large islands, cross-platform determinism and large-world support with
doubles, and says plainly: "I still consider Box3D to be alpha software." Planned work is the character
mover, ghost collisions and joint solving; users are the Legend of California, s&box (Facepunch),
the Esoterica engine and, per 80.lv, a multiplayer space game by Glenn Fiedler. The first engineering
post ("SIMD for Collision", 18 July 2026, <https://box2d.org/posts/2026/07/simd-for-collision/>)
vectorised the hull–hull separating-axis tests: 40,706 ms scalar to 17,337 ms with SSE2 on one thread,
with no effect on box–box pairs and no word on what SIMD does to determinism.
*Bearing:* the pedigree is the best in the field and the users are serious, but the API is unstable
by the author's own account and the README does not repeat the "doubles" claim — check the headers
before relying on it. Track it; do not build the first milestone on it.

### Jolt Physics

**Jorrit Rouwé. "Jolt Physics." GitHub repository, v5.0.0–v5.6.0, 2024–2026.** [code] [still-current]
<https://github.com/jrouwe/JoltPhysics> (architecture:
<https://github.com/jrouwe/JoltPhysics/blob/master/Docs/Architecture.md>; releases:
<https://github.com/jrouwe/JoltPhysics/releases>)

MIT, C++17, "used by Horizon Forbidden West and Death Stranding 2", integrated into Godot and the
Source engine, with community Unreal plugins; platforms are Windows, Linux, FreeBSD, Android, macOS,
iOS, MinGW and WebAssembly. Releases come every three to eight months, from 5.0.0 (5 April 2024) to
5.4.0 (27 September 2025, Cosserat rods for hair and rope) and 5.6.0 (11 July 2026, "an interface to
run compute shaders on the GPU with implementations for DX12, Vulkan and Metal"). The Architecture
document is exact about
determinism: results are identical if "the APIs that modify the simulation are called in exactly the
same order" and "the same binary code is used"; the `CROSS_PLATFORM_DETERMINISTIC` build is
"approximately 8% slower" and then independent of compiler, OS, architecture and word size, provided
the code is compiled in precise floating-point mode with contraction off (`-ffp-contract=off`). Not
deterministic: broad-phase query results (the tree is modified from several threads), the *order* of
narrow-phase results, and the order of listener callbacks. `JPH_DOUBLE_PRECISION` stores positions
as doubles and drops to floats "as soon as possible", at a 5–10 % cost. The solver is sequential
impulses with warm starting; islands sleep as a unit and wake on contact.
*Bearing:* the only candidate that satisfies (a) to (e) of the brief today, in one library, with two
shipped open worlds behind it. Its determinism contract is exactly what a Windows client and a Linux
server need, and the listed exceptions are the tests to write first.

**Jorrit Rouwé. "Architecting Jolt Physics for Horizon Forbidden West." GDC 2022.** [talk]
[still-current]
<https://jrouwe.nl/architectingjolt/> (video:
<https://gdcvault.com/play/1027560/Architecting-Jolt-Physics-for-Horizon>; slides:
<https://media.gdcvault.com/GDC+2022/Speaker+Slides/ArchitectingJoltPhysics_Rouwe_Jorrit.pdf>)

Guerrilla replaced a commercial engine with Jolt and "saved memory, executable size and were able to
double our simulation frequency while using less CPU time." The two systems explained are the
lock-free broad phase, so streaming a world tile in never stalls the simulation, and lock-free island
building, so the multithreaded game-object update and the physics step do not serialise on each other.
*Bearing:* both problems are Forge's: streaming chunks of a huge world while simulating, and an ECS
update that must not block on physics. This is the design rationale to read before writing the
binding.

**Jorrit Rouwé. "Jolt Physics Multicore Scaling" and "Performance Test." jrouwe.nl and repository
docs, 2022–2025.** [web] [still-current]
<https://jrouwe.nl/jolt/JoltPhysicsMulticoreScaling.pdf>;
<https://github.com/jrouwe/JoltPhysics/blob/master/Docs/PerformanceTest.md>;
<https://github.com/jrouwe/JoltPhysics/discussions/327>

The scaling document drops 16 piles of 10 motorised ragdolls (3,680 bodies) on a level and compares
Jolt with PhysX 4.1 (default 4 position + 1 velocity iterations) and Bullet 3.21 (5 iterations) on an
i7-7700HK laptop, 3rd-gen Xeon and EPYC, and Graviton 2, with source for the PhysX and Bullet ports;
the text conclusion is that "PhysX has a much higher penalty for CCD at higher thread counts than
Jolt" (the curves are images; the numbers are not in the text). The performance-test tool has four
scenes — the ragdoll pile, 484 convex shapes on a 2,000-triangle mesh, a 1,240-box pyramid "to profile
large island splitting", and 4,410 boxes on a large mesh — takes `-t` threads and `-i` iterations, and
prints steps per second and "a determinism hash of final body positions and rotations". In the Jolt
vs PhysX discussion Rouwé says "PhysX is obviously a much bigger project and it has more features",
that dense clusters used to defeat Jolt's multicore scaling (since fixed), and Pierre Terdiman
(PhysX) warns that "the 'winner' in these tests depends on where you take your performance samples."
*Bearing:* the ragdoll scene and the hash are the determinism and scaling test bed for free; Forge's
spike should run it at 1, 4, 8 and 16 threads and diff the hash before writing a line of gameplay.

### The others

**NVIDIA. "PhysX SDK 5" and its documentation (Rigid Body Dynamics, GPU Rigid Bodies).** [code] [web]
[still-current]
<https://github.com/NVIDIA-Omniverse/PhysX>;
<https://nvidia-omniverse.github.io/PhysX/physx/5.4.1/docs/RigidBodyDynamics.html>;
<https://nvidia-omniverse.github.io/PhysX/physx/5.4.0/docs/GPURigidBodies.html>

BSD-3 since the 2022 open-sourcing; the README today says PhysX SDK 5.11.0 and the repository also
carries the Blast (destruction) and Flow (fluid, fire) SDKs and `ovphysx`, a C API with Python
bindings aimed at Omniverse and Isaac Sim. The TGS solver splits the step into as many sub-steps as
position iterations, solving each constraint once per sub-step and integrating immediately, which the
docs credit with "improved convergence", better high mass ratios and joint drives, at a slightly higher
per-iteration cost than PGS. Determinism is "limited": identical results require "the same scene using
the same time-stepping scheme and same PhysX release running on the same platform", and
`eENABLE_ENHANCED_DETERMINISM` additionally guarantees that "the simulation of an island will be
identical regardless of any other islands in the scene" for a performance cost. GPU rigid bodies move
contact generation, shape management and the solver to CUDA with fixed pre-sized buffers
(`gpuDynamicsConfig`) that discard contacts when full; D6 joints are native on the GPU.
*Bearing:* the GPU rigid-body pipeline is unique and would suit a client-side debris field, but a
server on a 3080 with fixed buffers and platform-only determinism is the wrong authority. The C++
surface is enormous and the Rust binding is archived (§7).

**Dimforge. "Rapier" — repository, determinism guide, launch benchmarks.** [code] [web] [still-current]
<https://github.com/dimforge/rapier>; <https://rapier.rs/docs/user_guides/rust/determinism>;
<https://www.dimforge.com/blog/2020/08/25/announcing-the-rapier-physics-engine/>

Apache-2.0, pure Rust; `rapier3d` 0.35.3 (28 August 2026, 1.66 M downloads) with `f64` twins,
`parallel` (rayon across broad phase, narrow phase and solver), `simd-stable` / `simd-nightly` and
`enhanced-determinism`. By default Rapier is "locally deterministic" only; with the feature it is
cross-platform, on the conditions that bodies, colliders and joints are inserted in the same order,
that the target strictly follows IEEE 754-2008, that transcendental functions go through nalgebra's
`ComplexField` rather than `f32::sin`, and that the wide-SIMD (`simd8`) path is not enabled. The 2020
launch post is the only benchmark from the authors: on a Ryzen 9 3900X and an i7-7920HQ, over 8,000
balls, 3,000 boxes, a 4,900-box pyramid, a trimesh with 3,000 bodies, 5,320 KEVA planks and joint
scenes, "PhysX and Rapier are both 4 to 8 times faster than nphysics", Rapier slightly ahead of PhysX
on the Ryzen and slightly behind on the Intel; in 2D it is slightly faster than Box2D v2.4 with more
stable joints. That predates Box2D v3, Jolt 5 and Avian 0.4.
*Bearing:* the safe pure-Rust fallback (wasm, no C++ toolchain) and the one whose determinism rules
match Forge's own `dmath` rule word for word; but it has no vehicles beyond a raycast controller, no
soft bodies, no ragdoll tooling, and no third-party 2024–2026 benchmark could be verified.

**Joona Aalto. "Avian" — repository and release posts 0.1 (2024) and 0.4 (2025).** [code] [web]
[recent]
<https://github.com/Jondolf/avian>; <https://joonaa.dev/blog/06/avian-0-1>;
<https://joonaa.dev/blog/09/avian-0-4>

MIT/Apache, ECS-native for Bevy ("no wrappers around existing engines"); `avian3d` 0.7.0 (20 June
2026) targets Bevy 0.19 and exposes `enhanced-determinism`, `parallel`, `f64`, `simd` and
`xpbd_joints`. The 0.1 post (the rename from `bevy_xpbd`) explains the move from XPBD contacts to an
impulse-based "TGS Soft" solver "using substepping and soft constraints together with warm starting
and relaxation": Bevy's maintainers had raised NVIDIA's XPBD patent (Macklin later said use should be
fine), XPBD's deep-overlap resolution was "very energetic and explosive", and the new solver was 4–6×
faster in collision-heavy scenes; joints stayed XPBD. Avian 0.4 is "approximately 3x as fast as Avian
0.3" from solver bodies (contiguous memory instead of ECS queries), greedy graph colouring for parallel
constraint solving ("over 3x solver speedup") and persistent simulation islands, and adds voxel
colliders through Parry 0.20; the author tried AVBD and kept impulses for CPU game physics. There is
no built-in character controller or vehicle.
*Bearing:* the clearest Rust re-derivation of the Box2D v3 design and worth reading as a spec; but
Forge is not a Bevy application, and pulling Bevy's ECS in for physics is the wrong dependency.

**Erwin Coumans et al. "Bullet Physics SDK." GitHub repository, v3.25, 2022.** [code] [foundational]
<https://github.com/bulletphysics/bullet3>; <https://github.com/bulletphysics/bullet3/releases>

zlib licence, C++03, last release 3.25 on 24 April 2022; the README now points robotics and
reinforcement-learning users to PyBullet, and OpenCL GPU dynamics stayed experimental. It has a
double-precision build option, kinematic character and raycast vehicle helpers and soft bodies, and it
is the engine Rocket League runs deterministically inside Unreal (§6).
*Bearing:* reference only; the design ideas live on in Jolt and Rapier's vehicle controller, and
nothing new should be built on it.

**Havok. "Havok Physics." Product page, 2026.** [web] [still-current]
<https://www.havok.com/havok-physics/>

Proprietary and licensed by evaluation contact; the page claims it "guarantees identical output across
all supported platforms and targets", lists continuous collision, a robust constraint solver, character
controllers and lightweight "Physics Particles", and names Call of Duty, Assassin's Creed, Elden Ring,
Final Fantasy XVI, Helldivers 2 and id Tech 8 (Doom: The Dark Ages) among over a hundred titles, plus a
drop-in Unreal integration and a Babylon.js build.
*Bearing:* the bar for "deterministic across platforms" that a commercial studio expects; not a
candidate (closed, priced per title, no Rust story).

**Epic Games. "Chaos Physics" — overview, Large World Coordinates, Networked Physics, Destruction
quick start. Unreal Engine 5 documentation, 2022–2026.** [web] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/chaos-physics-overview>;
<https://dev.epicgames.com/documentation/en-us/unreal-engine/large-world-coordinates-in-unreal-engine-5>;
<https://dev.epicgames.com/documentation/en-us/unreal-engine/networked-physics-overview>;
<https://dev.epicgames.com/documentation/en-us/unreal-engine/destruction-quick-start>

"Chaos is Unreal Engine's high-performance physics and destruction system." Under Large World
Coordinates the engine's vectors are doubles (`WORLD_MAX` 88 million km) but Chaos keeps "the data …
stored as a set of floats" and "a sufficiently large world will be divided into grid cells", with the
one explicit narrowing cast made visible — the local-`f32`-islands design in production. Networked
physics offers three replication modes, the newest, Resimulation, running the client "half a Round Trip
Time (RTT) ahead of the server", caching history and rewinding on mismatch. Destruction is Geometry
Collections fractured in Fracture Mode (Uniform Voronoi in the quick start), a hierarchy of levels, a
connection graph ("how the fractured pieces are connected to one another") that breaks under strain,
and physics fields that apply that strain.
*Bearing:* reference for what a full engine ships, and the cautionary tale in Box3D's announcement:
its float-grid large-world scheme is the shape Forge's islands should take, its resimulation mode is
what Forge's prediction must do, and its tree collision is what Catto left.

**Russell Smith et al. "Open Dynamics Engine." ode.org.** [web] [foundational]
<https://www.ode.org/>

LGPL 2.1+ or BSD, C/C++, "stable, mature and platform independent", advanced joints and integrated
collision with friction; development is on Bitbucket and the site gives no current version.
*Bearing:* historical; it is where many of the joint formulations came from, and nothing else.

### Comparison table

Cells marked † are prior knowledge not re-verified for this document; everything else comes from
the entries above.

| Engine | Licence | Language | Determinism | Multithreading | f64 / large world | Character controller | Vehicles | Soft bodies | GPU | Rust bindings | Shipped titles | Maturity risk |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Box3D v0.1.0** | MIT | C17 | Cross-platform and worker-count independent by default (Box2D lineage) | Graph colouring, wide SIMD, optional threads | Announcement says doubles; README silent | "Character mover", enhancements planned | Wheel joint only | No | No | 4 competing crates, weeks old | s&box (in progress), Esoterica, unreleased games | Alpha, API unstable, no PRs, 3 months old |
| **Jolt 5.6** | MIT | C++17 | Same binary by default; cross-platform build (~8 %) across compiler/OS/arch | Job system, lock-free broad phase and islands | `JPH_DOUBLE_PRECISION` (5–10 %) | `CharacterVirtual` + rigid `Character` | Wheeled, tracked, motorcycle; ray/sphere/cylinder testers | Yes (cloth, pressure, rods) | Compute-shader interface (5.6), not rigid bodies | `jolt-rust` (early WIP, Jolt 5.0–5.3) | Horizon Forbidden West, Death Stranding 2, Godot | Low; binding upkeep is the cost |
| **PhysX 5.11** | BSD-3 | C++ | Same platform + release; enhanced-determinism = island independence | Task-based CPU; CUDA pipeline | f32 only† | `PxController`† | Vehicle SDK† | FEM soft bodies† | Yes, rigid bodies and soft bodies | `physx-rs` archived (PhysX 4.1) | Unity, UE4 era† | Huge surface, NVIDIA/Omniverse roadmap |
| **Rapier 0.35** | Apache-2.0 | Rust | `enhanced-determinism` cross-platform (IEEE 754, insertion order, no `simd8`) | rayon across the whole step | `rapier3d-f64` | Kinematic, translation only | Raycast controller | No | No | Native | None verified | Small team, no recent third-party benchmark |
| **Avian 0.7** | MIT/Apache | Rust | `enhanced-determinism` feature | Graph colouring, islands (0.4+) | `f64` feature | None built in | None | No | No | Native (Bevy only) | None verified | Bevy-coupled; joints still XPBD-gated |
| **Bullet 3.25** | zlib | C++03 | Same binary when configured (Rocket League) | Limited† | Double build option | Kinematic helper† | Raycast vehicle† | Yes | OpenCL experimental | None maintained (§9) | Rocket League | Last release 2022; PyBullet focus |
| **Havok** | Proprietary | C++ | "Identical output across all supported platforms" (vendor claim) | Yes† | † | Yes | † | Cloth separate | — | None | CoD, Elden Ring, Helldivers 2, Doom TDA | Closed, priced |
| **Chaos (UE5)** | UE EULA | C++ | Resimulation-based prediction; no cross-platform claim found | Yes† | Float grid cells under LWC (beta) | CharacterMovementComponent† | Chaos Vehicles† | Cloth† | Partial† | None | Fortnite, UE5 titles† | UE-only; Catto's tree/gyro complaints |
| **ODE** | LGPL/BSD | C/C++ | † | No† | Double build† | No | No | No | No | † | Legacy | Legacy |

---

## 2. Solver theory needed to judge them

**Erin Catto. "Fast and Simple Physics using Sequential Impulses." GDC 2006.** [talk] [foundational]
<https://box2d.org/files/ErinCatto_SequentialImpulses_GDC2006.pdf> (index:
<https://box2d.org/publications/>; reference code: <https://github.com/erincatto/box2d-lite>)

The projected Gauss–Seidel formulation every engine in §1 still uses: iterate over constraints,
apply an impulse per constraint, clamp the *accumulated* impulse (so friction and non-penetration stay
in their cones), warm-start from last step's accumulated impulses, and correct drift with Baumgarte
stabilisation. Box2D-Lite is the 1,000-line reference.
*Bearing:* the vocabulary of every solver comparison below; read the slides, then Solver2D.

**Erin Catto. "Soft Constraints: Reinventing the Spring." GDC 2011.** [talk] [foundational]
<https://box2d.org/files/ErinCatto_SoftConstraints_GDC2011.pdf>

Replaces stiffness and damping coefficients with a frequency and a damping ratio, solved implicitly
as a constraint with mass-independent tuning, so a "spring" never explodes and a hard constraint is
just the limit. This is the "soft" in soft step; Box2D v3 applies it to contacts as well as joints.
*Bearing:* the reason a character standing on a ship a thousand times its mass does not jitter, and
the parameters (hertz, damping ratio, push-out velocity) Forge will expose.

**Erin Catto. "Physics for Game Programmers: Continuous Collision." GDC 2013.** [talk] [foundational]
<https://box2d.org/files/ErinCatto_ContinuousCollision_GDC2013.pdf> (video:
<https://gdcvault.com/play/1017644/Physics-for-Game-Programmers-Continuous>)

Time-of-impact sweeps for the fastest bodies versus *speculative contacts*: look ahead for the
contact points a body would reach this step and add them as constraints that limit the velocity toward
them, so bullets and thrown debris stop at surfaces without sub-stepping the whole world. Both Jolt
and Box3D ship continuous collision built on these ideas.
*Bearing:* speculative contacts are what makes a 60 Hz server tick safe for projectiles, falling trees
and boats hitting rocks; time-of-impact only for the few things that must never tunnel.

**Miles Macklin, Matthias Müller, Nuttapong Chentanez. "XPBD: Position-Based Simulation of Compliant
Constrained Dynamics." Motion in Games (MIG), 2016.** [paper] [foundational]
<https://mmacklin.com/xpbd.pdf>; <https://dl.acm.org/doi/10.1145/2994258.2994272>

Fixes position-based dynamics' dependence on iteration count and time step by adding a compliance
term (inverse stiffness) and a Lagrange-multiplier update, so stiffness becomes a physical constant
and the solver handles arbitrary elastic and dissipative potentials implicitly.
*Bearing:* the right solver for ropes, cloth, sails, nets and bending plants (all client-visual); the
rigid-body engines have moved away from it for contacts, and Avian's experience says why.

**Miles Macklin, Kier Storey, Michelle Lu, Pierre Terdiman, Nuttapong Chentanez, Stefan Jeschke,
Matthias Müller. "Small Steps in Physics Simulation." SCA 2019.** [paper] [still-current]
<https://mmacklin.com/smallsteps.pdf>; <https://dl.acm.org/doi/10.1145/3309486.3340247>

The observation behind TGS and soft step: one large step with *n* solver iterations is less
effective than *n* small steps with one iteration each, because error falls with the square of the
step size while iterations only redistribute it. Sub-stepping needs collision data reused across
sub-steps, which is what PhysX TGS, Box2D v3 and Avian do.
*Bearing:* the paper that lets Forge run 4 sub-steps × 1 iteration at 60 Hz instead of 1 × 8, and
get better stacks, mass ratios and joint drives for the same cost.

**Matthias Müller, Miles Macklin, Nuttapong Chentanez, Stefan Jeschke, Tae-Yong Kim. "Detailed Rigid
Body Simulation with Extended Position Based Dynamics." SCA 2020 / Computer Graphics Forum 39(8).**
[paper] [still-current]
<https://matthias-research.github.io/pages/publications/PBDBodies.pdf>;
<https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.14105>

Rigid bodies inside XPBD with a quasi-explicit, unconditionally stable sub-stepped scheme that works
with the most recent constraint directions rather than a linearisation, so it traces high-speed
motion against curved geometry and needs fewer constraints. Solver2D's "XPBD" column is this method.
*Bearing:* the alternative lineage; useful for coupling a rigid body to ropes and cloth in one solver
on the client, not for the authoritative step.

**Erin Catto. "Solver2D." Box2D blog and repository, 5 February 2024.** [web] [code] [still-current]
<https://box2d.org/posts/2024/02/solver2d/>; <https://github.com/erincatto/solver2d>

The best public solver comparison: PGS (Baumgarte), PGS + NGS, block PGS, PGS_Soft, TGS_Sticky,
TGS_Soft, TGS_NGS and XPBD implemented in one MIT test bed and run on large mass ratios, long chains,
big stacks, deep overlap and scenes 30 km from the origin, at 4 primary / 2 secondary iterations and
60 Hz. Conclusions carried into Box2D v3: "smaller time steps are more effective than more
iterations"; XPBD was "fairly simple to implement" but needed to track position deltas to survive far
from the origin; TGS_Soft — sub-stepping plus soft constraints plus a relax pass — became "Soft Step".
*Bearing:* the 30 km scene is the argument for local-frame `f32` islands, whatever the solver; and the
ranking is why Box3D, Avian and (in spirit) Jolt are all on the sub-stepped-impulse side.

**Erin Catto. "Releasing Box2D 3.0" and "Determinism." Box2D blog, 9 and 27 August 2024.** [web]
[still-current]
<https://box2d.org/posts/2024/08/releasing-box2d-3.0/>; <https://box2d.org/posts/2024/08/determinism/>

The release post: Soft Step is "more stable in almost every way than version 2.4", the engine uses
graph colouring and enkiTS for threads, an AVX2 contact solver (body state fits in 32 bytes), "v3 is
more than twice as fast as v2.4", and it "scales quite well with cores as long as they share an
L2/L3 cache". The determinism post defines the three levels — algorithmic, multithreaded (independent
of worker count) and cross-platform — and states "There are no settings for this. It is the default."
The recipe: no fast-math, FMA off (`-ffp-contract=off`), custom `sin`/`cos`/`atan2` because "atan2f
gives different answers on different platforms", `sqrtss` is fine, and a "Falling Hinges" CI scene
that hashes final transforms across compilers and architectures. It also says "Box2D does not have
roll-back determinism": there is no snapshot of solver-internal state to rewind.
*Bearing:* this is the determinism policy Box3D inherits, and the checklist Forge should hold any
engine to; the roll-back caveat means prediction must re-simulate forward from a server state, not
rewind an engine.

**Erin Catto. "Simulation Islands." Box2D blog, 8 October 2023.** [web] [still-current]
<https://box2d.org/posts/2023/10/simulation-islands/>

Islands exist mainly for sleeping — "Sleeping rigid bodies are removed from the solver and this
drastically reduces their CPU load" — and can be simulated on separate threads. Rebuilding them with
depth-first search every step is deterministic but serial (Amdahl); parallel union-find is fast but
makes constraint order non-deterministic and breaks warm starting; persistent islands, merged on
contact and split lazily, are about 10× faster than DFS and keep determinism.
*Bearing:* physics LOD in a huge world is mostly sleeping done well; persistent islands are also the
natural unit for Forge's per-island local frames and for handing an island between server workers.

---

## 3. Characters, vehicles, ragdolls and boats

**Jorrit Rouwé. "Character" and "CharacterVirtual" in the Jolt Architecture document.** [web]
[still-current]
<https://github.com/jrouwe/JoltPhysics/blob/master/Docs/Architecture.md>

Two models. `Character` is "essentially a rigid body that has been configured to only allow
translation", simulated with everything else and meant for simple AI. `CharacterVirtual` "is
implemented using collision detection functionality only (through NarrowPhaseQuery)", is not in the
world so rigid bodies never see it, and adds sliding along walls, elevators and moving platforms,
steep-slope detection, stair stepping (`ExtendedUpdate`), sticking to the ground downhill, a custom
local coordinate system ("walking in rotating spacecraft"), concurrent updates of many characters, and
its own `CharacterContactListener`; since 5.1 virtual characters collide with each other.
*Bearing:* the kinematic capsule-and-sweep model the previous project already planned, with the two
features it needs and rarely gets — an arbitrary "up" for ships and stations, and platforms carrying
the character — already there.

**Unity Technologies. "Character Controller." Unity Manual.** [web] [still-current]
<https://docs.unity3d.com/Manual/class-CharacterController.html>

The industry-default kinematic controller: "a capsule shaped Collider which can be told to move in some
direction", which "does not react to forces on its own and it does not automatically push Rigidbodies
away", with Slope Limit, Step Offset, Skin Width ("one of the most critical properties to get right")
and Min Move Distance to kill jitter.
*Bearing:* the four parameters every designer expects; expose exactly these, with the same names.

**Dimforge. "Character controller." Rapier user guide.** [web] [still-current]
<https://rapier.rs/docs/user_guides/rust/character_controller>

A shape-cast kinematic controller with max-climb and min-slide angles, autostep (height and width
limits), snap-to-ground and platform support, translation only, and gravity left to the user, on the
honest note that "character-control (especially for the player's character itself) is often very
game-specific."
*Bearing:* a compact Rust reference implementation of collide-and-slide to compare Forge's own
controller against; being pure Rust it is also the easiest to make bit-identical on client and server.

**Jorrit Rouwé. "VehicleConstraint" and `VehicleCollisionTester` (Jolt); Dimforge,
`DynamicRayCastVehicleController` (Rapier).** [code] [still-current]
<https://github.com/jrouwe/JoltPhysics/blob/master/Jolt/Physics/Vehicle/VehicleCollisionTester.h>;
<https://docs.rs/rapier3d/latest/rapier3d/control/struct.DynamicRayCastVehicleController.html>

Jolt's vehicle is a constraint on a chassis body with virtual wheels or tracks (wheeled, tracked and
motorcycle controllers), and the wheel–ground test is pluggable: `VehicleCollisionTesterRay` (a
raycast), `VehicleCollisionTesterCastSphere` and `VehicleCollisionTesterCastCylinder`. Rapier's
controller is the classic raycast vehicle ("using ray-casting for the wheels") with suspension rest
length and radius per wheel, engine force, brake and steering.
*Bearing:* raycast wheels are deterministic, cheap and what most games ship; a sphere or cylinder cast
is the upgrade for kerbs, rubble and voxel edges. Both run on the server tick; no wheel colliders as
bodies.

**Jacques Kerner (Avalanche Studios). "Water interaction model for boats in video games." Game
Developer (Gamasutra), 27 February 2015.** [web] [still-current]
<https://www.gamedeveloper.com/programming/water-interaction-model-for-boats-in-video-games>

The reference buoyancy model for boats: cut the hull mesh by the water surface, keep the submerged
triangles, apply hydrostatic pressure per triangle with a corrected application point (centroids
produce spurious torques), and sample the water either analytically ("if the water is flat or
described by a simple function … directly sampling it") or from a local height patch; under 1 ms per
boat, with drag and slamming in the follow-up. Kerner is candid that it is "creative physics".
*Bearing:* this is the coupling Forge needs between a rigid body and the wave field of §5: the server
samples the same deterministic height function the client renders, per triangle, per tick.

**Cem Yuksel, Donald H. House, John Keyser. "Wave Particles." ACM Transactions on Graphics (SIGGRAPH)
26(3), 2007.** [paper] [foundational]
<http://www.cemyuksel.com/research/waveparticles/>

Waves as particles that the GPU splats into a height field with horizontal warping, "unconditionally
stable", with two-way coupling: floating bodies receive wave forces and emit waves, so hundreds of
boats with propellers and rudders run in real time.
*Bearing:* the cheapest good wake and splash model for ships and swimmers, purely visual, layered on
the analytic field; and a way to spawn the near-field detail without a fluid solver.

**Ragdolls and physical animation.** Jolt ships `RagdollSettings` / `Ragdoll` with motorised
constraints (5.5.0 added "ragdoll constraint priority calculation"), and its performance scene is a
pile of *motorised* ragdolls, which is the "active ragdoll" case: an animated pose drives joint motors
on a simplified body that still reacts to hits. No separate paper of standing was found for active
ragdolls (§9); the previous project's note on Euphoria-style controllers stands as prior knowledge.
*Bearing:* server-side only the resting pose and hit volumes; the detailed motorised ragdoll is
client-side, which is why its non-determinism does not matter.

---

## 4. Destruction

**NVIDIA. "Blast" — GameWorks repository (1.1.5) and blast-sdk 5.0.6 documentation.** [code] [web]
[still-current]
<https://github.com/NVIDIAGameWorks/Blast>; <https://nvidia-omniverse.github.io/PhysX/blast/index.html>

The successor to APEX Destruction, in three layers: NvBlast, a C API of stateless functions that "do
not spawn tasks, allocate, or deallocate memory"; NvBlastTk, the C++ toolkit that owns assets, actors
and damage processing and splits actors on events; and extensions for authoring (Voronoi fracture from
a point cloud, hierarchical splitting), serialisation and a PhysX reference integration. The runtime
idea is the *support graph*: chunks at any hierarchy depth are nodes, bonds are edges, damage removes
bonds, and connected components become separate actors. It is "physics and graphics agnostic" — no
collision or render representation of its own — and now lives in the PhysX repository as blast-sdk.
*Bearing:* the support graph, not the fracture tool, is the reusable part: it is exactly the structure
a sandbox building system needs (what stands, what falls when a beam goes), and it is small enough to
re-implement in Rust rather than bind.

**Epic Games. "Destruction Quick Start" (Chaos Destruction). Unreal Engine 5.8 documentation.** [web]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/destruction-quick-start>

Geometry Collections are fractured in the editor (Uniform Voronoi with a site range in the tutorial),
producing levels of clusters ("Level 0 has 1 piece, and Level 1 has 20 pieces"), a connection graph
between pieces that breaks under strain, and physics fields that deliver the strain at impact.
*Bearing:* the same pre-fracture-plus-support-graph pattern as Blast, packaged for artists; Forge's
constructs (built from pieces) can skip the fracturing step because the pieces already exist.

**Dennis Gustafsson. "The unlikely story of Teardown Multiplayer." Voxagon blog, 13 March 2026.** [web]
[recent]
<https://blog.voxagon.se/2026/03/13/teardown-multiplayer.html> (blog index: <https://blog.voxagon.se/>)

The most relevant destruction-networking write-up that exists. Teardown is a fully destructible voxel
world with mods, and its multiplayer is "semi-deterministic": a host acts as server; every scene-
modifying change travels on a reliable stream as deterministic commands ("cut hole in this shape at
voxel coord x,y,z", "change ownership of that shape", "reconnect joint to this shape"), so bandwidth is
"the same regardless of object size" and the destruction code was "rewritten … in fixed-point integer
math" to make the replay exact; everything else — transforms and velocities of debris — is eventual
consistency over unreliable packets, chosen per client from a priority queue by proximity within about
1 Mbit/s. Where many bodies need correction the author admits "visible snapping". Scripts keep client
and server parts in one file for mod compatibility.
*Bearing:* the design Forge should copy: destruction as tick-stamped deterministic commands (already
the shape of `SimCommand`s), body state as prioritised snapshots, and integer or fixed-point maths for
the edit itself so the voxel or SDF result never depends on a float.

**Petri Purho (Nolla Games). "Exploring the Tech and Design of Noita." GDC 2019.** [talk]
[still-current]
<https://www.gdcvault.com/play/1025695/Exploring-the-Tech-and-Design>;
<https://www.youtube.com/watch?v=prXuyMCgbTc>

A falling-sand automaton scaled to a large continuous world: the world is 64×64-pixel chunks,
liquids and gases use the same simple rules as sand, chunks are updated in a checkerboard so threads
never touch neighbours at once, and rigid bodies are cut out of the pixel world, simulated, and
rasterised back in.
*Bearing:* the one shipped proof that cellular water and rigid bodies coexist at scale, and the
checkerboard scheduling is directly the pattern for a deterministic parallel column-water solver.

**Volition. "Geo-Mod 2.0" (Red Faction: Guerrilla, 2009). Red Faction Wiki.** [web] [foundational]
<https://www.redfactionwiki.com/wiki/Geo-Mod_2.0>

"Pre-broken meshes that respond in a realistic manner to physical forces" with a stress model: weaken
a building at key structural points and it collapses. The wiki records the GDC lectures of 2009 only
as having happened (§9).
*Bearing:* the earliest shipped structural-stress destruction, and the gameplay bar for "sandbox
destruction": the player reasons about supports, not hit points.

---

## 5. Fluids and water

### Surfaces: oceans, lakes, rivers

**Jerry Tessendorf. "Simulating Ocean Water." SIGGRAPH course notes, 1999–2004 (2001 edition
commonly cited).** [paper] [foundational]
<https://jtessen.people.clemson.edu/reports/papers_files/coursenotes2004.pdf> (2001 slides:
<https://www.cg.tuwien.ac.at/courses/Rendering/Tessendorf_SIGGRAPH_Slides2001.pdf>)

The statistical ocean: a Phillips spectrum of wave amplitudes with dispersion ω² = gk, synthesised by
inverse FFT into a tiling height field, plus the choppy horizontal displacement and the Jacobian that
flags breaking crests for foam. Still the basis of Sea of Thieves and of every "FFT ocean" asset.
*Bearing:* the far tier. Because the field is a sum of known waves it can be evaluated analytically
(or from a small band-limited spectrum on the CPU with `dmath`) at a ship's hull, so the server and
client agree on the height without exchanging it.

**Mark Finch (Cyan Worlds). "Effective Water Simulation from Physical Models." GPU Gems, chapter 1,
2004.** [book] [foundational]
<https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models>

Sums of sines and Gerstner waves — vertices move in circles, so crests sharpen with a steepness
parameter — combined as "geometric undulations of a base mesh with generation of a dynamic normal map".
*Bearing:* the cheap analytic alternative to FFT for lakes, directed swell and the deterministic
CPU evaluation; a handful of Gerstner trains is what the boat solver should sample.

**Nigel Ang, Ed Catling, Valentino Ciardi, Nikolay Kozin (Rare). "The Technical Art of Sea of
Thieves." SIGGRAPH 2018 Talks.** [talk] [still-current]
<https://history.siggraph.org/wp-content/uploads/2022/09/2018-Talks-Ang_The-Technical-Art-of-Sea-of-Thieves.pdf>;
<https://dl.acm.org/doi/10.1145/3214745.3214820>

How Rare stylised an FFT ocean (Tessendorf 2001) in Unreal 4: colour from a scattering approximation
blended by view angle, sun and a wave-peak mask derived from the FFT choppiness offsets, foam and
detail on top. It is a rendering talk; the ship physics is not described.
*Bearing:* the reference look for open water; and confirmation that even the benchmark ocean game
runs an FFT surface, not a fluid solver.

**Hugh Malan (Guerrilla). "Rendering Water in Horizon Forbidden West." SIGGRAPH 2022, Advances in
Real-Time Rendering in Games.** [talk] [recent]
<https://advances.realtimerendering.com/s2022/SIGGRAPH2022-Advances-Water-Malan.pdf>

The headline feature is breaking waves with an overhanging shape, authored in the editor rather than
simulated because "art directability is a priority", then "baked out, stored with the world tile data,
and streamed in at runtime".
*Bearing:* shorelines at AAA quality are baked animation per tile, not a solver; a procedural world
can bake the same way at chunk-build time from the shore geometry.

**Nuttapong Chentanez, Matthias Müller. "Real-time Simulation of Large Bodies of Water with Small
Scale Details." SCA 2010.** [paper] [foundational]
<https://matthias-research.github.io/pages/publications/hfFluid.pdf>;
<https://dl.acm.org/doi/10.5555/1921427.1921457> (follow-up, tall cells, SIGGRAPH 2011:
<https://matthias-research.github.io/pages/publications/tallCells.pdf>)

A GPU shallow-water height-field solver over arbitrary terrain with wet–dry tracking and
non-reflecting open boundaries, coupled to particles: wherever the height field cannot represent the
liquid — breaking waves, waterfalls, splashes from rigid and soft bodies — it turns that water into
spray, splash and foam particles that "exchange mass and momentum with the height field fluid", with
small procedural waves advected by the flow. Everything is CUDA and real time. The 2011 tall-cell grid
adds a thin 3D layer on top for overturning.
*Bearing:* the mid tier and the coupling recipe in one paper: heightfield where the player digs and
swims, particles born from it where it fails, rigid bodies pushing it. This is the design Forge's
gameplay water should be a coarse, deterministic CPU shadow of.

**Alex Vlachos (Valve). "Water Flow in Portal 2." SIGGRAPH 2010, Advances in Real-Time Rendering in
Games.** [talk] [foundational]
<https://advances.realtimerendering.com/s2010/index.html> (slides linked from the course page)

Flow maps: a painted or generated 2D vector field advects the normal map in two phase-offset layers
that cross-fade, so rivers and eddies read as flowing without any simulation.
*Bearing:* rivers at distance and any water with a known drainage direction — which a hydrology-first
terrain already has — cost a texture lookup; the flow field can come straight from the flow
accumulation pass.

### Volumetric liquid for gameplay moments

**Matthias Müller, David Charypar, Markus Gross. "Particle-Based Fluid Simulation for Interactive
Applications." SCA 2003.** [paper] [foundational]
<https://matthias-research.github.io/pages/publications/sca03.pdf>

SPH for games: smoothing kernels for density, pressure and viscosity, surface tension, and a few
thousand particles at interactive rates on 2003 hardware. Everything since is faster SPH or a
different constraint on the same particles.
*Bearing:* the vocabulary for the near tier; not the solver to ship (compressibility and stiffness
make it stiff and slow), but the baseline any particle water is measured against.

**Miles Macklin, Matthias Müller. "Position Based Fluids." ACM Transactions on Graphics (SIGGRAPH)
32(4), 2013**, with **Macklin, Müller, Chentanez, Kim. "Unified Particle Physics for Real-Time
Applications." SIGGRAPH 2014**, and **NVIDIA FleX 1.2.0.** [paper] [code] [still-current]
<https://mmacklin.com/pbf_sig_preprint.pdf>; <https://mmacklin.com/uppfrta_preprint.pdf>;
<https://github.com/NVIDIAGameWorks/FleX>

PBF enforces constant density as a position constraint with an artificial pressure term, vorticity
confinement and XSPH viscosity, so large steps are stable; the unified paper puts liquids, cloth, rigid
and gas particles in one PBD solver, which shipped as FleX — CUDA and DirectX 11/12, a 15-commit
repository that has not moved since 1.2.0.
*Bearing:* the near tier's algorithm of choice on the client (stable, parallel, GPU-native, simple
to write in Slang); FleX itself is a reference, not a dependency.

**Yongning Zhu, Robert Bridson. "Animating Sand as a Fluid." ACM SIGGRAPH 2005**, and **Robert Bridson.
*Fluid Simulation for Computer Graphics*, 2nd ed., CRC Press, 2015.** [paper] [book] [foundational]
<https://www.cs.ubc.ca/~rbridson/docs/zhu-siggraph05-sandfluid.pdf>; <https://www.cs.ubc.ca/~rbridson/>

FLIP for graphics: particles carry velocity, a grid solves pressure, and the particle velocities are
updated with the grid *change* (FLIP) blended with a little PIC for damping; sand is the same with a
yield condition. The book is the textbook for the grid side.
*Bearing:* FLIP/PIC is what the offline and high-end real-time liquids use; on a 5070 Ti a few
hundred thousand particles are fine for a burst dam or a waterfall pool, visual only.

**Chenfanfu Jiang, Craig Schroeder, Andrew Selle, Joseph Teran, Alexey Stomakhin. "The Affine
Particle-In-Cell Method." ACM Transactions on Graphics (SIGGRAPH), 2015.** [paper] [still-current]
<https://doi.org/10.1145/2766996> (record confirmed through the Semantic Scholar API)

Each particle carries an affine velocity field, which transfers angular momentum to and from the grid
losslessly; APIC has PIC's stability without its dissipation and FLIP's detail without its noise.
*Bearing:* the transfer scheme to use if Forge writes its own FLIP; it is a small change and removes
the PIC/FLIP blend knob.

**Alexey Stomakhin, Craig Schroeder, Lawrence Chai, Joseph Teran, Andrew Selle. "A Material Point
Method for Snow Simulation." SIGGRAPH 2013**, and **Yuanming Hu, Yu Fang, Ziheng Ge, Ziyin Qu, Yixin
Zhu, Andre Pradhana, Chenfanfu Jiang. "A Moving Least Squares Material Point Method with Displacement
Discontinuity and Two-Way Rigid Body Coupling." SIGGRAPH 2018.** [paper] [code] [still-current]
<https://disneyanimation.com/publications/a-material-point-method-for-snow-simulation/>;
<https://github.com/yuanming-hu/taichi_mpm>

MPM: particles carry deformation, a background grid does the momentum update, and an elasto-plastic
constitutive model gives snow, mud, sand and foam from one solver with fracture and self-collision for
free. MLS-MPM makes the transfer cheaper than APIC and adds two-way coupling with rigid bodies; the
reference implementation is MIT ("Feel free to use it commercially") and famously fits in 88 lines.
*Bearing:* the solver for mud, wet sand, snow and lava in a sandbox — the materials a heightfield
cannot do — as a client-side GPU effect in a bounded box around the player.

**Zibra AI. "Zibra Liquid" (Unity) and ZibraVDB (Unreal).** [web] [recent]
<https://zibra.ai/>

The commercial state of the art for shipped games: a GPU liquid plug-in for Unity, and for Unreal a
compressed-VDB streaming path — baked, not simulated.
*Bearing:* even the specialised vendor ships baked volumetrics for Unreal; live GPU liquids remain a
bounded local effect.

**Dark Energy Digital. *Hydrophobia* (2010) and its "HydroEngine"; Ubisoft Montpellier / Éric Chahi.
*From Dust* (2011).** [web] [foundational]
<https://en.wikipedia.org/wiki/Hydrophobia_(video_game)>; <https://en.wikipedia.org/wiki/From_Dust>

Two shipped games whose whole design was flowing water. Hydrophobia's engine ran "realistic fluid
dynamics technology for flowing water" through a flooding ship, dynamic every time; critics praised
the water and the studio folded on the rest. From Dust simulated water, lava, sand and vegetation on a
heightfield with layered rules — rivers emerge from flow and erosion, sediment thickens lakes — and
Chahi called simulation "the most challenging part of the game" for its cost (GDC Europe 2010 lecture
"Creating a High-Performance Simulation: A Dynamic Natural World to Play With").
*Bearing:* From Dust is the closest ancestor of "dig a channel and the lake drains" on a terrain of
Forge's kind, and it did it with a heightfield, in 2011, on consoles.

### Cellular water in games

**Keen Games. *Enshrouded*, "Wake of the Water" update, 10 November 2025.** [web] [recent]
<https://www.terminals.io/games/enshrouded/news/4887>;
<https://www.gtxgaming.co.uk/building-new-worlds-exploring-enshroudeds-voxel-based-system/>;
<https://en.wikipedia.org/wiki/Enshrouded>

Enshrouded (early access January 2024, 1.0 on 15 October 2026, 16-player co-op) runs on Keen's
proprietary voxel engine — the press calls it "Holistic"; Keen's own material says only "a proprietary
engine and voxel technology". Water was "the most requested feature since the Early Access launch"
and took almost two years "due to the technical complexity of designing realistic, dynamic water using
a custom-made voxel system"; the shipped water is "dynamic and voxel-based": swim in it, collect it,
direct it, water plants with it, terraform around it, explore underwater caves. No technical talk by
Keen on the water could be found (§9).
*Bearing:* the benchmark the brief names is a voxel *column/cell* water, not a particle or grid fluid.
Beating it means the same class of solver with better flow (Chentanez-style momentum instead of pure
cell rules) plus the visual tiers above it, not a different class.

**Bay 12 Games. "Water." Dwarf Fortress Wiki; Mojang. "Water." Minecraft Wiki.** [web] [foundational]
<https://dwarffortresswiki.org/index.php/Water>; <https://minecraft.wiki/w/Water>

The two canonical cell automata. Dwarf Fortress: "7 depth levels per tile", fluid moves "down, and to
the sides", never diagonally, evaporates at 1/7, and pressure is emulated by *teleporting* incoming
water to the nearest legally reachable non-full tile so it appears to rise. Minecraft: source blocks
and flowing levels 1–8, spread "1 block every 5 game ticks", "7 blocks horizontally from a source", down
without limit, and water "spends most of its time as stationary" because it only updates on block
updates.
*Bearing:* both are volume-*less* (Minecraft) or volume-conserving with a pressure hack (DF). A Forge
column model should keep DF's conservation, replace the teleport with a hydrostatic pressure
relaxation between connected cells, and keep Minecraft's laziness (only dirty cells tick).

### What is feasible at 60 fps in a big world, and how it tiers

Not an entry; the synthesis of the section.

- **Far (beyond ~200 m): analytic.** FFT or Gerstner spectra (Tessendorf, Finch) plus flow maps
  (Vlachos) for rivers, baked shore waves per chunk (Malan). Cost is rendering only. The *same* spectrum
  is evaluated on the CPU, in `f64` with `dmath`, wherever a body floats, so the server and every
  client agree on the height and normal without a byte on the wire.
- **Mid (the player's neighbourhood, lakes, rivers, canals): a shallow-water heightfield.** The
  gameplay-authoritative part is a coarse column model (DF/Minecraft class, volume-conserving, lazy,
  deterministic, integer volumes) on the server, edited by commands like terrain; the GPU runs
  Chentanez & Müller's momentum-carrying version at finer resolution *seeded from it* for the look,
  with foam and wetness written into maps the renderer reads.
- **Near (a few metres): particles.** PBF or FLIP/APIC on the GPU, spawned where the heightfield fails
  (waterfalls, splashes, a wave hitting a hull) exactly as the 2010 paper does, dying back into the
  heightfield; MPM for mud, snow and lava in a box around the player. Never gameplay truth.
- **Coupling needs to note now.** Rigid bodies read height and normal from the mid or far field
  (Kerner's triangles); the field reads back displaced volume and velocity from bodies (Yuksel's wave
  particles, or the 2010 two-way term); foam and wet maps are outputs of the mid solver that the
  renderer consumes; swimming is a controller mode driven by the same sampled height.

---

## 6. Large worlds and networking

**Jared Cone (Psyonix). "It IS Rocket Science! The Physics of Rocket League Detailed." GDC 2018.**
[talk] [still-current]
<https://www.gdcvault.com/play/1024972/It-IS-Rocket-Science-The> (slides:
<https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf>; video:
<https://www.youtube.com/watch?v=ueEmiDM94IE>)

Rocket League runs Bullet inside Unreal Engine 3 at a fixed tick, server-authoritative, with the
client predicting its own car and the ball and re-simulating from the server's state on a
misprediction; the whole design rests on the physics step being deterministic for identical inputs on
identical state. The 182-page slide deck is the detailed record.
*Bearing:* the proof that an off-the-shelf engine can be made deterministic enough for prediction if
it is driven with fixed steps and replayed forward from server state; Forge's `Prediction` (restart
from snapshot, replay unacknowledged inputs) is the same scheme.

**Timothy Ford (Blizzard). "Overwatch Gameplay Architecture and Netcode." GDC 2017.** [talk]
[still-current]
<https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and>;
<https://www.youtube.com/watch?v=8QHHVpBiG-I>

A strict ECS under a server-authoritative simulation that "leverages determinism to achieve
responsiveness and precision": the client predicts movement and, by default, abilities and
projectiles, and the server corrects.
*Bearing:* the architectural argument that prediction is a property of the *whole* simulation step —
the character controller, the ability system and the physics query must all be deterministic and
re-runnable, not just the rigid-body solver.

**Large worlds, in one paragraph.** The two production answers are in §1: Jolt's doubles for
positions only (5–10 %), and Chaos's float grid cells under a double engine world. Forge can take
either; local `f32` islands around an `f64` origin, as the previous project did for constructs, keeps
the engine's fast path, matches Chaos, and makes each island's origin part of the replicated,
deterministic state. Unreal's Resimulation mode (half an RTT ahead, cached history, rewind on
mismatch) is what the vehicle and boat prediction has to do on top.

**Determinism tests to run before trusting an engine** (synthesis, from the Box2D determinism post and
Jolt's documented exceptions): (1) same binary, same inputs, twice — hash final transforms and sleep
tick; (2) the same at 1, 4, 8 and 16 worker threads; (3) Windows client build against Linux server
build (Jolt: `CROSS_PLATFORM_DETERMINISTIC`, precise FP, contraction off; Box3D: default); (4) add
1,000 sleeping bodies 10 km away and re-hash (island independence, PhysX's enhanced-determinism
notion); (5) insert the same bodies in a different order and expect a *different* hash, then fix the
insertion order in Forge's own code; (6) record → save → load → replay (the Box2D "Replay" model,
<https://box2d.org/posts/2026/06/replay/>, which "work[s] cross platform and regardless of worker
count"); (7) a Falling-Hinges-style scene with joints, motors, CCD and sleeping in CI, as Box2D does.

---

## 7. Rust ecosystem

**Second Half Games. "jolt-rust" (`joltc-sys`, `rolt`) and "JoltC."** [code] [recent]
<https://github.com/SecondHalfGames/jolt-rust>; <https://crates.io/crates/joltc-sys>;
<https://github.com/SecondHalfGames/JoltC>

Bindgen over JoltC, a hand-written C wrapper of Jolt. The README says "This project is an early work
in progress. Watch for exposed nails", and `rolt`'s safety "is currently provided on a best-effort
basis"; both crates support double precision and 16- or 32-bit object layers; MIT/Apache. The
repository tracks Jolt 5.3.0, but the published `joltc-sys` is 0.3.1+Jolt-5.0.0 (last updated 19 May
2024, 9,166 downloads) — three minor versions behind Jolt 5.6.0.
*Bearing:* usable as a starting point and a map of the C surface, not as a dependency: Forge would
own its JoltC fork and regenerate bindings per Jolt release. Budget it as a week to stand up and a day
per Jolt release to maintain, all `unsafe` behind one crate as the rules require.

**Embark Studios. "physx-rs."** [code] [still-current]
<https://github.com/EmbarkStudios/physx-rs>

Archived (read-only); the last push was 12 February 2024 and the code targets PhysX 4.1. The README
was already candid: the safe wrapper "only covers a part of PhysX functionality" and PhysX "is a large
C++ codebase which requires a C++ toolchain, and comes with a non-trivial build system".
*Bearing:* binding PhysX 5 would mean regenerating and maintaining a huge surface alone; the cost
Embark stopped paying is the cost Forge would start.

**Dimforge. "Parry" (and "ncollide", passively maintained).** [code] [still-current]
<https://github.com/dimforge/parry>; <https://github.com/dimforge/ncollide>

Parry (`parry3d`, `parry3d-f64`) is the collision-detection half of Rapier, born from a merge of
Rapier's geometry with ports from ncollide, with better contact-manifold generation and a more robust
convex hull; ncollide is superseded and in passive maintenance.
*Bearing:* the queries a pure-Rust character controller, projectile casts and voxel/heightfield
contacts need, deterministic under `f64` even if the engine underneath is a C++ one — a plausible
split: Parry for Forge's own controller and queries, the bound engine for dynamics.

**Box3D bindings (`box3d-sys` / `box3d`, `boxddd`, `korch-box3d-sys`, `box3d-rs`).** [code] [recent]
<https://crates.io/crates/box3d-sys>; <https://github.com/Tebarem/box3d-rs>;
<https://github.com/wolfd/boxddd>; <https://docs.rs/box3d>; <https://docs.rs/korch-box3d-sys>;
<https://github.com/cark-dev/box3d-rs>

Four independent binding efforts appeared within weeks of the announcement: `box3d-sys` was created on
1 July 2026, the day after, and reached 0.1.16 by 4 August (831 downloads), generating bindings from
`box3d.h` with bindgen and building the vendored C with CMake; `boxddd` compiles the vendored C sources
through the `cc` crate; the others mirror the C API one-to-one. None has a user of record.
*Bearing:* the encouraging fact is how *cheap* the binding is — a C17 header and bindgen — which is
Box3D's structural advantage over Jolt for Rust; the discouraging fact is that none of the four is
older than the engine's alpha.

**Wrapping cost, in short.** A C API (Box3D, JoltC, NvBlast) is a bindgen run plus a safe layer for
handles, lifetimes and callback order; a C++ API (Jolt native, PhysX) needs a hand-kept C shim or
`cxx`, and upkeep per upstream release. The expensive part is always the callbacks (contact and
character listeners, broadphase layers), which must keep the ordering guarantees the engine documents.

---

## 8. Recommendation for Forge

*Opinion, shaped by the brief and by what was verified above.*

**Bind Jolt first.** It is the only engine that meets all five constraints today with evidence: two
shipped open-world AAA titles and Godot behind it; a written determinism contract (same binary by
default, `CROSS_PLATFORM_DETERMINISTIC` for the Windows-client/Linux-server case at ~8 %); a
double-precision build *and* a design that drops to floats inside, so per-island `f32` frames are
natural; a job system built for streaming worlds and a multithreaded game update (the GDC 2022
problems are Forge's problems); and `CharacterVirtual` with an arbitrary up axis and moving platforms,
three vehicle controllers with pluggable wheel casts, motorised ragdolls and soft bodies in the same
library. The cost is the binding: fork JoltC, keep it current per release, wrap it in one `unsafe`
crate. The three exceptions the docs list (broad-phase query determinism, result order, callback order)
become Forge's first three tests; Forge sorts every query result by entity id before it touches
gameplay.

**Track Box3D as the second option, and re-evaluate at v1.0 or at twelve months, whichever first.**
Its determinism story is better than Jolt's — default-on, worker-count independent, cross-platform
without a build flag — and its C17 API makes the Rust binding almost free, which is exactly the two
axes Jolt is weakest on. Against that: it is three months old, alpha by its author's word, has no
ragdoll, vehicle or soft-body layer, a character mover still being enhanced, a joint set without a
spherical or 6-DoF joint, a "doubles" claim that appears in the announcement but not the README, and
no pull requests. Keep the physics behind a trait as before, with the trait shaped by what *both*
engines expose (bodies, shapes, casts, character mover, joints of Box3D's set); write the ragdoll and
vehicle layers Forge-side where they are cheap to port, and re-run the same determinism and scaling
suite against Box3D every quarter. If Box3D reaches a stable 1.0 with the doubles path confirmed and
Glenn Fiedler's multiplayer game ships on it, it becomes the better authoritative-server engine.

**Do not bind PhysX 5** (platform-only determinism, fixed GPU buffers, archived bindings), **do not
use Avian** (Bevy coupling), and keep **Rapier + Parry** as the pure-Rust reserve and, plausibly, as the
query library for Forge's own controller and projectile casts.

**Fluids: three tiers, one truth.** The authoritative water is a coarse column model on the server —
volume-conserving integer cells, lazy updates, DF-style connectivity with a proper pressure
relaxation, edited by the same command stream as terrain — because that is the class Enshrouded ships
and it is the only class that is deterministic and networkable. The mid tier is a GPU shallow-water
heightfield (Chentanez & Müller 2010) seeded from it; the far tier is the analytic spectrum evaluated
identically on CPU and GPU; the near tier is GPU PBF/APIC particles and, later, MPM for mud and snow,
all visual. Boats are Kerner's submerged triangles on the analytic field, on the server tick; wakes are
wave particles on the client.

**The demo that proves each** (one scene per claim, each with a golden capture, a benchmark and a hash):

1. *Ragdolls on a ship.* 200 motorised ragdolls and 2,000 crates on a moving, turning construct in a
   local frame 30 km from the origin; proves the engine, the mass ratios, the island frame, and the
   character-on-platform case. Hash across 1/4/8/16 threads and across Windows/Linux.
2. *The quarry.* A voxel/SDF cliff blasted into 10,000 convex debris with a support graph deciding what
   falls; proves multithreaded scaling on 8 server threads, speculative CCD, and destruction-as-
   commands over the loopback link conditioner (100 ms, 2 % loss).
3. *The storm.* A boat in a wind-25 m/s FFT sea with a swimmer overboard; proves the CPU/GPU field
   agreement (server height vs rendered height at the hull, to the millimetre), buoyancy stability
   and the swim controller.
4. *The canal.* Dig a channel from a lake to the sea and watch it drain, on the server, with two
   clients predicting; proves the column model, its determinism digest, and the heightfield/particle
   tiers reading from it.
5. *Ride and fall.* A wheeled vehicle over kerbs and rubble, then a crash into a ragdoll; proves the
   vehicle controller with a cylinder cast, ragdoll hand-off, and prediction of a fast body.

## 9. Checked and left out

- **A third-party 2024–2026 physics-engine benchmark on GitHub** (Jolt vs Rapier vs Avian vs Box3D) —
  none could be located and opened before the search budget ran out; the only comparisons above are
  first-party (Rouwé's scaling PDF against PhysX 4.1 and Bullet 3.21; Dimforge's 2020 launch post;
  Catto's own Box2D/Box3D benchmark pages, which the publications index mentions but which returned
  404 at `box2d.org/benchmarks/`). Treat every performance ranking here as vendor-run.
- **Valheim's structural-integrity system** (blue→red support colours, material reach) — the Fandom
  and wiki.gg pages both refused automated fetching (402/401), as did two press guides. The concept
  is described in the Blast and Chaos entries instead; verify Valheim's exact rules on the wiki by
  hand before citing them.
- **A DICE/Frostbite talk on Bad Company 2 "Destruction 2.0"** — press coverage confirms the feature,
  but no GDC or Frostbite publication page could be found or opened; left out rather than cited from
  memory.
- **Volition's GDC 2009 Red Faction: Guerrilla destruction lectures** — the GDC Vault entry located is
  the *multiplayer level design* talk; the destruction lectures are referenced only second-hand. The
  Geo-Mod 2.0 wiki page stands in.
- **An Assassin's Creed IV: Black Flag ocean-technology talk** — a gamedev.net thread discusses one and
  Ubisoft Singapore's "fully physics simulated" ocean is quoted in press, but no talk page or slides
  were reachable. Sea of Thieves and Horizon cover the same ground with verified sources.
- **A Keen Games technical talk on Enshrouded's engine or water** — nothing found on GDC Vault or
  Keen's site; the engine name "Holistic" appears only in press. The water entry is built from the
  update announcement.
- **Unreal's Character Movement Component documentation** — every dev.epicgames.com page for it
  returned an empty table of contents to the fetcher (three URLs tried), so Unity's manual is the
  cited controller reference and Unreal appears only through its networked-physics and LWC pages.
- **`bullet-rs` or any maintained Bullet binding for Rust** — not verified; not listed in §7.
- **An "active ragdoll" paper or GDC talk of standing** (Euphoria / NaturalMotion) — not searched
  before the budget ran out; kept as prior knowledge in §3.
- **Jolt's thread-count independence** — the Architecture document lists API-call order and identical
  binary as the conditions and does not mention worker count either way; the performance test's hash
  across `-t` is the way to find out, and the Recommendation assumes nothing.

## 10. Verification notes

- Every URL above was opened with WebFetch during this session (23–24 September 2026) except where
  an entry says otherwise; no browser was used. The web-search budget was exhausted after the engine
  and solver sections, so the fluid and networking sections were verified by direct fetch of known
  publisher, author and archive pages.
- YouTube links were not opened; each talk with a YouTube link (Noita, Rocket League, Overwatch) was
  confirmed through its GDC Vault page, which was fetched, and the YouTube URL is carried as it
  appeared in the search-result listing.
- Version and date facts come from machine-readable sources where possible: the GitHub API for Box3D
  (created 2026-05-10, tag `v0.1.0`, pushed 2026-09-20), Jolt releases (5.0.0 2024-04-05 … 5.6.0
  2026-07-11), physx-rs (`archived: true`, pushed 2024-02-12) and Bullet (3.25, 2022-04-24); the
  crates.io API for `rapier3d` 0.35.3, `avian3d` 0.7.0, `joltc-sys` 0.3.1+Jolt-5.0.0 and `box3d-sys`
  0.1.16. PhysX "5.11.0" is the README's statement; its tags are Omniverse-numbered.
- PDFs that the fetcher could not read as text (Catto's 2006 and 2011 slides, the Rocket League deck,
  the Jolt scaling charts, PBF, FLIP) were confirmed to exist at their URLs and, where the entry
  states contents, those contents come from the author's index page (box2d.org/publications,
  mmacklin.com/publications, Bridson's page, the Jolt PerformanceTest document) or the venue page.
- The APIC record was confirmed through the Semantic Scholar API (title, authors, year, venue) because
  both the UCLA and UPenn PDF mirrors failed; the ACM DL refused every fetch, as it did for the
  previous file.
- Claims marked † in the comparison table are prior knowledge and are not load-bearing for the
  recommendation.
- Enshrouded's engine name and Keen's water internals are press-sourced; Box3D's "doubles" support is
  announcement-sourced and absent from the README — both are flagged in their entries.
