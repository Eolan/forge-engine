# Hit reactions (Phase 7): patents and prior art

*A research agent, 2026-10-24; the session checked US 7,403,202's expiry again (Google Patents
lists it expired). Built as D-012's strength schedule the same day (`physics-lab --lab creatures`).*

Search of 2026-10-24 for Forge's simulated hit reactions on Jolt powered ragdolls (D-012,
animation.md §4, `demos/physics-lab/src/lab/creatures.rs`). The technique checked:

1. A contact impulse lands on the struck part (the physics does it).
2. A strength schedule: the struck part's and its chain neighbours' motor torque limits (and/or
   stiffness) drop by an amount from the hit's strength, then recover to full within a fraction
   of a second; the clip keeps playing underneath.
3. Optionally limp (motors off) past a threshold, then the motors ramped back.
4. Optionally a balance check (centre of mass over the feet's support) picks stagger or fall.

**How it was searched, and what is missing.** About thirty queries: Google Patents (assignee
NaturalMotion; "ragdoll" in CPC G06T13/40; "hit reaction"; active/powered/partial ragdoll),
web searches per company in the brief, and searches restricted to the Google Patents and USPTO
indexes [33]. After three queries Google refused every request as automated traffic (HTTP
503, about 03:40 to the end), so **no status or expiry below is "as Google Patents states"**.
Claims were read from the USPTO's own PDFs [1–20] by OCR; expiries are computed (20 years from
the earliest US or PCT filing plus the printed term adjustment). Fee lapses and the EP/CN
family members' status are **unverified**.

## Headline

