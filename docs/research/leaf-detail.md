# Leaf detail on alpha-tested cards (research, 2026-10-22, #346)

Problem: every leaf is one flat tinted grey, so near crowns read as cut-outs. Below is what games and engines put on leaf cards, and what fits our pipeline (cards, bent normals, RT shadows, GI probes, DLAA).

## 1. Atlas content

- **Usual map set.** SpeedTree: Color, Opacity, tangent-space Normal, Gloss, Specular (default; metallic 0), Subsurface Color + Amount, a mostly white AO [1]. Megascans leaf atlases: albedo, normal, translucency, roughness, opacity, AO, displacement [2]. Crysis: a 128² "subsurface" map in which veins and inner branches show [3].
- **Variation is per leaf, in the albedo.** Scanned atlases carry it naturally. Sucker Punch scanned real Tsushima leaves, "pushed colour values", *reduced texture noise* and raised translucency [4].
- **What reads at card distance.** Holm oak and olive leaves are 3–7 cm [5]. At 1440p with a 70° FOV, one pixel at 10 m is about 1 cm, so a leaf spans 3–7 px. Midribs show only within about 3 m. What reads: per-leaf value/hue jitter, darker bases where leaves overlap, lighter undersides. Veins belong in the translucency map, where backlighting shows them.
- **Undersides (high value for these species).**
  - Holm oak: dark glossy green above, grey/whitish downy below [5].
  - Olive: silvery underneath from dense peltate hairs [6].
  - Aleppo pine: yellowish green. Cypress: dark green scale leaves [7].

  Paint some underside leaves into each spray, and tint card back faces with an underside colour (front-facing flag, like UE's TwoSidedSign).
- **Procedural generation.** Runions et al.'s venation model generates vein textures [8]. At our sizes, a midrib plus 4–8 secondary vein lines (SDFs) are enough. I found no game reference for a fully procedural leaf atlas. Store a per-leaf random ID channel for shader jitter.

## 2. Normals

- **Bent normals are standard.** SpeedTree interpolates smoothed and per-leaf normals, and "Puffiness" sphere-izes them [9]. DreamWorks blended aggregate canopy normals with each leaf's own normal [10].
- **Atlas normal on top.** Noisy leaf normal maps break the round crown shading; practitioners tilt each leaf instead [11]. Combine with the bent normal as the base, using RNM, or UDN for BC5 maps [12].
- **Suggested split.** Diffuse and specular use the combined normal; the GI probe lookup uses the bent normal only.

## 3. Light through leaves

- **Crysis** [3]: back = lerp(saturate(E·−L)⁴, saturate(−N·L·0.6+0.4), viewDep) × back colour × subsurface map × shadow.
- **Frostbite** [13]: `L' = L + N·distortion; t = pow(saturate(V·−L'), power)·scale; lt = atten·(t + ambient)·thickness`, multiplied by albedo and light colour. A few ALU; an `exp2` spherical-gaussian form replaces the `pow`.
- **UE Two Sided Foliage** [14]: Subsurface Color is the transmitted colour and masks stems and veins. Default Lit leaves undersides near black. The engine source combines a wrapped −N·L with a forward lobe around −L; I could not re-check this (login-gated).
- **SpeedTree** [1]: the transmitted colour may be brighter and more saturated than the albedo.
- **Physically based models** (Habel 2007) [15] cost more than games pay.
- **Our case.** Multiply the transmission by the leaf's RT sun shadow, so inner leaves stay dark. The moon uses the same term.

## 4. Specular and per-instance colour

- **Leaf specular comes from the cuticle.** It is spectrally invariant (white) and shaped by roughness [16]. So: dielectric F0 ≈ 0.04, glossy holm oak and olive tops, rough downy undersides. More gloss shows veins [1].
- **Per-instance tint.** Unity's SpeedTree hue variation tints each tree by world position [17]. UE users dropped the position term from SpeedTreeColorVariation for PerInstanceRandom alone, because adding instances shifted colours [18].
- **Season.** The plane tree needs an autumn parameter; the other species are evergreen.

## 5. Stability under DLAA/DLSS

- **Coverage-preserving alpha mips** [19]: we already have this.
- **Mip bias.** FSR's bias is `log2(render/display) − 1` (−1 at DLAA) and warns that high-frequency textures may need their own [20]. The DLSS guide §3.5 has a similar caveat. Give the leaf normal and vein detail a less negative bias.
- **Motion vectors.** Alpha-tested geometry writes them for every covered pixel, including wind [20].
- **Specular aliasing.** Toksvig turns the shortened mip normal into roughness; do not renormalise mips [21]. Geometric specular AA uses normal derivatives (Kaplanyan 2016; Tokuyoshi & Kaplanyan) [22]; it matters where bent normals turn fast across a card.
- **Where the industry is going.** The Witcher 4 demo used no alpha-tested foliage. The talk measured cards 3–4× slower than Nanite leaf geometry, though geometry costs ~63 MB per tree against 6 MB. Nanite voxels store a normal distribution [23]. Not applicable before we have virtualised geometry.

## Patents met

| Patent | Holder | Jurisdiction | Status | Alternative |
|---|---|---|---|---|
| US10417813B2, hashed alpha testing [24] | NVIDIA | US | Active, expiry 2037-11-10 | Plain alpha test + coverage mips (ours); screen-space blue-noise dither for LOD fades (check the claims) |
| US7589723B2, translucent leaves via PRT [25] | Microsoft | US | Expired (fees) | Not needed |
| US7348977B2, thickness-map subsurface [26] | Pixar | US | Expired 2020 | Not needed |

I searched for patents on the translucency terms, bent/aggregate normals, RNM, Toksvig, NDF filtering, coverage mips and hue variation, and found none. This is a search, not a clearance.

## Recommended for this engine (most look per cost first)

1. **Per-leaf jitter and spray AO in the atlas.** A leaf ID channel drives about ±15 % value and a small hue shift; darken leaf bases and overlaps. Costs regenerating the atlas only.
2. **Species underside colour on card back faces**, plus some underside leaves painted into each spray. One select.
3. **Translucency.** Crysis/Frostbite back term × RT sun shadow × an atlas thickness map (thicker veins) × a species transmission colour. A few ALU and one channel.
4. **Per-leaf tilt and a midrib fold in a BC5 normal map**, combined with the bent normal at reduced strength. Probes keep the bent normal.
5. **White dielectric specular with species roughness**, Toksvig mip roughness, geometric specular AA, and a less negative bias on the detail maps.
6. **Per-instance hue/value from the instance ID**, plus the plane-tree season.
7. **Later:** leaf geometry for the nearest trees.

## Sources

1. https://docs.unity3d.com/speedtree-modeler/manual/materials-and-maps.html
2. https://fab.com/listings/6553b1a5-6e02-449d-92a1-ef1852da8ff9
3. https://developer.nvidia.com/gpugems/GPUGems3/gpugems3_ch16.html
4. https://blog.playstation.com/?p=333111
5. https://en.wikipedia.org/wiki/Quercus_ilex
6. https://frontiersin.org/journals/plant-science/articles/10.3389/fpls.2015.00633/full
7. https://en.wikipedia.org/wiki/Pinus_halepensis
8. https://algorithmicbotany.org/papers/venation.sig2005.html
9. https://docs.unity3d.com/speedtree-modeler/manual/leaf-generator-properties.html
10. https://research.dreamworks.com/wp-content/uploads/2018/07/0454-peterson-Edited.pdf
11. https://polycount.com/discussion/227189/src
12. https://blog.selfshadow.com/publications/blending-in-detail/
13. https://colinbarrebrisebois.com/2012/04/09/approximating-translucency-revisited-with-simplified-spherical-gaussian/
14. https://dev.epicgames.com/documentation/en-us/unreal-engine/shading-models-in-unreal-engine
15. https://www.cg.tuwien.ac.at/research/publications/2007/Habel_2007_RTT/
16. https://www.ipgp.fr/~jacquemoud/publications/bousquet2005b.pdf
17. https://docs.unity3d.com/2023.1/Documentation/ScriptReference/SpeedTreeImporter-hueVariation.html
18. https://answers.unrealengine.com/questions/246598/view.html
19. https://ludicon.com/castano/blog/articles/computing-alpha-mipmaps/
20. https://gpuopen.com/manuals/fsr_sdk/techniques/super-resolution-upscaler/
21. https://developer.download.nvidia.com/whitepapers/2006/Mipmapping_Normal_Maps.pdf
22. https://research.nvidia.com/publication/2016-06_filtering-distributions-normals-shading-antialiasing
23. https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage ; https://lilys.ai/notes/zh/unreal-engine-5-20251128/witcher-4-ue5-large-scale-animated-foliage-demo
24. https://patents.google.com/patent/US10417813
25. https://patents.google.com/patent/US20070018996A1/en
26. https://patents.google.com/patent/US7348977

Not found as public text: Guerrilla's HZD vegetation talk (GDC Vault, gated); any RDR2 or TLOU2 leaf-shading write-up.
