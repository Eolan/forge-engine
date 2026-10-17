# Patents near Forge's lighting: details and conditions of application

> Written 2026-10-17 at the owner's request: "try to get more details on those patents. And what
> are the conditions of application of their patents just in case". It follows D-067 and D-068
> and is a companion to `probe-leaks-indoors.md` §4 and `local-exposure.md` §6.
>
> **This is not legal advice.** The note gathers facts so the owner can judge, and so a patent
> attorney can be briefed quickly.
>
> **How facts were checked.** Each fact is marked:
> - *read*: fetched on 2026-10-17;
> - *search-recorded*: seen only through a search engine's record or commentary;
> - *from memory*: not re-read.
>
> **Patent records.** Every claim set, family list and legal-event list below was *read* on Google
> Patents. Its status and expiry fields are its own assumptions, and its national events come from
> INPADOC. Several registers could not be reached:
> - the EPO Register, the UK IPO register and INPI were behind bot checks, which were not bypassed;
> - the USPTO assignment service did not answer.
>
> So renewal states below are Google's INPADOC events, not the registers.
>
> **Forge's facts** come from D-067, D-068, `shaders/probes.slang`, `probe_update.slang` and
> `meshlet.slang`.

**The short answer.**
- **Imagination: probe lighting updated by ray tracing.**
  - Eleven members: seven US (one of them pending), three GB, and one German application. There
    is no EP, French, Japanese or Canadian member.
  - Expiry: 2036-03-03 to 2036-08-26 in the US, 2036-03-03 in the UK.
  - Forge's probes as built meet every element of the independent claims of US 10,008,032,
    10,672,183 and 11,127,198, and GB 2537728 and 2571656, on a plain reading.
  - US 11,127,198 claim 1 does not even require the rays to start at the probe.
  - No assertion of this family was found. In 2025 Imagination was reported to be selling about
    3,600 patents, and every US member carries a security interest recorded in 2024.
- **Square Enix: probe culling by zones.** Members in the US, EP (in force in DE, FR and GB by
  their fees), JP and CA, expiring 2036-10 to 2037-02.
  - Every EP claim, and the main claims elsewhere, need per-probe "occlusion planes".
  - The US and CA patents also have independent claims without planes:
    - US claim 37: zones plus "a grid of regions";
    - US claim 44: a per-probe visibility mask and a "bit-wise AND".
  - JP has one claim with a mask and no planes.
  - Claim 37 is the claim nearest D-068's regions. The earlier note quoted only claim 1. Whether a
    room number shared by all probes counts as "a plurality of zones for the selected probe" is a
    question for counsel.
- **STMicroelectronics: local tone mapping.**
  - US 11,756,172 B2 runs to 2041-06-23. EP 3872750 B1 was granted 2025-06-25; the records show it
    in force in DE, GB, IE and CH, with France not recorded.
  - Every independent claim uses a mini-image of a *first* image to change a *second* image.
  - The US claims add the gain `(LOCAL_GAIN − 1) × GLOBAL_POWER + 1`. The EP claims add a gain
    map that is erased after use.
  - D-067's same-frame grid meets none of these three elements, as read.
- **Conditions of application.**
  - **Where it bites.** A patent counts only where it is in force and where the act happens.
  - **United States.** Players who run the game infringe directly. A publisher who knows of the
    patent can be liable for inducing them.
  - **UK, EPC states and Japan.**
    - Private, non-commercial play is not infringement.
    - The supplier of the means can still be liable: UK s.60(2) with s.60(6), UPC Art. 26(3), and
      Japan Art. 101.
  - **France.** The owner's own work in France is reached only by the French parts of the Square
    Enix and ST EP patents.

---

## 1. Families and status

### Imagination Technologies: "Graphics processing using directional representations of lighting at probe positions within a scene"

- **Inventors:** Jens Fursund and Luke T. Peterson.
- **Priority:** US provisional 62/127,430, 2015-03-03.
- **Owner:** Imagination Technologies Ltd is the assignee of record. A security interest to
  Fortress Investment Group (UK) Ltd was recorded on every US member on 2024-07-31 (*read*).

