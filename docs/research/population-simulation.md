# Research — A persistent population of millions: tiers, days, the server and the clients

> Companion to `game-ai-ecosystems.md` (§1 schedules as the outer loop and utility inside, §3 Assassin's
> Creed Unity's 40/120/10 000, Census, Brom's and Chenney's simulation level of detail, §6 the three
> layers and their hand-offs for a planet), `smart-systems.md` (§3 population budgets, Mass's LOD,
> Kythera's traffic scheduled ahead, Take-Two's navigation patents; §8 Iris and server meshing; §9 the
> class table that says what may be learned or run on the GPU under D-016), `netcode.md` (§1 Fiedler's
> numbers and the priority accumulator, §3 the Replication Graph, EVE's time dilation, Star Citizen's
> replication layer and Fujimoto's parallel simulation; §6 the server pieces), D-010 (QUIC and Forge's
> own replication), D-016 (determinism, and its amendment of 2026-10-10: a GPU may decide only in
> integers), D-037 (1 km cells, `CellId`), D-053 (generated content as data) and D-015 (MongoDB
> write-behind). Written 2026-10-10 for the owner's question of that day: a city sandbox that is
> simulation-first and multiplayer from the start, with a persistent population of at least a million
> people growing toward one to four million, every one with an id, a personality, a home, a commute, a
> work schedule, dialogue and reactions; one in-game hour lasts 15 real minutes; the city starts 6–8 km
> across; the server is the owner's second PC (an Intel Core i7 with 8 threads and an RTX 3080 10 GB,
> Windows now, Linux later), the client a Ryzen 7 9800X3D with an RTX 5070 Ti. The sizing for that one
> city is in an appendix kept with the sandbox, which is not published; this note stays engine-generic. Sources were
> checked with WebFetch and WebSearch only, no browser pane, nothing downloaded (PDFs were not
> fetched); each entry says whether the page was read or only seen as a search record, and the
> [Verification notes](#verification-notes) list what refused. Numbers marked *(estimate)* are this
> note's arithmetic, shown where they appear.

The short answer: **a million people is a data problem, not an AI problem, and it fits one ordinary
server.** At any moment nearly everyone is either inside a place or on a known route begun at a known
time, so where a person is follows from their record and the clock. Kept as compact records (about
100–130 bytes a person: about 0.15–0.2 GB live at a million with the routes and graphs, 0.6–0.8 GB at
four) and advanced by **events** (an activity
ends, a trip enters its next link) instead of ticks, the whole city costs a few thousand events per real
second at 1 M and, even with congestion modelled link by link, a few percent of one core at 4 M by a
bottom-up count, or about 1.4 cores by the most conservative published queue simulation scaled to this
clock (MATSim's Hermes runs a million agents' day at hundreds of times real time on 48 threads; this
clock asks for four times). The expensive people are the hundred or two near each player — navigation,
avoidance, conversations, animation — which every shipped game caps at tens to a few hundred per viewer
(Assassin's Creed Unity's 40 full AIs; Vuntra City's roughly 100 spawned around the player out of four
million). Consistency at the seams comes from one rule — every tier answers "where is this person now"
from the same record — plus the hand-off details Census, Nivalis Nights and SimCity's mistakes spell
out. On the server the CPU should own all of it (deterministic, cache-friendly, and dedicated servers
rarely have a GPU); the 3080 earns its place offline (generating and calibrating the population,
fast-forwarding test days, heat maps), and the client's 5070 Ti draws the crowd. Per client, the near
agents cost 10–20 KB/s and the far crowd about 1 KB/s as link events: inside D-010's 24 KB/s.

---

## Contents

1. [Simulation tiers and hand-offs](#1-simulation-tiers-and-hand-offs)
2. [CPU or GPU at one to four million](#2-cpu-or-gpu-at-one-to-four-million)
3. [Days, appointments, needs and personalities](#3-days-appointments-needs-and-personalities)
4. [What Vuntra City and the shipped games show](#4-what-vuntra-city-and-the-shipped-games-show)
5. [Server, clients, interest and persistence](#5-server-clients-interest-and-persistence)
6. [What the RTX 3080 server can hold](#6-what-the-rtx-3080-server-can-hold)
7. [What professional engines do](#what-professional-engines-do)
8. [Recommendation for Forge](#recommendation-for-forge)
9. [What to measure](#what-to-measure)
10. [Patents](#patents)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

---

## 1. Simulation tiers and hand-offs

### 1.1 The clock: what four times real time does

At 15 real minutes a game hour the clock runs at *k* = 4 game seconds per real second: a game day lasts
6 real hours, a game minute 15 real seconds, and one 60 Hz server tick (D-010) is exactly 1/15 of a game
second, so a game minute is 900 ticks *(arithmetic)*. Keeping game time as an integer count of ticks
keeps every schedule exact (D-016).

**UESP, "Skyrim:Time"; alt:V documentation, "Time and date" (GTA V).** [web] [docs]
<https://m.uesp.net/wiki/Skyrim:Time> · <https://docs.altv.mp/gta/articles/references/time-and-date.html>
(both read)

Skyrim's default timescale is 20 (a real minute is twenty game minutes, so a day lasts 72 real
minutes); GTA V multiplies time by 30 (two real seconds a game minute, a 48-minute day). Nivalis Nights
(§1.2) runs at 60 by default; Vuntra City's developer wrote on 1 October 2026 that they play it with a mod
at 15, finding a slower clock better suited to a game whose life runs on schedules (Bluesky, read).
*Bearing:* four times is far slower than the open-world norm. Two consequences follow. A game day is a
play session, so the rush hour, the lunch and the night out happen at a pace a player can follow.
Travel, though, becomes long: a person moving at a visible, real-time speed covers four times less ground
per game hour than in life. Walking at 1.4 m/s is 1.26 km a game hour, so a 2.5 km walk takes two game
hours (30 real minutes); a metro averaging 25 km/h covers 6.25 km a game hour *(estimate)*. Every tier
must use the same real-time speeds — a far-tier commuter moving at life speed would arrive before the
near-tier one who walked — so the day planner budgets trips in game time from real speeds, and the
city's form (short trips, fast modes) decides whether commutes look like commutes (each city sizes it).

### 1.2 Tiers by what a person is doing

`game-ai-ecosystems.md` §6 proposes three layers for a planet (statistical, regional, full). In a city
where every person has an id and must be findable, the statistical layer cannot be anonymous densities;
it becomes *records with whereabouts*. Four tiers:

| Tier | Who | State held | Advanced by | Lives on |
|---|---|---|---|---|
| **T3 Record** | everyone, always | the person's record and whereabouts: *at place P since t*, or *on trip R, entered link L at t* | events: an activity ends, a trip enters or leaves a link | server (the authority) |
| **T2 Coarse** | outdoors within roughly 0.5–1 km of a player, not yet close | a position on the walkway or lane graph, a path cursor, a speed | a staggered 5–10 Hz step, no avoidance, no animation on the server | server, interpolated on clients |
| **T1 Full** | within about 100–150 m of a player, or talking to one | navmesh path, local avoidance, smart-object use, conversation, reactions | 60 Hz movement, about 10 Hz decisions | server; clients interpolate and animate |
| **T0 Drawn** | the visible crowd beyond T1, client only | a GPU instance placed from the T3/T2 whereabouts it received | each frame, as presentation | client |

The crowd a player sees at 800 m is therefore the real population drawn cheaply, not the anonymous
ambient crowd of GTA (`smart-systems.md` §3): a figure crossing a square far away *is* person 412 873
on her way to the bakery, and walking up to her finds her.

**Christopher Dragert (Ubisoft Toronto), Census in Watch Dogs: Legion: Kaito Okutani's report of the GDC
2021 talk for 4Gamer (27 July 2021), and Daniel O'Connor, "Watch Dogs: Legion – The Tools That Built
London", Ubisoft News (30 March 2021).** [web] [talk]
<https://www.4gamer.net/games/464/G046433/20210726074/> ·
<https://news.ubisoft.com/en-us/article/4po3S9Pwp1YcgBmGPmQxAh/watch-dogs-legion-the-tools-that-built-london>
(both read; the GDC Vault listing is in `game-ai-ecosystems.md` §3)

The sibling note has Census as a population database; the talk's report adds the mechanics. Profiles
span 19 demographic categories whose allowed combinations are bitfield matrices, narrowed by a
flood-fill so that choosing one field constrains the others (an elderly woman's voice limits the age,
which limits the jobs). A tuning slip made about 30 % of young Londoners professional footballers until
designers widened the high-income jobs. Schedules are not scripts but generated "contracts" of activity,
sub-activity and location; relationship contracts give every new person at least one tie; chats,
lunches and meetings happen at locations held in a 34 MB cache. Only inside a fixed loading bubble around
the player are people instantiated; outside it they are moving points whose data alone is simulated.
Detail is added in stages ("upres"): inspecting a passer-by fixes their name, job, age and schedule;
targeting them for recruitment adds a backstory, related characters and missions. Affinity runs 0–10,
and memories of the player's actions ("intrigues") spread to a person's relations; state persists across
sessions. Ubisoft's article confirms the two layers in Dragert's words: generation gives consistent
profiles, simulation makes characters persistent and generates their schedules, and memories and affinity
propagate across relationships.
*Bearing:* the closest shipped match to the brief, and the source of three rules for Forge: a record
outside the bubble is a moving point with data, not an agent; detail is generated lazily from the seed on
first scrutiny and then kept; relationships are part of the generated record, not a separate system.

**ION LANDS, "May 2026 Development Update: Optimization", Nivalis Nights (Steam news, 30 May 2026);
release post, 29 September 2026, and patches #1–#4 (1–9 October 2026).** [web]
<https://store.steampowered.com/app/1488490/Nivalis_Nights/> (the posts read through Steam's public
news API, `ISteamNews/GetNewsForApp`, app 1488490)

A cyberpunk city life game with thousands of NPCs living their days, released on 29 September 2026. The
studio says simulating so many characters fully would be too expensive, so a "Ghost system" separates the
simulated character from its visual body, in three tiers: characters in another district keep only what
their daily schedule needs, with visuals, animation and physics stripped; characters in the player's
district but out of sight have their visuals loaded but disabled and their animation state not updated
every frame; visible characters are fully materialised. The first week's patches improved crowd
simulation and animation performance, reduced bone weights on lower LODs, fixed crowd characters leaving
the navmesh during dialogue and staff getting stuck, and fixed a character whose eating overlapped his
work schedule. A March 2026 Q&A counts more than 130 named characters with side quests; travel between
districts is by automated taxis and trains.
*Bearing:* the most recent shipped statement of the tiers, from a team of the same scale as Forge's
ambition for the near tier. Its post-launch notes are the list of things that break first: schedule
overlaps, people stuck in places, crowd agents pushed off the navmesh by conversations — all tests for
§1.3.

**Ben Sunshine-Hill, Norman Badler (University of Pennsylvania), "Perceptually Realistic Behavior through
Alibi Generation." AIIDE 2010, 83–88, DOI 10.1609/aiide.v6i1.12389.** [paper]
<https://ojs.aaai.org/index.php/AIIDE/article/view/12389> (abstract page read; the 2011 dissertation,
*Perceptually Driven Simulation*, seen as a search record of Penn's repository)

Pedestrians in an open-world city are simulated in detail only where watched; a cheaper "perceptual"
simulation stands in elsewhere, statistically guaranteed to look the same, and when a viewer keeps
watching someone the system retroactively invents a consistent past for them (the "alibi"). Census uses
the same word for matching schedules to animations.
*Bearing:* Forge does not need statistical alibis, because T3 knows exactly who is where. It does need
the *retroactive detail* half: a name, a backstory, the friends listed in a conversation are generated
from the person's seed on first scrutiny, so they are the same for every player and on every replay.

Brom's and Chenney's simulation LOD (consistency and completeness) and Assassin's Creed Unity's pooled
promotion are in `game-ai-ecosystems.md` §3 and not repeated.

### 1.3 Hand-offs without visible inconsistency

The rules, assembled from the entries above and §4's failures:

1. **One source of truth for whereabouts.** T3 always holds *where*: a place and since when, or a trip,
   its current link and when it was entered. T2 and T1 hold *how* (a position on a path, a pose, an
   action) and write back to T3 only at transitions.
2. **Promotion places the person where the record says.** On a link: at the fraction
   (now − entered) / traversal time along it. Inside a place: at a slot chosen by hashing
   (person, place), and doing what the activity says (eating, working, dancing). Nothing is invented
   except detail that the seed decides.
3. **Doors are the seams.** Entering or leaving a place is an event at a door; the person's T3 state
   changes at that instant. A person seen entering the shop is "in the shop since 18:42" and, promoted
   later, is found inside; a person promoted at the door finishes the walk-in as T1.
4. **Deviations are overrides, then a return to plan.** If a player delays someone (a conversation, a
   blocked door, an event), the record carries an override with an end; the next activity shifts and
   the person rejoins the plan gradually, as Vuntra City's developer describes doing (§4). The far tier
   never "teleports to schedule".
5. **Hysteresis and sight.** Promote at a radius *R*in, demote beyond *R*out > *R*in, never demote or
   spawn a T1 body inside any player's view within the visibility distance (the sibling's 150 m spawn
   test, Stalker 2's lesson), and keep promoted people promoted while someone watches them.
6. **Appointments are two arrivals.** Each party's arrival is its own event; the first to arrive waits
   up to a limit; meeting or being stood up is an event that updates the relationship (§3).
7. **Tests, Chenney's two words.** *Consistency:* follow a thousand random people through tier changes
   and compare their arrival times, places and counts with an undisturbed T3-only run (equal to the
   tick, when nobody interfered). *Completeness:* nothing scheduled is skipped (a shift is worked, a
   shop opens), whatever the tier.

### 1.4 Events or ticks

**Dan Graur, Rodrigo Bruno, Joschka Bischoff, "Introducing HERMES", MATSim news, 30 October 2020.** [web]
<https://matsim.org/news/2020/introducing-hermes/> (read)

Hermes replaces MATSim's time-stepped queue simulation (QSim) with an event-driven one that processes
only the events of each moment instead of visiting every link and agent each step. On a 10 % sample of
Switzerland (about 1 million agents, 1.5 million links, 301 iterations, on a 48-core Xeon with 768 GB),
an iteration (one simulated day of up to 32 hours) took 3 min 33 s against QSim's 8 min 45 s, 2.5 times
faster, with Hermes on 48 threads and QSim on 18 and both using about 130 GB of memory; the motivation was
that QSim would need about 116 days for 1 000 iterations of the full population. Hermes drops dynamic
routing, within-day replanning and signals, and its results differ slightly (car use about 1 % lower).
*Bearing:* the clearest published measurement that a city's far population should be event-driven: the
work follows what happens, not how many people exist. The 130 GB is the warning on the other side:
research frameworks keep everything (plans, events, a national network) and a game cannot; Forge's
records must be two orders of magnitude leaner (§2.2).

**Eclipse SUMO documentation, "Simulation/Meso".** [docs]
<https://sumo.dlr.de/docs/Simulation/Meso.html> (read)

SUMO's mesoscopic model splits each road edge into queues and computes, per vehicle, the time at which it
leaves its queue from the state of this queue and the next, the minimum travel time and the junction; the
documentation says it runs up to 100 times faster than SUMO's microscopic model. Vehicles keep no speed,
no lanes and no lateral motion; in the GUI they jump in steps of about 100 m.
*Bearing:* the exit-time queue is the T3 trip model for vehicles and crowds on walkways: one event per
link, congestion from capacities, and positions inside a link interpolated only when someone looks.

**Randy Brown, "Calendar Queues: A Fast O(1) Priority Queue Implementation for the Simulation Event Set
Problem." *Communications of the ACM* 31(10), 1988, 1220–1227.** [paper] [foundational]
<https://www.maths.tcd.ie/~eoin/index/brown_calendar88.html> (abstract read; the ACM page refused the
fetch)

A bucketed priority queue for the future-event set of discrete-event simulation, constant time per
operation in practice and three times faster than splay trees at 10 000 queued events in the paper's
measurements.
*Bearing:* T3's event queue is a calendar in game time: one bucket per game second (900 ticks), each a
list sorted by (time, person id) for determinism; with a million people the queue holds a million future
events and each tick drains a handful of buckets.

Kythera AI's "Cities at Scale" (traffic scheduled a short time ahead instead of stepped each frame) is in
`smart-systems.md` §3; Fujimoto's conservative and optimistic synchronisation in `netcode.md` §3.

**The arithmetic of an event-driven city** *(estimate)*:
- **Activity level** (no congestion): a person's day has about 5 activities and 3–4 trips; with
  departures, arrivals and a transfer or two, about 12 events a person a game day. At 1 M that is
  12 M events per 21 600 real seconds, **about 560 events per real second**; at 4 M about 2 200.
- **Link level** (mesoscopic queues on 100 m walkway links, 200 m road links, transit by stations):
  about 100 events a person a day, so **4 600/s at 1 M and 18 500/s at 4 M**; if a tenth of a day's
  events fall in one game hour (900 real seconds), the peak is 11 000/s at 1 M and 44 000/s at 4 M.
- **Cost:** at 0.2–1 µs an event (a queue pop, a record update, a cache miss or two), the 4 M peak is
  9–44 ms per real second, **under 5 % of one core**. Scaling Hermes instead (1 M agents' 32-hour day in
  213 s on 48 threads, about 7 700 thread-seconds for 24 hours, spread over a 21 600-second game day)
  gives **0.36 of a core at 1 M and about 1.4 cores at 4 M**: a conservative bound, since Hermes moves
  cars across a national network and writes every event to disk. The truth lies between; step 2 of the
  plan measures it.
- **Ticks are right** where agents interact continuously: T1 (avoidance, conversations, physics) and T2
  (smooth motion near players). A tick over everyone is wrong on a CPU: touching 4 M records of 64 bytes
  60 times a second is 15 GB/s of memory traffic, a third or more of a desktop's dual-channel bandwidth,
  for nothing *(estimate)*.
- **Determinism:** events at equal times run in person-id order; events of one bucket may run in
  parallel per D-037 cell when they touch disjoint places, their effects merged by index (D-016);
  conflicts (two people claiming the last seat in a bar) resolve by id.

### 1.5 Time warp and fast-forward

- **When nobody watches**, warp is free in kind: T1 and T2 dissolve when no player is near, and T3 just
  drains its queue faster. The cost scales with warp/4: at 4 M, the bottom-up count allows a few hundred
  times real time on 4 cores, the Hermes-scaled one about eleven times *(estimate: 4 × 4 cores /
  0.044 or / 1.4 cores)*. Which bound is true decides whether "skip the night" is instant or takes
  minutes — a measurement, not a design question.
- **Fast-forward on load** has nothing to do for people: T3 never stops, so a district has no backlog
  when a player arrives. Non-person state (a shop's stock, litter, plant growth, a building's lights) is
  computed in closed form from the elapsed time when its district streams in, the way `memory-streaming.md`
  treats derived data.
- **A shared clock.** In multiplayer one player cannot warp alone. Sleeping through a night needs every
  player's consent (or a rule); overload is handled by EVE's time dilation (`netcode.md` §3): slow the
  clock, never drop events.
- **A server that was off.** Either the world clock stops with the server (the simple rule), or it
  catches up by draining events at full speed: a game day at 1 M is 12–100 M events, 2–100 s on one core
  at 0.2–1 µs each *(estimate)*.

---

## 2. CPU or GPU at one to four million

### 2.1 Published throughput

**Andreas Horni, Kai Nagel, Kay W. Axhausen (eds.), *The Multi-Agent Transport Simulation MATSim*.
Ubiquity Press, 2016, DOI 10.5334/baw, CC-BY 4.0; and Janek Laudan, Paul Heinrich, Kai Nagel,
"High-performance mobility simulation: implementation of a parallel distributed message-passing
algorithm for MATSim", *Information* 16(2), 116, 7 February 2025.** [book] [paper]
<https://matsim.org/the-book> (read) ·
<https://depositonce.tu-berlin.de/items/d9fb992d-7229-49a4-a910-501e0bdcb982/full> (record and abstract
read; MDPI refused the fetch)

The book (about 600 pages, open access, four parts from use to over 40 case studies) documents MATSim's
loop: a synthetic population with day plans is executed by a queue-based mobility simulation, the
executed plans are scored, and agents replan, iterating to a stable state; links are first-in-first-out
queues with a flow capacity (vehicles that may leave per time unit) and a storage capacity, plus a
stuck time that lets a blocked vehicle through after a delay (30 s in Hermes's example configuration);
the scoring function in common use is Charypar and Nagel's (secondary descriptions, search records). The
2025 paper ports the queue simulation to a distributed, message-passing design: 119 times faster than the
current implementation, 43 times faster than its own single-core run, 24 hours of large-scale traffic in
3.5 seconds (the scenario's size is not in the abstract). An April 2024 working paper of the same group
is titled as a distributed queue simulation written in Rust (search record; PDF not fetched).
*Bearing:* the domain's mature model for "everyone has a day plan and a route" — plans, link queues,
scoring — and evidence, in Rust as well, that a queue simulation of a whole region runs thousands of
times faster than real time on a cluster. Forge needs four times on a fraction of one PC.

**Paul Richmond, Robert Chisholm, Matthew Leach, Peter Heywood, Matthew Martineau, "Fast Large-Scale
Agent-based Simulations on NVIDIA GPUs with FLAME GPU", NVIDIA Technical Blog, 10 February 2023; and
Richmond et al., "FLAME GPU 2: A framework for flexible and performant agent based simulation on GPUs",
*Software: Practice and Experience* 53(8), 2023.** [web] [paper]
<https://developer.nvidia.com/blog/?p=60478> (blog read; the paper seen as a search record of the White
Rose repository, <https://eprints.whiterose.ac.uk/199416/>)

On an A100 80 GB, 100 steps of an 80 000-boid flock took about 51 ms (0.5 ms a step), at least a thousand
times faster than Agents.jl and far more than Mesa; a 250 000-agent Schelling model ran in about 70 ms.
The authors say 80 000 agents do not fill the GPU and that hundreds of millions fit on A100 or H100 cards.
The paper frames the framework as a trade between flexibility and speed, with sub-models to resolve agents
competing for resources without races.
*Bearing:* the GPU wins by two to three orders of magnitude when *every agent acts every step* with
neighbour queries. A schedule-driven city is the opposite case: almost nobody acts in a given second. The
GPU's advantage applies to Forge's T0 crowd on the client and to offline batch runs, not to T3.

**Pavan Yedavalli, Krishna Kumar, Paul Waddell, "Microsimulation Analysis for Network Traffic Assignment
(MANTA) at Metropolitan-Scale for Agile Transportation Planning", arXiv:2007.03614 (2020–2021). — Xuan
Jiang, Raja Sengupta, James Demmel, Samuel Williams, "Large Scale Multi-GPU Based Parallel Traffic
Simulation for Accelerated Traffic Assignment and Propagation" (LPSim), arXiv:2406.08496 (2024).**
[paper] [recent]
<https://arxiv.org/abs/2007.03614> · <https://arxiv.org/abs/2406.08496> (both abstracts read)

MANTA simulates all morning trips of the nine-county San Francisco Bay Area at half-second steps in just
over four minutes on a GPU, routing excluded; the paper's conclusion (search record) gives 3.2 M
origin-destination pairs, 62 minutes of CPU-parallel routing and 4.6 minutes for the 5 am–noon window, a
"traffic atlas" laying vehicles out contiguously in memory. LPSim completes 2.82 M trips in 6.28 minutes
on one GPU and 9.01 M trips in 21.16 minutes on two, about 113 times faster than a CPU run on dual A100s.
*Bearing:* two calibration points. Microscopic traffic for millions is a GPU job at tens to a hundred
times real time (MANTA: seven hours in 4.6 minutes, about 90 times *(arithmetic)*), and **routing**, not
movement, dominates (62 minutes against 4.6). Cities: Skylines II says the same in production (§4).

**Ayush Chopra, Shashank Kumar, Nurullah Giray-Kuru, Ramesh Raskar, Arnau Quera-Bofarull (MIT Media
Lab), "On the limits of agency in agent-based models" (AgentTorch), arXiv:2409.10568 (2024).** [paper]
[recent]
<https://arxiv.org/abs/2409.10568> · <https://arxiv.org/html/2409.10568v1> (abstract and HTML read)

8.4 million agents representing New York City in a COVID-19 study of isolation and employment, with
behaviour from a language model at the scale of "archetypes": agents sharing the behaviour-relevant
attributes (age and sex in the example) share one query, so *K* archetypes times *A* actions are queried
instead of *N* agents (100 archetypes in the NYC runs); each agent then samples its action from its
archetype's estimated probability. Experiments ran on a V100 with 32 GB; per-step times are not in the
text.
*Bearing:* the one published way to put an expensive behaviour model behind a million people: decide per
archetype, sample per person. Any learned component in Forge would follow it, offline or server-side, with
the sampled choice recorded as an input (`smart-systems.md` §9).

**Nico Karoly Ziperzik, "Rendering and AI Optimization in RTS games: Enhancing performance with Entity
Component Systems in Unreal Engine 5", master's thesis, FH Upper Austria, 2025.** [paper]
<https://pure.fh-ooe.at/en/studentTheses/rendering-and-ai-optimization-in-rts-games-enhancing-performance-/>
(abstract read)

A Mass-based RTS prototype stays stable at 60 fps up to about 50 000 units, against about 10 000 for an
instanced-static-mesh renderer.
*Bearing:* the order of magnitude for ECS agents that move and decide every frame on a desktop: tens of
thousands, consistent with City Sample's 35 000 pedestrians (`game-ai-ecosystems.md` §3). A million or
four ticked agents is two orders beyond it; only events reach that scale.

Cities: Skylines II, the largest shipped Unity DOTS simulation, is in §4.

**What these numbers say, normalised to this clock** *(estimate)*:

| System | Population | Work | Speed relative to real time | Hardware |
|---|---|---|---|---|
| Hermes (MATSim) | ~1 M agents, 1.5 M links | a 32 h day of cars and transit | ~540× (213 s per day) | 48 threads |
| MATSim distributed (2025) | not stated | 24 h of a large region | ~24 700× (3.5 s) | a cluster |
| MANTA | 3.2 M OD pairs | 7 h morning, 0.5 s steps | ~90× (4.6 min) | one GPU (+ 62 min CPU routing) |
| LPSim | 2.82 M trips | regional trips | trips in 6.28 min | one V100 |
| FLAME GPU 2 | 80 000 boids | neighbour search every step | 0.5 ms a step | A100 |
| Mass RTS thesis | ~50 000 units | ticked ECS | 60 fps | desktop |
| **This sandbox** | **1–4 M people** | **a day at 4×** | **needs 4×** | **8 threads + 3080** |

### 2.2 Data layout and bytes per person

T3 is not a set of ECS entities. Four million records that change state a dozen times a day gain nothing
from archetype storage and lose by moving between archetypes on every change; they are plain columns
(structure of arrays) indexed by a `PersonId`, which also makes the snapshot a copy of contiguous arrays.
T1 and T2 people are `bevy_ecs` entities (D-006) with a `Person(PersonId)` component pointing back at
their record. A sketch of the columns, packed *(estimate)*:

| Column | Bytes | Notes |
|---|---|---|
| id | 0 | the index |
| home, anchor (work or school), household | 12 | three `u32` |
| traits: age band, sex, occupation, shift pattern, day template | 4 | packed |
| personality: five traits at 4 bits | 3 | §3 |
| seed | 4 | `Seed::derive(world, id)`; days and details derive from it |
| whereabouts: kind (2 bits) + place or link (30 bits); since / entered (game seconds) | 8 | the one source of truth |
| current trip: route reference, link index, mode, flags | 8 | meaningful while travelling |
| next event time | 4 | mirrors the calendar for checks |
| need clocks: four `u16` game minutes since meal, sleep, social, errand | 8 | needs as clocks, §3 |
| mood, energy, money | 4 | `u8`, `u8`, `i16` |
| four friends | 16 | seeded, editable |
| next person in the same place or link | 4 | intrusive list for "who is here" |
| override index | 4 | sparse table of deviations |
| **Total** | **~80** | 96–128 with alignment and spare fields |

Sparse tables beside it: memories between people that changed from the seeded baseline (about 12 bytes a
pair), appointments (about 24 bytes), overrides (about 16 bytes), what a person remembers of each player
they met (16–64 bytes a pair). Travellers' link sequences: at a peak, about 15 % of people travel, each
with about 30 links of 4 bytes, so 18 MB at 1 M and 72 MB at 4 M.

**Totals** *(estimate)*:

| Item | 1 M | 4 M |
|---|---|---|
| T3 columns at 96–128 B | 96–128 MB | 384–512 MB |
| travellers' routes at peak | 18 MB | 72 MB |
| places, links, walkway and lane graph | ~30 MB | ~60 MB (the city grows with it) |
| sparse tables (memories, overrides, appointments) | ~10–30 MB | ~40–120 MB |
| T2 at ~48 B for ~2 300 per player, 16 players | ~2 MB | ~2 MB |
| T1 at 1–2 KB for ~150 per player, 16 players | 2–5 MB | 2–5 MB |
| **live total** | **~0.15–0.2 GB** | **~0.6–0.8 GB** |
| plus a snapshot copy in flight (§5) | +0.1 GB | +0.5 GB |

T1 and T2 do not grow with the population; they grow with the players and with the local density (§6).

### 2.3 What may run on the GPU (D-016)

`smart-systems.md` §9's table puts simulation in the class "both sides, deterministic"; D-016's
amendment of 2026-10-10 lets a GPU decide only in integers, with a CPU reference and a digest on two
vendors. Applied here:
- **T3 on the server's GPU** would need integer schedule evaluation in Slang, a CPU twin, readback over
  PCIe each tick and digest tests on the 5070 Ti and the AMD iGPU. It would buy little: the event-driven
  CPU path already runs T3 in a few percent of a core, and it would tie hosting to GPU machines
  (dedicated and cloud servers usually have none).
- **Offline, the GPU is the right tool**: generating and fitting a synthetic population, running a whole
  city's day many times to calibrate plans against congestion (MATSim's loop, MANTA's speed), what-if
  fast-forwards for designers, heat maps of where everyone is. These produce data (D-053) that the
  deterministic simulation then reads.
- **On the client, the GPU draws and animates** the T0 crowd and interpolates positions along links:
  presentation, free to use floats.

---

## 3. Days, appointments, needs and personalities

### 3.1 Day plans

**John L. Bowman, Moshe E. Ben-Akiva, "Activity-based disaggregate travel demand model system with
activity schedules." *Transportation Research Part A* 35(1), 2001, 1–28.** [paper] [foundational]
<https://ideas.repec.org/a/eee/transa/v35y2001i1p1-28.html> (abstract read)

A person's day is chosen as a *pattern* (the primary activity, the type of tour for it, the number and
purposes of secondary tours), then each tour gets its time of day, destination and mode, conditioned on
the pattern; the pattern choice weighs the expected best utility of the tours it allows. A tour is the
travel from home to one or more activity places and back. Prototyped on a 1991 Boston survey.
*Bearing:* the shape of Forge's day planner: choose the day's pattern first (work day, day off with
errands, night out), then the tours, then times and places. It is cheaper and more legible than
simulating needs minute by minute, and it is what activity-based travel models have done for 25 years.

**ActivitySim consortium, "ActivitySim: An Open Platform for Activity-Based Travel Modeling"
(BSD-3-Clause), GitHub.** [repo]
<https://github.com/ActivitySim/activitysim> (README read; the model structure from the project's wiki as
a search record)

Open-source activity-based demand software led by a consortium of metropolitan planning organisations.
Its run is a sequence of models in four clusters: long-term choices (work and school places, car
ownership), a coordinated daily activity pattern decided jointly for a household's members, then tour-level
and trip-level choices; the household-level step is the slowest because it weighs every member
combination.
*Bearing:* households plan together: who takes the child to school, who cooks. Forge's day planner should
decide the household's pattern first and give members linked activities (an appointment within the
household), not plan each person alone.

**MATSim plans and scoring** (§2.1's book; secondary descriptions as search records): each agent holds a
few day plans with their scores, picks among them by a logit-like rule, and the worst is replaced by a
mutated copy each iteration (times, routes, modes).
*Bearing:* useful **offline**: run the city's day many times at world generation to settle departure
times and routes against congestion, then ship the settled plans as data. On the live server the plans
change only by events (a new job, a move, a player's influence).

**U.S. Bureau of Labor Statistics, "American Time Use Survey — 2025 Results", news release, 25 June
2026.** [web]
<https://www.bls.gov/news.release/atus.nr0.htm> (read)

Full-time employed people worked 8.1 hours on the days they worked; 81 % of employed people worked on an
average weekday and 30 % on an average weekend day; leisure and sports took 5.2 hours a day, half of it
television, and socialising and communicating 35 minutes; household activities about 2 hours.
*Bearing:* time-use surveys are the calibration target for day templates: the population's aggregate
hours (asleep, at work, out, at home) must match a survey's curve. A city elsewhere uses its own
country's survey.

Census's generated "contracts" (§1.2), Bethesda's package stacks (`game-ai-ecosystems.md` §1) and
Cyberpunk's hour-based communities (§4) are the game-side versions of the same idea.

### 3.2 Personalities usable in games

**Funda Durupınar, Nuria Pelechano, Jan M. Allbeck, Uğur Güdükbay, Norman I. Badler, "How the OCEAN
Personality Model Affects the Perception of Crowds." *IEEE Computer Graphics and Applications* 31(3),
2011, 22–31, DOI 10.1109/MCG.2009.105.** [paper]
<https://repository.bilkent.edu.tr/items/3c7f7651-f264-426f-b9ab-1a59f08f9219/full> (record and abstract
read)

Builds on the HiDAC crowd simulator, whose agents have individual psychological and physiological
parameters (avoidance forces, wayfinding, how many actions they complete depending on curiosity), and maps
the Big Five (openness, conscientiousness, extraversion, agreeableness, neuroticism) onto those parameters
so a designer sets personalities instead of tuning forces; the 2008 AAMAS paper of the same group
introduced the mapping (search record).
*Bearing:* the Big Five is a usable game personality because each trait maps to numbers the simulation
already has. The same five values can drive the day planner (how often out, how punctual), the T1 motion
(speed, personal space, patience in queues) and the dialogue.

**Stephen J. Guy, Sujeong Kim, Ming C. Lin, Dinesh Manocha, "Simulating Heterogeneous Crowd Behaviors Using
Personality Trait Theory." SCA 2011, 43–52, DOI 10.2312/SCA/SCA11/043-052.** [paper]
<https://diglib.eg.org/items/f7e44fe5-a0cf-4319-b575-61892d86c9bc> (search record; the Eurographics
library refused the fetch and the UNC project page returned 404)

A user study maps the parameters of an RVO-based crowd (neighbour distance, number of neighbours
considered, planning horizon, radius, speed) to perceived personality on Eysenck's three-factor model,
giving a linear mapping and a two-dimensional factorisation of how crowds read.
*Bearing:* the measured link between ORCA's parameters and how a walker looks (hurried, timid, assertive),
so T1's avoidance can express personality at no extra cost.

**Dwarf Fortress Wiki, "Personality trait".** [web]
<https://dwarffortresswiki.org/index.php/Personality_trait> (read)

Fifty personality facets per creature, each 0–100 in seven bands, about 78 % of values in the neutral
40–60 band; species shift the medians. Facets create needs (a very active dwarf needs activity), gate
which thoughts occur and which social skills can be learnt, raise the odds of depression or catatonia
under stress, and feed grudges when two creatures' values differ widely; memories and beliefs can change
facets.
*Bearing:* the upper bound of what a personality can do, and a warning: fifty facets are opaque even to
players who read the wiki. Five traits at four bits, each with visible effects, are the right size for a
million people.

**RimWorld Wiki, "Opinion".** [web]
<https://rimworldwiki.com/wiki/Opinion> (read)

One pawn's opinion of another runs from −100 to +100 (rival below −20, friend above +20), the sum of
modifiers: beauty, traits, relations (spouse, lover, family) and timed memories of interactions — a chat,
a deep talk for 20 days, an insult for 20 days, a social fight, being rescued for 30 days — each with a
stacking limit; a hidden compatibility per pair makes positive or negative interactions likelier.
*Bearing:* the memory model to copy for the people who interact: an opinion is a seeded baseline plus a
short list of timed modifiers. It costs nothing for the millions who never meet anyone notable and stays
small for those who do.

**James Ryan, Adam Summerville, Michael Mateas, Noah Wardrip-Fruin (UC Santa Cruz), "Toward Characters
Who Observe, Tell, Misremember, and Lie." Experimental AI in Games, AIIDE 2015, DOI
10.1609/aiide.v11i3.12825; with Emily Short's commentary on Ryan's dissertation (28 May 2019).** [paper]
[web]
<https://ojs.aaai.org/index.php/AIIDE/article/view/12825> ·
<https://emshort.blog/2019/05/28/curating-simulated-storyworlds-james-ryan-ch-6f/> (both read)

*Talk of the Town* simulates an American farming town from 1839 to 1979, and its characters form beliefs
by observation, by being told (truly or not), by transferring or confabulating from what they already
know; salient features are noticed more; repeating a lie can make the teller believe it; memory is
fallible. Short's commentary notes the weakness: most simulated knowledge (hair and eye colours) makes
little story.
*Bearing:* gossip — the news that a player helped someone spreading through a friend graph — is a
feature to add on top of RimWorld-style memories (Census's propagation is the same). The lesson: simulate
knowledge only about things that matter to play.

**Joon Sung Park et al. (Stanford, Google), "Generative Agents: Interactive Simulacra of Human
Behavior", arXiv:2304.03442 (2023).** [paper] [recent]
<https://arxiv.org/abs/2304.03442> (abstract read)

Twenty-five language-model agents in a Sims-like town store experiences as natural-language memories,
reflect on them and plan; from one seeded intention (a Valentine's party) they spread invitations,
make acquaintances and arrive together at the right time.
*Bearing:* the qualitative ceiling for appointments and memory, at 25 agents. With AgentTorch's
archetypes (§2.1) it marks the line: language models may author content offline or flavour a T1
conversation on the client; they do not run a million days, and nothing they produce enters the
simulation except as a recorded input (D-016).

The Sims 3's traits and Boltzmann choice are in `game-ai-ecosystems.md` §1.

### 3.3 What is cheap enough for a million

- **Needs as clocks, not decaying floats.** Store the game minute each need was last satisfied; hunger at
  any moment is (now − last meal) × rate. Nothing updates between decisions, and decisions happen at
  activity boundaries — about five a day — not every tick. A utility choice over a few dozen candidate
  activities at 5–20 µs is 0.025–0.1 ms per person-day; at 4 M people that is 100–400 core-seconds per
  game day, or 0.5–2 % of one core over its 21 600 real seconds *(estimate)*.
- **The day is planned lazily.** At the end of each activity, the planner picks the next from the day's
  pattern, the clocks, the personality, the hour, open places and any appointment, as Bethesda's package
  stack picks the topmost satisfied package (`game-ai-ecosystems.md` §1). No day plan is stored; the seed
  makes it reproducible.
- **Appointments are a nightly batch.** At a quiet hour (04:00 game time), a social planner walks the
  friend graph per district, pairs or groups people whose patterns allow an evening out (weighted by
  extraversion and the weekday), picks a venue near both homes with capacity at that hour, and inserts a
  fixed activity into each plan. Cost: proportional to the people who go out, a few percent of the
  population.
- **Personality as five 4-bit traits**, each with effects a player can notice: extraversion (how often
  out, crowd tolerance), conscientiousness (punctuality, errands done), openness (venue variety, chance of
  a new place), agreeableness (reaction valence, queue patience), neuroticism (reaction intensity:
  who flees a fire first, who complains). The same values drive T1's motion (Durupınar, Guy) and the
  bark and dialogue matcher (`game-ai-ecosystems.md` §1, Ruskin's fact matching).
- **Relationships**: implicit where they can be (household, coworkers by workplace, neighbours by
  building), four seeded friends each, and RimWorld-style timed memories only for pairs whose opinion
  changed. Memory of players: a small record per (person, player) created on first interaction.

---

## 4. What Vuntra City and the shipped games show

**Lara Davidova (Aevica Software), Vuntra City: Steam store page; Steam announcements (26 March 2025 –
20 April 2026, read through Steam's news API); the developer's Bluesky posts (16 July – 5 October 2026);
YouTube devlog titles; press kit; Theodore McKenzie, "Vuntra City: A WiP Unreal-Powered Game Featuring a
Gigantic Procedural City", 80.lv, 24 July 2023.** [web]
<https://store.steampowered.com/app/2859220/Vuntra_City/> ·
<https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid=2859220> ·
<https://bsky.app/profile/vuntracity.bsky.social> (read through Bluesky's public API) ·
<https://vuntracity.com/presskit.html> ·
<https://80.lv/articles/vuntra-city-a-wip-unreal-powered-game-featuring-a-gigantic-procedural-city>
(all read; the videos' titles read through YouTube's oEmbed, their content not watched; the X account
refused the fetch)

What is public, in order:
- **The game.** A solo developer's city exploration game, begun in January 2022 (press kit), in Unreal
  Engine for PC VR (80.lv), single-player, playable in VR or on a flat screen, unreleased (Steam). The
  city is procedurally generated, 20 × 20 km, every building enterable with interiors on every floor; it
  is generated from a seed on the fly and only the player's changes are saved (80.lv, 2023). Far
  buildings step down to sprite impostors near 180 m, cubemaps near 300 m, flat textures near 700 m and
  flat sprites near 3 km, for a 20 km draw distance.
- **The population.** On 1 December 2025 the developer announced a system spawning over a million
  persistent procedural NPCs, each with its own schedule, name, appearance and personality, with work and
  home phases and leisure in parks and on rooftop terraces. On 16 July 2026 the developer described overrides that
  let an NPC leave its "GPU calculated schedule" and gradually return to it, triggered also when an NPC
  strays too far from its anchor; on 20 July, work hours per interior type, so lit windows follow the
  day. On 24 July they raised the population to about four million and named two bottlenecks: computing
  basic information about all four million to know whom to spawn near the player, and the cost of the
  roughly 100 NPCs in range. On 22 September they reported more than 60 fps in VR at maximum settings in
  the city's most crowded area with four million NPCs, not yet as stable as they want.
- **Life.** An NPC playtest opened on 20 April 2026, with NPCs called feature-complete but dialogue not
  final; its challenges were befriending an NPC (who then greets you as a friend), finding uniquely named
  NPCs, and giving an NPC an existential crisis, which only certain personalities can have. In late
  September 2026 the developer added an event system through which NPCs react to situations (fireworks; an
  uncontained fire past a size makes them evacuate), reactions that depend on personality, remarks outside
  the player's dialogue, an outfit system (people change at the spa), and on 5 October NPC-to-NPC
  conversations with placeholder text.
- **Devlogs** (titles only): "How I Simulate Over a Million Persistent NPCs For My Procedural City";
  "The Daily Rhythm Of My Procedural City" (21 August 2026); "Increasing My Procedural City's Population
  to 4 Million NPCs" (31 August 2026); "How I Optimized My Procedural NPCs for VR" (26 September 2026). A
  LinkedIn post ties the first title to a conference talk (search record); the Everything Procedural
  Conference 2026 page did not list it.
- **Not public in text:** the data layout, bytes per NPC, the tiers beyond "spawn the ~100 in range", how
  the GPU evaluates schedules, how persistence of four million is stored.

*Bearing:* the existence proof for the brief, with the shape this note arrives at independently: every
person a seeded record with a schedule; positions as a function of schedule and time (on the GPU, every
frame, which suits a single-player client with a GPU to spare); about a hundred full NPCs around the
player; deviations as overrides that return to plan; personality-dependent reactions through an event
system. The developer's named bottleneck — evaluating everyone to find who is near — is the one an event-driven T3
with a spatial index (who is in which place or on which link, per D-037 cell) removes, and the one a
multiplayer server must remove, since its "player vicinity" is every player's at once.

**Paradox Interactive, "Cities: Skylines II Feature Highlight #11: Citizen Simulation & Lifepath", 28
August 2023. — Paavo Huhtala, "Why Cities: Skylines 2 performs poorly", 5 November 2023. — Aaron Down,
PCGamesN, on Iceflake Studios' fourth City Corner blog, 10 April 2026.** [web]
<https://www.paradoxinteractive.com/games/cities-skylines-ii/features/citizen-simulation-lifepath> ·
<https://blog.paavo.me/cities-skylines-2-performance/> · <https://www.pcgamesn.com/cities-skylines-2/performance-blog>
(all read)

Citizens live in households that may span generations, in four age groups; they work, study, shop,
sleep, seek treatment, commit crimes and move away; teens and adults weigh earning now against studying;
transport preferences differ by age (teens cheap, adults fast, seniors comfortable and close); leisure
weighs how much and how fast a place satisfies against the path cost and the weather; following a citizen
opens a Lifepath journal and their social feed. Huhtala's analysis a week after launch: Unity 2022.3 with
DOTS (ECS and Burst), about 1 200 systems, the CPU work spread across cores far better than in the first
game, nothing simulated on the GPU, and the game GPU-bound because character meshes had no LODs, culling
was frustum-only and shadows took about 40 ms of a test frame. In April 2026 Iceflake Studios, which took
over development, called simulation performance the harder problem: more citizens mean more pathfinding
requests, citizens' pathing had been made to stop generating excessive requests, and they planned to
lower population density so it better matches a city's scale, with a benchmarking tool to collect data.
The development diary on traffic (no agent caps, routing with time, comfort and money costs) is in
`game-ai-ecosystems.md` §2.
*Bearing:* the largest shipped ECS city, and its two lessons. One: drawing people is the first
bottleneck if near and far people share one mesh path (Forge's T0 must be a crowd renderer with LODs
from day one, and the owner sees pops). Two: once drawing is fixed, **routing requests grow with the
population** and become the simulation's cost; a 1:1 population makes it worse, so routes must be
habitual (computed once, reused daily) and the far tier must not re-route anyone without a reason.

**Cities: Skylines (2015) limits** (Steam community threads quoting the game's statistics; search records,
Steam throttled the fetch): about 1 048 576 citizen records, 524 288 "citizen units" (households, shared
with tourists, which in practice run out first, as a developer explained) and 65 536 citizen *instances*
— the people physically moving on the map.
*Bearing:* the first game already separated records (a million) from moving agents (65 536): the T3/T2
split, with the cap on agents and the household units as the real limits.

**Frank Cifaldi, "GDC 2012: Breaking down SimCity's Glassbox engine", Game Developer, 7 March 2012. —
"Maxis finally responds to SimCity AI concerns, admits there is a problem", MCV/Develop, 15 March
2013.** [web] [talk]
<https://www.gamedeveloper.com/design/gdc-2012-breaking-down-em-simcity-em-s-glassbox-engine> ·
<https://www.mcvuk.com/maxis-finally-responds-to-simcity-ai-concerns-admits-there-is-a-problem/> (both
read; Andrew Willmott's talk page read, his slides not fetched)

GlassBox models resources in bins on units (houses, factories, people), map resources, town-wide globals,
scripted rules that move resources, and lightweight agents that carry resources between units without
running rules; Maxis aimed at more than 10 000 agents at once. After launch, players found that Sims had no
home or job: each went to the nearest available one. Lead designer Stone Librande said Sims keep their
happiness, money, sickness and education, but not particular homes or jobs, nor names, clothing or other
traits, which were dropped to fit more Sims in the city; traffic jammed because cars always took the
shortest route, and the pathing was being retuned. PC Gamer's fact-check judged the nearest-building
behaviour real and by design (search record).
*Bearing:* the cautionary tale this brief exists to avoid: anonymous agents are cheap and players notice
at once when the person who left the house is not the one who comes back. Identity (home, job, name) is
cheap in a record — 12 bytes for home, anchor and household — and is what makes the city believable. The
same incident shows shortest-path routing without congestion costs producing absurd jams.

**Transport Fever 2 Wiki, "Towns".** [docs]
<https://wiki.transportfever2.com/doku.php?id=gamemanual:towns> (read)

Each resident has a home, a job and a place to shop; the unemployed do not travel to industry; residents
avoid public transport without a stop nearby; towns grow with reachability and shrink with crowded
stations, poor traffic and emissions. A developer post on the first game (search record) named the daily
commutes of simulated residents as the main CPU load.
*Bearing:* the minimal persistent identity — home, job, shop — already gives a transport game its demand.
The CPU cost sits in the commute routing, as in Cities: Skylines II.

**Workers & Resources: Soviet Republic (3Division) and Anno 1800 (Ubisoft Mainz), from their wikis.** [web]
(search records: the Workers & Resources wiki refused the fetch; the Anno 1800 wiki was not opened)

Workers & Resources simulates each citizen with a name, an age, an education and preferences, a routine
of work or school, free time and travel, walking up to about 480 m to work at full walking speed and
taking transit beyond. Anno 1800 consumes needs per house (farmers' houses hold 10 residents, artisans'
30, engineers' 40), not per person.
*Bearing:* the two ends of the city builder's scale. Per-person routines with a walking range are what
Forge's T3 records do; per-house aggregates are what its aggregates per place are.

**Paradox Interactive, "Victoria 3 Dev Diary #56: Cultures and Religions", 24 August 2022.** [web]
<https://www.paradoxinteractive.com/games/victoria-3/news/dev-diary-56-cultures-and-religions> (read)

Pops are grouped by where they live, their profession and the building they work in, and by culture and
religion; the first dev diary (search record) gives every such variation its own pop, many tens of
thousands in the world.
*Bearing:* the aggregate end. Forge's aggregates per place and per cell (counts by activity, mood
averages) are pops computed from records rather than replacing them, for the far view, the city's
statistics and the economy.

**Mount & Blade II: Bannerlord and X4: Foundations** (TaleWorlds dev blogs 2018–2019 and Egosoft patch
notes; search records, both hosts refused the fetch). Bannerlord's campaign map has its own navigation mesh
so that many parties can path efficiently; parties constantly measure distances to potential targets,
which the lead campaign programmer worked to optimise; heat maps of party paths and goods flows were used
to balance long campaigns. X4 runs ships far from the player in a reduced "low attention" mode, whose
rules (weapon damage, movement, targeting) were adjusted over many patches, and players report fights
going differently when they are away.
*Bearing:* two more instances of the same layered simulation; X4's reports are Chenney's consistency
failure in a shipped game — a far tier whose rules differ from the near tier's produces outcomes players
notice. Forge's far tier must use the same rules at coarser granularity, not different rules.

**EVE Online Team, "Tranquility Tech IV", 8 March 2023.** [web]
<https://www.eveonline.com/news/view/tranquility-tech-iv> (read)

A node is a proxy or a "sol" process with assigned tasks; 170 nodes simulate all of EVE's solar systems,
13 nodes on each 16-core machine; Jita, the trading hub, runs alone on its own node, and the Forge region's
market on another. Time dilation is in `netcode.md` §3.
*Bearing:* static partitioning by region, with hot spots given a machine of their own, is how the largest
single-shard world is hosted — the model for Forge's second server PC (§5.6): districts are the regions,
and a crowded square can be its own process.

**Tony Zurovec (Cloud Imperium Games), "Star Citizen Birthday AMA Recap", RSI Comm-Link, 9 October 2020;
Star Citizen Wiki, "Quantum (game system)" (now StarSim), last edited 30 January 2026.** [web]
<https://robertsspaceindustries.com/en/comm-link/transmission/17821-Star-Citizen-Birthday-AMA-Recap> ·
<https://starcitizen.tools/Quantum_(game_system)> (both read)

Quantum, renamed StarSim, is a back-end service that simulates hundreds of thousands of lightweight NPCs
("quanta"), each with desires, skills, a job and an inventory but no pathfinding, animation or geometry,
whose aggregate behaviour sets the economy and from which the game generates contextual NPCs and events
for players; developers steer it through resources, points of interest and variables (refinery times,
wages). In 2020 the team tested up to two million quanta and expected no more than 100 000 per star system
to be needed; first outputs fed fuel and repair prices, encounter types and service beacons. The wiki
still describes the system in the future tense and cites a roadmap item of November 2025; the CitizenCon
2949 (2019) talk "Building a Dynamic Universe" introduced it.
*Bearing:* a T3 that never becomes T1 — quanta drive the world but are not the people you meet. The idea
to keep is the separate service: the population as its own process that the game servers query (§5.6).
The idea to avoid for this brief is the indirection: here the person you meet must *be* a record.

**Rockstar's GTA V and Red Dead Redemption 2** are covered by the sibling notes (ambient populations
spawned by density per road link and view, Take-Two's patents, NVIDIA's settings guide, RDR2's observable
routines: `smart-systems.md` §3, `game-ai-ecosystems.md` §4). *Bearing:* ambient people with no identity
beyond a model and a scenario; persistent named characters are a separate, authored set. This brief asks
for the opposite default.

**Charles Tremblay (CD Projekt RED), "Building Night City: The Technology of 'Cyberpunk 2077'", GDC 2023
(abstract); REDmodding Wiki, "Creating Communities".** [talk] [docs]
<https://gdcvault.com/play/1028981/Building-Night-City-The-Technology> ·
<https://wiki.redmodding.org/cyberpunk-2077-modding/modding-guides/world-editing/ai-and-npcs/creating-communities>
(both read; the talk itself is members-only)

The talk covers rebuilding REDengine 4 for a city far larger than The Witcher 3's: a prefab hierarchy,
streaming of game systems, memory and CPU. The modding wiki documents how non-crowd NPCs are placed:
"communities" near AI spots, one entry per NPC template, periods by hour that say how many spawn where,
spots chosen at random each time the sector loads; crowd NPCs are a separate system.
*Bearing:* schedules attached to places rather than to people, rerolled on load — cheap and good-looking,
and exactly what a persistent population cannot do. It is a fine model for Forge's *venues* (a bar's
staffing and attendance curve by hour) used as the target the people's own schedules must fill.

**2024–2026 talks on very large persistent populations.** No GDC talk of 2024–2026 on the subject was
found (§11). The recent public material is from developers' posts (Vuntra City 2025–2026, Nivalis Nights
2026, Iceflake 2026) and from transport research (AgentTorch 2024, LPSim 2024, MATSim's distributed
simulation 2025).

---

## 5. Server, clients, interest and persistence

**Epic Games, `UMassReplicationSubsystem` and the MassReplication plugin. Unreal Engine 5.8 API
reference.** [docs]
<https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/MassReplication/UMassReplicationSubsystem>
(read)

Mass replicates agents through per-client "bubbles": the subsystem creates network ids per replicated
agent (identical on server and client), one bubble per client connection (and a replication info per
client machine for split screen), and a 2D hash grid that finds the entities near each client; agent types
replicate through handler classes per bubble.
*Bearing:* the shipped shape of D-010's interest management applied to crowds: a stable id per agent, a
grid query per client, a per-client set. Forge's version keys the grid by D-037 cells and replicates two
kinds of thing — T1/T2 bodies as snapshots, T3 whereabouts as events (§5.3).

**Martin Fowler, "Event Sourcing", 12 December 2005.** [web] [foundational]
<https://martinfowler.com/eaaDev/EventSourcing.html> (read)

Record every change to an application's state as a sequence of events, so state can be rebuilt by
replaying them, queried at any past time, or corrected by replaying from before a bad event; snapshots
(say, overnight) avoid replaying from the beginning.
*Bearing:* the persistence for a deterministic simulation is snapshots plus the log of what is not
deterministic (players' inputs and server decisions), exactly D-010's replay format (§5.5).

### 5.1 What lives where

| Data | Server | Each client |
|---|---|---|
| T3 records (everyone) | all, authoritative | none at first; optionally the seeded base, derived locally (§5.4) |
| T3 whereabouts | all | people within the draw radius (~1–1.5 km), as events |
| T2 coarse agents | near any player | its own surroundings, snapshots at low rate |
| T1 full agents | near any player | its own surroundings, snapshots and action events |
| aggregates (counts per place and cell, link travel factors) | all | its district and the city's summary |
| appearance, name, voice, backstory | derivable from the seed | derived from the seed (same code, D-016) |

### 5.2 Who simulates the people near a player

The server, always. A client predicts only its own avatar and the physics group it touches (D-010).
For NPCs it interpolates 100 ms behind the newest snapshot and animates from replicated action ids (sit,
order, dance, flee) and events (a remark, a reaction); a conversation is a server state machine whose
lines are ids. Two players talking to the same person see the same person because there is one record and
one T1 body. A client never promotes anyone; the server promotes from every player's position and view.

### 5.3 Bandwidth per client *(estimate)*

- **T1 bodies.** A delta-coded update (D-010's bit writer): entity id as a delta varint (~1 byte),
  position delta 2 × 12 bits at 1 cm, heading 8 bits, action 8 bits, flags 4 bits: about 6.5 bytes.
  100 people at 15 Hz cost 9.8 KB/s; 150 at 15 Hz, 14.6 KB/s; 100 at 30 Hz, 19.5 KB/s. People seated
  or queueing hit Fiedler's at-rest case and cost almost nothing (`netcode.md` §1). The priority
  accumulator fills the 24 KB/s budget from the nearest and most watched first.
- **The far crowd as events.** Within 1 km at 20 000 residents per km², 62 800 people, of whom about
  10 % are outdoors; walkers cross a 100 m link in about 71 s and vehicles a 200 m link in about 25 s, so
  about 140 link entries per second, plus about 35 place entries or exits per second (12 a person a day
  over 21 600 s). At about 6 bytes an event (person delta, link or place, time): **about 1 KB/s**, giving
  each client the exact whereabouts of tens of thousands of people.
- **Aggregates:** occupancy per place and per cell each game minute, link travel factors each 15 game
  minutes: a few hundred bytes per second.
- **Join or teleport:** the whereabouts of the 62 800 people in range at about 6 bytes each, about
  380 KB on a reliable stream outside the snapshot budget: instant on a LAN, under half a second at
  1 MB/s.

Total about 10–20 KB/s per client, inside D-010's 24 KB/s.

### 5.4 Deriving the far crowd on the client (later)

Because the base population is generated from the world seed (D-053) and the day planner is
deterministic, a client could compute far people's whereabouts itself and receive only overrides and
the aggregates congestion depends on (link travel factors per 15 game minutes). It saves the ~1 KB/s and
the join burst, and costs determinism of the planner on both sides, a mesoscopic far tier whose
congestion the client cannot reproduce exactly (so far positions would drift, corrected silently at
promotion beyond detail distance), and a bigger client. Not for the first steps; worth measuring once the
event path works.

### 5.5 Persistence (#234's idea: persist without overloading the engine)

- **What is not stored:** the base population (records as generated) is derived data from the seed and
  the generator's version (D-053); a server restores it by regenerating, as Vuntra City restores its city.
- **What is stored:** the T3 columns that change (whereabouts, trips, clocks, mood, money, friends,
  overrides, memories, life events: births, deaths, moves, jobs) as column files compressed with LZ4
  (D-054), one snapshot per game hour (15 real minutes); and an append-only log of inputs and
  non-deterministic decisions since the last snapshot (D-010's replay format). Restore = snapshot + log
  replay to the same digest.
- **Cost:** at 4 M, 384–512 MB raw per snapshot, perhaps 130–260 MB after LZ4 at a 2–3× ratio (to be
  measured); copying it is about 50 ms of memcpy at 10 GB/s, spread over ten ticks, and writing it
  0.3–0.5 s on a SATA SSD on a background thread *(estimate)*.
- **MongoDB (D-015)** keeps player-facing records (accounts, inventories, player-placed objects) and
  is not the population's store: four million documents upserted every 15 minutes would overload the
  Pi 5 for no benefit, while column files are a sequential write.

### 5.6 A second server PC and, later, server meshing

- **Now, one process** on the server PC: T3 and the aggregates on one or two workers, T1/T2, interest
  and replication on the rest (`forge-task`, every hardware thread on a server, D-005).
- **Next, two roles:** a *population service* (T3, routing, aggregates, persistence: StarSim's role, on
  one machine because it is cheap and global) and *zone servers* holding T1/T2 for the players in their
  districts (EVE's static regions, a hot square alone on a node), behind D-010's replication layer. A
  zone asks the population service to promote a person and returns them demoted with their overrides; a
  person walking across a zone border is D-010's authority hand-off, at most one tick.
- **The second PC** can then run zone servers or the population service with persistence. The traffic
  between them is promotions, demotions and overrides: kilobytes per second on a LAN.

---

## 6. What the RTX 3080 server can hold

**Wikipedia, "GeForce 30 series".** [web]
<https://en.wikipedia.org/wiki/GeForce_30_series> (read)

The RTX 3080 (10 GB): GDDR6X on a 320-bit bus, 760 GB/s, 68 SMs, 8 704 CUDA cores, 29.8 FP32 TFLOPS
at boost, 5 MB of L2, launched 17 September 2020.
*Bearing:* far more memory and bandwidth than the population needs (4 M × 64 B is 256 MB, 2.5 % of its
memory; one pass over it 0.34 ms at 760 GB/s *(estimate)*); its limits for the authoritative path are
D-016 and hosting, not capacity.

The server PC's exact CPU model and RAM are not recorded; the arithmetic assumes four cores with eight
threads (or eight cores without SMT), a desktop single-thread speed about half the 9800X3D's, dual-channel
DDR4 at about 40 GB/s, and at least 16 GB of RAM — all to be checked.

**CPU budget per real second at 4 M, 16 players** *(estimate)*:

| Work | Count | Cost each | Cores |
|---|---|---|---|
| T1 people | 16 × 150 = 2 400 | movement and avoidance 3 µs at 60 Hz + decisions 30 µs at 10 Hz = 0.48 ms/s | 1.15 |
| T2 people | 16 × 2 300 = 36 800 | 0.1 µs at 10 Hz | 0.04 |
| T3 events | 18 500/s average, 44 000/s peak | 0.2–1 µs (bottom-up) | 0.01–0.04 |
| T3, Hermes-scaled bound | — | — | up to 1.4 |
| day planning (utility at activity ends, §3.3) | ~20 M choices a game day | 5–20 µs | 0.005–0.02 |
| routing | 4 M × 3.5 trips a day = 650/s, peak ~1 600/s; habitual routes reused, ~⅓ new | 50–500 µs a query | 0.03–0.3 |
| interest and replication | 16 clients × 30 Hz × ~2 500 candidates | 30 ns | 0.04 |
| snapshot copy and compression | one per 900 s | ~1–3 s of one core | <0.01 |
| **total** | | | **~1.3–3.0 of 8 threads** |

With four players instead of sixteen, T1 falls to 0.3 cores. **The limit is T1: players × local density
× the per-agent cost**, not the population. At 1 M the T3 lines shrink four times and nothing else
changes.

**Memory:** §2.2's 0.6–0.8 GB live at 4 M, plus a snapshot in flight (0.5 GB), plus navmesh tiles around
players (16 players × 9–25 tiles of 256 m at 100–300 KB, about 15–120 MB): **about 1.1–1.4 GB at 4 M,
0.3–0.45 GB at 1 M** *(estimate)*.

**Tick rates:** 60 Hz movement for T1, 10 Hz decisions, 5–10 Hz T2, T3 by events with no rate; the 60 Hz
tick is the clock that drains T3's buckets.

**Where the limits are:**
1. **T1 per player** at the densest places: about 150 within 150 m at 20 000 residents per km² with
   10 % outdoors, more in a full bar or at a festival; cap with the accumulator (promote the nearest and
   watched) and keep the rest T2.
2. **Routing load** if routes are recomputed per trip (Cities: Skylines II's lesson): habitual routes,
   a hierarchy over the walkway and lane graph, and re-routing only on disruption.
3. **RAM**, only if the server has less than 8 GB.
4. **The 3080 for authority**, by D-016 and by hosting; it serves the offline jobs of §2.3.
5. **Windows timers and scheduling** for a 60 Hz server: measure tick jitter on Windows before the Linux
   move (`netcode.md` §4's Valorant entry on scheduler tuning).

---

## What professional engines do

| Game or system | Persistent identity | Far tier | Near tier | Hand-off and consistency | Scale published | What to take |
|---|---|---|---|---|---|---|
| Vuntra City (2025–26) | yes: name, schedule, look, personality | seeded schedules evaluated on the GPU | ~100 spawned around the player | overrides that return to plan; anchors | 1 M → 4 M, 60+ fps in VR | the whole shape; the near-player query as the bottleneck |
| Watch Dogs: Legion (2020) | yes, persists across sessions | moving points with data | loading bubble | detail added in stages on inspection | London; numbers unpublished | contracts, upres, propagated memories |
| Nivalis Nights (2026) | thousands of NPCs with schedules | schedule tracking only | visible: full | in district, unseen: visuals loaded, animation sparse | thousands | three tiers; what breaks first |
| Cities: Skylines II (2023) | yes, households and life paths | DOTS simulation, every citizen | same simulation, drawn | — | no agent cap; slows at large populations | routing grows with people; draw LODs |
| Cities: Skylines (2015) | records | 1 M citizen records | 65 536 moving instances | — | caps | records versus agents |
| SimCity 2013 | no homes or jobs | agents carrying resources | same | none: nearest home and job | 10 000+ agents | identity is cheap; its absence is noticed |
| Transport Fever 2 | home, job, shop | every person tracked | — | — | town scale | commutes are the CPU load |
| Workers & Resources | name, age, education | routines per citizen | — | — | republic scale | walking range, transit beyond |
| Anno 1800 / Victoria 3 | no / pops | per house / per pop group | — | — | tens of thousands of pops | aggregates from records |
| Mount & Blade II, X4 | lords and parties / ships | campaign map / low attention | battle / high attention | differing rules noticed (X4) | — | same rules at coarser grain |
| EVE Online | players | one process per group of systems | — | time dilation | 170 sim nodes | static regions, hot spots alone |
| Star Citizen StarSim | quanta (not the NPCs you meet) | service simulating quanta | NPCs generated from it | contextual generation | 2 M tested, 100 k per system | the population as a service |
| GTA V, RDR2 | ambient: no | density by link and view | spawned out of sight | — | — | the anonymous default to avoid |
| Cyberpunk 2077 | no, for communities | schedules on places by hour | spawned on sector load | rerolled each load | — | venue curves as targets |
| Assassin's Creed Unity | no | crowd particles | 40 full AIs, 120 hi-res | pooled promotion | 10 000 on screen | the near budget (sibling note) |
| MATSim / Hermes, SUMO meso | research agents with plans | event-driven queues | — | — | ~1 M at ~540× real time | event-driven far tier |
| MANTA, LPSim, FLAME GPU 2 | research | GPU microsimulation | — | — | millions of trips in minutes | the GPU for batch, not authority |

---

## Recommendation for Forge

**Verdict.** Build the population as a deterministic, event-driven record store in `forge-sim` on the
CPU, with the full simulation only around players, and treat the GPU as the offline generator's and the
client's tool. Nothing in the published record suggests that 1–4 M is beyond one 8-thread server: the
population itself is a few percent of a core to about 1.5 cores and under a gigabyte; the costs that
grow are routing (contain it with habitual routes) and the full agents per player (contain them with the
priority accumulator). Every step below ends in a number.

**`forge-sim::population`, the design:**
- **Tiers:** T3 records for everyone (events), T2 coarse agents near players (5–10 Hz), T1 full agents
  within ~100–150 m (60 Hz movement, 10 Hz decisions), T0 drawn on the client; hand-offs by §1.3's rules.
- **Time base:** game time as integer ticks of 1/15 game second (one 60 Hz tick); schedules in game
  minutes (900 ticks); the time scale a constant (4 now), so a change of mind is one number.
- **Data layout:** T3 as structure-of-arrays columns indexed by `PersonId`; T1/T2 as `bevy_ecs`
  entities with `Person(PersonId)`; places, links and the walkway and lane graph per D-037 cell; sparse
  tables for overrides, appointments and memories.

```rust
/// Index into the population's columns (4 M fits in 22 bits).
pub struct PersonId(pub u32);
/// Game time in ticks of 1/15 game second: one 60 Hz server tick, 900 a game minute.
pub struct GameTick(pub u64);

pub struct Population {                  // structure of arrays, index = PersonId
    pub home: Vec<PlaceId>,              // u32
    pub anchor: Vec<PlaceId>,            // work or school, or PlaceId::NONE
    pub household: Vec<HouseholdId>,     // u32
    pub traits: Vec<Traits>,             // packed u32: age band, sex, occupation, shift, day template
    pub personality: Vec<Personality>,   // five 4-bit traits
    pub seed: Vec<u32>,                  // from Seed::derive(world, id); days and details derive from it
    pub whereabouts: Vec<Whereabouts>,   // At { place } | OnTrip { trip } | Promoted { entity }, + since
    pub next_event: Vec<u32>,            // game seconds, mirrors the calendar queue
    pub need_clock: Vec<[u16; 4]>,       // game minutes since meal, sleep, social, errand
    pub condition: Vec<Condition>,       // mood u8, energy u8, money i16
    pub friends: Vec<[PersonId; 4]>,     // seeded, editable
    pub next_here: Vec<PersonId>,        // intrusive list of people in the same place or on the same link
}
pub struct Trip { pub route: RouteRef, pub link: u16, pub mode: Mode, pub entered: u32, pub arrive_by: u32 }
pub struct Place { pub kind: PlaceKind, pub cell: CellId, pub capacity: u16, pub hours: OpeningHours,
                   pub doors: SmallVec<[DoorId; 2]>, pub first_here: PersonId, pub count: u16 }
pub struct Appointment { pub at: PlaceId, pub start: u32, pub people: SmallVec<[PersonId; 4]>, pub max_wait_min: u8 }
pub struct Override { pub person: PersonId, pub until: u32, pub cause: OverrideCause }   // a day off its plan
pub struct Memory { pub a: PersonId, pub b: PersonId, pub delta: i8, pub until_day: u16, pub kind: MemoryKind }
```

- **Tick model:** each 60 Hz tick (1) drains T3's calendar buckets up to now in (time, id) order,
  parallel per cell where disjoint, merged by index; (2) every 0.5 s, recomputes each player's
  candidates from the place and link lists of nearby cells and promotes or demotes with hysteresis;
  (3) steps T2 for a sixth of them (id mod 6); (4) runs T1 decisions for a sixth and T1 movement for all;
  (5) emits events and fills each client's packet. Aggregates per game minute; the social planner once a
  game day.
- **CPU/GPU split:** the server's CPU decides everything; the server's GPU runs offline and batch jobs
  (population synthesis and fitting, day calibration, fast-forward tests, heat maps) whose outputs are
  data; the client's GPU draws, animates and interpolates T0 and T1.
- **Server/client protocol** (on D-010): unreliable datagrams for T1 (and T2 at low rate) through the
  priority accumulator; a reliable "population" stream carrying a join burst and then whereabouts events
  (place enter or leave, link enter, trip start) for the draw radius, plus aggregates; a reliable
  "interaction" stream for dialogue line ids, reactions and visible relationship changes; appearance,
  names and backstories derived on the client from the seed by the same generator.
- **Persistence:** §5.5 — base from the seed, changed columns as LZ4 snapshots per game hour, an input
  log between them; MongoDB only for player records.

**Step plan** (each step ends in a measured result; starts small, then 1 M, then 4 M):
1. **Records and the generator.** Households, ages, jobs, homes, personalities and friends from the
   seed for 10 k, 100 k and 1 M people. *Measure:* bytes per person, generation time, the digest at one
   and six workers, debug and release.
2. **The event core, activity level.** Calendar queue, lazy day planner, needs as clocks, trips with
   free-flow times (no congestion), one game week of 1 M headless. *Measure:* events per second, µs per
   event, ms per real second at 4×, the highest warp, the digest; aggregate hours against a time-use
   curve.
3. **Places, capacity, appointments.** Opening hours, capacities, the nightly social planner, waiting
   and being stood up. *Measure:* share of appointments met, venue occupancy by hour against designed
   curves, people turned away.
4. **Routes and link queues.** Walkway, cycle, transit and lane graphs; habitual routes; mesoscopic
   queues with capacities and stuck times. *Measure:* route requests per second and their cost (A* versus
   a hierarchy), link events per second, peak congestion; 1 M.
5. **The spatial index and a debug view.** Who is in each place and on each link per cell; a heat map of
   the population (the GPU's first offline job). *Measure:* µs for "everyone within R of a player"; the
   map against the generator's intent.
6. **T2 and T1 near one player, with the hand-offs.** Walkway agents, then the navmesh, ORCA, smart
   objects, conversations and reactions with personality (`game-ai-ecosystems.md`'s step plan). *Measure:*
   µs per T1 agent, Chenney's consistency (a thousand people followed across tiers: arrival times and
   places equal to the T3-only run when undisturbed), "seen entering is inside", no spawn in view within
   150 m.
7. **The client crowd.** Whereabouts events into a GPU crowd renderer with LODs (impostors or vertex
   animation far, skeletons near). *Measure:* bytes per second per client, frame milliseconds at 10 k
   and 50 k figures on the 5070 Ti, the ꟻLIP of LOD changes (no visible pops).
8. **Multiplayer on the server PC.** Two clients and 50 bots on the LAN through the link conditioner.
   *Measure:* tick p50/p99, bytes per client, promotions per second, the digest of a replay.
9. **Persistence and restart.** Snapshots and the log; kill and restart the server mid-evening.
   *Measure:* snapshot bytes and milliseconds, restore time, the same digest after restore.
10. **Four million.** The generator and the city at 4 M. *Measure:* memory, tick p99, routing load, the
    warp limit; decide then whether the population service gets its own process or PC (§5.6).

**For a decision later (D-0xx 🟡, when the city's crate starts):**
1. The time base (ticks of 1/15 game second) and the warp policy in multiplayer.
2. The tiers, their radii and budgets per player (defaults: T1 ~150 within 150 m, T2 within 0.6–1 km,
   T0 to 1.5 km).
3. The record layout and `PersonId`'s width; what is seeded and what is stored.
4. Activity-level or link-level congestion in the far tier, for which modes.
5. CPU authority with the GPU offline (this note's view), or an integer GPU path under D-016.
6. The persistence format and cadence, and the role of MongoDB.
7. Whether clients later derive the far crowd from the seed (§5.4).
8. Language models: none on the tick; whether any authoring or client-side flavour is wanted.
9. The patent check of §10 before any server meshing work.

---

## What to measure

- **T3:** events per second (average and peak hour), µs per event, ms per real second at 4×, the highest
  warp on 1, 2 and 4 cores, at 100 k, 1 M and 4 M; bytes per person and the live total.
- **Routing:** requests per second, share of habitual reuse, µs per query by method, its share of the
  tick.
- **Days:** aggregate hours by activity against a time-use curve; appointments met and missed; venue
  occupancy by hour; people stuck (no progress for a game hour) — zero is the target.
- **Hand-offs:** promotions and demotions per second; the consistency test's differences (zero when
  undisturbed); spawns or vanishings inside a view within 150 m (zero); the arrival-time error of
  disturbed people after their override ends.
- **T1/T2:** µs per agent per tier, agents per player at the densest places, tick p50/p99 on the server
  PC, on Windows and later Linux.
- **Network:** bytes per client per second by stream (T1 snapshots, population events, aggregates), the
  join burst's bytes and time, seconds since a visible person was last updated.
- **Client:** frame milliseconds of the crowd at 10 k and 50 k figures, the ꟻLIP of LOD transitions,
  memory.
- **Persistence:** snapshot bytes raw and compressed, copy and write milliseconds, restore time, the
  digest after restore.
- **Determinism:** the tick digest (T1, T2, then T3 columns touched) at one and six workers over 1 000
  ticks, and a replayed evening reproducing the live server's stream.

---

## Patents

Checked briefly on 2026-10-10 on Google Patents and through search records; claims were not read in full
(the pages were truncated before the claims), so the relevance below is from titles and descriptions.
This is a flag list for the owner, not legal advice; expiry dates are Google's estimates.

| Patent | Holder | Jurisdiction | Status | Expiry (estimate) | Touches | Alternative |
|---|---|---|---|---|---|---|
| US 10,579,434 B2, "Simulation systems and methods using query-based interest" (priority 24 Aug 2016, granted 3 Mar 2020) | Improbable Worlds Ltd | US | active | 28 Nov 2036 | entities on workers whose interest is a component holding query subscriptions to entity databases, with update frequencies | D-010's interest computed by the server per client from positions and cells, with a priority accumulator (Tribes 1999, Fiedler 2015): no entity-held query components |
| US 11,936,734 B2, same title (granted 19 Mar 2024) | Improbable Worlds Ltd | US | active | 28 Nov 2036 | the same, with authority deciding which subscriptions apply | as above |
| US 10,878,146 B2, "Handover techniques for simulation systems and methods" (priority 24 Aug 2016, granted 29 Dec 2020) | Improbable Worlds Ltd | US | active | 15 Jul 2037 | handing authority over entity components between worker processes with advance notice and buffered updates | hand-off at a tick boundary under authority epochs (Fujimoto's conservative synchronisation, `netcode.md` §3); check the claims before building §5.6 |
| US 10,839,116; US 11,416,305 ("Commands for simulation systems and methods", 2022); US 11,816,402 | Improbable Worlds Ltd | US | granted (search records) | not checked | workers, bridges and an administrative layer; commands between workers by component authority; load balancing by transfer scores | same as above; Forge's zone/population split is by district and service, not by component authority |
| US 11,071,916 B2, "System and method for virtual navigation in a gaming environment" (priority 24 Apr 2019, granted 27 Jul 2021) and continuation US 11,684,855 B2 (27 Jun 2023, from `smart-systems.md`) | Take-Two Interactive | US | active | 24 Apr 2039 | NPC routes found on a coarse graph derived from the road graph so that fewer detailed nodes are loaded; route building on a server; density spawning in the description | the city's whole graph is resident on the server, so plain A* or Dijkstra over it, or a hierarchy published before 2019, avoids the "fewer nodes loaded" premise; check the claims before shipping a coarse-graph router |
| US 10,315,113 B2 (priority 14 May 2015, granted 11 Jun 2019) and US 11,524,237 (search record) | Activision Publishing | US | active | 11 Mar 2038 | simulating NPC gameplay on spare capacity of networked end-user devices | keep NPC simulation on servers; §5.4's client-side derivation is presentation of a server's state, but worth a claims check if built |

Not found or not checked: a patent by Cloud Imperium on server meshing or the replication layer (none
found); a patent on promoting NPCs to full simulation near a player (none found; the technique is in
Brom 2007 and Chenney 1997); Ubisoft's Census, Hadean's Aether Engine and EP or GB equivalents of the
above (not searched).

---

## Checked and left out

- **Vuntra City's devlogs' content:** the four videos could only be identified by title (YouTube's page
  text and descriptions did not load); the X account refused the fetch (402); the conference talk named
  in a LinkedIn post (search record) was not on the Everything Procedural Conference 2026 page. How the developer's
  GPU evaluates schedules and stores four million people is not in any text found.
- **A GDC 2024–2026 talk on very large persistent populations:** searched, none found; the material of
  those years is developers' posts and transport research.
- **Cities: Skylines II's own diaries on citizens' simulation cost:** Paradox's forum was not fetched;
  Iceflake's City Corner blog was read through PCGamesN's report, not in the original.
- **Cities: Skylines (2015)'s record, unit and instance limits:** Steam threads, throttled (429); search
  records only.
- **SimCity 2013:** Willmott's slides are a PDF (not fetched); PC Gamer's fact-check page loaded without
  its article (search record).
- **Workers & Resources** (wiki 403), **Mount & Blade II's dev blogs** (403), **X4's change logs** (429),
  **Anno 1800's wiki** (not opened): search records only, used as one combined entry each.
- **MATSim's book chapters** (Ubiquity Press 403), the Hermes paper and the Rust working paper of Laudan
  et al. (PDFs, not fetched): the loop's description rests on secondary search records.
- **Guy et al. 2011:** the Eurographics library refused (403) and the UNC project page is gone (404):
  search record.
- **Star Citizen's Quantum in Massively OP's report** (403): the RSI AMA and the wiki were read instead.
- **RimWorld's world pawns and their garbage collection:** only mod pages and forum claims, no official
  source; left out.
- **"The Absence" (Permafrost, 2016):** the Unreal forum post was read (about 800 persistent NPCs on a
  simulation thread with variable frequency and simulation LOD) but gives no numbers worth an entry; the
  ModDB original refused (403).
- **Kenshi's "Virtual Simulation Engine" mod** (background NPCs whose identity passes to a spawned
  NPC): itch.io refused (403).
- **EVE: Aether Wars (Hadean, GDC 2019, 14 274 connected clients, mostly bots):** search records; a
  cloud tech demo, not a population.
- **Unity's Megacity Metro:** about players per server (128+), not populations; left out.
- **The City Sample's pedestrian and vehicle counts:** not confirmed by Epic's page today; the sibling's
  press figure stands.
- **The cost of the Generative Agents run:** not checked.
- **Contraction hierarchies and HPA\* as routing references:** named as alternatives, not re-verified
  today.
- **The server PC's CPU model and RAM:** not recorded; §6 states its assumptions.

---

## Verification notes

Checked on 2026-10-10 with WebFetch and WebSearch only; no browser pane, nothing downloaded, no PDF
fetched. "Read" means the page (or a public API's JSON of it) was returned and the facts come from it;
"search record" means only a search engine's extract was seen. Sources are paraphrased; one phrase of the
Vuntra City developer is quoted.

- **Read:** UESP "Skyrim:Time"; alt:V "Time and date"; 4Gamer (27 July 2021) and Ubisoft News (30 March
  2021) on Census; Steam's news API for Nivalis Nights (app 1488490) and Vuntra City (app 2859220); the
  Vuntra City store page, press kit and 80.lv article (24 July 2023); Bluesky's public API for
  `vuntracity.bsky.social` (posts of 16 July – 5 October 2026); YouTube oEmbed for three devlog titles;
  AAAI's pages for Sunshine-Hill & Badler (2010) and Ryan et al. (2015); matsim.org's Hermes news (30
  October 2020) and book page; TU Berlin's DepositOnce record of Laudan et al. (2025); SUMO's Meso page;
  Brown's abstract at maths.tcd.ie; NVIDIA's FLAME GPU blog (10 February 2023); arXiv abstracts 2007.03614,
  2406.08496, 2409.10568 (and its HTML), 2304.03442, 1802.09335; FH Upper Austria's thesis record (2025);
  IDEAS for Bowman & Ben-Akiva (2001); ActivitySim's README; BLS's ATUS release (25 June 2026); Bilkent's
  record of Durupınar et al. (2011); the Dwarf Fortress Wiki's "Personality trait"; the RimWorld Wiki's
  "Opinion"; Emily Short's blog (28 May 2019); Paradox's CS2 feature page (28 August 2023); Huhtala's blog
  (5 November 2023); PCGamesN (10 April 2026); Game Developer (7 March 2012); MCV (15 March 2013);
  Willmott's talk page; the Transport Fever 2 wiki's "Towns"; Paradox's Victoria 3 dev diary 56 (24 August
  2022); EVE's "Tranquility Tech IV" (8 March 2023); RSI's Birthday AMA Recap (9 October 2020);
  starcitizen.tools "Quantum (game system)"; GDC Vault 1028981 (abstract); the REDmodding wiki's
  "Creating Communities"; Epic's `UMassReplicationSubsystem` page (5.8); Fowler's "Event Sourcing";
  Wikipedia's "GeForce 30 series"; the Unreal forum post on "The Absence" (24 October 2016); Google
  Patents US10579434B2, US10878146B2, US11936734B2, US11071916B2, US10315113B2.
- **Search record only:** the FLAME GPU 2 paper; MANTA's routing and OD figures; MATSim's queue and scoring descriptions; the Rust working
  paper of Laudan et al. (2024); the ActivitySim model clusters; Guy et al. (2011); Durupınar et al.'s
  2008 paper; Sunshine-Hill's dissertation; Victoria 3's first dev diary; Cities: Skylines (2015)'s
  limits; PC Gamer's SimCity fact-check; the Transport Fever developer post; Workers & Resources; Anno
  1800; Bannerlord's blogs; X4's low attention; the Vuntra talk's LinkedIn post; Improbable's US
  10,839,116, 11,416,305, 11,816,402 and Activision's US 11,524,237.
- **Refused or empty:** x.com (402), MDPI (403), cacm.acm.org (403), Ubiquity Press (403), the
  Eurographics digital library (403), UNC's personality page (404), wiki.hoodedhorse.com (403),
  taleworlds.com (403), Steam community pages (429), massivelyop.com (403), moddb.com (403),
  itch.io (403), techpowerup.com (403), Eurostat's Statistics Explained article pages (rendered without
  data), YouTube watch pages (no description text).
- **From Forge, read in the repository:** `CLAUDE.md`; `docs/research/galaxy-sky.md` (the format),
  `game-ai-ecosystems.md`, `smart-systems.md`, `netcode.md`; `docs/DECISIONS.md` D-005, D-006, D-010
  (with its first-stage note), D-015, D-016 (with the 2026-10-10 amendment), D-037, D-053;
  `docs/ARCHITECTURE.md`'s `forge-sim` row; the owner's machine notes for the server PC (RTX 3080,
  8 threads; nothing about its RAM).
- **Numbers to re-check before a spec:** every *(estimate)*: events per person-day, µs per event, the
  Hermes scaling, bytes per person and the totals, the T1 per-agent cost (the sibling's 20–50 µs), the
  bandwidth arithmetic, the snapshot's compression ratio, and the server PC's CPU and RAM.
