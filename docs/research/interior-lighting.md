# Lighting the insides: lamps, room-scale light, windows from within

The research for #270: what the village's entered houses need to be lit (the first map, #266),
where D-008's plan stands against the owner's patent rule, and a path that does not wait on a live
patent. It adds to [lighting-gi.md](lighting-gi.md) (§1 ReSTIR, §5 clustered shading), whose
entries are referenced, not repeated. Labels follow the house convention: **[paper] [web] [docs]
[patent]** and **foundational / still-current / recent**; sources read on 2026-10-16 are marked
*read*, those seen in search results only *search-recorded*.

> **In five sentences.** The houses are lit only by daylight through their doors and windows: no
> lamps, GI probes 4 m apart (as wide as a room), and windows that show nothing from inside.
> D-008 (accepted 2026-09-24) chose ReSTIR for direct light in the ray-traced tiers, before the
> owner's rule of 2026-10-10 to ask before building on a live patent; NVIDIA holds US 11,315,310 B2
> on reservoir resampling with a global-illumination structure (active to about 2041, a family of
> later filings), and whether a patent covers ReSTIR's direct-light method alone was not settled
> by this search. A village's homes need few lights at once (a ceiling lamp a room, one or two
> lamps by the beds and the sofa), so clustered light lists (Olsson et al. 2012, no patent found)
> with one shadow ray a pixel towards a light picked in its cluster by its unshadowed contribution,
> denoised in time, light them without reservoirs. The rooms' bounce light needs finer probes
> where the walker is (the models lab's rooms use 1 m and 256 rays), and the windows need their
> glass seen from inside.

**Contents**

1. [What #270 asks](#1-what-270-asks)
2. [Lamps: three ways](#2-lamps-three-ways)
3. [The rooms' bounce light](#3-the-rooms-bounce-light)
4. [Windows from inside](#4-windows-from-inside)
5. [Patents](#patents)
6. [Recommendation for Forge](#recommendation-for-forge)
7. [Verification notes](#verification-notes)

---

## 1. What #270 asks

- **Lamps:** a ceiling lamp a room, lamps by the beds and the sofa, lit at night by the households
  at home (the population's routines already light the faked windows).
- **Room-scale light:** daylight through a window reaching the far wall, a lamp's light reaching
  the walls and the ceiling.
- **Windows:** a house whose insides are drawn shows its real rooms through its glass from the
  street, and the street through its glass from inside.

The renderer today has no local lights (street lamps and lit windows are emissive surfaces), GI
probe cascades of 4, 8, 16, 32 and 64 m (`forge-render::probes`), the sun's shadow traced and
denoised (NRD's SIGMA; its diffuse denoisers are not integrated yet), and 256 probe rays on the
village's maps since #272.

## 2. Lamps: three ways

- **ReSTIR DI** (Bitterli et al., SIGGRAPH 2020 *[paper, still-current; search-recorded]*;
  [lighting-gi.md](lighting-gi.md) §1). Per pixel, a few dozen candidate lights resampled into one
  reservoir, then reused across neighbours and frames: millions of lights at a few rays a pixel.
  D-008's choice for the ray-traced tiers; RTXDI is NVIDIA's library of it, under the NVIDIA RTX
  SDKs licence *[web; search-recorded]*. See [Patents](#patents).
- **A light tree, sampled stochastically** (Moreau, Pharr, Clarberg, HPG 2019 *[paper,
  still-current; search-recorded]*). Lights in a two-level BVH refitted each frame; each pixel walks
  it towards the clusters that matter and traces one shadow ray; thousands of lights in 16 ms with
  filtering in their tests. Its patent status was not searched.
- **Clustered light lists** (Olsson, Billeter, Assarsson, HPG 2012 *[paper, foundational;
  search-recorded]*; [lighting-gi.md](lighting-gi.md) §5). The view frustum cut into 3D clusters,
  each with the list of lights that reach it; up to about a million lights in their tests. In a
  ray-traced renderer the list gives each pixel its few candidate lamps: one picked by its
  unshadowed contribution, one shadow ray towards it, and the result accumulated in time and
  denoised (NRD's diffuse denoiser). No patent was found on it.

A village's room has one to four lamps and the walker sees a few rooms at once, so the clustered
lists hold a handful of lights a pixel: one ray a pixel, picked well, converges in a few frames.

## 3. The rooms' bounce light

- The finest probes are 4 m apart, as wide as a room: a probe stands in a wall or in the next room.
  The models lab's rooms use 1 m and 256 rays (#171). A finer finest cascade on walked-in maps (2 m,
  or 1 m round the walker) is the measure to take, with its cost.
- The probes' rays must see the lamps: the hits a probe's rays make are lit by the sun and the
  sky today; they would take the lamps' light too, unshadowed or by one shadow ray a hit.

## 4. Windows from inside

- Seen from the street, a window's glass shows a faked room (interior mapping, D-060's step 3).
  From inside a house whose insides are drawn, the same glass faces away: the walker sees no
  street. Near houses with drawn insides need their glass drawn as glass both ways, with their real
  rooms behind it, and the far ones keep the faked rooms (#270's point 3; D-060's interiors plan,
  "faked rooms from outside, real rooms near").

## Patents

| What | Found | Status |
|---|---|---|
| Reservoir resampling with a global-illumination structure (NVIDIA; Wyman, McGuire, Shirley, Lefohn) | US 11,315,310 B2, priority 2020-03-12; family US 2022/0198746 A1, US 2025/0182387 A1, DE and CN filings | active, to about 2041 (Google's estimate); its claims were not read (the page was cut before them) |
| ReSTIR's direct-light method alone | none found in this search; NVIDIA's later filings in the same family may reach it | to check before building on it |
| Light trees sampled stochastically (Moreau et al. 2019) | not searched | — |
| Clustered shading (Olsson et al. 2012) | none found | — |
| Forward+ tiled light culling (AMD), met in passing on 2026-10-18 (`lamp-penumbra-denoising.md` §6) | US 10,803,655 B2, US 12,136,165 B2 | active, to about 2036-01-11; claims not read. To check before Olsson's clusters for the streets' lamps (the houses' rooms as light lists are another method) |
| Shadow denoising and sampling (NVIDIA) | see `lamp-penumbra-denoising.md` §6 | — |

Per the owner's rule, building D-008's ReSTIR waits on the owner's word with this table, or on a
professional search.

## Recommendation for Forge

1. **Lamps by clustered lists**, one shadow ray a pixel towards a light picked in its cluster by its
   unshadowed contribution, accumulated in time and denoised; ReSTIR (D-008) kept for when the owner
   decides on its patents.
2. **Lamps as data in the plan:** a ceiling lamp a room, one by each bed and sofa
   (`city::furnish` places those pieces), lit from the households' routines (the faked windows'
   rule), in cd and kelvin.
3. **Probes for rooms:** a finer finest cascade on walked-in maps, measured against the 4 m one in
   the hall and the room views; the probes' hits lit by the lamps.
4. **Glass both ways** for the near houses with drawn insides.
5. **Checks:** the map check's inside views at night (a new time of day in it), the still-camera
   and dolly shimmer scripts, the F1 overlay and `PROFILE.md` for each new pass.

## Verification notes

- *Read on 2026-10-16:* Google Patents US 11,315,310 B2 (bibliography, abstract and description up
  to the claims).
- *Search-recorded:* Bitterli et al. 2020 (NVIDIA Research and Dartmouth pages); Moreau, Pharr,
  Clarberg 2019 (NVIDIA Research, EG diglib); Olsson, Billeter, Assarsson 2012 (Chalmers); the
  RTXDI pages and the RTX Path Tracing SDK's licence page.