| Member | Filed | Granted | Expiry (Google) | Fees, notes |
|---|---|---|---|---|
| US 10,008,032 B2 (15/059,940) | 2016-03-03 | 2018-06-26 | 2036-08-26 (adjusted) | 4th-year fee paid 2021-12-15, 8th-year 2025-12-17 |
| US 10,410,408 B2 | 2018-05-23 | 2019-09-10 | 2036-03-03 | 4th paid 2023-03-01; 8th not shown (due 2027-03-10) |
| US 10,672,183 B2 | 2018-05-23 | 2020-06-02 | 2036-03-03 | 4th paid 2023-11-22 |
| US 11,127,198 B2 | 2020-04-27 | 2021-09-21 | 2036-03-03 | 4th paid 2025-01-08; certificate of correction 2022 |
| US 11,734,879 B2 | 2021-08-19 | 2023-08-22 | 2036-04-30 (adjusted) | 4th not shown (due 2027-02-22) |
| US 12,243,156 B2 | 2023-08-21 | 2025-03-04 | 2036-03-03 | first fee due 2028 |
| US 2025/0157141 A1 (19/022,671) | 2025-01-15 | pending | — | "ready for examination" 2025-02-06 |
| GB 2537728 B | 2016-03-03 | 2019-11-27 | 2036-03-03 | register change recorded 2024-08 (content not read) |
| GB 2571656 B (divisional) | 2016-03-03 | 2019-11-27 | 2036-03-03 | same |
| GB 2571657 B (divisional) | 2016-03-03 | 2019-11-27 | 2036-03-03 | same |
| DE 10 2016 103 854 A1 | 2016-03-03 | pending | — | examination requested 2021-10-12 |

- GB 2537728 B and GB 2571657 B were missing from the earlier notes.
- GB renewals are not in Google's record.
- A pending German application gives no injunction. It can give reasonable compensation from
  publication (§33 PatG, *from memory*).

### Square Enix: "Virtual scene lighting with light probe culling based upon visibility"

- **Inventors:** Luc Leblanc and Jean-François Dufort.
- **Priority:** CA 2,944,435, 2016-10-04.
- **US chain of title** (*read*):
  - the inventors to Eidos Interactive Corp. (Canada), effective 2017-12-06;
  - Eidos to Square Enix Limited (UK), effective 2018-02-21.
- **After the 2022 sale.** Square Enix sold Eidos-Montréal and its other Western studios to
  Embracer in 2022 (*search-recorded*). No later assignment appears, and Square Enix's fees were
  paid in 2025–2026.

| Member | Filed | Granted | Expiry | Fees, notes |
|---|---|---|---|---|
| CA 2944435 C | 2016-10-04 | 2024-01-09 | 2036-10-04 | 8th and 9th fees paid 2024-09-23 and 2025-09-22; the 10th fell due 2026-10-04 and is not shown yet |
| US 10,137,372 B2 (15/377,913) | 2016-12-13 | 2018-11-27 | 2037-02-01 (adjusted) | 4th paid 2022-05-18, 8th 2026-05-20 |
| EP 3306569 B1 | 2016-12-14 | 2024-05-01 | 2036-12-14 | See below |
| JP 6970508 B2 | 2017-01-26 | 2021-11 | 2037-01-26 | two refusals answered by amendment; annual fees received 2024-10-31 and 2025-10-29 |

EP 3306569 B1 in detail:
- No opposition was filed.
- 10th-year fees were paid in DE (2025-12-11), GB (2025-12-19) and FR (2025-12-29).
- It has lapsed in 31 other states. AL, MK and MT are not recorded.
- No unitary effect is recorded; the national fees in DE and FR point to a classic bundle of
  national patents.

### STMicroelectronics: "Local tone mapping for HDR video"

- **Inventors:** Héloïse Gresset and Brian Stewart.
- **Priority:** EP 20305180.0, 2020-02-25.
- **Assignees:** STMicroelectronics France (formerly STMicroelectronics SA) and
  STMicroelectronics (Research & Development) Ltd (*read*).

| Member | Filed | Granted | Expiry | Notes |
|---|---|---|---|---|
| EP 3872749 A1 | 2020-02-25 | — | — | deemed withdrawn 2022 |
| EP 3872750 B1 | 2021-01-22 | 2025-06-25 | 2041-01-22 | See below |
| US 11,756,172 B2 (17/168,106) | 2021-02-04 | 2023-09-12 | 2041-06-23 (adjusted) | first maintenance fee due 2027-03-12 |