- **The whole technique is in an expired EA patent: US 7,403,202 B1 [1].** It was filed
  2005-07-12 as the parent application and granted 2008-07-22, with a 0-day adjustment, so it
  expired no later than 2025-07-12. Its description has, nearly step for step:
  - A ragdoll driven by forces and torques toward the motion-capture pose, clamped by
    per-part force and torque limits that can be changed on the fly.
  - After a collision, the strength and speed limits of the affected parts are lowered for
    several ticks, so the parts take the impulse and move back to the pose over a few frames
    (step 2).
  - Each joint's torque limit ramped each tick toward a target set by a supervisor, and
    strength cut while falling or lying down (steps 2–3).
  - A balance check: an estimated centre of mass, a support circle from the shins or feet,
    and a threshold that triggers a dynamic fall (step 4).
  - A get-up with forces ramped on and a blend back to the clip (step 3's recovery).

  Its claim 1 claims the balance check itself (step 4), and that claim has expired.
- **No live patent was found that claims a strength schedule or stiffness ramp after an
  impact.** Every live claim read needs something Forge's four steps do not do:
  - a motion-capture frame, clip or start frame chosen by matching against the hit or pose;
  - EMG-recorded muscle data;
  - a learned policy or neural model;
  - a quasi-physical root force;
  - a live camera-tracked user, or a separate motion-synthesis behaviour layer;
  - NaturalMotion's elastic/rigid target-frame scheme;
  - a limb's mass, length or link count changed for an injury.
- **Published prior art, by date:**
  - JP 2001-005999 (2001) [16]: a hit sent, attenuated, up the chain to the parent parts.
  - Zordan & Hodgins 2002 [21]: the stiffness and damping terms change on impact.
  - Zordan et al. 2005 [23]: games' "go limp on a hit" practice, and the simulated burst.
  - US 7,403,202 (filed 2005, published 2008) [1].
  - PuppetMaster 2015 [27]: collisions unpin muscles, which regain their pin weight over
    time, with a knock-out threshold.
  - Unreal's Physical Animation component, 2016 [25].

## Live patents read, and steps 1–4 against their independent claims

Expiry is computed (see above); "ind." = independent claims.

| Patent (holder) | Filed / est. expiry | What the ind. claims require | Steps 1–4 |
|---|---|---|---|
| US 9,827,496 B1 (EA) [2] | 2016-03-25 (prov. 2015-03-27) / ~2036-04 | Target pose; collision or external force; the ragdoll's final pose from both; reference features from that pose; **a motion-capture frame matched to them, shown as the current frame** (claims 1, 10, 17) | 1–4 do not select or show a mocap frame matched to the ragdoll; Forge draws the bodies. **Not within.** Caution: a get-up that starts the clip at the frame nearest the ragdoll's pose would do this. |
| US 10,388,053 B1 (EA) [3] | 2016-03-25 / ~2036-06 | On an in-game event, choose a new animation by game rules; **choose its start frame from a context-dependent subset by comparing reference features**; transition (1, 13, 18) | 1–4 switch no clip. **Not within.** Same caution for step 3's get-up clip. |
| US 10,022,628 B1 (EA) [4] | 2016-03-25 (prov. 2015-03-31) / ~2036-12 | Rigid-body character; detect collision; target feature from it (e.g. centre-of-mass change); **identify motion-capture data in a store by the collision and the feature, modify it** to an updated pose; render (1, 11, 16) | The clip keeps playing; nothing is looked up by the hit. **Not within.** It would be, if a reaction clip were chosen by the hit and warped toward a computed feature. |
| US 10,497,163 B1 (EA) [5] | 2017-05-16 / ~2038-01 | Muscle state per body part from the animation; a collision changes the motion by that state; **motion capture containing EMG muscle-activation data** (1, 6, 10) | No EMG data. **Not within**, even if strengths are authored per clip. |
| US 8,860,732 B2 (Adobe) [7] | 2010-11-30 / ~2032-06 | Quasi-physical simulation as a linear system enforcing a centre-of-mass goal; joint torques minimising **a non-physical root force**, which is added (1) | Jolt's full simulation; no such optimisation. **Not within.** The dog's existing upright spring on its root is a plain spring, with no linear system or goal constraint. |
| US 8,665,277 B1 (Lucasfilm) [8] | 2008-01-23 / ~2031-10 | A "macro behavior system" mapping engine behaviours to a separate **motion-synthesis system's** behaviours on a simplified skeleton, and back to the full skeleton (1) | No behaviour-request layer. **Not within.** |
| US 9,177,409 B2 (NaturalMotion) [9] | 2010-04-29 / ~2032-07 (Google's listing said active, a search snippet said expired: unverified) | Target frame **elastically** connected to a part's control frame, child parts' frames **rigidly** connected to the target frame, controls applied in isolation (1) | Per-joint motors toward the clip's pose. **Not within.** |
| US 11,790,587 B2 [10], US 12,175,582 B2 [11] (Tencent) | PCT 2020-11-02 / ~2041-01 (the second under a terminal disclaimer) | Terrain feature, state and task fed to an **animation processing model** giving joint actions, then torques and the pose update | No model in 1–4. **Not within.** It matters for a later learned tracker (DReCon line). |
| US 11,928,765 B2 [12], US 12,236,513 B2 [13] (Tencent) | PCT 2020-10-26 / ~2041-02, ~2040-10 | **PPO policy** training (the first); target posture from an **RL policy**, then joint forces (the second) | No policy. **Not within.** |
| US 11,113,862 B1 (Sony Interactive) [14] | 2020-05-11 / ~2040-05 | On an injury, **increase a limb's mass, change a limb's length or the skeleton's link count** to mimic an injured gait (1, 11) | 1–4 change motor limits only. **Not within.** Caution: a "wounded limb" made heavier would meet claim 1. |
| US 11,132,606 B2 (Sony Interactive) [15] | 2019-03-15 / ~2040-03 | **RL training** of bone subsets on different target animations, composited by physics (1) | No training. **Not within.** |
| US 11,928,764 B2 (NVIDIA) [15b] | 2020-09-15 / ~2040-09 | **Rebalancing groups of neural-network training data** by accuracy (1, 11) | No training. **Not within.** |
| US 8,803,889 B2 (Microsoft) [17] | 2009-05-29 / ~2032-01 | **A camera-tracked user's live motion** blended per body part with pre-recorded motion (1) | **Not within.** |

Read and unrelated to steps 1–4: Sony's US 9,165,392 B2 (joint-gap correction in a solver)
[18]; NaturalMotion's US 8,730,245 B2 (animation-graph editor) [19], US 8,199,151 B2
(animation events), US 9,652,879 B2 (blend-space weights), US 10,062,197 B2, US 10,297,066 B2
and US 10,410,393 B2 (IK chains fitted to curves) [20]; Zynga's US 9,449,416 B2 [20]; EA's
US 10,885,691 B1 (mocap recording; ragdoll strength only in its description) [6].

Not read (**unverified**): ETRI's KR 101098834 B1 and Tencent's CN 113781615 B and
CN 112927331 A, listed for "ragdoll" [32]; Activision's 2024 applications (US 2026/0021402 A1
and two siblings) and Take-Two's US 2025/0095261 A1 in the same listing [32]. Microsoft's
KR 101751078 B1 family is US 8,803,889 [17]. **Unity, Epic, Nintendo, Rockstar, Ubisoft:** no
patent on physics hit reactions turned up. **NaturalMotion/Euphoria:** none of the utility
patents in its listing [31] claims a powered ragdoll's hit reaction; its "dynamic motion
synthesis" root is the expired Isis patent below.

## Expired patents and published prior art (the non-infringing path)

- **US 7,403,202 B1 (EA, Nash)** [1]: steps 1–4 as above. It is expired and was published on
  2008-07-22.
- **US 7,685,081 B2 (Isis Innovation, Reil)** [16b]: NaturalMotion's founding "bipedal walking"
  patent, with neural oscillators driving torque-limited joints. PCT filed 2002-01-25, a
  70-day adjustment, so it ended about 2022-04. EP 1392556 (same inventor and filing date)
  looks like its EP member [34] (via search, not opened). A Google listing showed "Active",
  which conflicts with the computed term (unresolved).
- **JP 2001-005999 A**, as summarised in the background of US 7,012,608 [16]:
  - When an enemy's forearm is hit, it moves by physics.
  - The hit vector is passed, attenuated, to the upper arm, chest and waist (step 2's chain
    neighbours).
  - A restoring force pulls each part to its default posture.
  - Playback resumes after a time, or when a "bodily power" parameter reaches zero.
- **Zordan & Hodgins, SCA 2002** [21]: mocap tracked by PD control and a balance controller.
  The system reacts to collision forces by changing its stiffness and damping terms, and
  returns to tracking. Abstract read; whether the gains drop and ramp is **unverified** (full
  text not read).
- **Shapiro, Pighin & Faloutsos, Pacific Graphics 2003** [22]: switches characters from
  kinematic to dynamic control on an impact and back. Dynamic controllers fall gracefully.
- **Mandel 2004 (CMU thesis)**: fall and recovery controllers take over on impact. Known only
  from Zordan 2005's citation [23]; **not read**.
- **Zordan, Majkowska, Chiu & Fast, SIGGRAPH 2005** [23]: a short simulated burst after the
  impact (PD tracker, fixed gains, 0.1–0.2 s reaction delay), then a motion-library search for
  the clip to re-enter; it records that games of the time handed a hit character to the physics
  engine to go limp like a ragdoll (step 3).
- **Wrotek, Jenkins & McGuire, Sandbox 2006** [24]: simultaneous simulation and mocap
  tracking, with a weak root spring for balance whose strength decides falls under hard hits.
  This matches Forge's existing dog balance spring. Abstract via search; **PDF not opened**.
- **PuppetMaster (RootMotion, Unity), released 2015-10-01** [27, 28]: collisions unpin muscles
  (resistance per group, spreading to parent, child and same-group muscles); each group regains
  its pin weight at its own speed (step 2); past a knock-out distance from the target pose the
  puppet goes unpinned (step 3), and gets up once the hip has slowed.
- **Unreal Engine 4.13, 2016-09-01** [25, 26]: the Physical Animation component drives skeletal
  animation through motors with set strengths, applicable to a bone and those below it (API
  pages via search, **unverified**); Epic's hit-reaction recipe ramps a per-bone physics blend
  weight from the hit bone up to 1 and back to 0.
- **Jolt** (MIT) [29, 30]: `DriveToPoseUsingMotors` (position and velocity targets per
  constraint motor), `MotorSettings::mMaxTorqueLimit` and spring settings, and the
  `PoweredRigTest` sample playing an animation through the motors.
- **Havok "powered ragdoll" and Euphoria:** forum mentions and Wikipedia only [35, 36];
  nothing technical was read.

## Patents on "strength schedules" or "stiffness ramps" after impacts

None found live. The schedule (limits lowered on collision, recovered over ticks, ramped each
tick) is disclosed in the expired US 7,403,202 [1], and in PuppetMaster's regain-pin speed
[27]. The closest live claims are:
- US 10,497,163 [5]: muscle state × collision, but only with EMG data.
- US 9,827,496 / 10,388,053 / 10,022,628 [2–4]: ragdoll or collision, then a mocap
  frame or clip lookup.

## Step by step

- **Step 1 (the impulse on the part):** no live claim read requires only this.
- **Step 2 (the drop and recovery):**
  - Disclosed in expired US 7,403,202 [1], JP 2001-005999 [16] and PuppetMaster [27].
  - No live claim found covers it without a mocap lookup, EMG data or a model.
- **Step 3 (limp, then the ramp back):**
  - Disclosed in [1] and [23].
  - Live claims are approached only if the get-up chooses its clip or start frame by matching
    the ragdoll's pose to animation frames [2, 3].
  - Choosing get-ups by rule (face up or down) and blending the pose in, as [1] does, avoids
    that element.
- **Step 4 (the balance check):**
  - Claimed in [1]'s claim 1 (centre of mass against a support circle), which has expired.
  - Fall decisions without a centre-of-mass test: [24]'s root-spring strength and [27]'s
    knock-out distance (pose error).
  - No live claim read requires a centre-of-mass support test.

## Sources

1. US 7,403,202 B1, EA. USPTO PDF (OCR read): https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7403202; Google page not reachable: https://patents.google.com/patent/US7403202B1/en
2. US 9,827,496 B1, EA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/9827496
3. US 10,388,053 B1, EA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10388053
4. US 10,022,628 B1, EA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10022628
5. US 10,497,163 B1, EA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10497163
6. US 10,885,691 B1, EA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10885691
7. US 8,860,732 B2, Adobe: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8860732
8. US 8,665,277 B1, Lucasfilm: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8665277
9. US 9,177,409 B2, NaturalMotion: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/9177409
10. US 11,790,587 B2, Tencent: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11790587
11. US 12,175,582 B2, Tencent: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12175582
12. US 11,928,765 B2, Tencent: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11928765
13. US 12,236,513 B2, Tencent: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/12236513
14. US 11,113,862 B1, Sony Interactive: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11113862
15. US 11,132,606 B2, Sony Interactive: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11132606. 15b. US 11,928,764 B2, NVIDIA: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11928764
16. US 7,012,608 B1 (background on JP 2001-005999): https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7012608. 16b. US 7,685,081 B2, Isis Innovation: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7685081
17. US 8,803,889 B2, Microsoft: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8803889
18. US 9,165,392 B2, Sony: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/9165392
19. US 8,730,245 B2, NaturalMotion: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8730245
20. NaturalMotion and Zynga, read and unrelated: https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8199151, …/9652879, …/10062197, …/10297066, …/10410393, …/9449416 (same prefix)
21. Zordan & Hodgins, "Motion capture-driven simulations that hit and react", SCA 2002, abstract: https://publications.ri.cmu.edu/motion-capture-driven-simulations-that-hit-and-react (DOI 10.1145/545261.545276; full text not read)
22. Shapiro, Pighin & Faloutsos, "Hybrid control for interactive character animation", PG 2003: https://www.eecs.yorku.ca/~pfal/Petros_Faloutsos/Publications_files/PG2003.pdf
23. Zordan, Majkowska, Chiu & Fast, "Dynamic response for motion capture animation", SIGGRAPH 2005: http://strangepaths.com/forum/files/Zordan2005.pdf
24. Wrotek, Jenkins & McGuire, "Dynamo", Sandbox 2006, DOI 10.1145/1183316.1183325: https://cs.brown.edu/research/pubs/theses/masters/2006/pwrotek.pdf (not opened; abstract via search)
25. Unreal, physics-driven animation, hit-reaction recipe: https://dev.epicgames.com/documentation/en-us/unreal-engine/physics-driven-animation-in-unreal-engine
26. UE 4.13 release with the Physical Animation component, 2016-09-01: https://gamefromscratch.com/unreal-engine-4-13-released/
27. PuppetMaster BehaviourPuppet docs: http://root-motion.com/puppetmasterdox/html/page10.html
28. PuppetMaster release thread, 2015-10-01: https://discussions.unity.com/t/puppetmaster-advanced-character-physics-tool-released/599766
29. Jolt Ragdoll class: https://jrouwe.github.io/JoltPhysics/class_ragdoll.html
30. Jolt MotorSettings.h and PoweredRigTest: https://github.com/jrouwe/JoltPhysics/blob/master/Jolt/Physics/Constraints/MotorSettings.h, https://github.com/jrouwe/JoltPhysics/blob/master/Samples/Tests/Rig/PoweredRigTest.cpp
31. Google Patents, assignee NaturalMotion (read before the block): https://patents.google.com/?assignee=NaturalMotion
32. Google Patents, "ragdoll" in CPC G06T13/40 (read before the block): https://patents.google.com/?q=%22ragdoll%22&cpc=G06T13%2F40
33. Restricted web searches of patents.google.com and image-ppubs.uspto.gov (index snippets only, used to find candidates)
34. EP 1392556 A2, Isis Innovation (via search, not opened): https://data.epo.org/publication-server/rest/v1.2/patents/EP1392556NWA2/document.html
35. Euphoria (software): https://en.wikipedia.org/wiki/Euphoria_(software)
36. Havok "powered ragdoll" (forum, unverified): https://gamedev.net/forums/topic/661745-havok-for-dynamic-animation-like-euphoria/
