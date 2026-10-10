# Research — Assets and their licences: humans, buildings, interiors, vegetation, vehicles, textures

> Companion to D-048 (external models are never committed: `tools/fetch-assets.sh` fetches them
> from a manifest, built as `assets/external.tsv`, CC0 or CC-BY only, credits in `CREDITS.md`,
> restricted ones "reference only"), D-039 (buildings are grammar-derived assemblies of kit modules,
> a style set per district), `vegetation-materials.md` §1 and §5 (SpeedTree, Sapling and Mtree,
> Fab's 2024 Megascans offer, the Poly Haven and ambientCG licences), `civilisations-styles.md` §3–§4
> (KitBash3D's kits as the market's unit of a style, procedural trims and materials) and
> `animation.md` §6 (TressFX, cloth). Written 2026-10-10 for the owner's question of that day: what
> can furnish a showcase built as if for a game release (the motivating example is a city sandbox:
> buildings with interiors that match their use, parks and trees, people near the quality of
> current AAA games with varied clothes, cars, boats and flying craft), with the engine public, the
> sandbox local, and assets that may not be redistributed kept out of the public repository. Every
> source was checked with WebFetch and WebSearch only, no browser pane, nothing downloaded; each
> entry says whether its page was read, read through a reader proxy, or only seen as a search
> record ([Verification notes](#verification-notes)). Licence terms and prices are quoted as read on
> 2026-10-10 and will drift.

The short answer: **the free, redistributable sources are good enough for surfaces, props and
placeholders, and not for a release-quality city of people, buildings and vehicles.** Poly Haven
and ambientCG (CC0 scans and models), Kenney and KayKit (CC0 low-poly kits), the MakeHuman / MPFB
assets (CC0, some CC-BY), NAVER's Anny body model (Apache-2.0 over CC0 MakeHuman data) and
Microsoft's Rocketbox avatars (MIT) may sit in or be fetched by a public repository. Quaternius
moved its site to a no-redistribution licence on 28 August 2026 while its pack pages still say
CC0: treat new downloads as shippable-only until that is cleared up. Everything that reaches AAA
quality is **shippable but not redistributable**: MetaHuman (since June 2025 usable in any engine
under the Unreal EULA, free below $1 M of revenue, and since June 2026 with its face-rig runtime,
RigLogic and DNA, open under MIT), Fab and Megascans (Standard licence, any engine), Reallusion,
Daz, KitBash3D, Synty, TurboSquid and CGTrader, BlendKit's royalty-free tier, Adobe's Substance
assets, Textures.com, and the trees grown with SpeedTree or The Grove. Unreal-only content (the City
Sample's extracted packs) and the research body models (SMPL, SMPL-X, STAR, BEDLAM, FLAME except its
2023 Open release) are out. Humans are therefore covered for a local sandbox by MetaHuman, and a
Forge-made generator is not needed now; the open pieces to seed one later exist (§3). The real gaps
are a coherent realistic building kit with interiors, interiors at one quality across many uses,
vehicles in one futuristic style, and park-species trees.

---

## Contents

1. [Licence classes and the clauses that decide them](#1-licence-classes-and-the-clauses-that-decide-them)
2. [Humans](#2-humans)
3. [What Forge would need to generate its own humans](#3-what-forge-would-need-to-generate-its-own-humans)
4. [Building kits and architecture](#4-building-kits-and-architecture)
5. [Props, furniture and interiors](#5-props-furniture-and-interiors)
6. [Vegetation](#6-vegetation)
7. [Vehicles](#7-vehicles)
8. [PBR textures and materials](#8-pbr-textures-and-materials)
9. [All sources at a glance](#9-all-sources-at-a-glance)
10. [Paid shortlist](#10-paid-shortlist)
11. [Recommendation for Forge](#recommendation-for-forge)
12. [Checked and left out](#checked-and-left-out)
13. [Verification notes](#verification-notes)

---

## 1. Licence classes and the clauses that decide them

| Class | Meaning | Where it may live |
|---|---|---|
| **1. Redistributable** | CC0, CC-BY, MIT, Apache-2.0 and similar | in the public repository or fetched by it (CC-BY with its credit line) |
| **2. Shippable, not redistributable** | royalty-free stock licences, vendor EULAs | the local sandbox and a released game's packed data; never the public repository or a public fetch script |
| **3. Engine- or platform-locked** | e.g. "licensed for use only with Unreal Engine-based products" | not usable by Forge |
| **4. Non-commercial or research-only** | academic licences | not usable for a release |

The clauses that move a source between classes, as they appear in the licences read below:
- **Standalone redistribution.** Every class 2 licence forbids giving the files away on their own,
  modified or not (Fab §5(a), TurboSquid, CGTrader §21A, Synty, KitBash3D, BlendKit, Quaternius
  QAL §3(a), Reallusion §2.1(B), Adobe §3.1). A public repository, or a public script that fetches
  the files, is that kind of distribution.
- **Protection in the shipped game.** Several ask that end users cannot extract the files:
  "contained in proprietary formats so that they cannot be opened or imported" (TurboSquid, and
  Reallusion §6.3), "commercially reasonable measures to prevent the end user from gaining access"
  (CGTrader §21A.3), "restrict end users from extracting" (Fab §4(c)), encryption "or any other
  resources at User's disposal" (Daz §3.0), "can't be extracted by the users in an easy way"
  (BlendKit), "the trees can not be extracted" (The Grove). An open-source engine's cooked format is
  documented by its own code, so a release needs packed and at least obfuscated archives.
- **Open-source combination.** Fab's EULA forbids combining Standard content with code or content
  under an open-source licence that would extend to it (§6(a)); Textures.com forbids releasing
  content "under Open Source Licences" (Art. 6(3)(xi)). The game's data must carry its own licence,
  separate from the engine's.
- **AI and machine learning.** MetaHuman (Unreal EULA §6(e)), Reallusion (§2.1(C)(5)), Daz, Synty,
  KitBash3D, Adobe (§3.3), Textures.com (6(3)(ix)), TurboSquid (without authorisation), Mixamo and
  Fab's NoAI-tagged content forbid using the assets to train or test models. CGTrader's standard
  licence allows it; its "No AI" variant does not. Any learned system in Forge (motion matching,
  neural compression, learned LODs) must train on class 1 or own data only.
- **Tiers, seats and revenue.** Fab: Personal below $100 000 of gross revenue in the last 12 months,
  Professional above, the same rights in both (Fab's licence summary, a search record). Unreal (MetaHuman): free below $1 M, seats above. SpeedTree:
  Indie below $100 k, Pro below $1 M. KitBash3D: Individual, Small Business (≤ 50 employees),
  Enterprise. Synty: five seats per purchase. Textures.com: free downloads under an Indie licence
  with a $150 k limit.
- **What happens when a subscription ends.** Adobe: assets downloaded and paid for stay licensed
  (§4). KitBash3D: projects already made stay licensed, no new projects (its help centre, a search
  record). SpeedTree (Unity's terms
  §2.7): rights to created and library assets **cease**, and productions not yet released may not be
  released. Synty: licensed "while your subscription plan is active". The Grove: lifetime.
- **Per character, per product, per use.** Reallusion: one Standard licence grants outputs for one
  character in a commercial game, Extended licences for CC Components, Enterprise for any in-game
  character creator. Daz: an Interactive licence per product, if its creator offers one.
- **Players' own content.** The Unreal EULA forbids letting end users put Licensed Technology in
  their own products (§4(c)); Synty forbids "Game Creation Software". Modding or user-generated
  content would need class 1 assets.

---

## 2. Humans

### 2.1 MetaHuman (Epic Games)

**Epic Games, MetaHuman licensing FAQ.** [web] [read through a reader proxy; the site redirects to a
login]
<https://www.metahuman.com/en-US/license>

"MetaHumans can be used with any engine or creative software." MetaHuman is included in the
standard Unreal Engine licence: its technology and tools "are Licensed Technology under the Unreal
Engine license", and it is "free to use for anyone making under $1 million USD in revenue", with
standard Unreal terms above. The answers on seats, royalties, selling and AI were blank in the
captured text. No date on the page.
*Bearing:* since June 2025 MetaHumans are no longer Unreal-only: class 2, not class 3.

**Epic Games, Unreal Engine EULA.** [docs] [read through a reader proxy; direct fetch 403]
<https://www.unrealengine.com/en-US/eula/unreal>

A "Non-Engine Product" includes no Engine Code and needs none to run; asset files developed with
the engine, "such as character models and animations", count as such, and no royalty is due on
distributing them (§4(a)(i)). A product incorporating Licensed Technology must do so "only in
object code and only as an inseparable part" of it (§4); end users may not put Licensed Technology
into their own products (§4(c)). The seat exception holds while the group has "generated less than
$1,000,000 USD in gross revenue over the last 12 months" (§6(b)(i)). §6(e) forbids using the
Licensed Technology "as a training input to any Generative AI Program" and using "MetaHuman digital
characters and animation curves" to build or enhance AI or machine-learning systems, training or
testing included. The EULA also notes that "certain assets that we make available under separate
agreements are available for use only with Unreal Engine". No date on the page (a change log is
linked).
*Bearing:* MetaHumans in a game built on Forge owe no royalty and need no seat below $1 M; they
must ship inseparable from the game (class 2), never in the public repository, never to modders,
never in a training set.

**CG Channel, "You can now sell MetaHumans, or use them in Unity or Godot", 4 June 2025.** [web]
[read]
<https://www.cgchannel.com/2025/06/you-can-now-sell-metahumans-or-use-them-in-unity-or-godot/>

The June 2025 release (MetaHuman 5.6, out of early access) put MetaHuman Creator, Mesh to MetaHuman
and MetaHuman Animator inside the Unreal Editor; the in-editor workflows "rely on cloud services
for autorigging and texture synthesis". MetaHuman characters and animations count as non-engine
products, so games in other engines avoid the 5 % royalty; above $1 M a year, Unreal seats cost
$1,850 a year; characters and clothing may now be sold on marketplaces.
*Bearing:* authoring needs Unreal 5.6 or later and an Epic account (free), with some steps on Epic's
servers; the result is used elsewhere.

**Epic Games, "MetaHuman 5.8 is now available", 11 June 2026; OpenRigLogic repository; MetaHuman
Devkit documentation.** [web] [repo] [docs] [news post read through a reader proxy; repository and
documentation read]
<https://www.metahuman.com/news/metahuman-5-8-is-now-available> ·
<https://github.com/EpicGames/openriglogic> ·
<https://dev.epicgames.com/documentation/metahuman/metahuman-devkit-in-unreal-engine>

5.8 made "core MetaHuman libraries available as open source under MIT for the first time":
OpenRigLogic holds the DNA library ("reading and writing MetaHuman DNA files") and RigLogic ("a
highly optimized C++ library providing realtime evaluation of the MetaHuman rig"), both C++ with
Python bindings, "released under the MIT License", the first part of a "MetaHuman Devkit" to
integrate "into the platform or application of your choice outside of Unreal Engine". The sample
assets and the Faceboard control rig come separately from Fab under the Standard licence. The
keynote (State of Unreal, 17 June 2026, search record) put it as delivering MetaHumans in "any
game, any engine, any platform" (quoted by unrealsource.com, read).
*Bearing:* the face rig's runtime evaluation, the part that was locked inside Unreal, is now class 1
code Forge may vendor and credit; the characters stay class 2.

**Epic Games, "MetaHuman Creator Export Tool".** [docs] [read]
<https://dev.epicgames.com/documentation/metahuman/metahuman-creator-export-tool-in-unreal-engine>

DCC Export writes head DNA, body DNA and textures as ".dna, .png, and .json files" (for MetaHuman
for Maya and for Houdini); Geometry Export makes skeletal-mesh assets in the project; Materials
Export copies the material instances and textures. No licence note, no date. MetaHuman 5.7's notes
(search record) say vertex order was "lost when exporting FBX from Unreal Engine", which blocked
round trips outside the DNA-aware tools; a third-party Blender DNA importer exists (search record).
*Bearing:* the path out is DNA plus textures plus the skeletal mesh, read by Forge with the MIT DNA
library; FBX or glTF export alone loses the face rig.

### 2.2 MakeHuman and MPFB 2

**MakeHuman community, "License".** [web] [read]
<https://static.makehumancommunity.org/about/license.html>

"The source code of MPFB is shared under GPL"; MakeHuman's "under AGPL". "All core assets are shared
under Creative Commons, CC0": no payment, notice or attribution, for the assets and their
derivatives. No date.

**MakeHuman community, asset packs.** [web] [read]
<https://static.makehumancommunity.org/assets/assetpacks.html>

Each pack has one licence. **CC0:** the system assets (proxies, eyes, teeth), skins 01–03, hair 01,
eyebrows and eyelashes, clothes (dresses 01, pants 01, shirts 01, shoes 01, skirts 01, suits 01–02,
underwear 01 and 04, gloves, hats 01–02), glasses 01, poses, **Visemes 01** (Microsoft style),
**Visemes 02** (Meta/Oculus style) and **Faceunits 01 (ARKit style)**. **CC-BY:** hair 02–03,
dresses 02–03, pants 02–03, shirts 02–03, shoes 02–03, skirts 02, suits 03, underwear 02–03,
glasses 02, hats 03–04 and several body-part and animal packs.

**CG Channel, "Check out open-source Blender character generation plugin MPFB 2", 11 March 2025;
Blender Extensions, MPFB.** [web] [read; the extension listing's version and date are a search record]
<https://www.cgchannel.com/2025/03/check-out-open-source-blender-character-generation-plugin-mpfb-2> ·
<https://extensions.blender.org/add-ons/mpfb/>

MPFB 2 (Blender 4.2+, GPL-3.0-or-later, 2.0.17 dated June 2026 on the listing) creates characters
"complete with skin textures and clothing", rigs them automatically with a choice of rigs including
Rigify, and exports in any format Blender writes (glTF, USD, Alembic); content made with it is CC0
and usable "including in closed-source games".
*Bearing:* the only complete human kit that is class 1 end to end (with CC-BY credits for some packs).
Its look is a generic game character of the 2010s, not AAA: a base and a source of data, not a
finished crowd.

### 2.3 Open parametric bodies, 2025–2026

**NAVER LABS Europe, "Anny: a free-to-use 3D human parametric model for all ages", 6 November 2025;
repository `naver/anny`; paper arXiv 2511.03589.** [web] [repo] [paper] [blog and repository read;
paper seen as a search record]
<https://europe.naverlabs.com/blog/anny-a-free-to-use-3d-human-parametric-model-for-all-ages/> ·
<https://github.com/naver/anny> · <https://arxiv.org/abs/2511.03589>

Anny is built on MakeHuman's base mesh (over 13 000 vertices, 163 bones in the full rig, 104 in its
default rig) with 564 artist-made blendshapes driven by phenotype parameters (age from infant to
elder, gender, height, weight, muscle, local features). Code: "licensed under the Apache License,
Version 2.0"; the MakeHuman-derived data (`data/mpfb2`) CC0; facial actions from Mika Suominen's
Face Units, CC0; a SOMA topology under Apache-2.0. The **`smplx` topology "can be downloaded for
non-commercial use only"**, and the "full install (non-free dependencies)" may fetch it. Version 0.6,
6 August 2026. It maps to and from SMPL-X and HumGen3D. No textures are described.
*Bearing:* a class 1 body space covering all ages, with the same topology as MPFB's CC0 skins and
clothes; Forge must use the `anny` or `makehuman` topology and never the `smplx` one.

**Meta, "MHR: Momentum Human Rig" (arXiv 2511.15586) and `facebookresearch/MHR`.** [paper] [repo]
[repository read; paper seen as a search record]
<https://github.com/facebookresearch/MHR> · <https://arxiv.org/abs/2511.15586>

"MHR is licensed under the Apache Software License 2.0." 45 identity parameters (20 body, 20 head,
5 hands), 204 pose parameters ("joint angles and scalings"), neural-network pose correctives, **72
sparse, artist-sculpted expression blendshapes following FACS**, and seven LODs (0–6). No separate
data licence is stated, and no date beyond the paper's (November 2025).
*Bearing:* the only open model found with face expressions and a LOD chain built in; its neural
correctives would need baking or a compute pass in Forge.

**NVIDIA, SOMA-X (Hugging Face `nvidia/SOMA-X`, arXiv 2603.16858, March 2026).** [repo] [model page
read; paper seen as a search record]
<https://huggingface.co/nvidia/SOMA-X>

"SOMA is released under the Apache 2.0 License … ready for commercial use." A canonical body (77
joints, 18 056 vertices at mid LOD) and hands (25 joints each) onto which SMPL, SMPL-X, MHR, Anny and
GarmentMeasurements bodies map. The page lists those backends without their terms.
*Bearing:* a converter between body models; each wrapped model keeps its own licence (SMPL and
SMPL-X stay class 4 behind it).

### 2.4 The research body models

**Max Planck, SMPL, SMPL-X and STAR model licences.** [web] [read]
<https://smpl.is.tue.mpg.de/modellicense.html> · <https://smpl-x.is.tue.mpg.de/modellicense.html> ·
<https://star.is.tue.mpg.de/license.html>

All three grant use for "non-commercial scientific research, non-commercial education, or
non-commercial artistic projects"; "any use for commercial purposes is prohibited", including
"incorporation in a commercial product"; commercial licences go through Meshcapade. © 2024 footers,
no effective date.
*Bearing:* class 4. Their topologies must not enter Forge, even through Anny or SOMA.

**Max Planck, FLAME model licence.** [web] [read]
<https://flame.is.tue.mpg.de/modellicense.html>

Two licences: the **FLAME 2023 Open** model is **CC-BY-4.0**, "for any purpose, including
commercial use"; FLAME 2017, 2019, 2020, 2023 (non-Open) and the Blender add-on are academic,
non-commercial.
*Bearing:* a statistical head with expressions in class 1 (with credit), if the Open variant suffices.

**Max Planck, BEDLAM licence.** [web] [read]
<https://bedlam.is.tue.mpg.de/license.html>

Non-commercial research only; the data explicitly includes "3D body and clothing meshes, 2D
textures". *Bearing:* class 4, clothes included.

### 2.5 Commercial character tools

**Reallusion, Content EULA ("Updated August 1st, 2025") and Content Policy ("License Update
(2025.8.1)").** [docs] [read]
<https://www.reallusion.com/Content/EULA/EULA.htm> · <https://www.reallusion.com/license/content.html>

The Standard licence covers "commercial games, AR/VR projects and interactive services" (§2.1(B))
and now exports "to any external software or game engine" (the legacy Export licence is folded in).
Excluded without an Enterprise licence: "online or in-software character generation" (§2.1(C)(2)),
"purchasable in-software content" (§2.1(C)(1)), "machine learning, AI training" (§2.1(C)(5)). "3D
Models must be contained in proprietary formats so that they cannot be opened or imported" (§6.3).
The policy's FAQ: for commercial games "one Standard License only grants outputs for one single
character", and CC Components (ccAvatar, ccSlider, ccHair, ccCloth…) on characters in mass-distributed
games need an **Extended** licence. Character Creator 5 (released 2025): **$299 Standard and $479
Deluxe, perpetual** (Reallusion's product page, read), or **$29 a month / $99 a year** as a
subscription (CG Channel, 12 January 2026, read). ActorCore's motions and characters are
Reallusion content under the same EULA (forum records only).
*Bearing:* class 2 with per-character accounting and no player-facing creator; usable for a few
hero characters, awkward for crowds.

**Daz 3D, "Interactive Licenses" and EULA §3.0.** [web] [docs] [read]
<https://www.daz3d.com/interactive-license-info> · <https://www.daz3d.com/eula>

"An Interactive License is required when using or distributing 3D content from the Daz 3D store" in
games; "the creator of each product chooses whether or not to offer" one; no refunds. §3.0: content
"not available to end users in their native formats", protected "by employing technology, asset
protection, encryption"; no product from which content "can be separately exported, extracted, or
de-compiled"; cloud distribution or later content updates need written consent above $1 M of
revenue; all content excluded from AI systems that generate derivative material. No dates.
*Bearing:* class 2, bought product by product (a search record shows a $35 licence add-on on one
Genesis 9 product); strict protection; not worth it beside MetaHuman.

**HumGen3D, Human Generator Ultimate (Blender add-on).** [web] [read]
<https://superhivemarket.com/products/humgen3d>

"Photorealistic humans" with hairstyles, clothing, expressions and poses; $68 for personal,
portfolio and educational use, **$128 for commercial use**; Blender 3.6–5.2; the licence text sits
behind a "license info" button that could not be read, and the listing shows "GPL" (the add-on code).
*Bearing:* unclassified: game use of its base meshes and textures is not stated where it could be read.

### 2.6 Free rigged characters and animation

**Microsoft, Rocketbox Avatar Library.** [repo] [read]
<https://github.com/microsoft/Microsoft-Rocketbox>

115 rigged characters "released under MIT License" (November 2020); facial blendshapes added in
February 2022 (15 visemes, 48 FACS, 30 for the Vive facial tracker) and ARKit-compatible ones in June
2022; 417 animations (April 2022); meshes "relatively low-poly" in hipoly to ultralowpoly levels;
the repository is archived, read-only.
*Bearing:* class 1 people with ARKit faces: the best free placeholder crowd for public demos, a
decade behind AAA in look.

**Quaternius, Universal Base Characters (August 2025) and Universal Animation Library (March
2025).** [web] [read]
<https://quaternius.com/packs/universalbasecharacters.html> ·
<https://quaternius.com/packs/universalanimationlibrary.html>

Six base bodies (superhero, regular and teen proportions, male and female), 20 hairstyles, about
13 000 triangles, a retargetable humanoid rig; over 120 animations on the same rig. Both pages state
CC0; the free Standard tier is "60-70%" of each pack, the rest and the `.blend` sources through
Patreon keys ($10, $20 or $50 a month). See §4 for the site's August 2026 licence.

**Adobe, Mixamo FAQ.** [web] [search records only; helpx.adobe.com returned 403]
<https://helpx.adobe.com/creative-cloud/faq/mixamo-faq.html>

Characters and animations are royalty-free for personal, commercial and non-profit projects,
video games included; the raw character and animation files may not be distributed (asset packs,
templates); training machine-learning models is the excluded research use; an Adobe ID is required.
*Bearing:* class 2 animation, not for the public repository; Quaternius's and Rocketbox's libraries
are the class 1 alternatives.

### 2.7 Clothing

**MakeHuman asset packs** (above): CC0 and CC-BY garments fitted to the MakeHuman body as proxies,
so they follow its shape targets; basic in cut and texture.

**Marvelous Designer, "License Changes", 4 November 2020.** [web] [read; the current pricing page
failed to render and the support article returned 403]
<https://www.marvelousdesigner.com/support/news/view/ed03ffe524f444bf80d86d28bdab32c1>

Personal licence $39 a month (40 % off when prepaid for a year), Enterprise $1,700 a year standalone;
the post says to check current prices; it does not say whether Personal allows commercial work.
*Bearing:* the industry's garment tool (sewing patterns simulated onto an avatar); the ownership of
its outputs could not be confirmed (Checked and left out).

**Korosteleva et al., GarmentCode and GarmentCodeData (ETH Zürich, ECCV 2024).** [repo] [web]
[read]
<https://github.com/maria-korosteleva/GarmentCode> · <https://igl.ethz.ch/projects/GarmentCodeData/>

GarmentCode is a programmatic sewing-pattern system labelled "MIT license"; GarmentCodeData is
115 000 garments with patterns on bodies "sampled from a custom statistical body model based on
CAESAR"; no dataset licence was stated on the pages read (the ETH record returned an error).
*Bearing:* the code is a class 1 seed for parametric garments; the dataset is unclassified.

---

## 3. What Forge would need to generate its own humans

A MetaHuman-like generator is six systems, each with open seeds (licence in brackets):

| Part | What it must do | Open seeds |
|---|---|---|
| **Body** | one topology, blendshapes over age, sex, build, height; proportions valid for children to elders | Anny (Apache-2.0, CC0 data, 564 shapes); MPFB targets (CC0); MHR's 45 identity parameters (Apache-2.0); SOMA-X to compare bodies (Apache-2.0) |
| **Skeleton and skinning** | a game rig with corrective shapes, retargetable animation | MakeHuman's 163-bone rig and Anny's 104-bone default (CC0 data, Apache code); MPFB's Rigify option; MHR's correctives (Apache-2.0); Quaternius UAL (CC0), Rocketbox (MIT) animations; Forge's `animation.md` §1 retargeting |
| **Face** | expression blendshapes on a standard control set, visemes for speech, eyes and teeth | Apple ARKit's **52** blend shape locations as the control set (Apple docs, read via its JSON); MPFB Faceunits 01 (ARKit style, CC0) and Visemes 01–02 (CC0); MHR's 72 FACS-based shapes (Apache-2.0); FLAME 2023 Open (CC-BY-4.0); the DNA format and RigLogic (MIT) as the runtime rig |
| **Skin, eyes, hair shading** | subsurface skin with pores and wrinkle maps, eye refraction and wetness, anisotropic hair | MPFB skins 01–03 (CC0) and its procedural skin and eye materials (GPL code, outputs free); the shading itself is engine work |
| **Hair** | cards for crowds and LODs, strands for close-ups | MPFB hair 01 (CC0), 02–03 (CC-BY); TressFX strands (MIT, `animation.md` §6); a hair-card baker is missing |
| **Clothing** | garments that fit any body, layer, and simulate where loose | MPFB clothes (CC0/CC-BY) as proxies; GarmentCode (MIT) for parametric patterns; Jolt soft bodies (`animation.md` §6) |
| **Crowd LODs** | a few thousand people on screen | MHR's seven LODs show the target; Forge's cluster-DAG near, impostors or baked vertex animation far |

**Verdict.** The body, the rig, the face control set and the face runtime are available as class 1
pieces; a generator built from Anny or MPFB with ARKit-named shapes and RigLogic-style evaluation
would produce class 1 people that could ship in the public engine. The hard parts are not the body:
they are skin shading at close range, hair (cards that hold up and a baker to make them), and a
wardrobe large enough for variety. MetaHuman already solves those for a local sandbox at no cost
below $1 M of revenue, and the runtime pieces Forge needs to show MetaHumans (DNA loading, RigLogic,
skin, eye and hair shading, LODs) are the same pieces a generator would need. Building that runtime
first loses nothing if a generator comes later.

---

## 4. Building kits and architecture

**Quaternius, licence page ("Quaternius Asset License (QAL) v1.0", last updated 28 August 2026)
and pack pages.** [web] [read]
<https://quaternius.com/license.html> · <https://quaternius.com/packs/downtowncitymegakit.html> ·
<https://quaternius.com/packs/modularscifimegakit.html>

The QAL summary: "You can use these assets, free of charge, in personal, educational, and commercial
games … with no credit required. You just can't resell or redistribute the assets themselves as
assets"; §3(a) forbids to "extract, repackage, sublicense, sell, or otherwise redistribute" them,
modified or not; §7: "the version in effect at the time you obtained the Assets governs". The page
never mentions CC0. The pack pages still say CC0: the **Downtown City MegaKit** (May 2026; 315
models plus 7 example buildings, a Boston/New York style, shared "optimized texture sets"; the paid
Source adds a fake-interior window shader and vertex-colour wear) and the **Modular Sci-Fi MegaKit**
(September 2024; 277 grid-based pieces). The site also lists Modular Streets, Buildings, Medieval
Village and Ruins kits, Cars, Public Transport, Ships and Ultimate Spaceships packs, Ultimate House
Interior and Furniture packs, and nature packs. Older packs are mirrored on OpenGameArt as CC0
(search records, e.g. the Ultimate Space Kit, the Modular Sci-Fi MegaKit's Standard version). No
announcement of the change was found.
*Bearing:* copies already received under CC0 stay CC0 (CC0 cannot be withdrawn); a download made
today is ambiguous. Until the licence file inside a fresh download says CC0, treat Quaternius as
class 2 for the public repository. Its look is stylised and simplified, consistent across packs.

**Kenney, support page and kit pages.** [web] [read]
<https://kenney.nl/support> · <https://kenney.nl/assets/city-kit-commercial> ·
<https://kenney.nl/assets/car-kit> · <https://kenney.nl/assets/furniture-kit>

"All game assets on the asset pages are public domain licensed (CC0)… even in commercial projects";
attribution optional. 3D kits include City Kit (Commercial, 50 files, v2.1; Roads; Industrial), Car
Kit (45 files, v3.1), Furniture Kit (140 files, 2018), Modular Space Kit, Factory Kit, Fantasy Town.
*Bearing:* class 1, one coherent low-poly look (an assessment, the pages give counts not style);
right for tests and blockouts, far from release realism.

**Kay Lousberg, KayKit.** [web] [read; the itch.io page returned 403, the Extra prices are search
records]
<https://kaylousberg.com/game-assets> · <https://kaylousberg.com/game-assets/city-builder-bits>

City Builder Bits: "Free for personal and commercial use, no attribution required. (CC0 Licensed)",
32+ low-poly models on one 1024² gradient atlas; Extra (from $3.95) and Source (from $5.95) tiers on
itch.io. Other kits: Furniture Bits, Restaurant Bits, Space Base Bits, Forest Nature.
*Bearing:* class 1, the most uniform style of all (one atlas), the least realistic.

**Poly Haven, licence and model categories.** [web] [read; counts from the public API]
<https://polyhaven.com/license> · <https://api.polyhaven.com/categories/models>

"Our assets are all licensed as CC0" (HDRIs, textures, models), shareable and includable in products
sold; the site's own renders, logos and text are not CC0. On 2026-10-10 the API lists **521 models**:
176 props, 110 nature, 97 industrial, 85 furniture, 57 plants, 37 rocks, 32 ground cover, 29
lighting, 26 structures, 20 trees, 13 buildings, 12 office, 4 ships, 4 grass, 15 rigged, and themed
collections (project_lighthouse 112, the_shed 69, hidden_alley 39, smugglers_cove 33, namaqualand 30,
verdant_trail 25, pine_forest 16).
*Bearing:* class 1 at scan quality, one world across textures, HDRIs and models (same capture and
shading); the library is small, and buildings are a handful.

**ambientCG, licence and API.** [web] [read]
<https://docs.ambientcg.com/license/> · <https://ambientcg.com/api/v2/full_json?q=facade&limit=100>

"All ambientCG assets are provided under the Creative Commons CC0 1.0 Universal License"; "you can
include the raw files in your project, for example a video game". 2 902 assets; a search for
"facade" returns 31, procedural PBR facade materials with displacement (Facade001–020).
*Bearing:* class 1; the facade materials are for distant buildings and proxies (D-039), not modules.

**Sketchfab: help centre on licences, Download API guidelines, the October 2024 Fab update, and its
sale to KitBash (10 August 2026).** [web] [read; the sale also as Digital Production, 12 August 2026,
read; Epic's own post returned 403]
<https://help.sketchfab.com/en/articles/16152217> ·
<https://sketchfab.com/developers/download-api/guidelines> ·
<https://sketchfab.com/blogs/community/sketchfab-update-what-you-need-to-know-now-that-fabs-live/> ·
<https://digitalproduction.com/2026/08/12/kitbash-buys-artstation-and-sketchfab/>

Downloadable models are under a Standard licence or a Creative Commons one (CC BY, BY-NC, BY-ND,
BY-SA, CC0); "the license you received the model under will always prevail" if the author changes it
later. "The Creative Commons license and attribution must follow the asset everywhere it is used."
The paid store moved to Fab in 2024 and free models stayed, with "plenty of notice before free
downloadable content is no longer supported"; Epic sold Sketchfab and ArtStation to KitBash in
August 2026 with no announced change for users (the licences page now carries a KitBash footer).
*Bearing:* CC0 and CC-BY models are class 1, BY-NC class 4, BY-ND unusable for modification, BY-SA
only if Forge accepts share-alike on the asset. Quality varies model by model; judge each by: the
uploader being the author (ripped game assets are uploaded under CC licences), real-world scale,
full PBR maps at a stated resolution, a triangle count suited to the view distance, clean UVs, and
the same texel density and wear as the rest of the set (advice, not a source). The platform's future
under its new owner is a risk: keep the licence and the page's URL with each file.

**Blendkit (formerly BlenderKit), licences, FAQ and pricing.** [web] [read]
<https://www.blendkit.com/docs/licenses/> · <https://www.blendkit.com/docs/licenses/licensing-faq/> ·
<https://www.blendkit.com/plans/pricing/>

Two licences: CC0, and Royalty Free, which "allows commercial use without mentioning the author" but
"doesn't allow for re-sale of the asset in the same form". Games may be sold "if these can't be
extracted by the users in an easy way". Free plan 23 626 models; Full Plan ($9 a month or $90 a year,
the labels are unclear) 82 444 models; Business $29 a month (≤ 5 employees, ≤ $100 k).
*Bearing:* CC0 items class 1, Royalty Free items class 2; quality and style vary per author.

**Fab, "Licenses and pricing in Fab" and the Fab EULA (last updated 1 October 2024).** [docs] [the
documentation read; the EULA read through a reader proxy, direct fetch 403]
<https://dev.epicgames.com/documentation/en-us/fab/licenses-and-pricing-in-fab> ·
<https://www.fab.com/eula>

Fab sells under **Creative Commons Attribution** (free) or **Standard** (free or paid); the legacy
"UE Marketplace License" is being phased out. Standard has a Personal tier (buyers below $100 000 of
gross revenue in the last 12 months, plus a "Personal - Reference Only" variant for UEFN content)
and a Professional tier, with the same rights. EULA: "you may not Distribute Content on a standalone
basis to third parties except to your collaborators" (§5(a), shared only in "a private online
repository"); "you must restrict end users from extracting or otherwise using Content outside of the
Project" (§4(c)); Standard content may not be combined with open-source-licensed code or content in a
way that would license it (§6(a)); NoAI-tagged content stays out of generative-AI datasets (§16(l),
§6(a)(vii)). CG Channel (6 May 2025, read) quotes the Standard licence as permitting use in "any
engine or tool".
*Bearing:* Standard content is class 2 in any engine; Fab's CC-BY items are class 1.

**Epic, City Sample on Fab.** [web] [the listing read through a reader proxy; the pack labels are a
search record]
<https://www.fab.com/listings/4898e707-7855-404b-af0e-a505ee690e68>

The City Sample (The Matrix Awakens' city) is free under Standard; its extracted packs (City Sample
Buildings, about 2 000 modules; Vehicles; Crowd) are labelled "UE-Only Content - Licensed for Use
Only with Unreal Engine-based Products" in the search records.
*Bearing:* class 3: the best free city kit in existence is the one Forge cannot use.

**Synty Studios, licences overview and One Time Purchase licence ("as at 9 July 2026").** [web] [read]
<https://syntystore.com/pages/licences-overview> ·
<https://syntystore.com/pages/one-time-purchase-licence>

"The licence is worldwide, and is not limited by game engine, OS, platform or device"; five seats
per purchase; source files never shared outside the team, nor uploaded to 3D-generation services; no
"Game Creation Software" or metaverse content; no generative-AI datasets or generation. Subscriptions
license assets "while your subscription plan is active". POLYGON Sci-Fi City: $49.99 on the Unity
Asset Store (search record).
*Bearing:* class 2, a very consistent low-poly stylised world, a look a realistic showcase would not
use.

**KitBash3D, licences and the Utopia kit.** [web] [read; the EULA page returned 404]
<https://kitbash3d.com/pages/licenses> · <https://kitbash3d.com/products/utopia>

Tiers: Individual (solo, no employees), Small Business (≤ 50 employees), Enterprise. Commercial use
covers "Films, games, advertising, visualization"; "You can't share, resell, sublicense, or
distribute our assets"; "Assets may not be used for training AI". Perpetual kits or a Cargo
subscription (no new projects after it lapses). **Utopia**: 31 models (city hall, museum of art,
sports arena, observatory, botanical garden), 1.6 M polygons, 7 PBR materials, "neo-futuristic
cities" after Zaha Hadid and Frank Gehry, FBX and USD, **$145** listed. Unlimited Individual is
$995 a year (reseller, search record). No protection clause for games appeared on the page read.
*Bearing:* class 2 landmark buildings at film density; a cluster-DAG renderer takes the polygons,
but there are no interiors and no modules on a grid.

**TurboSquid Royalty Free License (effective 3 February 2020) and CGTrader terms.** [docs] [read]
<https://blog.turbosquid.com/royalty-free-license/> ·
<https://www.cgtrader.com/pages/terms-and-conditions>

TurboSquid allows "Console, PC, web, mobile games; Virtual and augmented reality; Game mods" if the
model "is contained in an interactive experience … and not made available outside of" it and in
"proprietary formats so that they cannot be opened or imported"; "Giving away the 3D model file
itself is not allowed"; no machine learning or generative use without authorisation; "Editorial"
models may not be used in a video game; a purchase for an employer covers the company. CGTrader
§21A: allowed in a game "if the Product is contained inside a proprietary format", with
"commercially reasonable measures to prevent the end user from gaining access"; resale or
redistribution forbidden; the standard licence allows ML training input, the "No AI" variant does
not; editorial models for news only.
*Bearing:* class 2 per model; useful for single hero assets, rarely for a consistent set.

**Modular European kits and trim sheets.** [web] [search records only]
Searches found a stylised Central European kit (Cratial 3D, Superhive/CGTrader, 35+ pieces, royalty
free), classical facade modules on CGTrader, a Victorian kitbash with interior on TurboSquid, and a
"Nordic Harbour" modular kit on Fab with fake-interior window cubemaps; none was read, none is a
consistent modern European set with real interiors. Free trim sheets found were small (a 1K CC0 set
on OpenGameArt, a free sci-fi trim on Gumroad without a named licence).
*Bearing:* no source sells the building kit D-039 needs; its modules have to be made, with CC0
materials and procedural trims (`civilisations-styles.md` §4).

---

## 5. Props, furniture and interiors

The sources are those of §4; for interiors they give:
- **Poly Haven** (class 1, realistic): 85 furniture, 34 seating, 27 tables, 16 shelves, 12 office, 22
  appliances, 13 dishes, 19 food, 29 lighting, 11 wall decoration, 11 vases; items tagged for bars,
  offices, a barber's chair and workshop carts, few kitchen or shop fittings (API, read).
- **Kenney Furniture Kit** (140 files) and **KayKit Furniture Bits and Restaurant Bits** (class 1,
  low-poly).
- **Quaternius Ultimate House Interior, Ultimate Furniture, Furniture Pack** (stylised; class as §4).
- **BlendKit** (CC0 class 1, Royalty Free class 2; tens of thousands of items, mixed authors).
- **Fab** interior packs (class 2), many shipped as Unreal projects; Standard allows exporting them
  to other engines, at the cost of the conversion.

No free interior set covers homes, shops, spas, malls, museums, restaurants, bars, lofts and clubs at
one quality. The consistent way to get there is the one `civilisations-styles.md` §3 describes:
generated furniture and fittings (ShapeAssembly-style programs) over a CC0 material set, with
scanned hero props from Poly Haven placed by rule.

---

## 6. Vegetation

**Poly Haven plants** (class 1): 20 trees (fir, pine, jacaranda, quiver trees, island trees, dead
trunks), 57 plants, 9 flowers, 32 ground-cover items, 4 grass models, 6 potted plants, 37 rocks (API,
read). Scan quality, a narrow choice of species.

**Quaternius, Stylized Nature MegaKit (July 2024).** [web] [read]
<https://quaternius.com/packs/stylizednaturemegakit.html>
116 models (40 trees with seven swappable leaf sets, 35 plants and flowers, 27 rocks, grass, bushes),
"stylized … Ghibli-inspired", CC0 on the page (§4 for the site licence). Class 1 or 2, stylised.

**SpeedTree: store (2026) and Unity's "Professional Artistry Additional Terms" (last updated 10
September 2026).** [web] [docs] [store read through a reader proxy; terms read; Unity's SpeedTree
support article, updated 8 September 2026, read]
<https://store.speedtree.com/> · <https://unity.com/legal/professional-artistry-additional-terms> ·
<https://support.unity.com/hc/en-us/articles/15723241438228-What-is-the-difference-between-SpeedTree-Indie-Pro-Library-and-Enterprise>

**Indie $19 a month** (under $100 k), **Pro $899 a year** (under $1 M), Library add-on $999 a year,
Enterprise custom (over $1 M; the only plan with the Runtime SDK). Exports go to "the real-time engine
or DCC of your choice", so the runtime is not needed for exported meshes. The terms grant Created
Assets a "perpetual, royalty-free … worldwide license" only "as an embedded component of the Licensed
Production" which they may not be "a substantial portion" of; but **"Upon termination, your rights
to the Created Assets and Library Assets, shall cease"**, and productions not public before
termination may not be released (§2.7). The support article does not name these terms; that they
govern SpeedTree is this note's reading.
*Bearing:* class 2 for as long as the subscription runs: a local sandbox with no release date pays
every month or loses its trees. (`vegetation-materials.md` §1 recommended a seat; this clause was not
known then.)

**The Grove (Wybren van Keulen): buy page and licence.** [web] [read]
<https://www.thegrove3d.com/buy/> · <https://www.thegrove3d.com/license/>

Blender and Houdini add-ons growing trees by simulation; **Starter €99** (3 twigs), **Indie €199** (8
twigs, wind and growth animation, Houdini Indie), **Studio €799** (site licence); a "lifetime
license", upgrades at 30 % of the price. Trees may be distributed "in games or realtime
presentations" provided "the trees can not be extracted" and "are not the main subject"; "You may not
sell or distribute … 3D models of trees grown with The Grove", even with your own textures.
*Bearing:* class 2 with no time limit: the safer paid tree tool for a project without a release
date. Wind via shape keys or bones; no LOD or impostor export mentioned.

**Blender, "Sapling Tree Gen" 0.3.7 (25 December 2025) and Blender's licence page.** [web] [read]
<https://extensions.blender.org/add-ons/sapling-tree-gen/> · <https://www.blender.org/about/license/>

Sapling is GPL-3.0-or-later (Blender 4.4+, no longer officially supported, a 0.4.0 for 5.2 reported
by reviewers); Blender: "What you create with Blender is your sole property." Mtree is in
`vegetation-materials.md` §1.
*Bearing:* trees made with free generators are the maker's own (class 1 if the bark and leaf textures
are CC0); quality depends on the leaf and bark scans, which Poly Haven and ambientCG supply.

**Megascans plants outside Unreal.** Fab's Standard licence covers any engine (§4); Quixel's 2024
announcement said content claimed while free is "yours to keep forever" (80.lv, 18 September 2024,
read); prices are per listing, and a search record quotes single assets from $0.99 and packs from
$24.99 (not confirmed on a page). A 261-asset Megascans sample (derelict corridor) is "free
indefinitely" (CG Channel, 6 May 2025, read). *Bearing:* class 2; whether the owner's account claimed
the library in 2024 decides whether it is already paid for (`vegetation-materials.md` §1 asked the
same).

**Grass and ground cover** come from three places: Poly Haven's 4 grass models and 32 ground-cover
items (class 1), Megascans (class 2), and Forge's own GPU grass (`vegetation-materials.md` §3), which
needs only blade textures.

---

## 7. Vehicles

- **Free:** Kenney Car Kit (CC0, 45 files), Kenney Modular Space Kit, Quaternius Cars, Public
  Transport, Ships and Ultimate Spaceships packs (§4's licence question), Poly Haven's 4 ships (CC0),
  KayKit Space Base Bits. All low-poly or stylised; no realistic futuristic car, boat or flying craft
  in one style was found under CC0 or CC-BY.
- **Paid, single models:** search records show hover cars at $79–$129 on TurboSquid (glTF included)
  and packs on CGTrader and Renderhub, mostly cyberpunk in style, each from a different artist.
  Class 2 each; a set built from them would not look like one world.
- **Unreal's City Sample Vehicles** (class 3).

A consistent fleet is therefore something to make: a vehicle is a few hard-surface families (body
shell, canopy, propulsion, lights) with shared materials and decals, which suits the same
module-and-grammar approach as D-039 and ShapeAssembly (`civilisations-styles.md` §3).

---

## 8. PBR textures and materials

- **Poly Haven, ambientCG** (CC0, class 1; above and `vegetation-materials.md` §5).
- **cgbookcase** [web] [home page read; licence page 404; CC0 from search records]
  <https://www.cgbookcase.com/>: "100% free, no restrictions"; CC0 per its 2018–2019 announcements
  and 2026 guides. Class 1 once the licence is confirmed on the download.
- **Textures.com, Terms of Use** [docs] [read through a reader proxy; the licence page 404]
  <https://www.textures.com/about/terms-of-use>: content may be incorporated "in computer games and
  3D models"; not sold or distributed "by themselves or in a texture pack, material, shader"; **not
  released "under Open Source Licences"** (Art. 6(3)(xi)); no "deep learning, machine learning,
  algorithm training"; free downloads under an Indie licence "with an annual revenue or funding limit
  of $150k USD", premium by credits. Class 2.
- **Adobe Substance 3D Assets, Additional Terms (effective 22 March 2022) and FAQ** [docs] [read; the
  PDF through the fetch tool's saved copy]
  <https://wwwimages2.adobe.com/content/dam/cc/en/legal/servicetou/Adobe-Substance-3D-Assets-Addl-Terms-en_US-20220322.pdf> ·
  <https://www.adobe.com/cc-shared/fragments/products/substance3d/assets-faq>: a perpetual licence to
  distribute assets "only as modified into a Modified Work or as incorporated into a Larger Work",
  never "on a stand-alone basis" (§2.2, §3.1), only where the work's "primary value" is not the
  asset (§3.2); no AI or ML use (§3.3); assets downloaded and paid for stay usable after the
  subscription ends (§4). The FAQ allows use "in Substance 3D apps and other 3D design programs" and
  in video games. Plans page (read): Collection US$59.99 a month for individuals, $119.99 per team
  licence; a Texturing plan at $24.99 a month appears only in search records and not on that page.
  Class 2 for the assets; materials Forge authors in Designer are its own.

---

## 9. All sources at a glance

Quality and consistency are this note's assessments; licence and cost are from the pages above.

| Source | Quality | Style consistency | Class | Cost (2026-10-10) | Other duties |
|---|---|---|---|---|---|
| Poly Haven | scans, high | one world (one capture pipeline) | 1 | free | none |
| ambientCG | scans and procedural, high | consistent materials | 1 | free | none |
| cgbookcase | scans, good | consistent | 1 (to confirm) | free | none |
| Kenney | low-poly | one look across kits | 1 | free | none |
| KayKit | low-poly, one atlas | very consistent | 1 | free; Extra $3.95+, Source $5.95+ | none |
| Quaternius | stylised, textured | consistent across packs | 1 for CC0 copies; 2 for downloads under QAL | free Standard; Source via Patreon $10–$50/month | QAL: no redistribution |
| MPFB / MakeHuman assets | generic game humans | one body, one topology | 1 (CC-BY packs credited) | free | CC-BY credits |
| Anny | body model, no textures | MakeHuman topology | 1 (not its `smplx` topology) | free | Apache NOTICE |
| MHR | body and face model, LODs | own topology | 1 | free | Apache NOTICE |
| FLAME 2023 Open | head model | own topology | 1 | free | CC-BY credit |
| Rocketbox | dated, low-poly, ARKit faces | one library | 1 | free | MIT notice |
| MetaHuman | AAA | one system | 2 | free below $1 M; seats ($1,850/yr) above | UE EULA, no AI, no UGC |
| OpenRigLogic | runtime code | — | 1 (code) | free | MIT notice |
| Reallusion CC5 / ActorCore | high | one system | 2 | $299 ($479 Deluxe) or $99/yr, plus Extended per component | one character per Standard licence; no in-game creator; protected formats; no AI |
| Daz 3D | high | per vendor | 2 (if offered) | per product | encryption, no AI |
| HumGen3D | realistic | one system | unclassified | $128 commercial | licence not readable |
| Mixamo | mid | one rig | 2 | free (Adobe ID) | no raw files, no ML |
| Sketchfab CC0 / CC-BY | varies | none | 1 | free | CC-BY credit follows the asset |
| BlendKit | varies | none | 1 (CC0) / 2 (RF) | free; Full $90/yr; Business $29/mo | not easily extractable |
| Fab Standard / Megascans | high (Megascans scans) | Megascans consistent | 2 | per listing | no open-source combination; extraction restricted |
| Fab CC-BY items | varies | none | 1 | free | credit |
| City Sample packs | AAA | one city | 3 | free | Unreal only |
| Synty POLYGON | low-poly | very consistent | 2 | e.g. $49.99 a pack | 5 seats, no AI, no game-creation tools |
| KitBash3D | film quality | consistent per kit | 2 | Utopia $145; $995/yr unlimited | no AI; per tier |
| TurboSquid / CGTrader | varies | none across artists | 2 | per model ($79–$129 hover cars seen) | protected formats; editorial excluded |
| Textures.com | scans, high | consistent | 2 | free (Indie, $150 k cap) or credits | no open-source licences, no ML |
| Substance 3D Assets | high | consistent | 2 | Collection $59.99/month | no AI; perpetual after download |
| SpeedTree outputs | high | per artist | 2 while subscribed | $19/mo Indie | rights end with the subscription |
| The Grove outputs | high | per artist | 2 | €99/€199/€799 lifetime | not extractable, not the main subject |
| Sapling / Mtree outputs | depends on textures | per artist | 1 | free | none |
| SMPL / SMPL-X / STAR / BEDLAM | research | — | 4 | commercial via Meshcapade | — |

---

## 10. Paid shortlist

Only well-known tools and packs that beat the free options for a realistic modern or futuristic
city; prices as listed when read on 2026-10-10. Nothing has been bought or downloaded.

| # | Name | Link | Price | Class | What it would allow |
|---|---|---|---|---|---|
| 0 | MetaHuman (free, for comparison) | <https://www.metahuman.com/en-US/license> | free below $1 M revenue | 2 | AAA people, faces with RigLogic (MIT runtime), authored in Unreal |
| 1 | The Grove, Indie | <https://www.thegrove3d.com/buy/> | €199 lifetime | 2 | grown trees of chosen species with wind, kept forever, in a released game if not extractable |
| 2 | SpeedTree, Indie | <https://store.speedtree.com/> | $19/month (Pro $899/yr) | 2 while subscribed | the industry's tree modeller and LOD pipeline; rights end on cancellation |
| 3 | KitBash3D, Utopia | <https://kitbash3d.com/products/utopia> | $145 | 2 | 31 neo-futuristic landmark buildings (civic, museum, arena, observatory, botanical garden) |
| 4 | Adobe Substance 3D Collection | <https://www.adobe.com/products/substance3d/plans.html> | $59.99/month (individuals) | tools: own output; assets 2 | Designer for trims and facade materials; the asset library under §8's terms |
| 5 | Reallusion Character Creator 5 | <https://www.reallusion.com/character-creator/> | $299 Standard, $479 Deluxe (perpetual); $99/yr subscription (CG Channel), plus Extended licences | 2 | hero characters with a clothing ecosystem, one character per Standard licence |
| 6 | Fab Megascans (per listing) | <https://dev.epicgames.com/documentation/en-us/fab/licenses-and-pricing-in-fab> | from about $0.99 an asset (search record) | 2 | scanned rocks, plants, surfaces in any engine |
| 7 | Marvelous Designer, Personal | <https://www.marvelousdesigner.com/support/news/view/ed03ffe524f444bf80d86d28bdab32c1> | $39/month (2020 post; current price not read) | tool | sewing-pattern garments for a wardrobe; output terms to confirm |

Considered and not shortlisted: Synty (consistent but low-poly), Daz (per-product licences and
encryption duties), Human Generator (licence text not readable), TurboSquid and CGTrader single
vehicles (no common style), BlendKit Full (mixed quality, mostly Royalty Free), Textures.com (the
CC0 libraries cover it, and it forbids open-source licences).

---

## Recommendation for Forge

**Verdict.** For a public engine with a local, unpublished showcase, the asset question splits in
three, and only the first touches the repository:
1. **The public repository** carries only class 1 assets, and only through D-048's fetch (never
   committed): Poly Haven, ambientCG, Kenney, KayKit, MPFB's CC0 and CC-BY packs, Anny's free
   install, Rocketbox, FLAME 2023 Open, Quaternius copies whose licence file says CC0, Sketchfab and
   Fab CC0 or CC-BY models. Code under MIT or Apache (OpenRigLogic, GarmentCode, Anny's code if used)
   may be vendored with its notice in `CREDITS.md`. D-048's rule that external models serve the labs
   only stays until the owner widens it; the engine's demos keep their placeholders.
2. **The local sandbox** may use class 2: MetaHuman, Fab Standard and Megascans, KitBash3D, Substance
   assets, trees from The Grove or SpeedTree, Mixamo, TurboSquid or CGTrader models. They live in a
   library outside the repository with its own local manifest (source URL, licence name and version
   as read, class, tier or seat, date, conditions such as "subscription must stay active",
   "no AI", "protect in release", credit text), and no tracked file names its path; `credits --check`
   can enforce that the same way it keeps `assets/external/` out of shipped code.
3. **A release** ships class 1 and class 2 cooked into packed archives that are not plain glTF or PNG
   (obfuscated or encrypted: TurboSquid, CGTrader, Reallusion, Daz, Fab and BlendKit all ask for it),
   under the game's own data licence, separate from the engine's open-source licence (Fab §6(a),
   Textures.com 6(3)(xi)), with every CC-BY credit, an Unreal seat check if revenue passes $1 M, the
   SpeedTree subscription still active or the trees regrown, and Reallusion or Daz licences counted per
   character and per product. Class 3 and 4 never enter.

**Machine learning.** Any learned system in Forge trains on class 1 or Forge-made data only; most
class 2 licences forbid the rest (§1).

**A first consistent set to try, free first:**
- **Surfaces and lighting:** Poly Haven and ambientCG (textures, HDRIs, facade materials).
- **Props, furniture, plants and rocks:** Poly Haven's 521 models, which share the textures' capture
  and look.
- **People:** MetaHuman in the sandbox; Rocketbox (or MPFB characters) where a class 1 stand-in is
  needed.
- **Buildings:** D-039's modules made for Forge, textured with the CC0 materials; Quaternius's
  Downtown City and Sci-Fi MegaKits only as blockout.
- **Trees:** Forge's own growth (`vegetation-materials.md` §1) or Sapling, with CC0 bark and leaves,
  beside Poly Haven's 20 trees.
- **Vehicles:** placeholders (Kenney, Quaternius) until a family is made.
The low-poly CC0 kits (Kenney, KayKit, Quaternius) form their own consistent world and must not be
mixed with the scanned set: one look per scene.

**Where free falls short of release quality:** (1) a realistic modular building kit with interiors
on a grid (none exists free; the best one is Unreal-only); (2) interiors at one quality across many
uses (Poly Haven has 85 furniture items and little for shops, kitchens or clubs); (3) vehicles in one
futuristic style; (4) trees of chosen species at hero quality; (5) clothing variety for crowds;
(6) signage and decals in one design language.

**Order to fill the gaps:**
1. **People: use, then build the runtime.** MetaHuman authoring in Unreal (free), exported as DNA,
   textures and meshes; Forge loads DNA and evaluates RigLogic (MIT), and gains skin, eye and hair
   shading and crowd LODs. This is also the generator's runtime (§3); the generator itself waits
   until the sandbox shows a need (variety, children, a player-facing creator, or people in the
   public demos).
2. **Buildings and interiors: make.** D-039's module generators with CC0 materials and procedural
   trims, interiors by furniture programs over the same materials; buy KitBash3D's Utopia only if
   hand-modelled landmarks are wanted soon.
3. **Trees: buy one tool.** The Grove Indie (lifetime) before SpeedTree, whose rights end with the
   subscription; or Forge's own grower if the vegetation work reaches it first.
4. **Vehicles: make,** as part families with shared materials; buy single hero models only for
   reference.
5. **Clothing: make,** with MPFB's CC0 garments and GarmentCode patterns, Marvelous Designer if the
   owner wants hand-made hero outfits.
6. **Textures: free is enough;** Substance Designer only if trims and facades need an artist's tool.

**For a decision later (a D-0xx after the owner's review):**
1. Whether class 2 assets may be used at all, and where the sandbox library lives.
2. Whether D-048's "labs only" rule widens to let public demos fetch class 1 assets.
3. Quaternius: treat as class 2 until its licence is clarified, or rely on CC0 copies only.
4. MetaHuman as the people source for the sandbox, and OpenRigLogic vendored in the engine.
5. The release packaging's protection (format, obfuscation) and the game-data licence.

**What to check at each download** (for the manifests): the licence file inside the archive (not
only the web page), the page's URL and date, the tier chosen and why, the conditions in force, the
credit text, and a SHA-256 of the archive.

---

## Checked and left out

- **Epic's own pages:** metahuman.com redirects to a login (read through a reader proxy, with the
  FAQ's seat, royalty and AI answers blank); unrealengine.com's EULA and Fab's EULA returned 403
  (read through the proxy); Epic's KitBash announcement returned 403 (the sale is from Digital
  Production and search records).
- **MetaHuman grooms and clothing outside Unreal:** how strand grooms and hair cards export, and how
  many outfits Creator offers, were not found on a readable page (Fab outfit listings are search
  records).
- **Quaternius's licence change:** no announcement, changelog or explanation of how the QAL relates to
  the CC0 pack pages was found.
- **Marvelous Designer's terms on outputs and its current price:** the support article returned 403
  and the pricing page failed to render; only the 2020 post was read. CLO was not checked.
- **CLOTH3D's licence:** not found (search records only). **GarmentCodeData's licence:** not stated on
  the pages read; the ETH record returned an error.
- **Renderpeople** (scanned people): no readable licence text on game use; a forum record says games
  needed a separate licence. Not classified.
- **Human Generator's licence text:** behind a button not readable by fetch.
- **ActorCore's own licence page:** the socket closed; its terms are taken from Reallusion's EULA.
- **Mixamo's FAQ:** helpx.adobe.com returned 403 in two languages; its terms are search records.
- **Textures.com's licence page and cgbookcase's licence page:** 404; the terms of use (through the
  proxy) and the cgbookcase home page were read instead.
- **A newer Adobe Substance 3D Assets Additional Terms than March 2022:** not found.
- **Unity's SpeedTree support article** does not name the governing terms; that the Professional
  Artistry Additional Terms apply to SpeedTree is a reading of their scope.
- **Megascans prices on Fab:** listings are per asset and none was read; the $0.99 and $24.99 figures
  are a search record.
- **City Sample Crowd's listing:** not found; the UE-only label is from search records of the City
  Sample and Buildings listings.
- **A free or affordable realistic futuristic vehicle set** and **a modern European modular building
  kit with interiors:** none found (search records of single models and stylised kits only).
- **Free CC0 trim sheets of production size:** none found beyond a 1K set.
- **Poly Haven's model page** is client-rendered; counts come from its public API.

---

## Verification notes

Checked on 2026-10-10 with WebFetch and WebSearch only; no browser pane, nothing downloaded into the
repository or the machine beyond the fetch tool's own cache (the Adobe PDF was read from that cache).
"Read" means the page was returned and the quotes are from it; "read through a reader proxy" means
`r.jina.ai` rendered a page the direct fetch refused or redirected; "search record" means only a
search engine's extract was seen.

- **Read:** Poly Haven licence and API (`/categories/models`, `/assets?c=furniture`, `c=nature`,
  `c=lighting`); ambientCG licence and API; Kenney support, City Kit (Commercial), Car Kit, Furniture
  Kit and the 3D category page; KayKit's game-assets page and City Builder Bits; Quaternius home page,
  licence page, Downtown City MegaKit, Modular Sci-Fi MegaKit, Stylized Nature MegaKit, Universal
  Base Characters, Universal Animation Library; MakeHuman licence, asset packs and MPFB getting
  started; CG Channel on MPFB 2 (11 March 2025), MetaHumans (4 June 2025), Megascans' corridor
  sample (6 May 2025) and Reallusion's licensing (12 January 2026) and CC5 (21 July 2025); NAVER's Anny
  blog and repository; `facebookresearch/MHR`; Hugging Face `nvidia/SOMA-X`; the SMPL, SMPL-X, STAR,
  FLAME and BEDLAM licence pages; Microsoft Rocketbox; Reallusion's Content EULA and Content Policy;
  Daz's Interactive Licenses page and EULA; Superhive's Human Generator page; Marvelous Designer's
  2020 licence post; GarmentCode's repository and project page; Epic's OpenRigLogic repository, the
  MetaHuman Devkit and Export Tool documentation; unrealsource.com on the Devkit; Fab's "Licenses and
  pricing" documentation; Sketchfab's help article, Download API guidelines, October 2024 update and
  licences page; Digital Production on KitBash's purchase (12 August 2026); Blendkit's licences, FAQ
  and pricing; Synty's licences overview and One Time Purchase licence; KitBash3D's licences page
  and Utopia; TurboSquid's Royalty Free License; CGTrader's terms; Reallusion's Character Creator
  page (CC5 prices); Adobe's Substance 3D plans page; Unity's Professional Artistry
  Additional Terms and SpeedTree support article; The Grove's home, buy and licence pages; Blender's
  licence page and the Sapling extension page; cgbookcase's home page; Adobe's Substance assets FAQ
  and the 2022 Additional Terms (PDF); AMD's TressFX repository; Apple's ARKit blend shape list
  (through its documentation JSON); 80.lv on Megascans (18 September 2024).
- **Read through a reader proxy (`r.jina.ai`):** metahuman.com's licence FAQ and MetaHuman 5.8 post;
  the Unreal Engine EULA; the Fab EULA (1 October 2024); the City Sample's Fab listing; SpeedTree's
  store; Textures.com's terms of use.
- **Search record only:** the State of Unreal 2026 date (17 June 2026); MetaHuman 5.7's FBX remark
  and the third-party DNA importer; MPFB's extension version and date; the City Sample packs' UE-only
  label; Megascans' per-asset prices; Mixamo's FAQ; Daz's $35 licence add-on; KayKit's Extra and
  Source prices; KitBash3D's Unlimited price; Synty's POLYGON Sci-Fi City price; Adobe's Texturing
  plan price; the paid hover cars and European kits; cgbookcase's CC0 statements; Quaternius's
  OpenGameArt mirrors; Poly Haven's texture and HDRI counts; Renderpeople and CLOTH3D.
- **Refused or empty:** metahuman.com (login redirect), unrealengine.com EULA (403), fab.com EULA and
  listings (403), epicgames.com news (403), itch.io (403), support.marvelousdesigner.com (403),
  marvelousdesigner.com pricing (client error), helpx.adobe.com Mixamo FAQ (403), textures.com licence
  (404, then only a tagline), cgbookcase licence, FAQ and about (404), kitbash3d.com EULA (404), the
  ETH research-collection record (500), Reallusion's /license/ (404), actorcore.com (socket closed),
  fab.com/cla (not found through the proxy).
- **From Forge, read in the repository:** `CLAUDE.md`, `docs/research/galaxy-sky.md` (format),
  `docs/DECISIONS.md` D-039 and D-048, `docs/research/vegetation-materials.md` (§1, §5, Checked and
  left out), `docs/research/civilisations-styles.md` (§3), `docs/research/animation.md` (§6),
  `assets/external.tsv`, `CREDITS.md`.
- **To re-check before any purchase or decision:** every price; Quaternius's licence on a fresh
  download; MetaHuman's FAQ answers that were blank; whether the owner's Fab account holds the 2024
  Megascans claim; Marvelous Designer's output terms; HumGen3D's licence; Substance's current terms.
