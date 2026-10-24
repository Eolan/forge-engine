# Lights in the homes and the lanes: bulbs, fittings, lit windows and street lamps at night

This note answers the owner's request of 2026-10-24: "launch a research on the various light bulbs, lamps, and source of light power and lumen". They asked after seeing the first map (#266) at night. They said the lit windows seen from the street looked "too strong … they are house lights, not spotlights".

The note gives physical values for the village's lamps, windows and street lamps, and it says where Forge's night differs from them. It builds on five other notes, which it refers to without repeating them:

- [interior-lighting.md](interior-lighting.md) (D-065's lamps)
- [local-exposure.md](local-exposure.md) (D-067)
- [night-sky.md](night-sky.md) (D-046)
- [glass-credibility.md](glass-credibility.md)
- [lighting-patents.md](lighting-patents.md)

Labels: **[standard] [paper] [data sheet] [docs] [web] [patent]**. Each source in the list at the end is marked *read* (read on 2026-10-24) or *search-recorded* (seen only in search results).

> **In five sentences.**
> 1. Forge's lamps are close to real homes. 450–1,600 lm per room is what one ceiling light or two table lamps give, and 2,700–3,000 K is what European homes burn. The gaps are three: bare bulbs light every way where homes have shades, every room is lit where an occupied home lights one or two, and the windows have no shutters.
> 2. A lit window seen from the street is a few candelas per square metre. The average measured on curtained flats in Ostrava was 6 cd/m² [30], and 1–8 cd/m² was recorded behind blinds [31], so Forge's faked windows at about 5 cd/m² are right.
> 3. The windows read as white because the night exposure is keyed on moonlight. At EV100 −2, everything above about 2 cd/m² lands in the top 15 % of the display, so a curtained window, a lamp-lit wall and a street lamp's pool are squeezed together. Photographers meter "distant lit buildings" at EV 2 [29], and cinematographers set practical lamps 2.5–4 stops over the key [28].
> 4. The fix that keeps the units physical has four parts:
>    - key the night on the lamp-lit pixels where lamps are on (EV100 about +1);
>    - let local exposure lift the moonlit fields;
>    - give the fittings their shades and the windows their shutters and curtains;
>    - light only the rooms the household is using.
>
>    The same exposure is what made real street lamps look too bright, so they can go back to real values: 1,000–1,500 lm lanterns at 4–5 m, giving 3–7.5 lux on the lane.
> 5. No patent stands in the way of the values. Bosch's lit-window patent (US 10,380,790 B2, and now EP 3238179 B1, granted 2026-02-11) is avoided by taking each window's lamp and covering from its house, never from floors laid out in texture space.

**Contents**

1. [Light sources in homes](#1-light-sources-in-homes)
2. [Fittings and how they spread light](#2-fittings-and-how-they-spread-light)
3. [Rooms: how many lamps, which are on, how bright](#3-rooms-how-many-lamps-which-are-on-how-bright)
4. [A lit window seen from the street](#4-a-lit-window-seen-from-the-street)
5. [Street lighting in villages](#5-street-lighting-in-villages)
6. [How games and engines set these](#6-how-games-and-engines-set-these)
7. [Patents](#7-patents)
8. [Recommendations for Forge](#8-recommendations-for-forge)
9. [Verification notes](#9-verification-notes)
10. [Sources](#sources)

---

## 1. Light sources in homes

### 1.1 Watts, lumens, efficacy, colour

| Source | Typical sizes (W → lm) | Efficacy (lm/W) | Colour temperature | CRI | Typical home use | EU status |
|---|---|---|---|---|---|---|
| Incandescent bulb (GLS), 230 V | 40 W ≈ 415 lm; 60 W = 710 lm [6]; 100 W ≈ 1,380 lm | 8–14 [4] (60 W: 11 [6]) | 2,700 K; warmer when dimmed | 100 | everything until about 2010 | phased out 2009–2012 (Reg. 244/2009; not re-read) |
| Mains halogen (GLS, candle, G9, R7s uplighter) | 42 W ≈ 600 lm; R7s 300 W: several thousand lm | ~12–20 mains, 24 low-voltage [4] | 2,800–3,000 K | 100 | uplighters, spots, small capsules | GLS halogen 2018; R7s > 2,700 lm 2021; G9, G4, GY6.35 2023 [1] |
| Compact fluorescent (CFL) | 11 W ≈ 600 lm | 46–75 [4] | 2,700 K (some 4,000 K) | ~80–85 | "energy-saving" bulbs, 2000s–2010s | CFLi 2021; CFLni 2023 [1] |
| Linear fluorescent T8/T5 | T8 58 W ≈ 5,000 lm | 70–104 [4] | 3,000–4,000 K (6,500 K) | ~80–85 | kitchen and bathroom strip lights (*réglettes*), garages | T5 and T8 25 Aug 2023 (RoHS) [1] |
| LED bulb (A60, globe, candle, filament) | 7 W = 806 lm [5]; E14 candle 2.5–5 W = 250–470 lm [7] | 75–217 [4]; 115 for the Philips A60 [5] | 2,700 K usual; 2,200–2,500 K filament; 3,000/4,000 K; dim-to-warm 1,800–2,700 K [11] | 80 typical [5]; 90+ premium | nearly every socket sold since 2018 | the only general-purpose lamp left |
| LED built into a fitting | ceiling light 600–2,000 lm [9]; worktop strip 220–540 lm per 30–80 cm [8] | ~90–130 | 2,700–4,000 K | 80; strips > 90 [8] | ceiling lights, strips, mirror lights | — |
| Candle | ≈ 12.6 lm [19] | — | ≈ 1,900 K [19] | — | tables, decoration | — |

- **The EU "equivalent watts" scale** is the one printed on LED boxes: 25 W ≈ 249 lm, 40 W ≈ 470 lm, 60 W ≈ 806 lm, 75 W ≈ 1,055 lm, 100 W ≈ 1,521 lm [2][3]. It is generous. A real 230 V 60 W bulb gave 710 lm [6].
- **Energy label.** The Philips 806 lm, 115 lm/W bulb is class E [5]. The 2021 scale keeps the A–C classes for the most efficient lamps.
- **What Europe sells now.** Mains halogen bulbs left in 2018. CFLs with built-in ballast went on 1 September 2021. RoHS stopped CFLs without ballast on 25 February 2023, and T5 and T8 tubes on 25 August 2023. G9, G4 and GY6.35 halogen capsules went on 1 September 2023 [1]. Any socket refilled since 2023 gets an LED.

### 1.2 Colour temperature and colour rendering in homes

- **2,700 K is the home default; 3,000 K is common in kitchens and bathrooms.** In a pilot of 30 lighting professionals' homes, every respondent had 2,700 or 3,000 K in living rooms and bedrooms. Kitchens went up to a 3,500 K tube and a 4,000 K CFL. One home had LEDs at 1,900 K (a dim-to-warm lamp) [11].
- **Warmer sources.** Filament and candle LEDs run 2,200–2,700 K. Dimmed incandescent and dim-to-warm LEDs reach 1,800–2,200 K.
- **Colour rendering.** Incandescent and halogen score 100 (they are the reference). Home LEDs are CRI 80, and 90+ on premium bulbs and strips [5][8]. Tubes and CFLs are about 80–85.

### 1.3 What French homes and old Provençal houses have

- **Number of lights.** A French household has about 25 light points (ADEME guide, 2018) [17].
- **Energy use.** ADEME's ElecDom panel measures 100 homes appliance by appliance. In July 2023–July 2024, lighting used about 110 kWh per home, under 3 % of the household's electricity (Selectra's calculation from the panel) [17]. ADEME's 2018 guide gave 325–450 kWh. The fall fits a stock that is now mostly LED (an inference).
- **No French survey of LED share was found.** In Melbourne in 2019, 76 % of classified evenings were lit mainly by LED or fluorescent sources [10].
  - Estimate for a French village in 2026: 80–90 % of the lamps on in an evening are LED. The rest are halogen G9 and R7s capsules, CFLs and tubes left in old fittings until they fail.
- **Old Provençal village houses.** The list below comes from decor and antique listings [57] and common practice. It is not a survey.
  - A wrought-iron chandelier (*lustre*) with 3–6 E14 candle bulbs over the dining table.
  - A single E27 pendant in enamel or glass over the kitchen table.
  - Table lamps with fabric shades, and wall sconces.
  - A fluorescent *réglette* over the bathroom mirror or the worktop in unrenovated houses, cool white.
  - A lantern by the front door.
  - The halogen uplighter (*lampadaire*, R7s, usually with a dimmer) of the 1990s–2000s, still in some living rooms, now as an LED uplighter.

## 2. Fittings and how they spread light

### 2.1 Distribution classes

Luminaires are classed by the share of their light that goes below the horizontal [45]:

- direct: 90–100 %;
- semi-direct: 60–90 %;
- general diffuse (or direct-indirect): 40–60 %;
- semi-indirect: 10–40 %;
- indirect: 0–10 %.

EN 13032-2 and CIE 040 encode the downward share in finer cones (0–41.4°, 0–60°, 0–75.5°) as flux codes [46]. The figures above come from secondary sources; the CIE original was not read.

### 2.2 What a shade does

No measured light-output data for household shades was found (§9). The splits below are geometric estimates.

- **Fabric drum** (40 cm across, 25 cm tall, bulb at its middle). Each opening subtends a half-angle of 58° from the bulb, so 23.5 % of the light leaves directly through each opening. The other 53 % meets the fabric. Fabric on a styrene backing transmits perhaps 20–40 % (no measured value found). The white lining sends much of the rest out of the openings. Result: about 35–45 % down, 30–40 % up, 10–20 % through the fabric, about 10 % absorbed.
  - The fabric glows. A 400 lm table lamp sending 15 % through 0.19 m² of shade gives about 100 cd/m².
- **Opaque cone or dome** (enamel or metal, open at the bottom). Everything leaves downward, within a 45–60° half-angle of the rim, at 75–85 % of the bulb's lumens. The outside of the shade is dark (CIE "direct").
- **Opal glass globe.** Close to isotropic, at 80–90 % of the bulb's lumens (CIE "general diffuse"). An 18 cm globe around 1,000 lm is about 2,700 cd/m². Forge's pendant globes are this kind (isotropic, 9 cm radius).
- **A flush ceiling light (*plafonnier*)** sends all its light down with a near-cosine distribution. This is Forge's `Emission::flush`.

### 2.3 Fittings, lumens and distributions

| Fitting | Lumens | Where the light goes | Forge model |
|---|---|---|---|
| Pendant, opaque cone or dome | 470–1,055 lm bulb | down only, 50–60° half-angle | `Cone` down, inner cos 0.6, outer cos 0.4, candela ≈ lm/4 |
| Pendant or table lamp, fabric drum | 400–1,055 lm bulb | ~40 % down, ~35 % up, ~15 % sideways; the shade glows ~100 cd/m² | needs a two-cone profile (§8.4) |
| Opal globe pendant | 806–1,500 lm | every way (as now) | isotropic, candela = lm/4π |
| Iron chandelier, 3–6 bare E14 candles | 250–470 lm each, 2,200–2,700 K | every way, from many small bright points | several small isotropic lamps |
| Flush ceiling light | 600–2,000 lm [9] | down, cosine | `flush(−Y)`, candela = lm/π |
| Wall sconce (up/down or half-dome) | 200–600 lm | half-space off the wall, or scallops up and down | `flush(facing)`, or two cones |
| Floor uplighter | 1,500–3,000 lm (LED) | ~90 % up (indirect) | `Cone` up |
| Worktop strip under cabinets | 220–540 lm per strip, 2,700 K, CRI > 90 [8] | down, cosine, onto the worktop | `flush(−Y)` line of lamps |
| Bathroom mirror light | 400–1,000 lm, 3,000–4,000 K | forward from the wall, cosine | `flush(facing)` |
| Outdoor wall lantern by a door | 300–800 lm, 2,700 K (2,200 K filament) | every way below its cap | `Cone` down to 100° |

Unity's HDRP documentation gives the same orders of magnitude [19]:

- "ceiling lamp for a regular room": 400–800 lm;
- "large brightly lit room": 800–1,200 lm;
- decorative lamp: 200–300 lm;
- small LED: under 100 lm.

### 2.4 Why the distribution matters from outside

A shaded lamp lights a pool on the floor or table and a ring on the ceiling. The walls at window height stay in half-light. A bare isotropic bulb of the same lumens lights the walls and the window plane directly. Seen from the street, a room lit by shaded lamps shows a dimmer glass with one glowing shade in it. A room lit by a bare bulb shows a bright, even rectangle.

## 3. Rooms: how many lamps, which are on, how bright

### 3.1 Recommended levels

No European standard covers homes. EN 12464-1 is titled "Lighting of work places", and its office and kitchen values (300–500 lux) are task values. The IES Lighting Handbook (10th edition, Table 33.2, ages 25–65) gives residential values. They were read in DOE's Building America guideline [12], whose table layout is broken in the PDF. The row mapping follows a reproduction of the same table [13].

| Room | IES horizontal | IES vertical |
|---|---|---|
| Living room, general | 30 lux | 30 lux |
| Family room, general | ≈ 100 lux | 40 lux |
| Bedroom, general | 50 lux | 30 lux |
| Kitchen, general | 50 lux | 20 lux |
| Kitchen, prep counter | 500 lux | 80 lux |
| Bathroom, vanity (grooming) | 300 lux | 400 lux |
| Bathroom, toilet / shower | 100 / 50 lux | 30 / 20 lux |
| Dining, formal / informal | 50 / 100 lux | 20 / 40 lux |
| Hallway | 30 lux | 6 lux |
| Stairs | 50 lux | — |
| Foyer at night | 30 lux | 10 lux |
| Outside, primary entry | 9 lux | 16 lux |

Older guides and shop sites quote 100–300 lux for living rooms. Those values are task or design figures, not evening ambient light.

### 3.2 Measured evening light in homes

- **IES/IALD pilot, 30 homes (mostly US), light at the eye, "normal evening lighting"** [11]:

  | Room | Median | Range |
  |---|---|---|
  | Living room | 23 lux | 3–410 |
  | Kitchen, all lights on | 104 lux | 6–485 |
  | Bed, reading with a task light | 15 lux | 1–347 |

  Street light reaching the eye through a living-room window, with the room lamps off: median 0.5 lux, 20 lux at most. The authors note that the medians sit near the minimums: "high light levels in residential applications are likely more the exception than the rule".
- **Melbourne, 59 homes, 2019** [10]. A chest sensor recorded the 3 hours before bed.
  - Mean melanopic EDI: 17.9 lux (3.9–77.4). With melanopic-to-photopic ratios of 0.44–0.55, that is roughly 30–40 photopic lux at the chest.
  - LED and fluorescent homes had nearly twice the melanopic light of incandescent homes.
- **Forge today.** A 16 m² room gets a 1,120 lm isotropic pendant. That gives about 40–50 lux on every wall in a whitewashed room. It is 1.5–2 times IES and the measured medians, and lit evenly where real rooms have pools.

### 3.3 Lamps per room and hours of use

- **UK, Household Electricity Survey** [15]. Mean lamps (bulbs) per room: kitchen 4.7, living room 4.5, bedroom 1.9.
  - The evening peak moves from 18:00 in December to 22:00 in June.
  - 58 % of homes have outdoor lights, 2.2 bulbs on average where present.
- **US, PNNL's end-use study** [16]:
  - 67.4 lamps per home;
  - 4.1–7.9 per living room by state;
  - daily hours of use per lamp: kitchen 2.3, living room 1.7, bedroom 1.2, bathroom 1.2, hall under 1, outdoor 2.9.
- **France:** about 25 light points per home [17].
- **What this implies (inference).** A living room has about 4.5 bulbs, each used 1.7 h a day, over 4–5 dark hours of use. That means about 1.5–2 bulbs are on at a time: one or two fittings, not all of them.

### 3.4 Which lamps are on in an evening

- **Who is home decides.** The UK and Loughborough lighting models put lights where active occupants are and daylight is short [15][18]. The PNNL hours above say the same.
- **A typical evening in an occupied home** (estimates):
  - kitchen and living room from dusk to about 21:00;
  - the living room's one or two lamps until bedtime;
  - bedrooms with their bedside lamps for the last 30–60 minutes;
  - the bathroom in short bursts;
  - the hall only while someone passes.
- **A television** adds a median of 2 lux at the eye (up to 139) in the living room [11]. Seen from outside, it is a bluish flicker.

### 3.5 Dimming

No European or French survey of dimmers was found. An Australian survey found dimmers in 33–50 % of homes [55], which does not carry over to France. French wall switches are usually plain on/off. Halogen and LED uplighters usually have their own dimmer.

Forge default (estimate): about 20 % of lamps dimmed to 30–60 %, shifted warmer (2,200–2,500 K).

## 4. A lit window seen from the street

### 4.1 The physics

A matte surface lit at E lux shows L = ρE/π cd/m². A window of area A and average luminance L sends πLA lumens into the street.

| Window | How it works | Average luminance |
|---|---|---|
| Bare glass into a lit room | Walls at 20–50 lux (ρ 0.75) seen through glass (τ 0.9): L ≈ 0.9 × 0.75 × E/π | 4–11 cd/m², plus the lamp or its shade as a highlight |
| Sheer curtain (*voilage*) | Diffusing transmittance 0.4–0.7 (shops claim 60–80 % [44]), lit from inside at 20–40 lux: L ≈ τE/π | 3–8 cd/m² |
| Opaque curtain or roller blind | τ 0.02–0.1 | 0.1–1 cd/m², bright edges |
| Louvred shutters (*persiennes*) closed | Slats tilted; 10–30 % of the opening seen through at a slant | 0.2–1 cd/m², in stripes (estimate) |
| Solid or cross-slat shutters (*volets pleins*, *à lames croisées*) closed | Light only at the cracks | 0.01–0.1 cd/m² average; thin lines 1–5 cd/m² (estimate) |

### 4.2 Measurements

- **Ostrava, blocks of flats** [30]. The measured average lit-window luminance was **6 cd/m²**. The authors modelled the window as a cosine (Lambertian) radiator "because of the prevailing tradition of using curtains and drapes". With 1.44 m² per window, that is 27 lm per window. At most 50 % of the windows were lit when residents were most likely at home.
- **A residential building, 2025** [31]. A window with horizontal blinds measured 4.3 cd/m², and a less shaded variant 8.3 cd/m². An earlier study measured 1.2 cd/m² for a shaded window.
- **An upper bound used in planning** [32]. 500 lux on the glass, at visible transmission 1, gives 160 cd/m². This is a worst case for skylights and full glazing, not a typical home window.
- **Forge's faked windows** sit at about 5 cd/m² average (with 45 % behind net curtains in `meshlet.slang`). That is the measured value.

### 4.3 Coverings in France and Provence

- **Shutters.** Over 80 % of French homes have them. France is said to be the European country where they are closed every evening whatever the season [42]; this is popular press, not a survey.
- **Provençal shutters.** In the Var, shutters are solid (*pleins*) or louvred (*persiennes*), the louvres fixed, tilting or top-hung. The cross-slat panel is "known as the Provençal shutter". Persiennes appeared in the mid-18th century and became characteristic of southern towns because they keep out the sun but let in air and light [43]. Shutters are seen "open, closed or ajar".
- **Sheers.** The owner has said many French homes keep sheers so the street cannot see in (comment on `ROOM_CURTAINS` in `meshlet.slang`).
- **Seasons.** In summer, windows stand open behind louvres or ajar shutters for the cool. In winter, shutters close at dusk.

### 4.4 Light thrown on the street

- **A curtained window spills little light.** At 6 cd/m² over 1.44 m², its intensity is 8.6 cd straight out. That gives about 1 lux on the paving 1.5 m out from a ground-floor window and 0.3 lux at 3 m. A facade 4 m across the lane gets about 0.5 lux. All of this is well under the street lamps.
- **A bare bulb near the glass casts a sharp patch.** A bare 1,100 lm isotropic bulb 2 m inside an uncurtained window sends about 2–5 lux onto the paving outside, in a sharp-edged patch the shape of the window: a spotlight.
- **Forge today.** House lamps light only their rooms' regions (D-068; `lamps.rs`), so they throw no light on the paving directly.
- **Limits for comparison.** The ILP's limits for light entering windows from outdoor installations are 2 / 5 / 10 / 25 lux before curfew in zones E1–E4 [33]. Its average limits for floodlit facades are 5 / 10 / 25 cd/m² in E2–E4. Lit windows sit at E2–E3 facade levels.

### 4.5 Practicals in film and photography

- **Film.** Cinematographers set the hot spot of a shaded practical lamp "2½ to 3 stops above exposure". One took sconces to 2.5 stops over key and would now leave them "around 4.0 over" [28].
- **Photography.** ANSI's exposure tables give these EV100 values [29]:

  | Scene | EV100 |
  |---|---|
  | Home interiors | 5–7 |
  | Night street scenes and window displays | 7–8 |
  | Floodlit buildings | 3–5 |
  | Distant view of lit buildings | 2 |
  | Landscape under a full moon | −3 to −2 |

  Forge's EV100 −2 is the moonlit-landscape exposure. A photographer framing lit houses would give 4 stops less exposure.
- **Twilight photography.** Estate photographers shoot houses at blue hour, when the sky's luminance has come down to the interiors' [web, search-recorded].

### 4.6 How many windows are lit, and when

- **Over the night.** In A Coruña, light from residential windows was 22 % of the zenith sky brightness at the start of astronomical night. It fell steadily until about 04:00 [34]. The authors suggest using blinds and curtains more to cut it.
- **At the peak.** The Ostrava model caps lit windows at 50 % [30]. Village houses have more rooms per person than flats, so the share of street windows lit is lower.
- **Street lamps.** In France, 55 % of public lamps are switched off in the middle of the night (AFE, 2024) [39].

## 5. Street lighting in villages

### 5.1 EN 13201-2 classes for residential and pedestrian streets

The values are maintained average and minimum horizontal illuminance on the lane [36]. The class columns of the 2015 table, as read in a manufacturer's presentation, are below. Vertical and semicylindrical minimums also apply.

| Class | Average (lux) | Minimum (lux) |
|---|---|---|
| P1 | 15 | 3.0 |
| P2 | 10 | 2.0 |
| P3 | 7.5 | 1.5 |
| P4 | 5 | 1.0 |
| P5 | 3 | 0.6 |
| P6 | 2 | 0.4 |

Village lanes for pedestrians usually fall in P4–P6. The classes are chosen by traffic, setting and how lit the surroundings are (CEN/TR 13201-1, not read).

### 5.2 French rules: the decree of 27 December 2018 [37]

- **Colour temperature:** at most 3,000 K. 2,400 K in nature reserves. 2,700 K in a national park's core inside built-up areas, 2,400 K outside them.
- **Upward light:** under 1 % nominal for new luminaires and under 4 % as installed. More than 95 % of the downward flux must fall within the CIE flux code 3 cone.
- **Flux density:** at most 35 lm per m² of lit surface inside built-up areas, 25 outside them. Where accessibility rules apply, the average must stay under 20 lux.
- **Switching off:** no fixed hours for public lighting. The commune decides, and switching off in the middle of the night is encouraged.

**The national picture.**

- **2024:** 11.6 million public lamps, 48.6 % LED, 90 W on average, and 55 % switched off in the middle of the night (AFE, *Les chiffres de l'éclairage*) [39].
- **Switch-off by commune:** about 12,000 communes practised it in 2018 (ANPCEN) [40].
- **Dark corridors:** the *trame noire* adds them for wildlife to the *trame verte et bleue* (OFB and Cerema; not read here).

### 5.3 Village practice

A regional nature park's guidance (Haute Vallée de Chevreuse) [38]:

- **Old centres:** wall brackets on the buildings rather than posts, mounted below the gutter and under 5 m. At most 15 lux on the road and pavements. Flat clear glass, light aimed at the ground.
- **Housing estates:** 5 m posts, at most 10 lux.
- **Roads and village entries:** 7–8 m posts, at most 15 lux.
- **Light colour:** at most 3,000 K, "a very warm and rather soft atmosphere".
- **Switch-off:** some communes switch everything off at midnight on weekdays.

**Spacing.** A French vocational exercise uses spacing of 3 times the mounting height [53]. Staggered lanterns at 4–5 m sit every 13–20 m.

**Luminaire flux (estimate from the classes).** A 4 m lane with lamps every 15 m is 60 m² per lamp. At 5 lux (P4) with a utilisation factor of 0.3–0.4 (much of the light goes onto the facades), each lamp needs 750–1,000 lm. P3 needs 1,100–1,500 lm. The 35 lm/m² cap allows up to 2,100 lm. Catalogue lanterns start around 1,900 lm [54].

**Under the lamp.** A 1,000–1,500 lm lantern at 4.5 m gives 10–20 lux directly beneath it.

### 5.4 Forge's street lamps against these

- **What Forge has.** The lens is 1,000 cd/m² over 0.134 m², a peak of about 112 cd. Its full cut-off cone carries about 500 lm. The lamps are 8 m posts giving 1.9 lux beneath, below P6.
- **What the owner turned down.** On 2026-10-18 they turned down 4 and 6 times that (7.5 and 12 lux beneath), at EV100 −2. Real village lanterns give 10–20 lux beneath.
- **The likely cause.** At EV100 −2, 7.5 lux on paving (ρ 0.35, 0.84 cd/m²) displays at 70 % of white. The same exposure explains §4's windows (see §8.1).

## 6. How games and engines set these

### 6.1 Units

- **Frostbite** moved to physical units: lumens for lights, lux for the sun, luminance for emissive surfaces, and a camera exposure in EV100 [22]. Unity HDRP's light code cites Frostbite for its conversions, point lights in candela = lumens / 4π [23].
- **Unreal** lights point, spot and rect lights in candela, lumens or "unitless" [20].
  - A point light: 1 cd ≈ 12.6 lm.
  - A spot light keeps its lumens over its cone, so a narrower cone is brighter.
  - Emissive surfaces are in cd/m² (nits).
  - The extended luminance range lets "physically correct light values" be used without blowing out [21].
- **Unity HDRP** [19] gives lumen classes (§2.3). Its exposure cheat sheet, in EV100:

  | Scene | EV100 |
  |---|---|
  | Moonless | −2 |
  | Moonlit | 1 |
  | Interior | 4 |
  | Low sun | 7 |
  | Cloudy | 10 |
  | Sunlit | 14 |

### 6.2 Exposure at night

- **Unreal** adds an exposure compensation curve over the average scene EV100: artists can make night scenes darker than full adaptation [21]. Its local exposure (bilateral or fusion) takes highlight and shadow contrast "typically 0.6 to 1".
- **Ghost of Tsushima** [24]:
  - meters exposure "primarily" on illuminance, using luminance only when highlights exceed a threshold;
  - models the Purkinje shift (one comparison slide shows a scene at an average illuminance of 0.05 lux);
  - tone-maps locally with a 64 × 32 bilateral grid blended 40/60 with a wide Gaussian, "to maintain visibility while avoiding blow-outs".
- **Physically, night lies in mesopic vision**: the CIE system runs from 0.005 cd/m² (fully scotopic) to 5 cd/m² (fully photopic) [41]. ILP's glare rules assume an eye adapted to 0.1 cd/m² on roads without lighting and 1 cd/m² on lit residential roads [33].

### 6.3 Windows at night in shipped work

- **Epic's City Sample.** Its emissive windows suit the day but are "far too bright" at night. A blog's fix scales them by "0.1 or lower" [25], about 3.3 stops. A commenter lowers the interior material's own exposure instead.
- **Cyberpunk 2077's lighting talk** (GDC 2022, Knapik, Yablonsky, Poo) covered an interior case and an open-world case [26]. Its content was not available.
- **Marvel's Spider-Man** draws rooms behind windows, very likely with interior mapping. Insomniac has not said how [27].
- **Not found:** no published talk on night windows from Red Dead Redemption 2 or The Last of Us Part II.

### 6.4 What this says for Forge

1. Shipped games dim night windows by 3 stops or so. Done by scaling the emissive, that breaks the physical units. Done through exposure, it does not.
2. Highlights or illuminance, not a log average of mostly dark pixels, should key a night with lamps.
3. Local tone mapping is what keeps bright interiors and dark exteriors in one frame.

## 7. Patents

Lumens, lux, cd/m² and the standards' classes are facts and cannot be patented. The searches here were short (§9).

| Patent | Holder | Jurisdiction | Status | Expiry (estimate) | What it covers | Bearing, and the alternative |
|---|---|---|---|---|---|---|
| US 10,380,790 B2 [47] | Robert Bosch | US | granted 2019-08-13, active (Google) | ~2035-12-22 | Claim 1 (read): a facade texture with a window cutout layer; floors as unique texture-space coordinate ranges; per floor, a pseudo-random function along the horizontal compared with a threshold decides which windows are lit; a window lighting layer combined with the facade | Already on record ([glass-credibility.md](glass-credibility.md)). Take each window's lit state and covering from its house's rooms, lamps and household routine, keyed by world position. Never from texture-space floor ranges or a pseudo-random "curtain function" along a floor. |
| EP 3238179 B1 [48] | Robert Bosch | EP (FR, DE, GB and others designated) | **granted 2026-02-11** (B8 correction 2026-04-08); Google lists lapses in several states (ES, PL, NL, SE, AT…); FR not checked | ~2035-12-22 | Claim 1 (Google's text, read): as the US claim, plus ray intersections with interior walls, floors and ceilings, a pseudo-random "window curtain function" per floor, room boundaries from its minima, and an in-vehicle navigation application | **New on record:** the European member was granted this year. It is narrower (in-vehicle navigation). Same alternative as above. Its "curtain function" is a lit/unlit pattern, not physical curtains. Forge's per-window coverings are not it. |
| US 11,116,060 B2 [50] | Signify | US | granted 2021-09-07; "Expired – Fee Related" (Google) | (2038-01-20 adjusted) | Virtual occupant profiles drive a presence simulator that controls physical luminaires | None. It claims real luminaires and has lapsed per Google. Forge's routines lighting rooms are a simulation in software. |
| US 6,377,858 [51] | not checked | US | probably expired (late-1990s filing; not checked) | — | Recording and replaying a dwelling's device on/off events to simulate occupancy | None. Prior art for occupancy-driven lights. |
| US 12,293,452 B2 (Sony); US 12,700,170 B2 (Celsys) | Sony; Celsys | US (Celsys also JP, CN; EP pending) | active | ~2039; ~2042 | Stacked painted room layers; a see-through door and swapped interiors | On record ([city-generation.md](city-generation.md), [glass-credibility.md](glass-credibility.md)). Untouched by this note. |
| US 11,756,172 (STMicroelectronics) | STMicroelectronics | US | active | — | Local tone mapping for HDR video | On record ([lighting-patents.md](lighting-patents.md)). Raising D-067's night darkening or its highlight contrast stays within D-067 as built. Its three conditions still hold: no grid history, no gain lerp, no stored gain map. |
| Exposure keyed on highlights or illuminance at night | — | — | no patent found (two searches) | — | — | Published by Ghost of Tsushima [24] and Unreal [21]; no patent found for either in [local-exposure.md](local-exposure.md). |
| Luminaire intensity profiles by angle (IES LM-63, EULUMDAT) | — | — | open file formats, decades old; not searched | — | — | A 1D intensity table per fitting type is ordinary photometry. |

## 8. Recommendations for Forge

### 8.1 Why the windows read as spotlights

Forge's exposure maps the luminance 1.2·2^EV100 to 1, then applies ACES 2.0 SDR at 100 nits. The displayed values below were computed with OCIO's achromatic ACES 2.0 tone scale (chroma compression left out). "Lifted" means local exposure raising large dark areas by 3 EV (D-067's `max_lift`). Small windows are not lifted.

| Surface | cd/m² | Shown at EV100 −2 (now) | at EV100 +1 | at +1, large dark areas lifted 3 EV |
|---|---|---|---|---|
| Moonlit grass (0.12 lux, ρ 0.2) | 0.008 | 0.5 nit | 0 | 0.5 |
| Moonlit limestone wall | 0.017 | 2 | 0 | 2 |
| Paving under today's street lamp (1.9 lux) | 0.21 | 37 | 4 | — |
| Lane at P4 (5 lux, ρ 0.35) | 0.56 | 62 | 13 | — |
| Facade under a 4.5 m lantern (15 lux, ρ 0.45) | 2.1 | 87 | 43 | — |
| Window, solid shutters (cracks, average) | 0.05 | 9 | 0.3 | — |
| Window, louvred shutters closed | 0.5 | 59 | 12 | — |
| Window, opaque curtain | 0.6 | 64 | 15 | — |
| Window, sheer, dim room | 3 | 91 | 52 | — |
| Window, sheer (Ostrava average) | 6 | **96** | **69** | — |
| Window, bare, 50-lux room (Forge's rooms now) | 10.7 | **98** | **81** | — |
| Fabric shade of a 400 lm table lamp | ~100 | 100 | 98 | — |

**The answer to "how bright is a lit window from the street now".** At EV100 −2:

- a sheer-curtained window (3–6 cd/m²) shows at 91–96 % of white;
- a bare window into Forge's 50-lux rooms (≈ 11 cd/m²) shows at 98 %;
- both are close to white, and ACES drains their colour on the way.

**Everything is squeezed into the top of the curve.** A lamp-lit wall at 0.3 cd/m² already shows at 46 nits. That is why lit rooms at 4 % (0.12–0.3 cd/m²) "did not look lit" from the pavement (comment on `ROOM_LAMP` in `meshlet.slang`), and why 12 cd/m² blew out.

**The key.** A log-average meter is ruled by the moonlit majority of the frame. With 5 % of the frame lamp-lit at 0.5 cd/m² and 1 % windows at 5 cd/m², it moves from EV −4.0 to −3.6. With 15 % and 3 %, it reaches only −2.8 (computed).

### 8.2 Exposure (first, because everything else is judged through it)

1. **Key the night on the lamp-lit pixels where lamps are on.** Place the brightest few per cent of metered samples (lit windows, lamp-lit walls) 3–4 EV over middle grey, after the practicals rule [28].
   - A 5 cd/m² window at +3.5 EV gives **EV100 ≈ +1** (a 3–6 cd/m² window at +3 to +4 EV gives −0.2 to +1.8).
   - Keep the moonlit key as the floor where no lamp is lit.
   - First step: a console setting (D-069), e.g. `r.night_lamp_ev 1.0`, applied as a floor on the night target while the frame's light lists hold street lamps or lit house lamps. Then A/B it with the owner.
   - Ghost of Tsushima's highlight-threshold metering is the published model for doing it automatically [24].
2. **D-046's "2 stops under what the eye adapts to" still holds** if the eye's adaptation is taken as the lamp-lit field, not moonlight alone. EV100 +1 matches a field averaging about 0.06 cd/m² darkened by 2 stops. Unity's "Moonlit" sits at EV 1 [19], and ANSI's lit buildings at EV 2 [29].
3. **Let local exposure keep the countryside readable.** Lifting large dark areas by up to 3 EV gives moonlit fields and walls what they show today (0.5–2 nits). The bilateral grid works at region scale, so it lifts fields and sky, not single windows.
4. **Tie the Purkinje shift to the adaptation luminance.** A lane lit at 0.2–1 cd/m² is mesopic, near photopic [41]. Only moonlit areas should go blue.
5. **Keep bloom at 1 % at night.** Lit windows at 4–6 cd/m² do not glare.

### 8.3 Lumens per fitting, not per square metre

Replace the 70 and 93 lm/m² rules with fittings chosen per room. All values are physical lumens.

| Room | Fittings (lumens, colour temperature) | On at a time | Target |
|---|---|---|---|
| Living room (*séjour*) | ceiling pendant or chandelier 800–1,500 lm (or 3–6 E14 candles of 250–470 lm), 2,700 K; 1–2 table or floor lamps 400–800 lm, 2,700 K (2,200–2,500 K dimmed); an uplighter 1,500–2,500 lm in about 20 % of homes | 1–2 fittings; the ceiling light in about 40 % of evenings | 15–40 lux; walls 3–8 cd/m² |
| Dining table | pendant 470–1,055 lm, 0.7 m over the table, 2,700 K | while eating | 50–100 lux on the table |
| Kitchen | ceiling 1,000–2,000 lm, 3,000 K (4,000 K tube in about 15 % of old kitchens); worktop strips 300–600 lm | ceiling whenever occupied; strips in about half | 60–150 lux; 200–400 on the worktop |
| Bedroom | ceiling 470–806 lm (on briefly); 1–2 bedside lamps 250–470 lm at 2,400–2,700 K | bedside lamps for the last 30–60 min | 10–40 lux |
| Bathroom | ceiling 600–1,000 lm plus a mirror light 400–800 lm, 3,000–4,000 K, CRI 90 | in short bursts | 100–200 lux; 300 at the mirror |
| Hall and stairs | 470–806 lm per storey (Forge now: 500 lm, 3,000 K) | 1–5 min when passing; a 30–100 lm night light in some homes | 30–50 lux when on |
| Front door | wall lantern 300–800 lm, 2,700 K (2,200 K filament) | in 30–50 % of homes until bedtime; 5–10 % all night | 9 lux on the step [12] |

Forge's sconces (250 lm at 2,400 K over beds, 400 lm at 2,700 K over sofas) already fit this table.

### 8.4 Distributions to model

Most of the fittings in §2.3 fit one model: an axially symmetric intensity profile by vertical angle, for example 9 samples from 0° to 180° (a coarse IES file), normalised to the fitting's lumens. Four profiles cover a village:

- **opaque cone:** down only, 0–60°;
- **fabric drum:** two lobes, down 0–58° and up 122–180°, with a dim band between at about 1/6 of the downward intensity;
- **globe:** isotropic, as now;
- **uplighter:** up only.

The existing `Emission::Cone` and `flush` cover the cone, the ceiling light, the sconce and the strip. Give each drum shade a glowing emissive of about 100 cd/m² (§2.2). Seen from outside, the shade is the highlight.

### 8.5 Windows: coverings, and which are lit

- **A covering per window**, chosen by the household (not texture-space floors, §7). The night luminances to use, for real and faked rooms alike:
  - bare: 4–11 cd/m², from the room as lit;
  - sheer: 3–6 cd/m² (keep about 5 cd/m² as now);
  - opaque curtain or blind: 0.3–1 cd/m²;
  - louvred shutters: 0.2–1 cd/m², in stripes;
  - solid shutters: 0.02–0.1 cd/m², with crack lines up to 5 cd/m².
- **The coverings at night in a Provençal village** (Forge defaults to tune, not survey results):

  | Covering | October–April | June–September |
  |---|---|---|
  | Solid or cross-slat shutters closed | 35 % | 10 % |
  | Louvred shutters closed | 25 % | 40 % (or ajar) |
  | Sheer only | 25 % | 35 % (window open) |
  | Opaque curtain or blind | 10 % | — |
  | Bare | 5 % | 15 % |

  Extend `ROOM_CURTAINS` (0.45, the owner's sheers) with shutters.
- **How many windows are lit** (estimates, anchored on [30], [34] and [15]):
  - 15–30 % of street windows lit at 21:00 at the peak;
  - 10–15 % at 23:00;
  - 2–5 % after 01:00;
  - a small morning peak in winter.
  - The population's routines already decide who is at home and awake: light only their rooms (§3.4).
- **A TV glow:** a bluish flicker of 1–3 cd/m² in 10–20 % of lit living rooms (estimate).

### 8.6 Street lamps, once §8.2 is in

- **Old centre:** wall-bracket lanterns at 4–5 m (not 8 m posts), staggered every 15–20 m. 1,000–1,500 lm each at 2,700 K (2,200 K for a dark-sky village). Full cut-off.
  - Result: 3–7.5 lux on the lane (P5–P3) and 10–20 lux beneath.
  - In Forge's lens model: about 5–8 times today's lens luminance per lamp, or the same lens with the real flux.
- **The square and the bypass road:** 6–8 m posts, 2,500–4,000 lm, at most 15 lux [38].
- **Night switch-off:** about half the lamps off from midnight to 05:00 where the commune does it [39], as a map setting.
- At EV100 +1, a P4 lane shows at about 13 nits, near middle grey: about what today's dim lamps show at EV −2.

### 8.7 The expected result

At EV100 +1 with the changes above (computed, §8.1):

| Element | Display |
|---|---|
| Curtained window | 50–70 nits, warm |
| Bare window into a 30-lux room | ≈ 71 nits, with the shade glowing near white |
| Shuttered window | 0.3–12 nits, in stripes or lines |
| Lamp-lit lane | 10–15 nits |
| Facade under a lantern | ≈ 40 nits |
| Moonlit fields (lifted) | 0.5–2 nits, as today |

Read top to bottom, that is how a village looks at night: lamps, then windows, then the lit lanes, then the dark around them.

### 8.8 How to check

1. One fixed night view of the village from the street, 3 frames, fixed seed: (a) today; (b) the new key with local lift; (c) b with physical street lamps. Show them to the owner (memory: compare the changed images, push after the gate).
2. Measure the displayed values of a curtained window (target 50–75 nits), a shuttered window (under 20), the lane under a lamp (8–25) and a moonlit field (0.5–5).
3. Run a still-camera and a walking capture to check that the new key does not pump when a lamp enters or leaves the frame. It must adapt with the existing speeds.

### 8.9 What not to do

- Do not scale window emissives by 0.1 as the City Sample fix does [25]. It breaks D-046's absolute emission and the match between a room's lamp and its window.
- Do not cap emissive luminance. Keep the faked windows' values physical (§8.5) and move the exposure.
- Do not choose lit windows by a texture-space pseudo-random test (§7).

## 9. Verification notes

- **Not read:**
  - Frostbite's course notes: both PDFs exceed the fetch limit; cited from search [22][23].
  - The PNAS/Nature page (403; read through Europe PMC instead).
  - The MDPI 2025 window study (403) [31].
  - EN 13201-2, EN 12464-1, IES RP-11-26 and CIE 150:2017 (paid; secondary sources used).
  - Regulation 244/2009 and 1194/2012's equivalence table (a secondary table used [2]).
  - Richardson et al. 2009 (403) and the JOE reproduction of IES Table 33.2 (host not found).
- **No data found:**
  - measured light output and up/down splits of household shades (§2.2 is geometry);
  - lampshade fabric transmittance;
  - surveys of French dimmer use, LED share in homes, or how many households close their shutters each night;
  - published talks on night windows from Rockstar, Naughty Dog or Insomniac.
- **Estimates:** every share labelled "estimate" or "Forge default" in §1.3, §2, §3.4, §4.1 and §8 is a starting value to tune against the owner's eye, not a measurement.
- **Computations:**
  - Displayed values use OCIO's ACES 2.0 tone-scale constants (100-nit peak, achromatic path), with Forge's exposure mapping from `exposure.rs`.
  - The metering experiment in §8.1 is a log average over three made-up frames.
  - Forge's values are taken from `crates/forge-procgen/src/city/furnish.rs`, `crates/forge-render/src/lamps.rs` and `shaders/meshlet.slang`.
- **Patent searches:** four queries on Google Patents and the web, plus the two Bosch family pages read. Not exhaustive. EP 3238179 B1's French validation was not checked in the EPO register.

## Sources

**Lamps, regulations, data sheets**

1. LightingEurope. "One month to go before most general lighting lamps are phased out." Press release and timetable, 25 July 2023. [web] <https://www.lightingeurope.org/images/publications/pr/LightingEurope_-_Press_Release_-_Phase-out_of_most_general_lighting_conventional_lamps_1_month_away_-_20230725.pdf> — read
2. Selectra. "60W equivalent: what the LED box does not tell you" (the EU lumen-to-watt table). [web] <https://selectra.co.uk/energy/news/60w-equivalent-what-the-led-box-does-not-tell-you> — search-recorded
3. Commission Regulation (EU) No 1194/2012, Annex III (revoked 2021). [standard] <https://www.legislation.gov.uk/eur/2012/1194/annex/III/division/3> — search-recorded (page read, table not shown)
4. Wikipedia. "Luminous efficacy" (overall efficacy table). [web] <https://en.wikipedia.org/wiki/Luminous_efficacy> — read
5. Signify. Philips LEDbulb 7 W (60 W) A60 E27 2700 K, 929001243082: 806 lm, 115 lm/W, CRI 80, class E. [data sheet] <https://www.signify.com/global/prof/led-lamps-and-tubes/led-bulbs/philips-classic-frosted-led-lamps/929001243082_EU/product> — read
6. LEDVANCE. CLAS A CL 60 W 230 V E27: 710 lm, 11 lm/W, 2700 K (withdrawn 2011). [data sheet] <https://ledvance.com/pl/product-datasheet/7523/102819> — search-recorded (403)
7. E14 candle LEDs: Xanlite 470 lm 4 W 2700 K (Bricocash); Miidex 71302 470 lm 5 W CRI > 80; 250 lm 2.5 W (ManoMano). [data sheet] <https://www.bricocash.fr/p/ampoule-filament-led-flamme-opaque-culot-e14-470-lumens-blanc-chaud/3700619422589>, <https://res.cloudinary.com/afdb/raw/upload/s--HmpThVoq--/v1/documents/342462_71302-ft.pdf> — search-recorded
8. IKEA. MITTLED worktop lighting strip: 220–540 lm (30–80 cm), 2700 K, CRI > 90. [data sheet] <https://www.ikea.com/ie/en/p/-70528611/> — search-recorded
9. Philips Moire CL200 ceiling lights: 600, 1,700 and 2,000 lm at 2700 K (retail listings). [data sheet] <https://www.brico.be/fr/eclairage-luminaire/eclairages-interieur/plafonniers/plafonniers-basics/plafonnier-philips-moire-cl200-ciel-etoile-38cm-blanc-chaud-20w/10609686> — search-recorded

**Homes: levels, surveys, use**

10. Cain S.W. et al. "Evening home lighting adversely impacts the circadian system and sleep." Scientific Reports 10, 19110 (2020). [paper] <https://www.ebi.ac.uk/europepmc/webservices/rest/PMC7644684/fullTextXML> — read
11. Miller N.J., Kinzey B.R. "Home Nighttime Light Exposures: How much are we really getting in our residences?" LD+A, July 2018 (PNNL/DOE). [paper] <https://www.energy.gov/sites/prod/files/2018/07/f53/Home%20Nighttime%20Light%20Exposures.pdf> — read
12. Holton J. (IBACOS). "Strategy Guideline: High Performance Residential Lighting." DOE Building America, February 2012 (Table 1 from IES Lighting Handbook Table 33.2). [docs] <https://www1.eere.energy.gov/buildings/publications/pdfs/building_america/strategy_guideline_high_perf_lighting.pdf> — read
13. Home World Design. "How Bright Should a Room Be?" (reproduces the same table). [web] <https://homeworlddesign.com/how-bright-should-a-room-be/> — read
14. Journal of Extension, October 2013, v51 5tt7 (IES 10th edition Table 33.2 by age). [paper] <https://archives.joe.org/joe/2013october/pdf/JOE_v51_5tt7.pdf> — search-recorded
15. Terry N., Palmer J., Godoy D., Firth S., Kane T., Tillson A. "Further Analysis of the Household Electricity Survey: Lighting Study (Final Report)." DECC, May 2013. [docs] <https://assets.publishing.service.gov.uk/media/5a7dbc54e5274a5eaea66049/Lighting_Report.pdf> — read
16. PNNL / DOE. "Residential Lighting End-Use Consumption Study: Estimation Framework and Initial Estimates." PNNL-22182, December 2012. [docs] <https://www.pnnl.gov/main/publications/external/technical_reports/PNNL-22182.pdf> — read
17. Selectra. "Combien de points lumineux… éclairage, poids dans la facture" (ADEME 2018: 25 light points; ElecDom 2023–24: 110 kWh). [web] <https://selectra.info/energie/actualites/marche/combien-de-points-lumineux-eclairage-poids-facture> — search-recorded
18. Richardson I., Thomson M., Infield D., Delahunty A. "Domestic lighting: A high-resolution energy demand model." Energy and Buildings 41(7), 2009. [paper] <https://repository.lboro.ac.uk/articles/journal_contribution/Domestic_lighting_a_high-resolution_energy_demand_model/9567218> — search-recorded

**Engines and games**

19. Unity Technologies. "Physical Light Units and Intensities." HDRP 17 manual. [docs] <https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/Physical-Light-Units.html> — read
20. Epic Games. "Using Physical Lighting Units in Unreal Engine." [docs] <https://dev.epicgames.com/documentation/en-us/unreal-engine/using-physical-lighting-units-in-unreal-engine> — read
21. Epic Games. "Auto Exposure in Unreal Engine." [docs] <https://dev.epicgames.com/documentation/en-us/unreal-engine/auto-exposure-in-unreal-engine> — read
22. Lagarde S., de Rousiers C. "Moving Frostbite to Physically Based Rendering." SIGGRAPH 2014 course notes. [paper] <https://seblagarde.wordpress.com/wp-content/uploads/2015/07/course_notes_moving_frostbite_to_pbr_v32.pdf> — search-recorded (file over the fetch limit)
23. Unity HDRP `LightUtils.cs` (lumen-to-candela conversions citing Frostbite). [code] <https://plastichub.unity.cn/unity-tech-cn/ScriptableRenderPipeline/src/branch/main...com.unity.render-pipelines.high-definition/HDRP/Lighting/LightUtils.cs> — search-recorded
24. Patry J. "Real-Time Samurai Cinema: Lighting, Atmosphere, and Tonemapping in Ghost of Tsushima." SIGGRAPH 2021 Advances. [talk] <https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html> — read
25. Versluis J. "How to dim the window lights in Epic's City Sample project." Blog, March 2024. [web] <https://www.versluis.com/2024/03/how-to-dim-the-window-lights-in-epics-city-sample-project/> — read
26. GDC. "Cyberpunk 2077: Bringing Light to Night City" (Knapik, Yablonsky, Poo), GDC 2022. [talk] <https://gdconf.com/article/find-out-how-cyberpunk-2077-lit-up-night-city-in-this-gdc-2022-session/> — read (session page only)
27. Game Developer. "Interior mapping: rendering real rooms without geometry" (on Spider-Man's windows). [web] <https://gamedeveloper.com/programming/interior-mapping-rendering-real-rooms-without-geometry> — search-recorded
28. Cinematography Mailing List. "Photographing practicals and bare bulbs." [web] <https://cinematography.net/edited-pages/Photographing-practicals-and-bare-bulbs.htm> — read
29. Wikipedia. "Exposure value" (tables from ANSI PH2.7-1973 and PH2.7-1986). [web] <https://en.wikipedia.org/wiki/Exposure_value> — read

**Lit windows, obtrusive light**

30. Novak T., Valicek P., Gasparovsky D., Raditschova J., Brzobohaty A. "Large Residential Area Light Pollution Modelling." Proceedings of the CIE 2025 Midterm Meeting, DOI 10.25039/x051.2025/kc7797 (CC BY-NC 4.0). [paper] <https://files.cie.co.at/x051_2025/P0796_ID298TomasNovak_OA_with_cover.pdf> — read
31. "Nighttime Lighting Effect on a University Residential Building." Buildings 16(21), 4179 (2025). [paper] <https://www.mdpi.com/2075-5309/16/21/4179> — search-recorded (403)
32. South Downs National Park Authority. Dark Night Skies TAN 2021, Appendix: Glazing and Internal Light. [docs] <https://www.southdowns.gov.uk/wp-content/uploads/2021/06/DNS-TAN-2021-Appendix-Glazing-Internal-Light.pdf> — read
33. Institution of Lighting Professionals. "Guidance Notes for the Reduction of Obtrusive Light" GN01:2011 (Tables 2 and 3). [standard] <https://www.folkestone-hythe.gov.uk/downloads/file/2222/9-11-guidance-notes-for-the-reduction-of-obtrusive-light> — read
34. Bará S. et al. "Estimating the relative contribution of streetlights, vehicles and residential lighting to the urban night sky brightness." arXiv 1706.04458 (2017). [paper] <https://arxiv.org/pdf/1706.04458> — read
35. Kyba C. et al. "Direct measurement of the contribution of street lighting to satellite observations of nighttime light emissions from urban areas" (Tucson). Lighting Research & Technology, 2021. [paper] <https://www.pnnl.gov/publications/direct-measurement-contribution-street-lighting-satellite-observations-nighttime-light> — search-recorded

**Street lighting**

36. Oxytech. "EN 13201:2015 — the new standard for road lighting" (P-class table). [web] <https://www.oxytech.it/PDF/EN13201-2015_The_new_standard_for_road_lighting-Rv01_210316.pdf> — read
37. Ministère de la Transition écologique. "Arrêté du 27 décembre 2018 relatif à la prévention, à la réduction et à la limitation des nuisances lumineuses." [standard] <https://www.ecologie.gouv.fr/politiques-publiques/arrete-du-27-decembre-2018-relatif-prevention-reduction-limitation-nuisances> — read
38. Parc naturel régional de la Haute Vallée de Chevreuse. "Fiche-outil n°5: L'éclairage public." [docs] <https://www.parc-naturel-chevreuse.fr/sites/default/files/5ficheoutil_eclairage.pdf> — read
39. AFE et al. "Les chiffres de l'éclairage" 2024 edition (via LightZoomLumière). [web] <https://www.lightzoomlumiere.fr/communique/eclairage-public-en-france-2024-modernisation-et-consommation/> — read
40. Maire-info, 22 March 2018 (ANPCEN: 12,000 communes switch off at night). [web] <https://www.maire-info.com/les-communes-de-plus-en-plus-sensibilisees-%C3%A0-la-pollution-lumineuse-article2-21706> — search-recorded
41. CIE 191:2010, "Recommended System for Mesopic Photometry based on Visual Performance" (via a BIPM workshop presentation). [standard] <https://www.bipm.org/documents/20126/47972998/WGSP-13-WS-08-%5Bmesopic-photometry%5D.pdf/b7de5b2e-9fa2-fe6a-b758-123354a4415f> — search-recorded

**Windows and coverings in France**

42. Le Tribunal du Net. "Pourquoi les Français ferment-ils leurs volets la nuit ?" [web, popular press] <https://www.letribunaldunet.fr/insolite/pourquoi-francais-ferment-volets-nuit-origine.html> — read
43. DRAC PACA, UDAP du Var. "Fiche conseil centre ancien 08: les volets." [docs] <https://www.culture.gouv.fr/Media/Regions/Drac-Paca/Files/Aides-demarches/Demarches-et-conseils-architecture-patrimoine/Fiches-conseil/udap-du-var/08-Les-volets> — read
44. Madura. "Which curtain to choose to let the light pass through" (sheers 60–80 %); Westwing listing (60–80 %). [web] <https://www.madura.com/en/blogs/advices-and-tips/which-curtain-to-choose-to-let-the-light-pass-through> — search-recorded

**Fittings**

45. Ohio University. "Luminaire Classification" lecture (CIE classes). [web] <https://people.ohio.edu/ziff/ART%202640%202018/288Lecture%2015-Luminaire%20Classification2020.pdf> — search-recorded
46. TRILUX. "Luminaire classification by EN 13032-2 and CIE 040-1978." [docs] <https://trilux.com/en/lighting-practice/lighting-equipment/luminaires/photometric-classification/luminaire-classification-by-en-13032-2-and-cie-040-1978> — read

**Patents**

47. US 10,380,790 B2, Robert Bosch, "System and methods for generating procedural window lighting effects." [patent] <https://patents.google.com/patent/US10380790B2/en> — read
48. EP 3238179 B1, Robert Bosch (same family). [patent] <https://patents.google.com/patent/EP3238179B1/en> — read
49. Chandler J., Yang L., Ren L. "Procedural Window Lighting Effects for Real-Time City Rendering." I3D 2015 (Bosch Research). [paper] <https://leiy.cc/publications/procwin/procwin.pdf> — read
50. US 11,116,060 B2, Signify, "Presence simulation system and method." [patent] <https://patents.google.com/patent/US11116060> — read
51. US 6,377,858, "System and method for recording and controlling on/off events of devices of a dwelling." [patent] <https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/6377858> — search-recorded
52. Local: [city-generation.md](city-generation.md) (Sony US 12,293,452 B2), [glass-credibility.md](glass-credibility.md) (Celsys US 12,700,170 B2, Bosch), [lighting-patents.md](lighting-patents.md) and [local-exposure.md](local-exposure.md) (STMicroelectronics). — local

**Other**

53. Eduscol. BAC Pro MELEC E2, June 2021, answer key (spacing 3 × mounting height). [docs] <https://eduscol.education.fr/sti/sites/eduscol.education.fr.sti/files/concours-examens/13947/13947-elements-de-correction-e2-metropole-bac-pro-melec-juin-2021.pdf> — search-recorded
54. ACCROLED. "Lanterne urbaine Dominance L": 1,870–13,500 lm, 2700–4000 K (HelloPro). [data sheet] <https://www.hellopro.fr/lanterne-urbaine-dominance-l-2008384-6624216-produit.html> — search-recorded
55. Energy Rating (Australia). "Household Lighting Consumer Survey Report" (dimmers in 33–50 % of homes). [docs] <https://www.energyrating.gov.au/sites/default/files/2022-12/Household%20Lighting%20Consumer%20Survey%20Report.docx> — search-recorded
56. Jensen H.W., Premože S., Shirley P., Thompson W., Ferwerda J., Stark M. "Night Rendering." University of Utah UUCS-00-016, 2000. [paper] <https://users.cs.utah.edu/docs/techreports/2000/pdf/UUCS-00-016.pdf> — search-recorded
57. Antique and decor listings of Provençal wrought-iron chandeliers with E14 bulbs (Proantic, 1stDibs). [web] <https://www.proantic.com/1296238-lustre-3-lumieres-en-fer-forge-tulipes.html>, <https://www.1stdibs.com/fr/meubles/luminaires/lustres-et-suspensions/lustre-proven%C3%A7al-en-fer-peint-avec-guirlandes-de-perles-en-bois/id-f_45876592> — search-recorded
