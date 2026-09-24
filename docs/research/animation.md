# Research — Character and creature animation with physical interaction

> Companion to the in-house survey `world/docs/research/motion-physics.md` §4 (motion layers, gaits from
> a body plan, the IK solver table, active ragdolls, learned controllers). That survey was written from
> memory and says "verify before relying"; this file verifies its references, adds the production and
> research literature it lacked, and goes deeper on the requirement it only sketched: characters that
> touch the world and get touched back, without turning into ragdolls.
> Status: v0.1, 2026-09-24. Every source was fetched on 2026-09-23/24 (see [Verification notes](#verification-notes)).

The owner's requirement is that "animations for entities (creatures, players, various entities) are
important, and with physics and good interactions with the game world (not just ragdolls that don't feel
real)". Three facts shape the answer. Forge's creatures have generated bodies with variable limb counts,
so no clip library can be complete and the pose must be *computed* from the body plan at least part of
the time. Forge has an authoritative server with client prediction, so the pose is presentation and only
its inputs cross the wire. And a rigid-body engine (Jolt or similar) is coming anyway, and Jolt exposes
exactly the primitive that "physical but not a ragdoll" needs: a ragdoll whose constraint motors track an
animated pose. The literature sorts itself into a stack — clips or motion matching for what was
captured, IK and procedural motion for contact, powered ragdolls for force — and the recommendation at
the end is that stack, built in that order, proven by one demo.

**How to read the labels.** **[paper]** peer-reviewed, **[book]**, **[talk]** conference presentation,
**[web]** engineering write-up, **[code]** a repository, **[docs]** vendor documentation; by age,
**foundational** (old, still correct), **still-current** (the standard reference today), **recent**
(2019 or later). DOIs are recorded for everything published by ACM, Elsevier, Wiley or Taylor & Francis,
whose pages block automated fetching; the DOI is the durable handle.

> **State of the art in five sentences.** Production humanoid locomotion in 2026 is motion matching over
> a few hours of capture, with inertialization for transitions and a simulation object that the animated
> character is pulled towards — shipped by Ubisoft, Naughty Dog and EA, and since Unreal 5.4 by every
> Unreal licensee. The learned variants (learned motion matching, phase manifolds, codebook matching,
> control operators) shrink that data and widen control, but every one still needs the capture, and the
> research code that exists is licensed for research. Contact with the world is solved kinematically
> first — foot locking, two-bone and FABRIK IK, root motion warped onto targets — and that covers most of
> what a player sees. A "physical" character that is not a ragdoll is a powered ragdoll: constraint motors
> driving a physics body towards the kinematic pose (one call in Jolt), with the tracking policy either a
> hand-tuned strength schedule or, in the line from DReCon through SuperTrack to MaskedMimic, a small
> network trained offline in a GPU simulator. For bodies nobody authored the shipped answer is still
> Spore's — a morphology-independent gait plus IK — and the physics literature (Coros, Geijtenbeek)
> shows the same body plan can drive a simulated controller, which is what makes "procedural pose plus
> powered ragdoll" the right stack for generated creatures.

**Contents**

1. [The runtime animation system](#1-the-runtime-animation-system)
2. [Motion matching and learned motion](#2-motion-matching-and-learned-motion)
3. [Inverse kinematics and procedural animation](#3-inverse-kinematics-and-procedural-animation)
4. [Physically simulated characters](#4-physically-simulated-characters)
5. [Interacting with the world](#5-interacting-with-the-world)
6. [Cloth, hair and faces](#6-cloth-hair-and-faces)
7. [Animation over the network](#7-animation-over-the-network)
8. [Rust ecosystem, 2025–2026](#8-rust-ecosystem-20252026)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. The runtime animation system

Clips sampled into local joint transforms, a graph that blends them (layered, masked, additive),
transitions, root motion, compression, skinning, a pipeline from DCC formats. None of it is research
any more; these references settle *how*.

### Clips, blending, transitions

**David Bollo. "Inertialization: High-Performance Animation Transitions in 'Gears of War'." GDC, 2018.**
[talk] [still-current]
<https://www.gdcvault.com/play/1025165/Inertialization-High-Performance-Animation-Transitions>

Replaces cross-fades with a post-process: at a transition the pose offset and its velocity are recorded
and decayed to zero over the blend time, so only the *target* state is evaluated. Works on vectors and
quaternions alike, costs a fraction of a blend, shipped in Gears of War 4. Holden's 2026 foot-locking
article (§3) uses the same operator to lock and release feet.
*Bearing:* the one transition primitive Forge needs; it makes motion matching's constant clip jumps
cheap and lets IK targets snap without popping. Build it before any blend tree.

**Daniel Holden. "Code vs Data Driven Displacement." theorangeduck.com, 23 Sep 2021.** [web] [recent]
<https://theorangeduck.com/page/code-vs-data-driven-displacement>

Root motion as an engineering trade-off: a *simulation object* moved by gameplay code and a *character
entity* moved by animation data, reconciled by direct synchronisation, damped adjustment,
velocity-clamped correction, or position and rotation clamping — each implemented in the open-source
motion-matching demo of §2. The choice is per situation, not a doctrine.
*Bearing:* exactly the in-house survey's "root motion from the simulation, pose on the client"; the
article names the correction policies Forge's prediction layer needs when the server's capsule and the
client's animated body disagree.

### Compression, skinning, data

**Nicholas Frechette. "Animation Compression Library (ACL)." GitHub, 2016–2023 (v2.1, Dec 2023).**
[code] [still-current]
<https://github.com/nfrechette/acl> (design posts and release notes: <https://nfrechette.github.io/>)

Header-only C++11, MIT, no dependencies; per-track variable-bit-rate quantisation chosen against an
object-space error bound (the 2023 "dominant rigid shell" estimate, which also made compression 2–3×
faster). Epic made it Unreal's *default* animation codec in 5.3 (Sep 2023), reporting over 30 % less
memory than the previous codecs, and maintains the plugin in-engine.
*Bearing:* no Rust port exists (the `acl` crate is an unrelated POSIX ACL tool), but a bindgen wrapper of
a header-only library is a day's work when clip memory matters. Keep Forge's clip format ACL-compatible
and defer the wrapper until memory is measured.

**Ladislav Kavan, Steven Collins, Jiří Žára, Carol O'Sullivan. "Skinning with Dual Quaternions."
*Proc. Symposium on Interactive 3D Graphics and Games (I3D)*, 2007.** [paper] [foundational]
<https://users.cs.utah.edu/~ladislav/kavan07skinning/kavan07skinning.html> — DOI
10.1145/1230100.1230107

Blends rigid transforms as dual quaternions instead of matrices, removing linear blend skinning's
candy-wrapper and volume-loss artefacts at twisting joints with a shader as simple as LBS. Non-rigid
joint transforms need a separate path.
*Bearing:* Forge skins in a compute pass (the skinned vertices feed both the mesh-shader rasteriser and
the ray-tracing BLAS refit, so they must exist in memory); LBS by default, DQS per material where twist
matters — wrists, tails, tentacles. Both are a few lines of Slang.

**Guillaume Blanc. "ozz-animation." GitHub, 2011–present.** [code] [still-current]
<https://github.com/guillaumeblanc/ozz-animation> (glTF animation and skin semantics:
<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#animations>)

The reference architecture for a small, fast runtime: an offline toolchain (glTF, FBX, Collada, …) that
emits compact skeleton and clip assets, and a runtime of stateless *jobs* — sampling, partial and
additive blending, two-bone and aim IK, local-to-model — over structure-of-arrays SIMD transforms, with
no engine dependency; MIT, C++17. glTF 2.0 is the interchange to standardise on: channels and samplers
(LINEAR, STEP, CUBICSPLINE), targets on translation, rotation, scale and morph weights, skins as joints
plus inverse bind matrices.
*Bearing:* copy the shape — SoA transforms, jobs, no hidden state — and import only glTF (the `gltf`
crate, 1.4.1, handles skins and animations). The Rust port in §8 is the direct route.

### Retargeting

**Michael Gleicher. "Retargetting Motion to New Characters." *Proc. SIGGRAPH*, 1998.** [paper]
[foundational]
<https://research.cs.wisc.edu/graphics/Gallery/Retarget/> — DOI 10.1145/280814.280820

Defined the problem: adapt captured motion to a differently proportioned character while preserving what
matters (feet planted, hands meeting), as spacetime constraints solved over the whole clip. Everything
since is a faster or more local approximation.
*Bearing:* generated creatures share no proportions with anything; the constraints-first framing
(contacts are the invariant, joint angles are not) is what carries over, and why §3's IK sits *after*
the clip layer.

**Epic Games. "IK Rig Animation Retargeting." Unreal Engine documentation, 2022–2026.** [docs]
[recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/ik-rig-animation-retargeting-in-unreal-engine>

Gleicher's idea in production form: a rig is described as named *retarget chains* rather than bones,
chains are matched between source and target by fuzzy name, and an optional IK pass keeps hand and
foot contacts, across skeletons with different bone counts, names and orientations. Only biped-to-biped
is documented.
*Bearing:* chain-level, semantic retargeting is the right skeleton description for Forge — a generated
creature is a list of chains with roles ("leg 3 of 6", "tail") — and it is the same description §3's
gait generator consumes.

---

## 2. Motion matching and learned motion

The standard for humanoids with capture data. The learned line reduces memory and adds control but does
not remove the need for data, and its code is not licensed for shipping.

**Simon Clavet. "Motion Matching and The Road to Next-Gen Animation." GDC, 2016.** [talk]
[foundational]
<https://www.gdcvault.com/play/1023280/Motion-Matching-and-The-Road>

The For Honor talk: keep long, barely marked-up capture sessions and, every few frames, search them for
the pose-and-trajectory feature vector closest to what the character is doing and where the player wants
to go. Kristjan Zadziuk's companion talk (Ubisoft Toronto, GDC 2016, vault id 1023115) covers the
animator's side.
*Bearing:* the algorithm is small; the cost is the data. Forge has no capture, so a public or purchased
humanoid library is the prerequisite, and a blend space of a few clips is the honest baseline until then.

**Michal Mach, Maksym Zhuravlov. "Motion Matching in 'The Last of Us Part II'." GDC, 2021.** [talk]
[recent]
<https://www.gdcvault.com/play/1027118/Motion-Matching-in-The-Last>

Naughty Dog's account of adopting an experimental system for a flagship: the setbacks (control, memory,
animator workflow) and the fixes. JC Delannoy's "Motion Matching at EA: Five Years Later" (GDC 2023,
<https://www.gdcvault.com/play/1028945/Animation-Summit-Motion-Matching-at>) is the portfolio-scale
post-mortem and deliberately skips the algorithm for what changes for animators and designers.
*Bearing:* both say the algorithm is a week and the tooling and data discipline are the project. Read
before promising motion matching with two people.

**Daniel Holden, Oussama Kanoun, Maksym Perepichka, Tiberiu Popa. "Learned Motion Matching." *ACM
Transactions on Graphics* 39(4), SIGGRAPH, 2020.** [paper] [recent]
<https://theorangeduck.com/page/learned-motion-matching> — DOI 10.1145/3386569.3392440 (demo code:
<https://github.com/orangeduck/Motion-Matching>, MIT)

Replaces the database and search with three small networks — decompressor (feature → pose), stepper
(feature → next feature), projector (query → nearest valid feature) — trained to emulate the original;
memory stays small as data grows and the output is nearly indistinguishable. The MIT repository is a
complete C++ motion matcher with inertialization, the simulation-object model and the trained networks.
Holden's latest, "Control Operators for Interactive Character Animation" (Gou, van de Panne, Holden,
SIGGRAPH Asia 2025, best paper;
<https://theorangeduck.com/page/control-operators-interactive-character-animation>), decomposes control
into designer-meaningful operators (joystick, path, object), each with a network structure.
*Bearing:* the best readable reference implementation of production motion matching, licensed to copy.
The learned part is a memory optimisation Forge will not need until capture outgrows RAM.

**Daniel Holden, Taku Komura, Jun Saito. "Phase-Functioned Neural Networks for Character Control."
*ACM Transactions on Graphics* 36(4), SIGGRAPH, 2017.** [paper] [still-current]
<https://theorangeduck.com/page/phase-functioned-neural-networks-character-control> — DOI
10.1145/3072959.3073663

Network weights are a cyclic function of gait phase; trained on capture fitted to virtual terrain, a
controller of a few megabytes produces locomotion over rough ground and obstacles from a stick input at
game frame rates.
*Bearing:* the terrain conditioning is what matters for an open world; the phase dependence is what
makes it biped-only, which the next two entries fix.

**He Zhang, Sebastian Starke, Taku Komura, Jun Saito. "Mode-Adaptive Neural Networks for Quadruped
Motion Control." *ACM Transactions on Graphics* 37(4), SIGGRAPH, 2018.** [paper] [still-current]
<https://github.com/sebastianstarke/AI4Animation> — DOI 10.1145/3197517.3201366

A gating network blends expert weights per frame from the character's own state, handling a dog's
unaligned, aperiodic footfalls (walk, trot, canter, jumps, sits, sharp turns) from capture alone. The
"Neural State Machine" (Starke, Zhang, Komura, Saito, SIGGRAPH Asia 2019, DOI 10.1145/3355089.3356505)
extends it to goal-driven scene interactions. The repository is research-and-education only; the capture
is CC BY-NC.
*Bearing:* proof that learned controllers are not biped-bound; proof that Forge cannot ship this code. A
design reference for a creature controller's inputs and outputs, not a dependency.

**Sebastian Starke, Ian Mason, Taku Komura. "DeepPhase: Periodic Autoencoders for Learning Motion Phase
Manifolds." *ACM Transactions on Graphics* 41(4), SIGGRAPH, 2022.** [paper] [recent]
<https://github.com/sebastianstarke/AI4Animation> — DOI 10.1145/3528223.3530178

Learns a multi-dimensional periodic phase from any motion (biped, quadruped, dance) with a periodic
autoencoder, giving a manifold on which frames from different clips align, used for matching, blending
and control. "Categorical Codebook Matching for Embodied Character Controllers" (Starke, Starke, He,
Komura, Ye, SIGGRAPH 2024) is in the same repository under the same licence.
*Bearing:* the principled answer to aligning a walk and a run — or two generated clips — for blending;
trainable from Forge's own procedural motion, which sidesteps the data problem.

**Epic Games. "Motion Matching in Unreal Engine." Unreal Engine documentation (Pose Search plugin),
2023–2026; and "Unreal Engine 5.4 is now available," Epic blog, Apr 2024.** [docs] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine>,
<https://www.unrealengine.com/en-US/blog/unreal-engine-5-4-is-now-available>

The production state as of 5.4: the Pose Search plugin's node, a *Schema* (channels and weights), a
*Database* per situation, a Trajectory channel fed by a Character Trajectory component, and a Chooser
to pick the database at runtime; "battle-tested in Fortnite Battle Royale and shipped on all platforms
from mobile to console, running on all 100 characters plus NPCs". Unity's Kinematica
(<https://docs.unity3d.com/Packages/com.unity.kinematica@0.8/manual/index.html>) never left
0.8.0-preview.
*Bearing:* Epic's decomposition (schema, database, trajectory, chooser) is a good module boundary for
Forge's implementation; the Fortnite numbers settle that it is cheap enough for every humanoid on screen.

---

## 3. Inverse kinematics and procedural animation

The layer that makes feet meet Forge's terrain and hands meet its props, and the only layer that can
animate a body generated a second ago.

**Andreas Aristidou, Joan Lasenby. "FABRIK: A fast, iterative solver for the Inverse Kinematics
problem." *Graphical Models* 73(5), 2011.** [paper] [still-current]
DOI 10.1016/j.gmod.2011.05.003 (author page: <http://www.andreasaristidou.com/FABRIK.html>; survey:
Aristidou, Lasenby, Chrysanthou, Shamir, "Inverse Kinematics Techniques in Computer Graphics: A Survey",
*Computer Graphics Forum* 37(6), 2018, DOI 10.1111/cgf.13310)

Solves a chain by alternately reaching forward from the effector and backward from the root, moving
each joint along the line to its neighbour: no matrices, no singularities, a few iterations, joint
limits as a per-step projection. The 2018 survey covers the alternatives — analytic, Jacobian transpose
and pseudo-inverse, damped least squares and Buss & Kim's selectively damped variant (*Journal of
Graphics Tools*, 2005, DOI 10.1080/2151237X.2005.10129202), CCD, FABRIK, learned solvers — with costs
and failure modes.
*Bearing:* FABRIK for necks, spines, tails, tentacles; analytic two-bone for legs and arms; damped least
squares only when a whole body must reach. The in-house survey's table stands.

**Daniel Holden. "Simple Two Joint IK" (18 Jan 2017) and "Inverse Kinematics and Foot Locking"
(30 Jul 2026). theorangeduck.com.** [web] [recent]
<https://theorangeduck.com/page/simple-two-joint>,
<https://theorangeduck.com/page/inverse-kinematics-foot-locking>

The 2017 note derives two-joint IK in two steps (bend to match the distance, rotate the base to aim).
The 2026 article is the most complete practical treatment of foot sliding: a two-joint solve that places
the toe while preserving the pose, runtime locking and release through inertialization, contact
detection from velocity thresholds, and an offline solve that spreads corrections across a clip. Its
thesis: sliding is a velocity error, the toe matters more than the heel, and a little residual sliding
beats mangling the source.
*Bearing:* the specification for Forge's foot layer on uneven terrain — lock on contact, release on lift,
inertialize both — unchanged for a hexapod, one foot at a time.

**David Rosen. "Animation Bootcamp: An Indie Approach to Procedural Animation." GDC, 2014.** [talk]
[foundational]
<https://www.gdcvault.com/play/1020583/Animation-Bootcamp-An-Indie-Approach>

Overgrowth's method (Wolfire; also Receiver and Black Shades): a handful of key poses per action,
interpolated and modulated in code by speed, slope, momentum and physics, with IK and springs supplying
the rest — the existence proof that a tiny team gets convincing motion without capture.
*Bearing:* the design for Forge's fallback humanoid and the starting point for creatures; procedural
keyframes are assets generated by code, which is Forge's rule.

**Chris Hecker, Bernd Raabe, Ryan W. Enslow, John DeWeese, Jordan Maynard, Kees van Prooijen.
"Real-time Motion Retargeting to Highly Varied User-Created Morphologies." *ACM Transactions on
Graphics* 27(3), SIGGRAPH, 2008.** [paper] [foundational]
<http://chrishecker.com/Real-time_Motion_Retargeting_to_Highly_Varied_User-Created_Morphologies> — DOI
10.1145/1360612.1360626

Spore: animators author on a morphology-independent description ("the front-left foot", "the mouth",
relative to body frames) that preserves the animation's structure and style, and a robust IK solver
instantiates it at runtime on whatever creature the player built, with gait generation and foot planning
underneath.
*Bearing:* the only shipped, published system for Forge's exact problem; "author against roles, solve
against the body" is the architecture for creature animation, and its solver priorities (robustness
over accuracy) are the ones to copy.

**Joar Jakobsson, James Therrien. "Animation Bootcamp: 'Rainworld' Animation Process." GDC, 2016.**
[talk] [still-current]
<https://www.gdcvault.com/play/1023475/Animation-Bootcamp-Rainworld-Animation>

Rain World's creatures are segment chains driven procedurally — IK, springs, per-creature rules for
reaching, gripping and falling — with almost no keyframes; the talk is about designing the technique and
the animation principles (readability, performance, communication to the player) together. The
counter-example to "procedural looks robotic".
*Bearing:* readability comes from exaggeration and timing, not accuracy — the thing to hold onto when a
generated hexapod first walks.

**Stelian Coros, Andrej Karpathy, Ben Jones, Lionel Reveret, Michiel van de Panne. "Locomotion Skills
for Simulated Quadrupeds." *ACM Transactions on Graphics* 30(4), SIGGRAPH, 2011.** [paper]
[foundational]
<https://www.cs.ubc.ca/~van/papers/2011-TOG-quadruped/index.html> — DOI 10.1145/2010324.1964954

Physics-based quadruped control from *gait graphs* (per-leg contact phases), a dual leg-frame model, a
flexible spine and virtual forces through the Jacobian transpose: walk, trot, pace, canter, both gallops,
jumps, robust to pushes and terrain, matching real dogs. The gait graph is the in-house survey's "phase
offset per leg".
*Bearing:* the bridge from the procedural gait to the physical layer — a gait graph plus per-leg virtual
forces is a controller a generated body can be given without training.

**Thomas Geijtenbeek, Michiel van de Panne, A. Frank van der Stappen. "Flexible Muscle-Based
Locomotion for Bipedal Creatures." *ACM Transactions on Graphics* 32(6), SIGGRAPH Asia, 2013.** [paper]
[foundational]
<https://www.goatstream.com/research/papers/SA2013/> — DOI 10.1145/2508363.2508399

Optimises both muscle routing and control parameters of simulated muscles for a variety of bipedal
creatures, some plainly non-human, producing natural gaits at several speeds over uneven ground under
disturbances; the optimisation is offline and per morphology.
*Bearing:* the strongest evidence that generated bodies can be given physical locomotion controllers
automatically; the cost model (offline optimisation once per species) is the one to budget.

---

## 4. Physically simulated characters

"Not just ragdolls" means simulated *and* controlled. The engine primitive is a ragdoll with motors; the
controller varies from a strength schedule to a trained policy.

**Jorrit Rouwe. "Jolt Physics" — Ragdoll, constraint motors and soft bodies. Documentation and
repository, 2022–2026.** [docs] [recent]
<https://jrouwe.github.io/JoltPhysics/index.html>, <https://jrouwe.github.io/JoltPhysics/class_ragdoll.html>,
<https://github.com/jrouwe/JoltPhysics>

Jolt (MIT; Horizon Forbidden West, Death Stranding 2) ships the whole powered-ragdoll primitive:
`Ragdoll::SetPose`/`GetPose`, `DriveToPoseUsingKinematics` (velocities to reach a pose in a given time)
and `DriveToPoseUsingMotors` (constraint motors driven to a target pose *and* the velocity implied by
the previous pose), motors in Off/Velocity/Position modes with stiffness–damping or frequency–damping
springs and force/torque limits so a hit can overpower the tracking. The README lists hard keying, soft
keying and motor drive as the three ways to animate a ragdoll, and states the simulation is deterministic
so remote clients can be kept in sync from inputs. Jolt's ragdoll is bodies plus constraints; the
reduced-coordinate alternative is Featherstone's articulated-body algorithm (Roy Featherstone, *Rigid
Body Dynamics Algorithms*, Springer, 2008; <https://royfeatherstone.org/>), which Rapier's
`MultibodyJointSet` implements (§8).
*Bearing:* the physics layer's entire API surface: build the ragdoll from the same body plan as the
skeleton, feed it the kinematic pose every tick through `DriveToPoseUsingMotors`, lower motor limits per
body to let force through.

**NaturalMotion. "Euphoria." Middleware, 2006–2017 (Wikipedia article, verified 2026).** [web]
[foundational]
<https://en.wikipedia.org/wiki/Euphoria_(software)>

The commercial proof that simulated characters can carry a AAA game: a body-muscle-motor simulation
("Dynamic Motion Synthesis") producing grabs, stumbles and falls that differ every time, in Rockstar's
RAGE (GTA IV and V, Red Dead Redemption 1 and 2, Max Payne 3) and in The Force Unleashed and
Backbreaker. Zynga bought NaturalMotion and stopped licensing it in 2017; no technical paper was
published, so what is known is behavioural — authored balance, grab and protect behaviours over a
powered ragdoll.
*Bearing:* the bar the owner means by "don't feel real"; what is public says it is behaviours layered
on motors, which §4's research line automates.

**KangKang Yin, Kevin Loken, Michiel van de Panne. "SIMBICON: Simple Biped Locomotion Control." *ACM
Transactions on Graphics* 26(3), SIGGRAPH, 2007.** [paper] [foundational]
<https://www.cs.ubc.ca/~van/papers/Simbicon.htm> — DOI 10.1145/1276377.1276509

A finite-state machine of target poses per gait phase plus one balance feedback law (swing-hip angle
corrected by centre-of-mass position and velocity) gives a simulated biped that walks, runs and survives
pushes, with gait and style set by a few parameters.
*Bearing:* the hand-authored controller to have before any learned one — a few hundred lines, any tick
rate — and the balance-recovery fallback when tracking motors are not enough.

**Xue Bin Peng, Pieter Abbeel, Sergey Levine, Michiel van de Panne. "DeepMimic: Example-Guided Deep
Reinforcement Learning of Physics-Based Character Skills." *ACM Transactions on Graphics* 37(4),
SIGGRAPH, 2018.** [paper] [still-current]
<https://xbpeng.github.io/projects/DeepMimic/index.html> — DOI 10.1145/3197517.3201311

Reinforcement learning with a reward for matching a reference clip plus a task reward, with reference
state initialisation and early termination as the tricks that make it train; policies reproduce
locomotion and acrobatics, transfer to changed morphology and terrain, and resist perturbation.
*Bearing:* the origin of "track a kinematic reference with a policy" — what Forge's physics layer would
eventually run — and the reason the reference from §1–3 must be good before any training.

**Xue Bin Peng, Ze Ma, Pieter Abbeel, Sergey Levine, Angjoo Kanazawa. "AMP: Adversarial Motion Priors
for Stylized Physics-Based Character Control." *ACM Transactions on Graphics* 40(4), SIGGRAPH, 2021.**
[paper] [still-current]
<https://xbpeng.github.io/projects/AMP/index.html> — DOI 10.1145/3450626.3459670

A discriminator trained on an unstructured motion set replaces the per-clip imitation reward, so style
comes from data and the task from a separate reward. "ASE" (Peng, Guo, Halper, Levine, Fidler, SIGGRAPH
2022, DOI 10.1145/3528223.3530110, <https://xbpeng.github.io/projects/ASE/index.html>) generalises this
to a reusable latent skill space trained once and steered by small task policies.
*Bearing:* AMP-style training would give a Forge species a *style* (heavy, skittish) without authoring
it per clip; ASE is "train once per species, steer per behaviour". Both offline; neither on the shipping
path yet.

**Kevin Bergamin, Simon Clavet, Daniel Holden, James Richard Forbes. "DReCon: Data-Driven Responsive
Control of Physics-Based Characters." *ACM Transactions on Graphics* 38(6), SIGGRAPH Asia, 2019.**
[paper] [still-current]
<https://www.ubisoft.com/en-us/studio/laforge/news/VjEIwquaIyEZZSw5RZI0V/drecon-datadriven-responsive-control-of-physicsbased-characters>
— DOI 10.1145/3355089.3356536

La Forge's shippable design: a kinematic motion-matching controller produces the responsive reference
and a reinforcement-learned policy makes a simulated ragdoll track it, so pushes, collisions and uneven
ground produce physical reactions and recoveries within game performance limits. Clavet's "Ragdoll
Motion Matching" (GDC 2020, <https://www.gdcvault.com/play/1026712/Machine-Learning-Summit-Ragdoll-Motion>)
is the talk version.
*Bearing:* DReCon *is* the layered architecture recommended below — kinematic reference, tracking
controller over Jolt's motors — with the tracking learned. Build it with a PD schedule first; DReCon is
what the learned spike replaces that schedule with.

**Levi Fussell, Kevin Bergamin, Daniel Holden. "SuperTrack: Motion Tracking for Physically Simulated
Characters using Supervised Learning." *ACM Transactions on Graphics* 40(6), SIGGRAPH Asia, 2021.**
[paper] [recent]
<https://www.ubisoft.com/en-us/studio/laforge/news/7fMzaMaDgnd0gqPsCaJZYb/supertrack-motion-tracking-for-physically-simulated-characters-using-supervised-learning>
— DOI 10.1145/3478513.3480527

Trains a neural *world model* of the ragdoll's dynamics from simulation data, then trains the tracking
policy by back-propagating through it as a differentiable simulator — supervised learning, no reward
engineering. About forty hours of training tracks a wide range of motions where PPO baselines given the
same time reached far lower success.
*Bearing:* the most practical recipe for a small team: training data straight from Jolt, a small network
at runtime. The spike to run once the PD-tracked ragdoll exists.

**Jungdam Won, Deepak Gopinath, Jessica Hodgins. "Physics-based Character Controllers Using
Conditional VAEs." *ACM Transactions on Graphics* 41(4), SIGGRAPH, 2022.** [paper] [recent]
<https://research.facebook.com/publications/physics-based-character-controllers-using-conditional-vaes/>
— DOI 10.1145/3528223.3530067

A conditional VAE whose latent space is a controllable, physically valid motion prior: one model
produces long, diverse motion and solves downstream tasks without task-specific conditioning.
"ControlVAE" (Yao, Song, Chen, Liu, SIGGRAPH Asia 2022, DOI 10.1145/3550454.3555434,
<https://arxiv.org/abs/2210.06063>) reaches a similar generative controller model-based, with a learned
world model as in SuperTrack.
*Bearing:* where physics-based characters are heading; for Forge a possible later replacement for the
reference clip itself (sample the motion, track it), not a first-version concern.

**Chen Tessler, Yunrong Guo, Ofir Nabati, Gal Chechik, Xue Bin Peng. "MaskedMimic: Unified
Physics-Based Character Control Through Masked Motion Inpainting." *ACM Transactions on Graphics*
43(6), SIGGRAPH Asia, 2024.** [paper] [recent]
<https://research.nvidia.com/labs/par/maskedmimic/> — DOI 10.1145/3687951 (framework:
<https://github.com/NVlabs/ProtoMotions>, Apache-2.0)

Control as inpainting: one policy completes a *partially specified* motion — keyframes, a target joint,
text, scene geometry — on a simulated body, so one model does locomotion, reaching, sitting and object
interaction without per-task rewards. The code is in NVIDIA's ProtoMotions (now ProtoMotions3,
Apache-2.0; IsaacGym, IsaacLab, Newton, MuJoCo and Genesis backends, AMASS-scale imitation, a
retargeting optimiser), which also hosts PHC (Luo et al., ICCV 2023, <https://arxiv.org/abs/2305.06456>:
progressive mixture of experts with fall recovery), CALM (Tessler et al., SIGGRAPH 2023,
<https://research.nvidia.com/labs/par/calm/>) and PhysDiff (Yuan et al., ICCV 2023,
<https://nvlabs.github.io/PhysDiff/>: a physics projection inside motion diffusion that removes floating
and foot sliding).
*Bearing:* the tool to train Forge's tracking or generative policies with, on the server GPU, licensed
for it; every model in it is humanoid, so generated creatures need their own runs with Forge's body plans
exported to it.

---

## 5. Interacting with the world

Most world contact is kinematic and contextual: reach a ledge, plant a hand, step over, react to a hit,
share an animation with another body. The physics layer handles what these cannot predict.

**Epic Games. "Motion Warping." Unreal Engine documentation, 2021–2026.** [docs] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-warping-in-unreal-engine>

Warps a clip's root motion inside a marked window so the character arrives exactly at a named *warp
target* supplied by gameplay — ledge, wall top, enemy — whatever the start; windows are notify states on
montages; vaulting, mantling and melee are the canonical uses.
*Bearing:* small and high-value: a dozen authored or generated interaction clips then cover every ledge
height on procedural terrain. Implement as a root-motion filter over clip windows, independent of how the
clip was chosen.

**Henry Allen. "Animation Summit: Environmental and Motion Matched Interactions; 'Madden', 'FIFA' and
Beyond!" GDC, 2021.** [talk] [recent]
<https://www.gdcvault.com/play/1027465/Animation-Summit-Environmental-and-Motion>

EA's generalised interaction system: characters and environment pieces are interchangeable *reference
points* for a synchronised animation, passive participants are pulled in without owning it, and motion
matching enters two ways — single-character matching into the interaction, synchronised multi-character
matching within it — while the player keeps control.
*Bearing:* the pattern for mounting, riding, grappling and co-op interactions: one description, N
participants posed against a shared frame the server owns.

**Bryan Dudash (NVIDIA). "Animated Crowd Rendering." *GPU Gems 3*, chapter 2, 2007.** [web]
[foundational]
<https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-2-animated-crowd-rendering>

Instanced skinned characters whose bone matrices live in a texture indexed by instance and frame, so
thousands of independently animated meshes draw in a few calls — near 10,000 at 30 fps on 2007
hardware; the origin of every animation-texture crowd technique.
*Bearing:* Forge's far tier of creatures (herds, flocks) should be exactly this — baked gait cycles in a
texture, instance id and phase per creature, no CPU pose — through the same mesh-shader path as the
near tier; with generated species, the bake is a step of species generation.

**Kasper Fauerby. "Crowds in Hitman: Absolution." GDC Europe, 2012.** [talk] [still-current]
<https://www.gdcvault.com/play/1015526/Crowds-in-Hitman>

The techniques and optimisations behind 1,200-character crowds at 30 fps on the consoles of the time,
and the gameplay decisions that keep them interactive. Ubisoft's equivalent is "Living Crowds: AI &
Animation in Assassin's Creed: Brotherhood" (Barbeau, Laidacker, GDC 2011,
<https://www.gdcvault.com/play/1014676/>).
*Bearing:* the tiering (skeleton and IK near, clip-only mid, texture-animated far) is the in-house
survey's T0/T1/T2 with numbers attached.

### Traces: what the animation layer must tell the world

**Colin Barré-Brisebois. "Deformable Snow Rendering in Batman: Arkham Origins." GDC, 2014.** [talk]
[still-current]
<https://www.gdcvault.com/play/1020379/Deformable-Snow-Rendering-in-Batman>

Characters and objects walking, fighting and falling stamp *arbitrary* shapes into a displacement field
carried around the player, accumulated over frames and rendered with tessellation, cheaply enough to
ship across two console generations in an open world. The template for every footprint, trail and
crater system since: the renderer owns a field, gameplay owns the stamps.
*Bearing:* the deformation system is a consumer and the animation layer is the producer, so the contract
is the thing to design. Forge's foot layer already knows when a foot is down and where (§3), so it emits,
per foot, a **foot-down event** and a **planted** state until foot-up, carrying: world position
(frame-local `f64`, camera-relative for the GPU); the surface normal from the ground probe; the
pressure, as body weight times this foot's share of support from the support polygon, replaced by Jolt's
contact impulse when the physics layer is active; the foot shape, an oriented ellipse or small polygon
in the foot's frame from the body plan (hoof, pad, claw, boot); the ground material; the contact
velocity, so a slide leaves a streak and a landing a deeper print; and entity id and tick. Tails,
bellies, dragged bodies and fallen ragdolls emit the same event from their contact shapes. Snow, sand
and mud differ only in depth per unit pressure and recovery time (plastic versus elastic, from the
material library); the grass displacement field of the in-house survey is the same field with a fast
recovery. Where gameplay reads traces (tracking, stealth), the server keeps a compact contact log per
tile — position, material, creature class, tick — and clients render from their own copy of the same
events, so a print seen by two players is the same print.

---

## 6. Cloth, hair and faces

Faces and gestures are out of scope for Forge's first years; one line: a blend-shape or bone-driven face
fits the same clip and additive machinery, and nothing else here changes.

**Jorrit Rouwe. "Jolt Physics — Soft Bodies." Documentation, 2023–2026.** [docs] [recent]
<https://jrouwe.github.io/JoltPhysics/index.html> (Unreal's equivalent: Chaos Cloth,
<https://dev.epicgames.com/documentation/en-us/unreal-engine/clothing-tool-in-unreal-engine>)

Position-based particles with edge, dihedral bend, volume and long-range-attachment constraints,
internal pressure, collision with rigid bodies, and *skinning constraints* that let joint animation
partially drive the cloth; soft-soft collision and buoyancy are not implemented. Chaos Cloth is the same
idea at production scale: a particle solver colliding against the character's physics asset, LOD per
mesh section.
*Bearing:* capes, straps and loose gear come with the physics engine Forge already plans to adopt; the
skinning constraint keeps cloth on a running creature. XPBD ropes and cables (in-house physics survey)
share the solver.

**AMD. "TressFX." GPUOpen, v5.0, 2023.** [code] [still-current]
<https://gpuopen.com/tressfx/>

Strand-based hair and fur simulation and rendering (MIT) with skinning, LOD, anti-aliasing and a Maya
authoring plug-in; 5.0 targets Unreal 4.27 and 5.4 with Lumen support — the only open, complete strand
pipeline.
*Bearing:* fur on generated creatures is mostly a shading problem; TressFX is the reference when Forge
gets there, and its simulation is a compute pass portable to Slang.

---

## 7. Animation over the network

The pose never crosses the wire; what does is whatever the client cannot compute alone.

**Glenn Fiedler. "State Synchronization." gafferongames.com, 5 Jan 2015.** [web] [still-current]
<https://gafferongames.com/post/state_synchronization/>

The three ways to network a simulation — deterministic lockstep (inputs only, needs bit-exact
determinism), snapshot interpolation (state only), state synchronisation (both, with local extrapolation
and priority-based sending) — and what each needs: jitter buffers, quantising on both sides so they
agree, error smoothing so corrections are invisible.
*Bearing:* Forge's active ragdolls are client-side and need no synchronisation; what must agree is the
*input* to the pose (controller state, contacts, hit events) and, where gameplay reads a body at rest,
its final state as a snapshot. Quantise on both sides.

**Timothy Ford. "'Overwatch' Gameplay Architecture and Netcode." GDC, 2017.** [talk] [still-current]
<https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and>

Blizzard's ECS, server-authoritative design with client prediction and replay, and the discipline that
makes it work: deterministic gameplay systems, replication of what the client cannot infer, everything
visual derived locally.
*Bearing:* the model Forge already follows; the animation corollary is to replicate animation
*parameters* (stance, gait, aim, interaction id and phase) rather than poses, and to send hit reactions
as small events (impulse, body part, seed) that every client turns into the same ragdoll response.

---

## 8. Rust ecosystem, 2025–2026

Versions and dates as read from crates.io and GitHub on 2026-09-23.

**SlimeYummy. "ozz-animation-rs." crates.io 0.11.0, Oct 2025.** [code] [recent]
<https://github.com/SlimeYummy/ozz-animation-rs>

A from-scratch Rust rewrite (not a binding) of ozz's runtime, MPL-2.0, tracking ozz 0.16.x: sampling,
partial and additive blending, two-bone and aim IK, user channels, root motion, skinning, SIMD,
WebAssembly, rkyv/serde — and *cross-platform deterministic*, its stated reason to exist. The offline
toolchain is still ozz's C++. For IK beyond its two solvers the ecosystem is thin: `k` (openrr, 0.32.0,
Sep 2024) is a robotics kinematics library with Jacobian solvers, usable but not built for animation.
*Bearing:* the fastest path to a working clip layer, with the determinism Forge's simulation crates
already demand should any pose ever feed gameplay. FABRIK and foot locking are small enough to write
in-house.

**Bevy contributors. "bevy_animation." crates.io 0.19.1, Aug 2026.** [code] [recent]
<https://docs.rs/bevy_animation/latest/bevy_animation/> (Fyrox's alternative:
<https://fyrox-book.github.io/animation/animation.html>, Fyrox 1.0.1, Mar 2026)

An `AnimationGraph` (a DAG of clip, blend and add nodes with per-node weights and *masks* over target
groups, evaluated bottom-up), an `AnimationPlayer`, transitions and clip events; no IK, no state machine,
and clips are curves over entity properties rather than SoA joint transforms. Fyrox 1.0 ships the other
shape: a Mecanim-style blending state machine with states, transitions, blend-by-weight and
blend-by-index nodes, root motion and an editor.
*Bearing:* readable references for graph and state-machine API design in Rust; neither is a dependency
for Forge, whose graph must feed IK and a ragdoll in SoA form for hundreds of bodies.

**Second Half Games. "jolt-rust" (joltc-sys 0.3.1+Jolt-5.0.0 on crates.io, May 2024; repository at
Jolt 5.3.0).** [code] [recent]
<https://github.com/SecondHalfGames/jolt-rust> (Rapier's joints for comparison:
<https://rapier.rs/docs/user_guides/rust/joints>)

Unsafe bindings through JoltC plus a safe wrapper (`rolt`), MIT/Apache-2.0, self-described as early
work in progress; ragdoll, motor and soft-body coverage is not enumerated and must be checked against
JoltC. Rapier, the pure-Rust alternative, has fixed, prismatic, revolute and spherical joints (plus
generic variants), PD motors with stiffness, damping, targets and impulse limits on the articulated
types, and a `MultibodyJointSet` for reduced-coordinate chains.
*Bearing:* with Jolt, the ragdoll-with-motors API of §4 must be exposed through the binding — budget for
extending JoltC. With Rapier, the powered ragdoll is joints with motors plus a hand-written
drive-to-pose, a few hundred lines.

---

## Recommendation for Forge

**Four layers, one direction of data flow, one skeleton description.** The skeleton is a list of *chains
with roles* (Hecker; Epic's retarget chains): a biped is spine, head, two arms, two legs; a generated
creature is whatever its body plan says. Everything below consumes that description.

1. **Simulation layer (server, deterministic, replicated).** Capsule or point controller, velocity,
   facing, ground contacts, stance flags, interaction id and phase — Holden's *simulation object*. The
   pose is pulled towards it on each client, never the reverse.
2. **Clip layer (client).** SoA transforms, sampling and blending jobs in the ozz shape
   (`ozz-animation-rs` or an in-house copy), additive layers and masks, inertialization for every
   transition, root motion warped onto targets for contextual actions. Clips from glTF; for humanoids a
   small blend space first, motion matching (Holden's MIT demo as the reference) once a capture library
   exists. Compression deferred until measured, format kept ACL-compatible.
3. **Procedural layer (client).** Foot placement with locking and release, two-bone IK for limbs,
   FABRIK for spines, necks and tails, look-at, hand IK for props and ledges, springs for secondary
   motion; it emits the contact events of §5. For generated creatures this layer *is* the animation:
   the in-house survey's gait generator (per-leg phase from leg count and Froude number) plans footfalls,
   feet are solved onto the ground, body height and tilt follow the support polygon, and a few procedural
   key poses per action (Rosen; Rain World) supply idles, attacks and deaths — Spore's architecture,
   needing no data.
4. **Physics layer (client, Jolt).** A ragdoll from the same chain list, driven every tick with
   `DriveToPoseUsingMotors` towards layer 3's output. Motor strength is a schedule per body and
   situation — full when unhurt, limited on the hit limb, zero on death — and force from the world
   (a boulder, another creature, a landing) arrives for free because the body is a real body. Balance on
   a shove is first SIMBICON-style feedback on the gait generator, which already knows the support
   polygon, and later a learned tracking policy (SuperTrack's supervised recipe, trained on Jolt data
   with ProtoMotions on the server GPU) that replaces the schedule — DReCon, reached incrementally. Only
   what gameplay reads (a corpse's rest pose, hit volumes) goes back to the simulation layer.

**Tiers.** Near creatures get all four layers; mid gets 1–3 with cheaper IK and no physics; far is
instanced texture animation baked at species generation; beyond that, nothing. Hitman and Overwatch are
where the per-tier budgets and replication rules come from.

**Build order.** (a) Skeleton description, glTF import, SoA sampling and blending, inertialized
transitions, compute skinning in Slang (LBS, DQS optional) feeding the mesh-shader and BLAS paths;
(b) gait generator and IK layer, tested on a biped and a generated hexapod, emitting contact events;
(c) the Jolt ragdoll tracking the kinematic pose with a strength schedule and hit reactions; (d) motion
matching for the player once capture exists; (e) the SuperTrack spike and an AMP-style style pass per
species, with a kill criterion: under 0.2 ms per near creature on the client or it stays a spike.

**The demo that proves it.** One scene on Forge's procedural terrain: a biped (clips + IK) and a
generated hexapod (procedural + IK) walk a slope with steps and rocks, feet planted without sliding (toe
drift under 2 cm while in contact), bodies levelled to the support polygon, prints left in the sand; a
boulder rolls into both; each is displaced physically, staggers, keeps its feet or falls and gets up, and
is back on its path within two seconds; a second client sees the same events at the same ticks from the
same inputs, with poses that differ only in what no one could point at. Client cost: under 0.15 ms CPU
per near creature for layers 2–4, one compute dispatch for all skinning. When that passes, the owner's
requirement is met in kind; the rest is data and species.

---

## Checked and left out

Things looked for and *not* above, with the reason, so the bibliography is auditable.

- **Unreal's Physical Animation Component docs** — the page exists (5.x and the 4.27 legacy page), but
  three fetch routes (current docs, the `UPhysicalAnimationComponent` / `FPhysicalAnimationData` API
  pages, a rendering proxy on the 4.27 page) returned only navigation. Jolt's ragdoll docs (§4)
  describe the same technique, so nothing is lost; no Unreal citation is made.
- **An "Assassin's Creed Unity crowd" talk** — GDC Vault's search has no Unity crowd talk; it has
  "Taming the Mob" (Bernard, 2008) and "Living Crowds" (Brotherhood, 2011, cited in §5). An AC
  parkour/navigation talk was not located either.
- **Jedi: Fallen Order physics animation; Hitman ragdoll blending** — no primary source found for
  either claim.
- **Ghost of Tsushima cloth** — no talk located; the Tsushima grass talk is in the in-house survey.
  Water–character interaction likewise has no primary source here.
- **Buss's 2004 note on Jacobian IK methods** — the UCSD host failed certificate verification; the
  peer-reviewed Buss & Kim 2005 paper was verified through Crossref and referenced in the FABRIK entry.
- **"Position-Based Inverse Kinematics"** — no canonical paper of that title found; the CGF survey covers
  the position-based family.
- **ACL Rust bindings** — none; the `acl` crate (0.0.0) is an unrelated POSIX ACL utility.
- **ozz-animation's latest C++ release** — the releases page fetch returned an implausible reading and
  was not trusted; the version is recorded only as "0.16.x" from the Rust port's compatibility note.
- **Unity's current motion-matching status** — only the Kinematica 0.8.0-preview manual could be
  verified; no discontinuation page was fetched, so §2 says only that no later release was found.
- **"Animation Compression and GPU skinning for crowds"** — no talk of that title exists; GPU Gems 3
  ch. 2 and the Hitman talk cover it.
- **Featherstone on Springer** — redirected to an authentication wall; confirmed from the author's site,
  the 2008 date from the ISBN reference there.
- **Animation streaming; Houdini vertex-animation-texture tooling** — practice, not literature; no
  primary source fetched.
- **Codebook Matching (2024) and Control Operators (2025)** — verified from the AI4Animation repository
  and Holden's pages; no DOI confirmed for either, so none recorded.

---

## Verification notes

- The session's web-search budget was exhausted before this file started, so every source was verified
  by direct headless fetch of a known page: GitHub (ACL, ozz-animation, ozz-animation-rs, AI4Animation,
  Motion-Matching, ProtoMotions, JoltPhysics, jolt-rust); author pages (theorangeduck.com,
  xbpeng.github.io, cs.ubc.ca/~van, goatstream.com, chrishecker.com, users.cs.utah.edu/~ladislav,
  royfeatherstone.org, research.cs.wisc.edu); NVIDIA research pages and arXiv (MaskedMimic, CALM,
  PhysDiff, PHC, ControlVAE); Ubisoft La Forge (montreal.ubisoft.com URLs now redirect to
  ubisoft.com/studio/laforge — the new URLs are recorded); Meta Research (Won 2022); Epic documentation
  (only the Motion Matching, Motion Warping, IK Retargeting and Clothing pages returned content) and the
  5.4 blog post through a rendering proxy; GDC Vault play pages plus its `browse?keyword=` listing to
  locate the TLOU2, Rain World, Hitman, Brotherhood and Batman talks; Wikipedia (Euphoria); crates.io's
  JSON API and docs.rs for every crate version and date; rapier.rs; the Fyrox book's print page;
  jrouwe.github.io.
- ACM DL, Elsevier, Wiley and Taylor & Francis block automated fetching. Every DOI above was resolved
  through the Crossref API (`api.crossref.org/works/<doi>`), which returned title, authors, venue and
  year for each; Gleicher 1998 was confirmed through the Semantic Scholar API because Gleicher's paper
  index has moved and the old gallery page carries no citation.
- No browser pane was opened at any point. DuckDuckGo's HTML endpoint returned CAPTCHAs and Bing
  returned localised, unrelated results; neither was used. A YouTube search page was fetched once for
  the Naughty Dog talk and returned only the site footer; that talk and the other GDC talks are confirmed
  from GDC Vault's own listing and play pages, not from any video page.
- andreasaristidou.com failed certificate verification on HTTP and HTTPS; FABRIK and the survey are
  cited by Crossref-verified DOI with the author page as a courtesy link.
- Dates and versions as read on 2026-09-23: ACL 2.1 (Dec 2023), UE 5.4 blog (Apr 2024), `gltf` 1.4.1
  (May 2024), `k` 0.32.0 (Sep 2024), `joltc-sys` 0.3.1 (May 2024, repo at Jolt 5.3.0),
  `ozz-animation-rs` 0.11.0 (Oct 2025), Fyrox 1.0.1 (Mar 2026), `bevy_animation` 0.19.1 (Aug 2026),
  Holden's foot-locking article (30 Jul 2026); the Batman talk was fetched on 2026-09-24.
- Where a summary states a detail the fetched page did not itself show (DeepMimic's reference-state
  initialisation and early termination; MANN's gating network; SIMBICON's swing-hip feedback law;
  Bollo's decay), it is standard content of the paper or talk and stated as such. Everything in the
  citation lines — authors, titles, venues, years, URLs — was confirmed.