EP 3872750 B1 in detail:
- No opposition was filed.
- The DE fee was paid 2025-12-17.
- GB, IE and CH are validated.
- It has lapsed in 21 states.
- FR is not recorded either way.

No other member was found.

## 2. The claims

### Imagination

**US 10,008,032 and GB 2537728 B, independent claims.** Claim 1 is the method, 13 the GPU and 20
the medium holding instructions. Each requires:
1. for each frame, find the visible surfaces for the pixels;
2. for each of several probe positions, keep a "directional representation of lighting" by:
   - tracing rays from the probe;
   - finding each ray's lighting contribution from its direction;
   - updating the representation with those contributions;
3. use the probes to find "lighting indications" for the visible surfaces;
4. shade the pixels with them.

Claim 19 is a medium holding a chip description, which concerns chip makers.

**Dependent claims that matter for an engine:**
- 3: a running average over frames;
- 6 and 7: pseudo-random, low-discrepancy, round-robin or quasi-Monte-Carlo ray directions;
- 8: spherical harmonics;
- 9: distance-based weighting;
- 10: probes within a distance and with "a direct line of sight" to the surface;
- 12: tracing only from probes chosen from the visible surfaces.

**Continuations.**
- **US 10,672,183 and GB 2571656 B.**
  - Rays are traced from each probe, and the representation "from a previous frame" is updated
    with them.
  - It is then used to render the current frame.
  - There is no visible-surface step.
- **US 11,127,198.** The same, but the update comes from "a lighting contribution for each of one
  or more rays", with no tracing from the probe.
- **US 10,410,408 and GB 2571657 B.**
  - "High-level" probes are kept up to date.
  - "Low-level" probe positions are determined from the visible surfaces and filled from
    high-level data.
  - Pixels are shaded from the low-level probes.
- **US 11,734,879.** The same as 10,410,408, without the maintenance step.
- **US 12,243,156.** Only low-level probes built from "first" probes and used for shading. The
  step that places them from visible surfaces is gone. The description's low-level probes are
  screen-space, about one per 16 × 16 pixels.
- **US 2025/0157141 A1, as published.** Trace rays for a probe position, update its directional
  representation, and render a frame with it. Its claims can still change.

### Square Enix

US 10,137,372 has 44 claims. Their core:
- (a) identify a set of probes associated with the point;
- (b) for each probe, determine which of "a plurality of zones for the selected probe" contains the
  point, and whether that zone is visible from the probe;
- (c) combine the irradiance of the probes whose zone is visible.

The independent claims add:
- **1 (method), 34 (system), 35 (medium), 36 (game apparatus):** zones "defined by intersections
  of pre-defined occlusion planes for the selected probe".
- **37:** no planes. The probe set comes from "determining a region in which the selected point is
  located" by "accessing a grid of regions".
- **38:** the zone is found with each of the probe's planes.
- **42 and 43:** planes plus a per-probe "visibility mask".
- **44:** no planes. It requires a per-probe visibility mask, the zone given as a code, and a
  comparison that "comprises a bit-wise AND".

Dependent claims add three planes (5), mask comparison (6–8), a normalised weighted sum (10, 16),
spherical harmonics (17) and real-time rendering (29).

The description defines zones per probe:
- N planes around a probe make up to 2^N zones.
- A bit string marks the visible ones.
- The point's zone comes from the signs of its dot products with the planes and is tested with a
  bitwise AND.

The other members differ:
- **CA 2944435 C (87 claims):** the same structure as the US patent. Claims 37 and 44–46 have the
  grid of regions and no planes; claims 78 and 85–87 have the mask and AND and no planes.
- **EP 3306569 B1 (15 claims):** every independent claim needs per-probe occlusion planes: the
  method (1), system (13), medium (14) and game apparatus (15). The EP claims are therefore
  narrower.
