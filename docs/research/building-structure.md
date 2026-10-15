# Building structure: real dimensions, enterable interiors, destruction

Research note, 2026-10-11, for the owner's notes during D-059's sandbox work:
- wall thickness counts (old stone houses thicker than recent buildings, trams too);
- floor slabs have a thickness, so heights, widths and lengths must add up;
- a varied kit of modules may simplify physics and destruction later (Battlefield 6);
- interiors by use (#248).

Markers: **[est.]** is an estimate with no source found. **[unverified]** means the sources
disagree or only a search snippet was seen. Sources marked * in the list were opened; the others
come from search results. Nothing was downloaded beyond three PDFs a fetch read as text.

## 1. Real dimensions to model

### Walls

| Element | Thickness | Source |
|---|---|---|
| Old rubble-stone wall (two faces and fill, lime mortar) | 45–70 cm in old town centres; 50–80 cm in rural Ain | [1][2]; no Mediterranean figure found |
| Same wall, upper floors and courtyard side | thinner than the street face | [3] |
| Haussmann street façade, cut stone | no measured figure; load-bearing cut stone 30–40 cm [4], early-1900s rubble façades in Vaud about 50 cm [5]; take 45 cm at the ground, 40 cm above [est.] | |
| Haussmann courtyard or rear wall | poorer stone, brick, plaster or timber frame [6][7]; about 30 cm [est.] | |
| Interwar brick | 40–50 cm (Alsace) | [8] |
| Post-war concrete (1950–75) | 15–20 cm, little insulation | [est.] |
| New housing | over 70 % of collective schemes are concrete, mostly insulated inside; brick 30–37 cm [9]; the façade about 0.35 m in all [est.] | |
| Timber (CLT) with external insulation | 0.35–0.45 m | [est.] |
| Wall between new dwellings | 18–20 cm of concrete (the 53 dB rule) | [10] |
| Party wall in an old town | a shared stone wall of 40–60 cm | [est.] |
| Partitions | plasterboard 72 or 98 mm, plaster blocks 7 cm [11]; in the 1800s hollow brick, or timber with plaster fill [7] | |

### Floors and heights

- **New floors:** a 20 cm concrete slab is the acoustic standard between dwellings [10]; an
  acoustic underlay, screed and finish add about 5–7 cm [est.].
- **Old floors:** timber joists, or after about 1850 iron joists with plaster infill [12], about
  25–30 cm [unverified].
- **Clear height:** the legal minimum is 2.20 m under the décence decree [13] (the 2023 decree
  partly annulled in 2024 [14]; some departments ask 2.30–2.50 m). New housing has 2.50 m, so
  about 2.75–2.80 m floor to floor [est.].
- **Haussmann:** the piano nobile 2.80–3.20 m clear (to 3.50), the fifth and sixth floors
  2.50–2.70 m [15] (a real-estate agency's figures).
- **Shops:** no national minimum height; each local plan sets it. Lyon asks listed shop ground
  floors at least 7 m deep [16]. Forge's 4.2 m floor to floor is plausible [est.].

### Circulation and accessibility (France)

- **Lifts in housing:** in collective buildings of more than two storeys over the ground floor
  (R+3 and up), for permits since 1 October 2019 [17][18].
- **Lifts in shops and public buildings (ERP):** when 50 people can be on an upper or lower level
  (100 for schools), or when a service exists only off the ground floor [19]; sources differ on
  "more than 50" and "50 or more".
- **Lift cars (EN 81-70):** type 2 is 1100 × 1400 mm with a 900 mm door in every source. Type 1 is
  1000 × 1250 in summaries of the 2003 edition [21] [unverified] and 1000 × 1300 in KONE's table
  of the current one [20]; type 3 is 2000 × 1400 or 1100 × 2100. Check the current text.
- **Stairs:** in housing, risers at most 17 cm [17], goings at least 28 cm, at least 1.00 m
  between handrails [unverified]. In ERP, risers at most 16 cm, goings at least 28 cm, 1.20 m
  between handrails (1.40 m between two walls in one guide [22]).
- **Corridors and doors:** common paths 1.20 m, locally 0.90 m [23]; inside a dwelling at least
  0.90 m [24]; a turning circle of Ø 1.50 m [25]. Entrance doors 0.90 m (0.83 m clear); interior
  doors 0.80 or 0.90 m by guide [26]: take 0.90 m everywhere.
- **The ELAN law:** 20 % of new collective dwellings fully accessible, the rest adaptable [26].

### Dwelling sizes and gross to net

- Net sizes (Val-d'Oise, 2014): studio 37 m², T2 42–48, T2 bis 55, T3 65–67, T4 86, T4/5
  100 m² [27].
- New collective dwellings (2013): T1 8–13 %, T2 24–26 %, T3 37–39 %, T4 22–23 %, T5 about
  4 % [9].
- INSEE 2013: an apartment averages 63.0 m², 32.4 m² a person (45.1 in houses) [28]. A flat
  has 2.9 rooms on average, and its people 1.5 rooms each (1.3 in 1984), so about 1.9 people a
  flat; the rooms counted are the living rooms and bedrooms, the kitchen over 12 m², not the
  hall, bathroom or WC [28].
- **Gross to net:** over 1 400 certified schemes, gross (SHON, to the walls' outer faces) over
  net (SHAB, without walls, partitions, stairs, shafts, reveals and space under 1.80 m) is about
  1.22–1.23, flats and grouped houses alike [9]: net ≈ 0.82 × gross.
- **Forge's model:** 40 m² gross a person × 0.81 = 32.4 m² net, INSEE's apartment figure. The
  constant holds for modern buildings; in the old town, 50–70 cm walls bring net over gross to
  about 0.70–0.75 [est.].

### A low-floor tram (Citadis X05)

Body 2.40 or 2.65 m wide; step 325 mm over the rail at the middle doors, 334 mm at the front;
the aisle over the bogies up to 750 mm [29]; floor 350 mm, overall height 3.45 m (Tunis) [30].
Siemens' Avenio: 3.65 m tall, 350 mm entry [31]. No source gives the inside height or the wall
thickness: about 2.10–2.30 m clear, 60–100 mm walls, 2.20–2.25 m inside width [est.].

## 2. How games build enterable buildings

- **Kits on a snapping grid (Bethesda) [32]:** footprints in multiples (a 512 room tiles with a
  256 hall), snapped at half a footprint; floor thickness and door frames fixed early; the
  narrowest passage two characters wide (128 units ≈ 6 ft, so 256 ≈ 3.66 m, 512 ≈ 7.3 m). Two
  kit artists supplied eight designers; but fixing a piece touches hundreds of instances, and
  repetition tires the eye. For Forge: wall panels, door frames, stair flights, lift cores.
- **Procedural floor plans:** Merrell et al. 2010 (a Bayesian network trained on real plans, then
  stochastic layout; the best results, slow, made for houses) [33]; Marson & Musse 2010
  (squarified treemaps and added corridors, real time) [34]; Lopes et al. 2010 (rooms grown on a
  grid under constraints, always connected) [35]. For flats inside a fixed envelope round a fixed
  core, grid growth fits: fast, one plan per seed, on the dimension grid.
- **Interior mapping (van Dongen 2008) [36]:** the pixel shader intersects the view ray with
  evenly spaced virtual floors, ceilings and walls; no geometry, a cost independent of the rooms.
  Weak at repetition and at corners. For the windows of buildings one cannot enter, fed by the
  real floor heights and room walls so the fake matches the inside.

## 3. Destruction

- **Battlefield 6 (Frostbite, 2025)** [37][38][39]: part-based destruction, debris, health states,
  destructible materials tuned in the editor, bandwidth and culling for 64 players. In the April
  2025 tests surfaces degraded before breaking, rubble stayed and buildings "apple-cored" (the
  core stands). Dropping the last-generation consoles freed memory and CPU. Nothing public on a
  structural solver or the server's share [unverified].
- **The Finals (Embark)** [40][41]: destruction on the servers, so every player sees the same; a
  GDC 2024 talk on fracture, replication, performance and structural analysis; a structural layer
  and a debris layer, later a "gradual" layer that bends and sags. UE5; Chaos not confirmed
  [unverified].
- **Rainbow Six Siege (RealBlast):** walls, floors and ceilings breach by weapon; explosion damage
  by raycasts, capped by walls until Y5S1's shrapnel [42]; the internals in a paywalled talk [43].
- **Red Faction: Guerrilla (Geo-Mod 2.0):** pre-broken meshes and an in-house stress model [44].
  Its design director: the artists "have to build the insides of everything", and anything not
  built soundly falls [45]. Structure first.
- **Teardown:** several voxel volumes; buildings stand on a few voxels: connectivity, not stress
  [46]. Its 10 cm voxel is [unverified].
- **References:** Unreal's Chaos breaks a connection over an impulse limit, anchor fields hold
  pieces [47]. NVIDIA Blast: chunks, bonds and a support graph; its stress extension derives bond
  forces from accelerations and breaks bonds over pressure limits [48]. A BSD-3 Rust wrapper of
  Blast exists, integrated with Rapier, not Jolt [49].

**A building of walls, slabs and columns would need:**
1. a node per panel (a bay of one storey): mass, material, pre-cut chunks;
2. bonds along shared edges, with their contact area;
3. the foundations as anchors;
4. after each hit, a flood fill (union-find) from the anchors that frees what no longer stands;
5. optionally an overload pass;
6. the state on the server, the clients sent node and bond changes;
7. rendering, collision and navigation rebuilt per panel, not per building.

**Jolt [50]:** breakable constraints (read the impulse after the step, `GetTotalLambdaPosition` /
`GetTotalLambdaRotation`, then disable it), a runtime-editable `MutableCompoundShape`, cross-
platform determinism. No fracture, no Voronoi cutting, no stress solver. Dynamic bodies of
0.1–10 m: the buildings stay static and only the freed pieces become bodies.

## 4. Patents

Expiry dates are Google Patents' estimates; the search was web-only and not exhaustive.

| Patent | Holder | Covers | Status | For Forge |
|---|---|---|---|---|
| US 8,152,613 B2 [51] | Sony Interactive | claim 1: remove the tiles inside a damage radius and split those it crosses; claim 9: stress nodes per tile under gravity and a restoring force, breaking over a threshold | granted 2012, active, to about 2030-07-26 | the nearest to "panels and stress". A connectivity check or Blast-style bond forces differ. **Ask the owner before an overload pass.** |
| US 12,700,170 B2 [52] | Celsys (Japanese priority 2021) | full rendering of the user's room, interior mapping for the others, a partly transparent door to show passability, swapping as the user moves | granted 2026-08-04, to about 2042 | close to "enter one room, fake the rest". No door-transparency cue, no per-room swapping; **ask before building**. |
| EP 3238179 B1 / US 10,380,790 [53] | Robert Bosch | window lighting and interior maps, rooms inferred from texture coordinates | EP granted 2026, US 2019, to about 2035 | Forge takes its rooms from the building's data: low risk |
| US 12,220,640 B2 [54] | Electronic Arts | a floor-plan generator and furniture layout by a graph neural network | granted 2025, to about 2043 | no learned layout; rule-based growth is from 2010 papers |
| US 10,427,046 B2 [55] | Take-Two | loading assets (interior parts among them) by metadata tags | granted 2019, to about 2037 | a narrow claim: low risk |
| US 6,456,289 [56] | Georgia Tech | animating fracture with finite elements | filed 1999, expired by term [unverified] | free |

No patents found on interior mapping itself, the 2010 floor-plan papers, RealBlast, Geo-Mod,
Teardown, Frostbite's or Embark's destruction.

## 5. Recommendation for Forge

1. **The data:** a building holds its storeys (level, floor to floor, slab thickness, clear
   height); a wall graph (segments with thickness, material and role: façade, party, courtyard,
   core, partition; openings with sill, head and reveal); a core (stairs and lift); and its units
   (use, type, rooms). The façade grammar puts its windows in those openings, their reveals as
   deep as the wall: old stone stops looking flat.
2. **A 0.30 m planning module** (a 0.90 m door is 3, a 1.20 m corridor 4). The façade's outer face
   on the building line, the thickness inwards.

   | Metres | Old stone town | Haussmann, 1800s | Modern |
   |---|---|---|---|
   | Façade / party / rear wall | 0.60 / 0.50 / 0.45 | 0.45 → 0.40 / 0.40 / 0.30 | 0.35 / 0.20 / 0.35 |
   | Partition; slab with its finish | 0.10; 0.30 | 0.10; 0.30 | 0.07; 0.27 |
   | Floor to floor, ground | 3.6 | 4.2 (shop) | 4.2 shop, 3.0 housing |
   | Floor to floor, upper | 3.2 down to 2.9 | 3.4 noble, 3.1, 2.9, 2.8 attic | 2.80 |
   | Window bay | 3.0–3.6 | about 3.0 | 2.70 / 5.40 |

   The walls and slabs follow §1 (part sourced, part estimated); the floor to floor and the bays
   are estimates.
3. **Checks:** the storeys' floor to floor and the roof add up to the building's height; gross
   area (outer faces) and net area (the rooms) both kept, net over gross about 0.80 modern and
   0.70–0.75 in the old town.
4. **The population:** people by dwelling type, or net area over 32.4 m², in place of the 40 m²
   gross constant (right for modern buildings, too many in old stone ones).
5. **Accessibility as rules:** lifts in housing from R+3, and wherever shops or offices have
   public upper floors; type 2 cars (1.10 × 1.40 m), 0.90 m doors, 1.20 m corridors, Ø 1.50 m
   turning; step-free entrances.
6. **The order:**
   1. the structure's data, the dimension tables, and tests that heights, widths and areas add
      up; the population retuned;
   2. the façade kit reads the walls; collision from walls and slabs;
   3. cores and units on the grid, interior mapping from the same rooms, then enterable interiors
      by use;
   4. destruction: the panel graph, connectivity to anchors, pre-cut chunks as Jolt debris, the
      state on the server; an overload pass only after a patent check (Sony's, to about 2030).

## Sources

1. https://www.effinergie.org/web/images/attach/base_doc/1334/Guide%20Bati%20Ancien%20et%20Energie.pdf
2. https://www.ain.gouv.fr/contenu/telechargement/14104/105926/file/ficherenovationthermique3_maisonenpierre_voct2020.pdf
3. https://proreno.fr/storage/media/shares/pdf/00515/APAPU240_01.pdf
4. https://www.batiproduits.com/fiche/produits/pierre-de-taille-mur-porteur-massif-p68891993.html
5. https://www.vd.ch/fileadmin/user_upload/themes/culture/patrimoine_bati/fichiers_pdf/DMS_TypoRENO_VD_G2_immeuble_locatif.pdf
6. https://www.citedelarchitecture.fr/sites/default/files/documents/2017-09/fo_boulevardsaintmichel_def.pdf
7. https://oasisbr.ibict.br/vufind/Record/RCAP_b5f73fee80d777d08e36070f79cfe6ac
8. https://www.grand-est.developpement-durable.gouv.fr/IMG/pdf/HAA_fiche3_immeuble_rapport_mitoyen.pdf
9. \* https://www.qualitel.org/professionnels/uploads/tendances_constructives_2013.pdf
10. https://www.bruit.fr/revues/78_12958.PDF
11. https://www.placo.fr/documents/fiche-systeme/fichesysteme-sp00018691.pdf
12. https://assises.bruit.fr/images/webinaires/hippolyte-mack.pdf
13. https://www.service-public.gouv.fr/particuliers/actualites/A17662
14. https://www.legifrance.gouv.fr/ceta/id/CETATEXT000050150879
15. https://homeselect.paris/blog/appartement-haussmannien-paris-guide/
16. https://www.data.gouv.fr/datasets/5e3aecae6f4441741c8cdd02
17. \* https://qualiteconstruction.com/wp-content/uploads/2024/05/Fiche-Qualite-Reglementaire-F03-Accessibilite-Circulations-Interieures-Communes-AQC.pdf
18. https://informations.handicap.fr/a-logement-accessibilite-ascenseur-handicap-decret-r3-11369.php
19. https://www.finistere.gouv.fr/index.php/contenu/telechargement/4787/30370/file/FICHE_5.pdf
20. \* https://kone.co.uk/tools-downloads/codes-and-standards/en81-70-compliant-solutions
21. https://elevatorworld.com/?p=71343
22. https://www.cotes-darmor.gouv.fr/contenu/telechargement/4073/24351/file/GUIDE%20ERP.pdf
23. https://www.finistere.gouv.fr/index.php/contenu/telechargement/4782/30350/file/FICHE_2.pdf
24. https://www.les-caue-occitanie.fr/sites/default/files/fichiers/ressource/field_fichiers/accessibilite_des_maisons_individuelles_neuves.pdf
25. \* https://qualiteconstruction.com/wp-content/uploads/2024/05/Fiche-Qualite-Reglementaire-F05-Accessibilite-Logements-AQC.pdf
26. https://www.habitatpresto.com/mag/renovation/accessibilite-pmr-handicap/logement-pmr-amenagement-normes
27. \* https://www.val-doise.gouv.fr/contenu/telechargement/6997/49174/file/Adequation_DDT95_20141016.pdf
28. https://www.insee.fr/fr/statistiques/1287961
29. \* https://gestaourbana.prefeitura.sp.gov.br/wp-content/uploads/2025/11/Alstom_Product_Sheet_Citadis_X05_EN_0.pdf
30. https://www.alstom.com/press-releases-news/2004/6/tunis-chooses-alstoms-citadis-trams
31. \* https://assets.new.siemens.com/siemens/assets/api/uuid:fb283678-dfa7-413a-8cb2-c3c9bc900200/produktdatenblatt-avenio-d.pdf
32. \* https://www.gamedeveloper.com/design/skyrim-s-modular-approach-to-level-design
33. https://www.cs.princeton.edu/courses/archive/spring11/cos598A/pdfs/Merrell10a.pdf
34. https://doi.org/10.1155/2010/624817
35. https://graphics.tudelft.nl/~rafa/myPapers/bidarra.GAMEON10.pdf
36. http://joostdevblog.blogspot.com/2018/09/interior-mapping-real-rooms-without.html
37. \* https://www.ea.com/en/news/how-battlefield-6-redefined-destruction
38. \* https://www.ea.com/en/games/battlefield/battlefield-6/news/battlefield-labs-destruction
39. https://insider-gaming.com/battlefield-6s-destruction-improved-after-dropping-ps4-and-xbox-one/
40. https://gdcvault.com/play/1034280/Engineering-Mayhem-Technical-Deep-Dive
41. \* https://www.kitguru.net/gaming/joao-silva/embark-studios-reveals-major-destruction-system-update-for-the-finals/
42. https://www.ubisoft.com/en-us/game/rainbow-six/siege/news-updates/1QkezaGoRkDWqcQ6duGvtk/dev-blog-explosions-shrapnel-in-y5s1
43. https://gdcvault.com/play/1023003/The-Art-of-Destruction-in
44. https://www.redfactionwiki.com/wiki/GeoMod_2.0
45. https://www.redfactionwiki.com/wiki/Interview_with_James_Hague
46. \* https://en.wikipedia.org/wiki/Teardown_(video_game)
47. https://dev.epicgames.com/documentation/unreal-engine/destruction-overview
48. https://nvidia-omniverse.github.io/PhysX/blast/docs/api/extensions/ext_stress.html
49. \* https://docs.rs/blast-stress-solver
50. \* https://jrouwe.github.io/JoltPhysics/
51. \* https://patents.google.com/patent/US8152613B2/en
52. \* https://patents.google.com/patent/US12700170B2/en
53. \* https://patents.google.com/patent/EP3238179A1/en
54. \* https://patents.google.com/patent/US12220640
55. \* https://patents.google.com/patent/US10427046B2/en
56. https://patents.google.com/patent/US6456289B1/en (a search result, not opened)