- **JP 6970508 B2 (45 claims, read in Google's machine translation):** claim 1 and the system,
  medium and game-device claims need planes. The last independent method claim applies "a
  visibility mask associated with the selected probe" without planes.

### STMicroelectronics

**US 11,756,172 independent claims:** 1 (method), 11 (device), 19 (system with a display) and 22
(medium). Each requires:
1. divide "a first image of a succession of images" into sub-blocks;
2. take the logarithm of each pixel's intensity;
3. make a "mini-image" of the sub-blocks' log intensities;
4. store it;
5. modify "a second image of the succession" based on the first image's mini-image, with a
   per-pixel gain `GMPq = (LOCAL_GAINq − 1) × GLOBAL_POWER + 1`.
   - LOCAL_GAIN comes from the second image's logs and the first image's mini-image.
   - GLOBAL_POWER is the change's influence on the whole image.

**Dependent claims:**
- 2: weights by spatial proximity and "affinity in intensity" (bilateral);
- 8: the mini-image erased after use;
- 9: a mini-image for every image.

**Prosecution.** Published claim 1 (US 2021/0264579 A1) had neither the logarithm nor the gain
formula. Both appear in the granted claim, after a non-final rejection on 2022-11-09. The file
wrapper was not read.

**EP 3872750 B1.** Claim 1 (method) and claim 14 (device) need:
- the first image's mini-image;
- a gain map of the second image made from it;
- "the gain map being erased after the generation of the output image".

The gain formula is only in dependent claim 4.

## 3. Forge against the independent claims

These are literal readings. Equivalents are for counsel: the US doctrine of equivalents, the UK's
*Actavis v Lilly* test and Japan's *Ball Spline* conditions (all *from memory*).

### Imagination: Forge's probes as built (D-036)

| Element | Forge | Met? |
|---|---|---|
| Visible surfaces each frame | rasterised every frame | yes |
| Rays traced from probe positions | ray queries from each probe (`probe_trace_main`) | yes |
| Directional representation updated from the rays | octahedral irradiance and distance maps | yes |
| Updated from the previous frame | hysteresis blend; settled probes every `cadence` frames | yes |
| Used to light visible surfaces; pixels shaded | `probe_irradiance` in the resolve | yes |
| Low-level probes placed from visible surfaces (10,410,408; 11,734,879; GB 2571657) | world-grid cascades only | no |
| Low-level probes built from first probes (12,243,156) | probe ray hits are lit by `probe_irradiance`, which reads every cascade, finer first | open: a finer cascade may or may not count as "low-level probes"; the description means screen-space probes |

On the dependent claims:
- Claims 3, 6–7 and 9–10 are arguably met: hysteresis, Fibonacci rays with a random rotation each
  frame, trilinear weights and the Chebyshev test.
- Claim 8 is not met: Forge does not use spherical harmonics.
- Claim 12 is probably not met: probes are classified from their own rays, not from the camera's
  surfaces.
- D-068's regions neither add nor remove an element of these claims.

### Square Enix: D-068's regions

- **Claims with occlusion planes** (US 1, 34–36, 38, 42–43; all EP claims; most JP claims): not
  met. Forge has no per-probe planes.
- **US 44 and CA 78, 85–87** (a mask and a bitwise AND):
  - Not met as designed: each probe has one region number, compared for equality, with no mask
    and no AND.
  - Storing a set of regions per probe as bits would move toward these claims, for example for
    probes that light both sides of a doorway.
- **The last JP independent claim** (a per-probe "visibility mask", no planes): not met if one
  region number is not a mask. It needs no AND, so it sits closer than claim 44.
- **US 37 and CA 37, 44–46** (grid of regions, no planes): the nearest.
  - Element (a): Forge picks a point's eight probes from its cell in the cascade grid. That reads
    on "accessing a grid of regions".
  - Element (c): Forge combines only the probes that pass. This is met if (b) is.
  - Element (b) is the open question. Is a room number from the plan, shared by every probe, "a
    plurality of zones for the selected probe"? And is comparing two room numbers "determining
    visibility of said determined zone from the selected probe"?
  - For a narrow reading: the description makes zones per probe, from that probe's planes.
  - For a broad reading: claims 1 and 37 differ by the planes, so "zones" alone may not need them
    (claim differentiation, *from memory*).
  - Counsel could also say whether DDGI's own Chebyshev test, a per-probe direction texel
    compared with a distance, could be argued to be a zone and a visibility test. It weighs
    softly rather than deciding yes or no.

### STMicroelectronics: D-067's local exposure

| Element | Forge | Met? |
|---|---|---|
| Sub-blocks of a first image | 64 × 36 cells over the resolved HDR frame | yes in substance |
| Logarithm per pixel | log luminance | yes |
| Stored mini-image of log intensities | the grid, and the wide Gaussian of the cells' means | arguably yes |
| Applied to a second image | used on the frame it came from; no grid kept across frames | no, if the first and second images must differ |
| US gain `(L − 1) × P + 1` | `2^((c − 1)(B − M))`, clamped | no |
| EP gain map erased after use | gain computed per pixel in the display pass | no |

Two future changes would each add one more element:
- reusing the previous frame's grid, which D-067 already excludes;
- a "strength" slider written as `lerp(1, gain, s)`, which is the claimed formula.

A strength applied in the exponent (scaling c) has a different form.

## 4. Conditions of application

Statutes are *read* unless marked. Cases are marked one by one.

| | US | UK | FR, DE (national courts or UPC) | Japan | Canada |
|---|---|---|---|---|---|
| In force here | all three families | Imagination ×3; Square Enix EP; ST EP | Square Enix EP; ST EP (DE; FR unknown); Imagination DE pending | Square Enix | Square Enix |
| Direct acts | make, use, offer, sell, import (35 U.S.C. §271(a)) | product: make, dispose of, offer, use, import, keep; process: use, or offer for use with knowledge (s.60(1)) | the same (UPCA Art. 25; CPI L.613-3, *from memory*) | in the course of trade (Art. 68): produce, use, transfer, import, offer; for programs, "providing it through a telecommunications line" (Art. 2(3)) | "making, constructing and using the invention and selling it to others to be used" (s.42) |
| Players at home | no exception | "privately and for purposes which are not commercial": excused (s.60(5)(a)) | excused (UPCA Art. 27(a); CPI L.613-5(a), *from memory*) | not "in the course of trade", so no infringement | no private-use exception found |
| Publisher | inducement, §271(b); contributory infringement, §271(c): a component "especially made or especially adapted", not a staple with substantial non-infringing use | supplying "means, relating to an essential element", with knowledge, for use in the UK (s.60(2)); s.60(6): excused private users still count | UPCA Art. 26 and 26(3); L.613-4 (supply on French soil, *search-recorded*) | Art. 101(i)–(ii) for products, (iv)–(v) for processes: articles used only for it, or indispensable and supplied knowingly | inducement: a direct infringement, influence without which it would not happen, knowingly exercised (*Corlac*, 2011 FCA 228, *search-recorded*) |

In plain words:
- **Players or publisher?**
  - **US, players.** A US player running the game performs the method claims and uses the GPU and
    medium claims, so is a direct infringer.
  - **US, publisher.** The publisher can be liable for inducement if it knew of the patent and
    meant the acts (*Global-Tech v SEB*, 2011, *from memory*). Inducement needs a direct
    infringement (*Limelight v Akamai*, 2014, *from memory*). A foreign company can induce
    infringement in the US (*from memory*).
  - **US, contributory infringement.** It looks at the component itself; putting it inside a
    larger product does not escape it (*Ricoh v Quanta*, 2008, *from memory*).
  - **Elsewhere.** Home players are excused, but the supplier stays exposed: in the UK (s.60(6)),
    at the UPC (Art. 26(3)) and in France.
  - **Japan.** In *Ichitaro* (IP High Court, 2005, *search-recorded*), selling software that
    buyers install themselves was held to be indirect infringement under Art. 101(ii). The patent
    was then found invalid.
- **Method vs apparatus vs medium.**
  - A method claim needs every step performed, and in the US every step in the US (*NTP v RIM*,
    2005, *search-recorded*).
  - US medium and system claims were held infringed by selling software that only had the
    capability, locked and never run (*Finjan v Secure Computing*, 2010, *search-recorded*).
- **One machine.**
  - US method claims need one actor, or steps attributable to one by direction or control
    (*Akamai v Limelight*, en banc 2015, *from memory*).
  - Forge runs every claimed step on the player's PC, so this rule gives no shelter.
  - Cloud rendering would move the steps to the server operator, in the server's country.
- **Where things are.**
  - **The developer in France.** Work done in France is reached only by patents in force in
    France: Square Enix's EP (whose claims all need planes) and ST's EP if its French part is in
    force. No Imagination member covers France.
  - **Being sued at home for foreign patents.** Under *BSH v Electrolux* (CJEU C-339/22,
    2025-02-25, *search-recorded*), a French-domiciled defendant can be sued in France for
    infringing other states' patents, including the UK's.
  - **Players.** Their country decides which patent their use falls under.
  - **Servers.**
    - US: a system is "used" where it is controlled and benefits are obtained, even with a part
      abroad (*NTP v RIM*).
    - UK: a system with its host computer in Curaçao was held used in the UK (*Menashe v William
      Hill*, [2002] EWCA Civ 1702, *search-recorded*).
    - Japan: a system with servers abroad was held "produced" in Japan, and programs sent from
      abroad "provided" there (*Dwango v FC2*, Supreme Court, 2025-03-03, *search-recorded*).
- **Downloads and medium claims.**
  - Each download writes a copy on a disk in the player's country.
  - Japan counts sending a program through a network as working a program invention. Square
    Enix's JP claims cover a method, system, medium and device, not a program.
  - Whether selling a download "sells" or "disposes of" a claimed medium in the US or UK was not
    checked against case law. This is for counsel.
- **The pending application.** US 2025/0157141 A1 could earn a reasonable royalty from
  publication against anyone with "actual notice" of it, if it is granted with substantially
  identical claims (35 U.S.C. §154(d); *Rosebud v Adobe*, 2016, *from memory*).
- **The UPC.**
  - The DE and FR parts can be enforced in national courts, or at the UPC unless opted out (UPCA
    Art. 83).
  - A UPC decision covers every UPC state where the patent is in force (Art. 34).
  - The opt-out status was not checked.
- **Knowledge.**
  - Forge's public notes date its knowledge of these patents to 2026-10-17.
  - In the US, knowledge is an element of inducement and contributory infringement.
  - It also bears on enhanced damages for wilful infringement (35 U.S.C. §284, *from memory*).

## 5. Licensing and enforcement history

- **Imagination.**
  - **Licensing.** In 2019 it opened its ray-tracing IP to chip makers, citing "over 220 patents
    and applications" (*search-recorded*). No licence for software makers was found.
  - **Litigation.** Its known US litigation is defensive: ITC 337-TA-884, -984 and -1037
    (2015–2016) concerned other parties' patents (*search-recorded*). No assertion of this family
    was found.
  - **The patent sale.** IAM reported on 2025-06-09 that Imagination was selling about 3,600
    patents, and that a $100 m Fortress loan made bidders cautious (*read*, the free teaser
    only).
  - **The company sale.** A 2026 report says its owner asked Lazard to find a buyer for the company
    (*search-recorded*).
  - No completed sale was found. A new owner may enforce differently.
- **Square Enix and ST.** No licensing programme, assertion or litigation was found for either
  family.
- **Comparable features already shipping.** None was found licensed from or challenged by these
  holders, though licences can be private.
  - **Imagination-like.** These engines update directional lighting at probe positions by tracing
    rays every frame:
    - NVIDIA's open-source RTXGI-DDGI, whose 2021 paper says its methods went into Unity, Unreal
      Engine 4 and several commercial games' engines (*search-recorded*);
    - Unreal's Lumen;
    - Godot's SDFGI;
    - AMD's GI-1.0.
  - **Square Enix-like.** Unity 6's Adaptive Probe Volumes tie each probe to up to four
    rendering-layer masks at bake time; the eight probes around a pixel prefer those of its layer
    (*read*). That is closer to Square Enix's mask claims than D-068's equality test.

## 6. Validity notes (facts only)

Prior art must be published before the priority date the claims are entitled to:
- Imagination: 2015-03-03;
- Square Enix: 2016-10-04;
- ST: 2020-02-25.

**Imagination.**
- **Cited by the examiners:** Sloan's "Stupid SH Tricks", Microsoft US 6,023,523, Pixar US
  2004/0160441, Crytek US 2014/0327673, NVIDIA US 2014/0327690, Square Enix US 2014/0313200, and
  Sukys (2014) (*read*).
- **Others a professional might look at:**
  - Greger et al., the irradiance volume (1998);
  - Tatarchuk, irradiance volumes for games (2005);
  - Nijasure, Pattanaik and Goel, "Real-time global illumination on GPUs" (JGT 2005; the record
    was seen, the content not read);
  - Papaioannou's radiance hints (HPG 2011): a grid of SH probes refreshed every frame from
    reflective shadow maps (*search-recorded*);
  - Ward's irradiance caching (1988, *from memory*).
- **Too late to count:** McGuire et al. (2017) and DDGI (2019) came after the priority date.
- **NVIDIA's own DDGI application** (US 2021/0012562) was rejected, appealed, and abandoned in
  2023. The Board's reasons were not found.

**Square Enix.**
- **Cited by the examiners:** Imagination's US 2016/0260247 A1, Cupisz (GDC 2012), Tatarchuk
  (2005), Sugden's "Mega Meshes" (2011), and Valient on Killzone Shadow Fall (2014). The EPO's 2017
  search report was not read.
- **Imagination's application.** It was published on 2016-09-08, before Square Enix's priority
  date. It describes an artist or tool giving probes "a volume of effect": a box, a tetrahedron or
  a CSG volume. A probe in a room's centre then reaches far, and one by a wall does not (*read*).
- **Valve's Source engine (2004)** stores ambient cubes per BSP visleaf (*search-recorded*).
- **Too late to count:** McGuire (2017), Iwanicki and Sloan (2017) and DDGI (2019).

**ST.**
- **Cited by the examiners:** Texas Instruments US 2014/0152686, Apple US 2015/0071537, Dolby US
  9,961,237 and Xerox US 2007/0269132 (*read*).
- **Others:** Durand and Dorsey (2002); Chen, Paris and Durand's bilateral grid on real-time video
  (2007); Reinhard et al. (2002).
- **What distinguished the claim.** The logarithm and the gain formula were added during
  prosecution. That history bears on how far equivalents can reach.

## 7. What would lower the risk, if the owner wants to

**Square Enix (cheap):**
- Keep D-068 to one region number per probe and per surface, compared for equality.
- Make the doorway ramps a weight computed from the surface's position in the opening, never
  per-probe masks or bit tests. This costs nothing in look or speed.
- To move further from US/CA claim 37, pick the probe set from the surface's room up front instead
  of testing each of the eight probes. That means separate probe grids per room.
  - Cost: probes on both sides of every partition, so more memory and more rays (not estimated).
  - The look would be about the same.
- In DE, FR and GB, the EP claims need planes.

**ST (free):**
- Keep the grid built and used within one frame.
- Apply any strength in the log domain, not as `(gain − 1) × s + 1`.
- Compute the gain per pixel instead of storing a gain map.

**Imagination (costly).** Every granted independent claim needs probes whose directional light is
updated from rays and used for shading. The options:
- Bake probes offline. This loses dynamic GI (time of day, lamps, doors) and saves the trace cost.
- Use per-pixel ray-traced GI instead. ReSTIR GI raises NVIDIA's patents (D-065).
- Use screen-space GI. This loses a lot indoors and off-screen.
- Render probe cube maps by rasterisation instead of rays. This is weak against US 11,127,198,
  which does not require tracing.
- Turn dynamic probes off in US and UK builds. Those builds lose the same look.

**For all three:**
- **A freedom-to-operate opinion** from US, UK and EP counsel, plus JP and CA counsel for Square
  Enix. Fees were not researched.
- **Cheap official views,** both published and non-binding (*from memory*):
  - the UK IPO's opinions on infringement or validity (s.74A);
  - the JPO's *hantei* on a patent's technical scope (Art. 71).
- **Validity challenges:**
  - US inter partes review;
  - UK revocation;
  - national or UPC revocation for the EP patents, whose opposition periods have passed.
- **A licence.** Imagination licenses chip makers; no route for software makers was found.
- **Waiting.**
  - Imagination expires 2036-03 to 2036-08.
  - Square Enix expires 2036-10 (CA) to 2037-02 (US).
  - ST expires 2041.
  - A member also expires early if a fee goes unpaid. Watch the US fees due in 2027 and the
    Canadian 10th-anniversary fee.
- **Watching.** Follow the pending US 2025/0157141 and DE 10 2016 103 854 for allowed claims, and
  Imagination's sale for a new owner.

## Sources

**Patents.** All on Google Patents (patents.google.com/patent/NUMBER/en), *read*:
- Imagination: US10008032B2, US10410408B2, US10672183B2, US11127198B2, US11734879B2,
  US12243156B2, US20250157141A1, US20160260247A1, GB2537728B, GB2571656B, GB2571657B,
  DE102016103854A1.
- Square Enix: US10137372B2, EP3306569B1, JP6970508B2, CA2944435C.
- STMicroelectronics: US11756172B2, US20210264579A1, EP3872750B1, EP3872749A1.
- NVIDIA: US20210012562A1.

**Statutes** (*read* unless marked):
- 35 U.S.C. §271: https://www.law.cornell.edu/uscode/text/35/271
- Patents Act 1977, s.60: https://www.legislation.gov.uk/ukpga/1977/37/section/60
- UPC Agreement, Arts. 25–27, 34 and 83:
  https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:42013A0620(01)
- Japanese Patent Act, Arts. 2, 68 and 101:
  https://www.japaneselawtranslation.go.jp/en/laws/view/4097
- Canadian Patent Act, ss.42 and 44: https://laws-lois.justice.gc.ca/eng/acts/p-4/page-7.html
- CPI L.613-4 (*search-recorded*):
  https://cms.law/fr/fra/a-la-une/contrefacon-par-fourniture-de-moyens-un-papier-explicatif

**Cases** (*search-recorded*):
- NTP v RIM, 418 F.3d 1282:
  https://law.resource.org/pub/us/case/reporter/F3/418/418.F3d.1282.03-1615.html
- Finjan v Secure Computing, 626 F.3d 1197:
  https://www.finnegan.com/en/insights/articles/the-computer-readable-medium-claim-the-best-of-the-apparatus-and.html
- Menashe v William Hill:
  https://en.wikipedia.org/wiki/Menashe_Business_Mercantile_Ltd_v_William_Hill_Organization_Ltd
- Dwango v FC2:
  https://jonesday.com/en/insights/2025/03/japanese-supreme-court-building-a-system-that-includes-a-server-located-outside-japan-constitutes-patent-infringement
- Ichitaro: https://www.ip.courts.go.jp/eng/vc-files/eng/file/decision_summary2005ne10040.pdf
- BSH v Electrolux:
  https://www.aippi.org/news/court-of-justice-of-the-eu-rules-on-jurisdiction-in-cross-border-ip-disputes-bsh-v-electrolux-c-339-22
- Corlac v Weatherford: https://cassels.com/insights/fca-confirms-test-for-inducing-patent-infringement/
- *From memory*, not re-read: Global-Tech v SEB (2011); Limelight v Akamai (2014); Akamai v
  Limelight (2015); Ricoh v Quanta (2008); Rosebud v Adobe (2016); Actavis v Lilly (2017); Ball
  Spline (1998).

**Enforcement and market:**
- IAM, 2025-06-09 (*read*): https://www.iam-media.com/article/uk-ai-chip-company-selling-3600-patents
- Imagination's 2019 licensing announcement: https://www.imaginationtech.com/?p=4231
- Jones Day on the ITC cases:
  https://www.jonesday.com/en/practices/experience/2016/11/imagination-defends-its-computing-and-graphics-systems-from-infringement-claims-in-itc
- The Embracer purchase:
  https://www.gematsu.com/2022/05/embracer-group-to-acquire-crystal-dynamics-eidos-montreal-and-square-enix-montreal-from-square-enix
- Unity's APV rendering layers (*read*):
  https://docs.unity3d.com/Manual/urp/features/rendering-layer-masks-apv-prevent-light-leaks.html
- Majercik et al. 2021:
  https://research.nvidia.com/publication/2021-05_scaling-probe-based-real-time-dynamic-global-illumination-production

**Prior art** (*search-recorded*):
- Valve's ambient cubes: https://developer.valvesoftware.com/wiki/Ambient_cube
- Papaioannou 2011: https://diglib.eg.org/items/d76ad5a8-4d26-4603-9a15-902d25e99e35
- Nijasure et al. 2005: https://ftp.math.utah.edu/pub/tex/bib/idx/jgraphtools/10/2/55-71.html
- The remaining papers and engines are listed in `probe-leaks-indoors.md` and
  `local-exposure.md`.
