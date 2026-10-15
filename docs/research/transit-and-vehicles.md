# Research — Transit and vehicles: trams, public automated taxis, river boats, walking and cycling, and the vehicles a player rides and pilots

> Companion to `city-generation.md` §12–§13 (transit-first street profiles, taxis on paved tram lanes
> only, the 4 m emergency strip, Ghent's and Pontevedra's filters, the vehicles a car-free city still
> needs), `population-simulation.md` (§1.1 the clock at four game seconds per real second, §1.2 the tiers
> T3 to T0, §1.3 the hand-offs, §1.4 events and SUMO's mesoscopic queues, §5 interest and bandwidth),
> `people.md` (§4.2 smart objects, §4.5 first person, §5.6 the smart pad, §7 netcode for people),
> `game-ai-ecosystems.md` (§2 the lane graph and Watch Dogs 2's vehicle replication, §5 ship AI),
> `physics-fluids.md` §3 (Jolt's `CharacterVirtual` and `VehicleConstraint`, Kerner's boats),
> `netcode.md`, D-009 (Jolt), D-010 (netcode), D-016 (determinism), D-037 (cells), D-038 and D-041 (the
> water and the rivers), and the physics lab (`docs/demos/physics-lab.md`). Written 2026-10-10 for the
> owner's question of that day: a city sandbox, a current European city a few years ahead and much
> greener, by the sea with a river, whose transport is public and all electric — trams; public automated
> taxis hailed in the street or from a smart pad, which may run on tram tracks laid flush with the street;
> electric river taxi-boats crossing at several points; no private cars or vans — with ships, helicopters,
> planes and drones later; a first-person player who boards, rides and later pilots; live times on the
> pad and at the stops from the real simulation; a million people at first, a game hour of 15 real
> minutes, multiplayer with a dedicated server from the start. The sizing for that one city is in an
> appendix kept with the sandbox, which is not published; this note stays engine-generic. Sources were
> checked with WebFetch, WebSearch and `gh` only, no browser pane, nothing downloaded; each entry says
> whether it was read or only seen as a search record ([Verification notes](#verification-notes)).
> Numbers marked *(estimate)* are this note's arithmetic.

The short answer: **the transit layer is a timetable with delays, not a traffic simulation, and the
clock is its hardest number.**
- **Trams and boats are runs on a timetable** (GTFS's model: routes, trips, stop times, frequencies).
  A run's position is a pure function of its schedule, its delay and a speed profile along its track.
  The server, every client, every stop display and the pad therefore compute the same tram from a few
  bytes. The event-driven population boards and alights as events, the way MATSim's and SUMO's transit
  models work.
- **The clock changes the arithmetic.** At four game seconds per real second, with vehicles moving at
  real speeds, a game day's trips arrive in six real hours. A network or a fleet copied from a real city
  of the same population carries about a quarter of its trips at the peak, unless the trips are about
  four times shorter.
  - A 45 m tram every three minutes carries about 5 800 people per real hour each way, but only about
    1 450 per game hour *(estimate)*.
  - A taxi fleet that would carry 7 % of a million people's trips in life (about 3 700 vehicles) carries
    under 2 % here *(estimate)*.
  - Capacity is planned in real time against demand in game time; §1.5 has the arithmetic.
- **Public automated taxis are a dispatcher, not drivers.**
  - Requests are inserted into vehicles' routes (SUMO's taxi device, MATSim's DRT) in batches every few
    seconds, from fixed stands rather than arbitrary kerbs.
  - Idle vehicles return to stands, and the quoted wait feeds the population's mode choice.
  - Fleet size follows Little's law in real time. Wait follows the square root of the idle vehicles'
    density, until too few are idle and the "wild goose chase" sets in.
  - Pooling and rebalancing methods are heavily patented (Uber, Lyft, MIT, Didi, live to 2035–2041); the
    published open ones avoid them.
- **On tram track, taxis are guests.** Street trams drive on sight with priority at junctions. Taxis may
  use a paved tram lane where local law admits them (it varies: Germany excludes other users when buses
  share a track). In the engine they enter a shared segment only in a booked gap between trams and never
  stop on it.
- **River boats are frequency shuttles.** Near a player, a boat is a Jolt body on the buoyancy, river
  current and wakes the engine already has; it docks itself under a PD controller, as the autonomous
  ferries in Trondheim and Stockholm do.
- **Walking and cycling carry the largest share** in every transit-first city measured (Vienna,
  Zurich, Copenhagen, Utrecht: 41–74 % on foot or by bike). Mode choice is a logit over a generalised cost in
  game minutes, with waiting and walking weighted above riding.
- **Near players:**
  - trams are kinematic along their track spline, sections placed from bogie points;
  - taxis are kinematic on lanes, with the Intelligent Driver Model;
  - boats and player-piloted craft are dynamic.
  The hand-off follows the population's rule: the record says where.
- **The player** rides on Jolt's `CharacterVirtual`, which already stands on moving platforms in the lab.
  Inside a vehicle the controller runs in the vehicle's frame (Star Citizen's "physics grids"), seats are
  smart objects, and the cabin is the camera's rest frame.
- **Netcode:**
  - schedule-driven vehicles cost almost no bandwidth (only delays change);
  - a piloted vehicle is predicted by its pilot;
  - passengers predict only themselves, in the vehicle's space.
- **The pad** routes with RAPTOR over the timetable (Microsoft's patent on it has lapsed), with
  ULTRA's shortcuts for walking, bike and taxi legs later. It reads a GTFS-Realtime-shaped feed that the
  server's simulation writes.
- **Patents:** live on ride pooling and autonomous dispatch, on timing transit signal-priority requests
  (to 2037), lapsed on transit routing; none found on game vehicle hand-off.

---

## Contents

1. [Transit simulation](#1-transit-simulation)
2. [Public automated taxis as a fleet](#2-public-automated-taxis-as-a-fleet)
3. [River taxi-boats](#3-river-taxi-boats)
4. [Pedestrians and cyclists as the main traffic](#4-pedestrians-and-cyclists-as-the-main-traffic)
5. [Vehicle simulation near players](#5-vehicle-simulation-near-players)
6. [The player](#6-the-player)
7. [The smart pad and live displays](#7-the-smart-pad-and-live-displays)
8. [Later vehicles: ships, helicopters, planes and drones](#8-later-vehicles-ships-helicopters-planes-and-drones)
9. [What professional engines do](#what-professional-engines-do)
10. [Recommendation for Forge](#recommendation-for-forge)
11. [What to measure](#what-to-measure)
12. [Patents](#patents)
13. [Checked and left out](#checked-and-left-out)
14. [Verification notes](#verification-notes)

---

## 1. Transit simulation

### 1.1 Timetables or headways, dwell, priority, coupled trams, depots

**MobilityData / GTFS, "GTFS Schedule Reference".** [docs] (read)
<https://gtfs.org/documentation/schedule/reference/>

The open format nearly every journey planner reads.
- **Files:** agency, routes, trips and stop_times are required; stops and calendar are conditionally
  required. Optional files include shapes, frequencies, transfers, and pathways with levels for the
  inside of stations.
- **Route types:** 0 is "Tram, Streetcar, Light rail", 4 is "Ferry".
- **Stop times:** each stop of a trip has an arrival and a departure time, which may run past 24:00:00
  for a service day that ends after midnight.
- **Frequencies:** `frequencies.txt` gives a headway (`headway_secs`) over a time window. With
  `exact_times` empty or 0 the trips are frequency-based ("operators attempt to strictly maintain
  predetermined headways"); with 1 they are a compressed timetable with an exact headway.
- **Transfers and pathways:** `transfers.txt` gives a minimum transfer time between stops;
  `pathways.txt` describes a station as a graph of walkways, stairs, escalators, lifts and gates.

*Bearing:* the data model to copy, not to invent. A line is a set of stop patterns, a run is a trip with
stop times, and a high-frequency line is a headway over a window, not a list of departures. Forge's
network can be exported as GTFS, so a real journey planner can check it (§7).

**Carlos F. Daganzo, "A Headway-based Approach to Eliminate Bus Bunching: Systematic Analysis and
Comparisons", *Transportation Research Part B* 43(10), 2009, 913–921.** [paper] (abstract read)
<https://its.berkeley.edu/publications/headway-based-approach-eliminate-bus-bunching-systematic-analysis-and-comparisons>

On busy short-headway routes, vehicles "usually arrive irregularly at their stops, often in bunches",
and timetable slack cannot stop local disruptions from spreading. The paper sets holding times at control
points from real-time headways instead of the schedule; this needs less slack, so vehicles run faster.
*Bearing:* the control law for frequent lines.
- Below about 5 minutes, run by headway: hold a vehicle at a few control points (the terminals and a
  central stop) when the gap to the one ahead is short.
- Above about 10 minutes, run to the timetable, so the stop displays can promise times.
- In a deterministic simulation, bunching still appears as soon as dwell depends on boarding (below); the
  holding rule is what keeps it in check.

**Dwell time.** Three sources, none of them tram-specific in its numbers:
- **MATSim, `SimpleTransitStopHandler.java`** [repo] (read, the Doxygen source page
  <https://matsim.org/doxygen/_simple_transit_stop_handler_8java_source.html>). The simple handler's
  stop time is 4 per boarding passenger plus 2 per alighting passenger, plus a fixed 15 "for
  door-operations and similar stuff", summed, not split by doors. MATSim's vehicle types also carry
  per-person access and egress times (1 s by default) and a serial or parallel door mode (search
  record of `VehicleUtils`).
- **TRB, *Transit Capacity and Quality of Service Manual*** [docs] (search records of secondary papers;
  the manual was not fetched). Dwell is the busiest door's boardings and alightings times their service
  times, plus a fixed door time. The 2003 edition's boarding times run from 2.5 s per person with
  pre-paid fares to 4.2 s with magnetic cards; alighting is 2.1–3.3 s.
- **Christoforou, Chandakas and Kaparias, "Investigating the Impact of Dwell Time on the Reliability of
  Urban Light Rail Operations", *Urban Rail Transit* (2020)** [paper] (search record; Springer and the
  Southampton repository refused). Using the Nantes tramway's on-board passenger counts, boardings,
  alightings and on-board load all matter, as do the vehicle type (low or high floor), the time of day
  and the stop's location.

*Bearing:* dwell = door time (about 10–15 s) + the largest over the door channels of
(boarders × t_b + alighters × t_a). A low-floor tram with pre-paid fares has t_b ≈ 1.5–2.5 s and
t_a ≈ 1–2 s per door channel *(estimate, below the TCQSM's bus values)*, plus a crowding penalty when the
standing load is high. Twenty people through eight doors is a 20–25 s stop; a full tram unloading at a
hub is 45–60 s *(estimate)*.

**Wikipedia, "Transit signal priority".** [web] (read)
<https://en.wikipedia.org/wiki/Transit_signal_priority>

Passive priority times all signals for transit; active priority detects the vehicle and changes the
signals.
- **Green extension** holds a green up to a maximum for a vehicle about to miss it.
- **Early green** shortens the conflicting phases.
- **Phase rotation** reorders the phases.
- **Actuated transit phases** run only for a detected vehicle, often with a "T" signal for trams.
- **Phase insertion** serves a phase twice in a cycle.
- Pre-emption, which ends conflicting phases at once, is for emergency vehicles.

*Bearing:* the junction controller's vocabulary. A tram requests its phase at a detection point ahead of
the junction, and the controller extends or advances it. Emergency vehicles pre-empt (`city-generation.md`
§13 already routes them through filters).

**Light Rail Transit Association, "Manchester Metrolink — signalling".** [web] (read)
<https://www.lrta.info/archive/Manchester/tram_ms.html>

A shipped system that mixes the two worlds.
- The former railway lines used track-circuit block signalling; the city-centre street running and the
  newer lines run on sight.
- There is no central interlocking. Trams carry their timetable data and route themselves at junctions,
  first come, first served: a transponder sends the route request to a loop, and a proceed aspect shows
  when the junction is free.
- Points are detected so they cannot move under a tram (axle counters off street, mass detectors or track
  circuits on street). Flashing beacons warn of trams passing signals at single-line sections and blind
  junctions.
- On street, tram detection asks the highway signal controller to change the traffic lights and the tram
  signals.

**German Wikipedia, "Fahren auf Sicht".** [web] (read)
<https://de.wikipedia.org/wiki/Fahren_auf_Sicht>

Driving on sight means a following driver must always be able to stop safely before the vehicle ahead.
It is the usual mode for trams, mostly on their own track bodies; Swiss trams generally run on sight;
German trams also follow road traffic law.
*Bearing:* street trams need no block signalling, which makes the simulation cheap. They need three
mechanisms:
- a following distance by sight and braking distance;
- point locking under a tram;
- a junction request at a detection point.
A shared segment that taxis also use needs one more rule (§2.5).

**Braking and acceleration: Wikipedia, "Bombardier CR4000" and "Siemens-Duewag Supertram".** [web] (read)
<https://en.wikipedia.org/wiki/Bombardier_CR4000> · <https://en.wikipedia.org/wiki/Siemens-Duewag_Supertram>

- **CR4000** (London Tramlink): 30.1 m, 70 seated and 138 standing, 80 km/h; it accelerates at
  1.2 m/s², brakes at 1.3 m/s² in service and 2.73 m/s² in an emergency.
- **Supertram** (Sheffield): 34.75 m, 86 seated and 155 standing; it accelerates at 1.3 m/s²; service
  braking is 1.5 m/s² by design (1.16 limited); emergency braking 3 m/s² by design (2.46 limited) with a
  magnetic track brake.

*Bearing:* the speed profile's constants. Accelerate at 1.2 m/s², brake at 1.2–1.3 m/s², emergency at
about 2.7 m/s². From 50 km/h a tram stops in about 74 m in service and 35 m in an emergency, which is the
sight distance it needs *(arithmetic)*.

**Coupled trams: Urban Transport Magazine on Munich (read); Manchester (search record).** [web]
<https://www.urban-transport-magazine.com/en/?p=36947>

Munich runs a two-section and a three-section Siemens Avenio coupled as one unit on line 20. Double
traction was approved by the technical supervisory authority in summer 2022, and regular service began on
4 October 2022; the rear cab runs empty. Manchester added trams to run more "doubles" at peak on two lines
after new substations (search record of a 2017 local news item).
*Bearing:* a run's *consist* (one or two units) is part of its record. Coupling happens at a depot or a
terminus siding, out of service. Stops on coupled lines must be twice as long. The power supply limits
how many doubles run at once.

**Depots** are not a simulation problem but a visible one: runs start and end there, so the first trams
of the day and the last of the night run empty (deadhead) between the depot and the line. That is a scene
a player sees at dawn and should find correct. The timetable carries these deadhead runs explicitly.

### 1.2 How transit simulators model them

**Eclipse SUMO documentation, "Simulation/Public Transport".** [docs] (read)
<https://sumo.dlr.de/docs/Simulation/Public_Transport.html>

- **Stops:** a `busStop` (alias `trainStop`) is an area on a lane, with a person capacity.
- **Stopping:** vehicles stop with a `duration`, which is a minimum, and an optional `until`, an absolute
  departure time that makes them wait for the timetable.
- **Headways:** a `flow` with a `period` gives a headway; looped routes can keep one vehicle cycling so
  that its delays carry over.
- **Boarding:** a stop's `extension` caps how much boarding may lengthen it.
- **Routing:** persons travelling by public transport need the timetable fully defined at load; a
  `gtfs2pt.py` tool imports GTFS.

*Bearing:* the same three ideas as GTFS (stops, stop times, headways) plus the two Forge needs most: a
dwell that grows with boarding up to a cap, and a minimum dwell with a "not before" time for timed stops.

**MATSim: "MATSim 13.0 released" (12 April 2021); the transit schedule API; SwissRailRaptor.** [web]
[docs] (release read; the API pages as search records)
<https://matsim.org/news/2021/matsim-130-released-and-new-maven-repository/>

- **The schedule:** stop facilities, transit lines, transit routes, route stops and departures, read and
  written as XML; transit vehicles carry capacities and per-person access and egress times.
- **The router:** since 13.0, "SwissRailRaptor is now the default transit router". It is SBB's
  RAPTOR-based router, reported one to two orders of magnitude faster than the Dijkstra-based router it
  replaced (search records of the source and the SBB showcase).
- **The simulation:** transit vehicles in the queue simulation stop at their stops and exchange
  passengers (§1.1's handler).

*Bearing:* the closest published match to the population note's design. Agents with day plans route over
a timetable with RAPTOR, ride transit vehicles that have capacities, and are scored on their waits. The
only difference is that Forge's far tier runs events in real time instead of iterating a day.

**MATSim's DVRP and DRT extensions (Maciejewski, Bischoff, Hörl, Nagel; Springer 2017) and later DRT
work.** [paper] (search records)
<https://depositonce.tu-berlin.de/items/f53843ca-5257-4ee0-9c57-4c07485d418e>

A general dynamic vehicle routing extension, used for electric and autonomous taxis, demand-responsive
transport, personal rapid transit and car sharing. The DRT module on top of it assigns requests to
vehicles by pluggable algorithms, with several empty-vehicle relocation (rebalancing) strategies. A
Braunschweig–Wolfsburg study found 50 eight-seat vehicles cut the average peak travel time between the
two cities by 5 minutes.
*Bearing:* the open reference for §2's dispatcher. Insertion into routes, then rebalancing, as separate
strategies behind one interface.

**MobilityData / GTFS, "GTFS Realtime Reference".** [docs] (read)
<https://gtfs.org/documentation/realtime/reference/>

A feed of protocol-buffer messages fetched over HTTP. Each entity is one of:
- **a TripUpdate:** the predicted progress of a trip, its StopTimeUpdates carrying a `delay` in seconds or
  an absolute `time`;
- **a VehiclePosition:** a position, the current stop, congestion and occupancy;
- **an Alert:** text with the entities it informs.

Occupancy is an enumeration from EMPTY through MANY_SEATS_AVAILABLE, FEW_SEATS_AVAILABLE,
STANDING_ROOM_ONLY and CRUSHED_STANDING_ROOM_ONLY to FULL and NOT_ACCEPTING_PASSENGERS, plus an
experimental percentage. The header carries a POSIX timestamp; feeds are full datasets.
*Bearing:* the shape of Forge's live feed (§7.2): per run, a delay at its next stops; per vehicle, an
occupancy class. Stop displays and the pad consume it; its fields are what a real passenger sees.

### 1.3 How games did it, and what players noticed

**Paradox wiki, "Public transport" (Cities: Skylines, 2015).** [web] (read)
<https://skylines.paradoxwikis.com/Public_transport>

- **Lines:** bus lines are stops on roads that close into a loop; trams need two loops or bidirectional
  stops because they cannot turn; ferries are lines between ferry stops.
- **Taxis:** no lines; a depot and stands answer requests.
- **Vehicles:** the count per line follows its length and the budget slider.
- **Capacities:** bus 30, tram 90, metro 150, train 240, ferry 80.
- **Known trouble:** stopped buses blocking traffic and each other, metro lines congesting,
  vehicles despawning in severe jams, and buses from distant depots stuck on their way.

**Players on Cities: Skylines' holding ("unbunching") and stalls.** [web]
- **Improved Public Transport 2** (read, Steam Workshop
  <https://steamcommunity.com/sharedfiles/filedetails/?id=928128676>): about 755 000 subscribers. It
  configures the base game's unbunching per line and shows waiting people per stop.
- **Unbunching holds:** players report buses held at a stop for up to five minutes, and two lines
  sharing a stop holding each other into a jam (search records of workshop discussions; the page refused
  with 429).
- **Empty departures:** a 2020–2021 thread reports vehicles that stop, begin boarding and leave with
  nobody (read, <https://steamcommunity.com/app/255710/discussions/0/2972897380393377307>).

**Players on Cities: Skylines II (2023).** [web] (read)
<https://steamcommunity.com/app/949230/discussions/0/4202489789151576150> ·
<https://steamcommunity.com/app/949230/discussions/0/4031346899449226999>

- **Stuck boarding** (31 January–1 February 2024): trams and subway trains stop leaving stations, with
  queues behind them. One reply traces it to a passenger marked to board who never arrives, so the
  vehicle stays in its boarding state. Looped lines fail where point-to-point lines work.
- **Vehicle counts** (November 2023): a player complains that a long line forces "10+" buses and asks to
  skip empty stops; a reply says the count per line can be set.

`game-ai-ecosystems.md` §2 has the sequel's routing costs (time, comfort, money) and its lack of agent
caps.

**Players on Transport Fever (2016) and Transport Fever 2 (2019).** [web] (read)
<https://steamcommunity.com/app/446800/discussions/0/1693785035827252843> ·
<https://steamcommunity.com/app/1066780/discussions/0/3878218962819462562>

- **The first game:** a player gives its route cost as walking time + ¼ of the first stop's headway + ½
  of each transfer stop's headway + time in vehicles (23 January 2018). No developer confirmed it.
- **The sequel:** players say waiting time no longer counts. "passengers will wait forever" and 30-minute
  lines still fill (24–27 September 2023); one disagrees.

**Players on Workers & Resources: Soviet Republic.** [web] (read)
<https://steamcommunity.com/app/784150/discussions/0/4338725867372876954>

- **Travel time:** a worker's shift stops growing at four hours of travel, so longer trips cut work time;
  beyond five hours of total travel, happiness and loyalty drop (24 February 2024).
- **Forced disembarking** leaves workers waiting for another vehicle, or boarding the same one again.
- Other threads say workers go to the closest workplace (search record).

**OpenTTD wiki, "Timetable", and JGRennison's OpenTTD patch pack README.** [docs] [repo] (read; the
README through `gh api`)
<https://wiki.openttd.org/en/Manual/Timetable> · <https://github.com/JGRennison/OpenTTD-patches>

- **OpenTTD:** timetables set waits at stations and travel times per leg. Autofill records the first run.
  Vehicles are spaced by offsetting their start dates by the timetable's length over their number; late
  vehicles speed up and early ones slow down. Trains are usually better served by "full load" orders.
- **The patch pack** adds automated timetables and separation, "scheduled dispatch" from timing points
  for clock-face timetables, and departure boards.

*Bearing,* what the players noticed, as rules:
1. **Bunching is the visible failure of frequent lines.** Its cure, holding, has its own failure: holds
   too long, or stops shared by lines holding each other. Hold only at a few control points, with a cap,
   and never let a holding vehicle block another line's stop (Daganzo, §1.1).
2. **Boarding must end.** Close the doors at a maximum dwell, whoever is still walking up; a person who
   misses the vehicle waits for the next. Cities: Skylines II's stalls came from an unbounded boarding
   state.
3. **Waiting must count in the choice.** When it does not (Transport Fever 2, per its players), frequency
   stops mattering and the network design loses its point. Counting half the headway at the first stop
   is the classic minimum.
4. **People need a reason to go far,** and a limit. Workers & Resources' travel-time limits and
   Cities: Skylines' walking ranges are crude versions of the logit of §4.2.
5. **Departure boards and timetables are things players want,** even in a train game (the patch pack's
   boards). In Forge they are free: the server already knows every run.

### 1.4 Capacity and crowding

The line's capacity: the UITP range of 3 000–11 000 people per hour per direction, and the sizes of 33 m
and 45 m trams (210–290 people), are in `city-generation.md` §12. The densities that define those
numbers:

**Wikipedia, "Crush load"; a light-rail study and Sydney planners (search records).** [web] (read for
Wikipedia)
<https://en.wikipedia.org/wiki/Crush_load>

Crowding is counted in standing passengers per m². Wikipedia calls six a m² "often considered the
practical limit" before serious discomfort. A Mauritius light-rail study treats 4 per m² as tolerable in
Europe and Australia and 5–6 as very crowded. Sydney's planners use seats + 4 per m² as nominal capacity
and seats + 1.5 per m² as operational capacity (search records).
*Bearing:* a vehicle's capacity is seats + standing area × a density, and the density is a design choice.
Use 4 per m² for "full" on the displays (GTFS-RT's STANDING_ROOM_ONLY) and 6 per m² as the hard limit at
which boarding stops (FULL); between them, dwell grows and comfort falls.

**G. Keith Still, PhD thesis, chapter 3 "Crowd dynamics"; Fruin's levels of service (search records).**
[web] (read for Still)
<https://gkstill.com/CV/PhD/Chapter3.html>

- **Flows:** the UK Green Guide's fifth edition sets 82 people per metre per minute on the level and 66 on
  stairs.
- **Density:** 4 people per m² is its limit for a moving crowd; speed falls sharply below about 1 m² per
  person.
- **Fruin's queuing levels** (search records of the 1971 tables): A above 1.2 m² a person; C 0.6–0.9
  ("standing without touching" is D, 0.3–0.6); F under 0.2.

*Bearing:* platform and boarding numbers.
- **The platform:** a 3 m × 45 m tram platform at Fruin C holds 150–225 people, about one tram's worth
  *(estimate)*.
- **The doors:** a 1.3 m double door passes about 1.8 people a second at the Green Guide's level flow
  *(arithmetic)*, which bounds t_b from below.
- **The near tier** needs these as smart-object capacities (platform zones, door slots); the far tier
  needs only the counts.

### 1.5 The clock: what four times real time does to transit

The population note fixes the clock at k = 4 game seconds per real second, with every tier moving at
real speeds (`population-simulation.md` §1.1). Day plans are written in game time: the morning peak is a
game hour, which is 15 real minutes. Vehicles, though, run in real time. The consequences, all
arithmetic *(estimate)*:
- **Demand per real hour is k times life's.** A game day's D trips arrive in 24/k real hours. The busiest
  game hour, about a tenth of the day's trips, arrives in 15 real minutes. For a vehicle that takes real
  minutes per trip, the peak arrival rate is k × D × 0.1 per real hour.
- **Line capacity per game hour is 1/k of life's.** A tram every 3 real minutes is 20 trams per real hour
  but 5 per game hour. A line carrying 5 800 people per real hour each way carries 1 450 per game hour,
  against a peak game hour that holds a real peak hour's demand.
- **Loads scale with trip length.** Passenger-km per real hour = k × (trips in the peak game hour) × L.
  To load the network as much as a real city of the same population loads its own, trips must be k times
  shorter.
- **A worked case, a city of a million with three trips a person a day, 30 % of them by tram, on about
  100 km of route (200 km counting both directions):**

  | Mean tram trip | Peak game-hour pkm | Average load, per direction per game hour | Busiest sections (×3–4) | Against a 45 m tram every 2.5 min (1 800 per game hour) or a coupled pair (3 600) |
  |---|---|---|---|---|
  | 4 km | 360 000 | 1 800 | 5 400–7 200 | 1.5–4 times over |
  | 2 km | 180 000 | 900 | 2 700–3 600 | coupled pairs suffice |
  | 1 km | 90 000 | 450 | 1 350–1 800 | single trams suffice |

  The same city in real time, with 4 km trips, needs 1 800 people per hour each way on average and
  5 400–7 200 on its busiest sections: well inside the UITP range, which is why real cities of a million
  manage with trams. In the game the same network carries about a quarter of the same trips.
- **Fleet sizes do not change, capacity does.** The number of trams on a line is its real cycle time over
  its real headway, as in life. What changes is what they can carry per game day.
- **Displays read in game minutes.** A tram every 2.5 real minutes is "every 10 minutes" on the stop's
  game clock, and its countdown falls a minute every 15 real seconds. A player waits 2.5 real minutes; an
  NPC's plan sees 10.
- **Commutes become long in game time.** A 400 m walk to a stop is 4.8 real minutes, which is 19 game
  minutes. A 4 km tram trip from door to door (two such walks, a wait, the ride) is about 90 game minutes.
  A 25-minute game-time commute is about 600 m on foot or 1.7 km by bike.

*Bearing:* this is the note's central number. Three ways out, which a city chooses between:
1. **Short trips:** strongly mixed land use and a distance-decaying destination choice, so most trips are
   1–3 km and go on foot or by bike (§4).
2. **More capacity:** coupled trams, more lines, a metro trunk.
3. **A lower share for the slow modes,** accepting that the tram serves the long trips.

The engine's job is to make the trade-off visible: a load-per-section check that reads the population's
plans and the network's capacity per game hour, run before any vehicle exists (What to measure).

### 1.6 What the event-driven population needs from transit

The population's far tier holds whereabouts and advances by events (`population-simulation.md` §1.2,
§1.4). Transit adds three kinds of record and five kinds of event, none of them per tick:

```rust
/// A run: one trip of a line's pattern, with its consist and its live state.
pub struct Run { pub pattern: PatternId, pub depart: GameTick, pub consist: Consist,
                 pub delay_s: i16, pub load: u16, pub next_stop: u8 }
/// A stop's queue for one line direction (FIFO of PersonId, intrusive like `next_here`).
pub struct StopQueue { pub stop: StopId, pub pattern_dir: PatternDirId, pub first: PersonId, pub count: u16 }
/// A leg of a trip (`population-simulation.md` §2.2's `Trip` gains a leg list).
pub enum Leg { Walk { links: RouteRef }, Bike { links: RouteRef },
               Ride { pattern: PatternId, board: u8, alight: u8 },
               Taxi { request: RequestId }, Wait { stop: StopId } }
/// Whereabouts, extended: waiting at a stop, or aboard a run or a fleet vehicle.
pub enum Whereabouts { At { place: PlaceId }, OnLink { link: LinkId, entered: u32 },
                       Waiting { stop: StopId, since: u32 }, OnRun { run: RunId, since_stop: u8 },
                       InVehicle { vehicle: FleetId, since: u32 }, Promoted { entity: Entity } }
```

**Events:**
1. A person reaches a stop and joins its queue.
2. A run arrives: the queue boards, up to the capacity left (density 6 per m²); the remainder stay
   queued, and denied boardings are counted.
3. A run arrives at a person's alighting stop: they leave and start their next leg.
4. A run's delay changes: its future arrival events move.
5. A taxi request is placed, assigned, picked up or dropped (§2).

**Costs** *(estimate)*:
- **Run events:** a run touches about 25 stops, and a 100 km network runs a few thousand runs a game day,
  so about 10⁵ stop events a game day.
- **Passenger events:** about three per transit trip (board, alight, perhaps a transfer); a million
  transit trips a game day is about 140 a real second.
- Both are noise beside the population's 4 600 link events a second (`population-simulation.md` §1.4).

**What the day planner reads:** a stop-to-stop travel-time table per time band (15 game minutes), built
once from the timetable with the router of §7.1, plus the half headway as the expected wait at the first
stop. With 200 stops that is 40 000 pairs × 96 bands × 2 bytes ≈ 8 MB *(estimate)*: mode and route
choice without a search per trip. The full router serves the pad, and re-plans when a disruption changes
the table.

**What promotion needs:**
- **A person aboard a run** is placed at a seat or a standing slot chosen by hashing (person, run), in
  the section the run's state says; §1.3 of the population note applies unchanged.
- **A person waiting** is placed on the platform zone by the same hash.
- **Doors are the seams:** boarding is a door event, as entering a shop is.

---

## 2. Public automated taxis as a fleet

### 2.1 Dispatch, matching and pooling

**Eclipse SUMO documentation, "Simulation/Taxi".** [docs] (read)
<https://sumo.dlr.de/docs/Simulation/Taxi.html>

A vehicle with a taxi device joins the fleet; persons request rides by mode.
- **Dispatch algorithms:**
  - `greedy`: the closest taxi by travel time, in order of reservation;
  - `greedyClosest`: each free taxi takes its closest customer;
  - `greedyShared`: picks up another passenger during a trip, within absolute and relative detour
    thresholds;
  - `routeExtension`: extends a route with pick-ups along it, within capacity;
  - `traci`: hands dispatch to external code.
- **Idle algorithms:** `stop` where the last drop-off was, `randomCircling`, or `taxistand`, which
  returns to parking areas defined as stands.
- **Stop times:** drop-off takes 60 s by default and pick-up 0.

*Bearing:* the open baseline, and the shape of Forge's dispatcher.
- **Greedy insertion** into existing routes within detour limits.
- **Idle vehicles return to stands.**
- **A pick-up and a drop-off** each take a fixed dwell.

**Javier Alonso-Mora, Samitha Samaranayake, Alex Wallar, Emilio Frazzoli, Daniela Rus, "On-demand
high-capacity ride-sharing via dynamic trip-vehicle assignment", *PNAS* (January 2017), as reported by MIT
News.** [paper] [web] (MIT News read; PNAS and CSAIL refused)
<https://news.mit.edu/2016/how-ride-sharing-can-improve-traffic-save-money-and-help-environment-0104>

Using about 3 million New York taxi rides:
- 3 000 four-seat cars could serve 98 % of the city's taxi demand, with an average wait of 2.7 minutes;
- 2 000 ten-seat vehicles could serve 95 %, against nearly 14 000 taxis in service;
- sending idle cars toward demand sped service up by 20 %.

The algorithm is called "anytime optimal". It is patented ([Patents](#patents)).
*Bearing:* pooling plus rebalancing cuts the fleet by about four for the same demand in a dense city. The
specific trip-vehicle graph method is MIT's to license, so Forge uses insertion heuristics, which are
older and open (SUMO, MATSim DRT).

**Paolo Santi, Giovanni Resta, Michael Szell, Stanislav Sobolevsky, Steven Strogatz, Carlo Ratti,
"Quantifying the benefits of vehicle pooling with shareability networks", *PNAS* 111(37), 2014.** [paper]
(read on PubMed Central)
<https://pmc.ncbi.nlm.nih.gov/articles/PMC4169909>

- **Data:** about 150 million New York taxi trips of 2011 (13 586 taxis).
- **Method:** a "shareability network" whose nodes are trips and whose links are feasible shares. Pairs
  are matched by maximum matching; triples need heuristics; four or more were intractable.
- **Results:**
  - with full advance knowledge, cumulative trip length falls by 40 % or more;
  - online, with a one-minute booking window and a 5-minute delay tolerance, travel time falls by 32 %;
  - counting the empty trips, the gains are 30 % and 24 %;
  - shareability saturates near 100 000 trips a day.
- MIT holds a patent on the matching ([Patents](#patents)).

*Bearing:*
- **Shareability nears its maximum at about 100 000 trips a day** (in New York's area). A city of a
  million reaches that only if its taxis carry about 3 % of its trips; below it, pooling finds fewer
  partners.
- **Pairs are the useful case:** pool two parties, rarely three.
- **A 5-minute delay tolerance** is the passenger's limit in real minutes, 20 in game minutes.

**Juan Camilo Castillo, Dan Knoepfle (and E. Glen Weyl, per the extended abstract), "Surge Pricing Solves
the Wild Goose Chase", EC '17.** [paper] (Microsoft Research record read; extended abstract as a search
record)
<https://www.microsoft.com/en-us/research/?p=391988>

When supply is too low for demand, idle vehicles are "too thinly spread throughout a city". Vehicles are
then sent far to pick people up, which takes them off the road and deepens the shortage. The authors find
it in Uber's Manhattan data; their remedy is surge pricing.
*Bearing:* the failure mode of an under-sized public fleet, which has no surge price. The public
operator's remedies:
- cap the pick-up distance;
- quote the wait and let the request queue;
- let the population's mode choice see the quoted wait, so demand falls back to the trams and bikes
  instead of feeding the chase.

### 2.2 Rebalancing idle vehicles

**Marco Pavone, Stephen L. Smith, Emilio Frazzoli, Daniela Rus, "Load Balancing for Mobility-on-Demand
Systems", Robotics: Science and Systems VII (2011); journal version "Robotic load balancing for
mobility-on-demand systems", *IJRR* 31(7), 2012.** [paper] (RSS abstract read)
<https://roboticsproceedings.org/rss07/p34.html>

A fluid model of customers and vehicles between stations ("nonlinear time-delay differential
equations"). The optimal rebalancing policy, which minimises the vehicles making empty trips, is the
solution of a linear program; at equilibrium every station has excess vehicles and no waiting customers.
*Bearing:* for a fleet working from fixed stands, rebalancing is a small linear program solved every few
game minutes. Its inputs are the expected requests per stand per time band (from the population's plans)
and the expected arrivals; its outputs are empty trips between stands. Published in 2011, it predates
the fleet-objective patents of [Patents](#patents).

### 2.3 Published fleets and their numbers

**Waymo:**
- **Weekly rides** (Claims Journal's copy of Bloomberg, 12 February 2026; read): co-CEO Tekedra
  Mawakana said Waymo gives about 400 000 paid rides a week in six US cities and aims at more than a
  million a week by the end of 2026.
- **Fleet** (TechCrunch, 24 February 2026; read): about 3 000 robotaxis in six markets, ten cities
  served.
- **Later** (an Internet Info Agency report of 9 July 2026; read): about 3 500 robotaxis, more than 20
  million trips in all, about 500 000 paid rides a week earlier in the year.
- Waymo publishes no official fleet count; the counts are the press's.

<https://amp.claimsjournal.com/news/national/2026/02/12/335668.htm> ·
<https://www.techcrunch.com/2026/02/24/waymo-robotaxis-are-now-operating-in-10-us-cities/> ·
<https://english.news18a.com/news/english_273576.html>

**Baidu Apollo Go** (CnEVPost, 28 July 2026; read):
- more than 22 million public rides by April 2026;
- 3.2 million fully driverless rides in the first quarter of 2026, up over 120 % year on year;
- 27 cities by May;
- over 330 million autonomous kilometres, 220 million of them driverless;
- no fleet size given.

<https://cnevpost.com/2026/07/28/baidu-apollo-go-robotaxi-road-tests-london/>

**Zoox:**
- **Before fares** (Insurance Journal, 25 March 2026; read): more than 350 000 riders and nearly
  2 million autonomous miles in purpose-built robotaxis.
- **Paid rides** (Automotive World, 6 August 2026; read): paid rides in Las Vegas from 10 August 2026,
  after a commercial exemption from NHTSA (up to 5 000 units over two years) for a vehicle with no manual
  controls. The article gives both "almost a million passengers" and "half a million riders", which it
  does not reconcile.

<https://amp.insurancejournal.com/news/west/2026/03/25/863277.htm> ·
<https://www.automotiveworld.com/news/zoox-moves-from-free-rides-to-paid-service-in-las-vegas/>

**Waits in service: the Obi study as reported by Singularity Hub (2 February 2026).** [web] (read)
<https://singularityhub.com/2026/02/02/waymo-closes-in-on-uber-and-lyft-prices-as-more-riders-say-they-trust-robotaxis/>

Nearly 100 000 rides in San Francisco between Thanksgiving and New Year gave average quoted waits of
3.28 minutes for Uber, 4.20 for Lyft, 5.74 for Waymo and 15.32 for Tesla (under 200 vehicles over 400
square miles). Waymo's waits rose with "a capacity crunch during the 4 pm to 6 pm rush". Off-peak,
especially early in the morning, Waymo often had the shortest waits.

**Daniel J. Fagnant, Kara M. Kockelman, "The travel and environmental implications of shared autonomous
vehicles, using agent-based model scenarios", *Transportation Research Part C* 40 (2014); IEEE Spectrum
on its Austin model (21 November 2014).** [paper] [web] (CoMSES abstract and IEEE Spectrum read)
<https://catalog.comses.net/publications/859> · <https://spectrum.ieee.org/robotic-taxis-could-slash-fares-in-austin-texas>

- **The model:** in a 10 × 10 mile model of Austin, 1 715 shared autonomous vehicles served 56 324
  person-trips a day.
- **Waits:** an average of a bit over a minute, with 99 % of travellers waiting 10 minutes or less.
- **Fleet:** each vehicle replaced about 11 cars, for up to 10 % more travel.

**ITF/OECD, "Urban Mobility System Upgrade" (Lisbon, 2015).** [web] (search records; the ITF pages
refused)
<https://www.itf-oecd.org/sites/default/files/docs/2015-04-30-cpb-selfdriving.pdf>

- **Fleet:** shared self-driving fleets ("TaxiBots") with high-capacity public transport removed 90 % of
  cars in the model, with about a third of the fleet needed at peak.
- **Travel:** vehicle-km rose 6 % because the taxis also replaced buses.
- **Electricity:** an electric fleet needed only 2 % more vehicles for charging.

*Bearing,* the calibration points for §2.4:
- **Utilisation:** Fagnant and Kockelman's model ran 33 trips a vehicle a day. Waymo's ~500 000 rides a
  week on ~3 500 vehicles is about 20 a vehicle a day in sprawling US cities *(estimate)*.
- **Waits:** 2.7–6 minutes in real dense cities, rising in the rush when the fleet is short.
- **The best case:** shared fleets work best feeding high-capacity transit (Lisbon), which is the role a
  transit-first city gives them.

### 2.4 Wait time against fleet size for a city of a million

The model, from Little's law and the nearest-vehicle distance, all *(estimate)*:
- **Demand:** D = 3 million trips a game day; a taxi share s; a fraction p of the day's taxi trips in the
  peak game hour (8 %, flatter than commuting because taxis serve nights and errands); a pooled
  occupancy o = 1.4. The peak rate is D·s·p·k/o vehicle trips per real hour.
- **Time per vehicle trip:** τ ≈ 14 real minutes for a 3 km trip at 18 km/h door to door (tram lanes at
  30 km/h, shared streets at walking pace). That is 10 minutes driving, 15 % pooling detour, about 2
  minutes to the pick-up and 1.5 minutes of stops.
- **Fleet:** busy vehicles B = rate × τ. The fleet N = B / 0.75, keeping about 15 % idle and 10 % charging.
- **Wait:** for I idle vehicles spread over an area A, the expected rectilinear distance to the nearest is
  √(π/8) · √(A/I) ≈ 0.63 √(A/I) (the Poisson nearest-neighbour expectation under the rectilinear
  metric). Times 1.3 for the street network, at 18 km/h, plus 15 s of batching, doubled for the spatial
  imbalance real fleets show, gives the wait.

| Taxi share of trips | N in life (k = 1) | N at k = 4 | Peak wait at k = 4, A = 70 km² (real min) | Off-peak wait (real min) |
|---|---|---|---|---|
| 1 % | 530 | 2 100 | about 3 (12 game min) | about 2 |
| 2 % | 1 070 | 4 300 | about 2.3 (9 game min) | about 1.5 |
| 3 % | 1 600 | 6 400 | about 2 | about 1.3 |
| 5 % | 2 700 | 10 700 | about 1.7 | about 1.1 |
| 7 % | 3 700 | 14 900 | about 1.5 | about 1 |

Read the table as: **wait is cheap, capacity is dear.**
- **Wait changes slowly:** it falls with the square root of the idle vehicles' count. Once a fleet keeps
  a few hundred vehicles idle, the nearest one is a minute or two away, whatever the share served.
- **The busy vehicles set the size.** At k = 4 a fleet that serves 7 % of a million people's trips is
  15 000 vehicles. Over a few hundred lane-km of tram and service lanes that is about 25 moving vehicles
  a lane-km at the peak, which crowds the trams' lanes. A fleet of 4 000 serves about 2 %.
- **Undersized fleets fail suddenly.** A fleet below its busy count does not degrade gently: idle
  vehicles vanish, pick-up distances grow, and the wild goose chase sets in (§2.1). So the fleet is
  capped, the dispatcher quotes the wait, and mode choice reads it.
- **The real numbers bracket this:** Uber's 3.3 minutes and Waymo's 5.7 minutes are real dense fleets
  with imbalance and traffic; Fagnant and Kockelman's one minute is a model with rebalancing.

### 2.5 Sharing tram tracks safely

The street rules are in §1.1 (on sight, points locked under a tram, junction requests at loops). The law
on who may share a tram lane differs by country, and the engine must let a city choose:
- **Wikipedia's "Reserved track"** (read for `city-generation.md` §12): a semi-reserved lane is
  normally trams only, or trams, buses, taxis and emergency vehicles.
- **German Wikipedia, "Busfahrstreifen"** [web] (read, <https://de.wikipedia.org/wiki/Busfahrstreifen>).
  The bus lane (sign 245) is for scheduled buses. Additional signs may admit taxis, which "should" in
  principle be admitted, and bikes, ambulances and electric cars. Buses may use a suitably paved tram
  track body by an exception permit, and in that case "other users are excluded".
- **Belgium** [docs] (search records of the 1997 and 2023 texts). The *site spécial franchissable* was a
  part of the road reserved for regular public transport (buses and trams), with taxis only where a panel
  admits them; the definition was repealed in 2023 when article 72.6 was rewritten, with transitional
  access kept for taxis.
- **France** [docs] (search records; Légifrance's page was not reached). A lane reserved for some
  vehicles is closed to the others (article R412-7); taxis use bus lanes only where an M9 "TAXIS" plate
  admits them.
- **Siemens's autonomous tram** [paper] (read, Palmer et al., "The Autonomous Siemens Tram", ITSC 2020,
  <https://arxiv.org/abs/2102.04034>). A Combino tram was shown in Potsdam at InnoTrans 2018 with a
  multi-modal sensor suite for localisation and for detecting obstacles and signals.

*Bearing:* "taxis on tram tracks" is legal practice in some places and excluded in others. Forge models
it as a per-segment rule (who may enter, at what hours) that the city chooses, beside the filters of
`city-generation.md` §12. On a shared segment the engine adds what a human driver does by eye, as a
booking:
1. **Enter in a gap.** A taxi may enter a shared segment only if the next tram's predicted arrival at the
   segment's entry is at least the taxi's crossing time plus a margin away. 300 m at 30 km/h is 36 s;
   with a 30 s margin, about 80–90 s of each 150 s gap between trams every 2.5 minutes are open
   *(estimate)*.
2. **Leave before the tram.** The segment is a resource booked for the taxi's crossing, as an
   interlocking locks a route. A taxi that cannot leave (its exit blocked) does not enter, the "keep
   clear" rule at junctions.
3. **Never stop on the track.** Pick-ups use bays beside it (`city-generation.md` §12's profiles); a
   taxi that must stop leaves the track first.
4. **The tram keeps priority.** If a booking breaks (a player blocks the taxi), the tram brakes on sight
   to the obstacle, and the event is logged as a conflict, a number to keep at zero.

### 2.6 Pick-up and drop-off in pedestrian streets

**ITF/OECD, "The Shared-Use City: Managing the Curb" (May 2018).** [web] (search records; the OECD pages
refused)
<https://www.oecd.org/en/publications/the-shared-use-city_3af82772-en.html>

The report models moving kerb space from parking to pick-up and drop-off zones for passengers and goods.
With actively managed kerbs, ride services and public transport can grow together, and traffic falls as
the share of shared rides rises. It recommends designating streets by their primary purpose.

**France's *aire piétonne* and *zone de rencontre* (Code de la route, R110-2).** [docs] (search records)
- **Aire piétonne:** only vehicles serving the zone may drive in it, at walking pace, and pedestrians
  have priority.
- **Zone de rencontre:** pedestrians may use the whole width and have priority over vehicles, which are
  limited to 20 km/h.
- Vauban's and Pontevedra's walking-pace access is in `city-generation.md` §12.

*Bearing:* in a car-free core the kerb is not free. Fixed **stands** do the work of Uber's dynamic
meeting points ([Patents](#patents)):
- **Where:** short bays on service streets and at tram hubs, about 300 m apart, with chargers.
- **The core:** walking-pace entry into pedestrian and shared streets only for flagged trips (reduced
  mobility, night drop-offs at the door after a set hour, luggage).
- **The pad** offers the nearest stand and the walk to it; a flagged trip may ask for a door.

### 2.7 Accessibility

`city-generation.md` §12–§13 have Hamburg's ALIKE (automated pooled shuttles booked in the transit app,
HOLON's shuttle with an automated ramp) and Ljubljana's free electric carts. Since then:
- **HOLON's KBA approval** (search records of Benteler's press release, 13 November 2025): the vehicle,
  now called "Holon Urban", received German approval for autonomous testing (§16 AFGBV). It carries up to
  15 people at up to 60 km/h, with an automated ramp, a secured wheelchair space, and audible and visual
  aids.
- **ALIKE's service** (the Hochbahn page): test drives without passengers since autumn 2025.

*Bearing:*
- **A share of the fleet** is ramp-equipped shuttles; the dispatcher matches a wheelchair request only to
  them.
- **Walking-pace driving** is a speed class on the lane graph (about 6 km/h); the IDM of §5.2 respects it.
- **Dwell:** the boarding time of a ramp trip is a parameter (1–2 minutes), not zero.
- **Measure** the wait of these trips separately.

### 2.8 Charging and energy

**Electrive, "Inductive charging project starts test phase in Oslo" (25 June 2020).** [web] (read)
<https://electrive.com/2020/06/25/inductive-charging-project-starts-test-phase-in-oslo>

Twenty-five Jaguar I-Pace taxis retrofitted for inductive charging; plates rated 50–75 kW set into the
ground at several taxi stands, so the taxis charge while queuing for fares. The project's partners are
Jaguar Land Rover, Momentum Dynamics, Fortum Recharge and the operator.
*Bearing:* charging at the stand, invisible, is the near-future look the brief wants (no cables, no
chargers in the way). A stand is a queue that charges.

**Energy** *(estimate)*:
- **Consumption:** a car-sized electric taxi uses about 0.15–0.25 kWh/km. The I-Pace's 90 kWh for about
  240 EPA miles is 0.23 (search records of the vehicle's specifications).
- **Driving:** at k = 4 a game day is 6 real hours, so a vehicle busy 75 % of it drives about 60–80 km a
  game day.
- **Charging:** that is 9–20 kWh, about 10–25 real minutes on a 50 kW plate per game day.
- **The fleet:** a fleet of 4 000 draws about 6–13 MW on average over the game day.
- **Charging share:** the 10 % charging reserve of §2.4 is generous; charging is a scheduling problem
  (charge when idle at a stand), not a fleet-size one.

---

## 3. River taxi-boats

### 3.1 The services

**French Wikipedia, "Navibus" (Nantes).** [web] (read)
<https://fr.wikipedia.org/wiki/Navibus>

Three lines in the TAN network, with TAN tickets valid.
- **N1:** Gare Maritime – Trentemoult since 18 June 2005, about 1.4–1.5 km in 6 minutes. Boats of 95
  passengers and 10 bikes run every 20 minutes, every 10 at weekday peaks, about 07:00–20:00, later on
  summer Fridays and Saturdays.
- **N2:** since 2020, 0.8–1 km in 8 minutes.
- **N3:** on the Erdre since 1995, 160 m in 3 minutes, on demand: passengers ring a bell at the pontoon.
  Its *Jules Verne 2* is an electric catamaran on two 5 kW hydrogen fuel cells with batteries, for 12
  passengers (25 by derogation) and 6 bikes.
- **Ridership:** 755 700 passengers in 2019.

*Bearing:* the owner's reference, in numbers: crossings of 160 m to 1.5 km, 3–8 minutes, every 10–20
minutes, on demand where demand is thin, bikes aboard, one fare with the trams.

**Wikipedia, "Batobus" (Paris).** [web] (read)
<https://en.wikipedia.org/wiki/Batobus>

- **Service:** nine stops along the Seine, every 30 minutes (15 in high season), mostly tourists.
- **Boats:** trimarans of up to 200 passengers with an engine at each end, which lets them turn on the
  spot; the river authority limits them to about 6 knots (11 km/h).
- **Ticketing:** 1.6 million journeys in 2007–2008; Paris's transit tickets were not accepted.

**French Wikipedia, "Voguéo" (Paris).** [web] (read)
<https://fr.wikipedia.org/wiki/Vogu%C3%A9o>

- **The trial:** a commuter shuttle on the Seine and Marne, from 28 June 2008 to 5 June 2011, from
  Austerlitz to Maisons-Alfort with five stops.
- **Service:** every 20 minutes at peak (15 from May 2009), with four 75-person catamarans (35 seated).
- **The end:** it stopped for legal reasons. A three-line network was abandoned when the only bid came at
  about €74 M a year against an estimated €25 M.

*Bearing:* a river line parallel to the trams is a leisure line unless it is much faster. Crossings are
what boats do better than anything else.

**Wikipedia, "Vaporetto" (Venice).** [web] (read)
<https://en.wikipedia.org/wiki/Vaporetto>

ACTV runs vaporetti of 210–220 passengers and motoscafi of 154 on scheduled lines, 24 hours a day.
Line 1 follows the Grand Canal and can take 45 minutes from the station to San Marco.

**Wikipedia, "Copenhagen Harbour Buses".** [web] (read)
<https://en.wikipedia.org/wiki/Copenhagen_Harbour_Buses>

- **Service:** run by Movia since 2000, in the city's fares; one route through the harbour (991 south,
  992 north) with eleven stops, and an opera shuttle.
- **Boats:** seven battery boats from Damen (2020), six in service and one in reserve, "all-electric".
  They charge at night with top-ups at the stops during the day.

**Amsterdam's IJ ferries: NH Nieuws (6 September 2024) and Baird Maritime on *NZK Pont 103* (11 October
2022).** [web] (read)
<https://www.nhnieuws.nl/nieuws/340251/amsterdamse-ponten-propvol-nu-vakantie-voorbij-is-brug-now> ·
<https://www.bairdmaritime.com/work-boat-world/passenger-vessel-world/ro-pax/vessel-review-nzk-pont-103-amsterdams-gvb-welcomes-fourth-electric-ferry-in-series/>

- **The IJ ferries:** GVB runs 14 free ferries on the IJ. Seven take 400 pedestrians or 310 cyclists, the
  others 300 or 240. From Buiksloterweg a ferry leaves every few minutes, and between 8:15 and 9:15 they
  are full; a bike bridge elsewhere comes no sooner than 2034.
- ***NZK Pont 103*** (North Sea Canal): 42 m, 400 passengers or 20 cars, 680 kWh of batteries, two
  300 kW motors, 9.1 knots. Both jetties have 1 600 kW shore power, and several round trips run on one
  charge.

*Bearing:*
- **Bikes dominate** a busy urban crossing's load.
- **A busy crossing** wants a ferry every few minutes and 300–400 places.
- **Charging happens at the jetty** during the dwell.
- **Crowding at the landing** is the visible failure.

**Ships Monthly on Candela's P-12 *Nova* (Stockholm).** [web] (read)
<https://shipsmonthly.com/news/worlds-first-electric-hydrofoil-ferry-line-takes-off-in-stockholm>

An electric hydrofoil on a 15 km commuter route from 29 October 2024: just over 30 minutes at
25 knots, against an hour by conventional ferry. The maker claims 80 % less energy; it makes "minimal
wake", so it may run fast where others are restricted, and it charges at a car fast charger.
*Bearing:* the near-future look for a longer river or coastal line: foiling, fast, quiet and wake-free.

### 3.2 Automated crossings and docking

**Baird Maritime on NTNU's *MilliAmpere 2*; New Atlas on Zeabuz/Torghatten's *Estelle* (8 October
2024).** [web] (read)
<https://www.bairdmaritime.com/work-boat-world/passenger-vessel-world/ferries/vessel-review-milliampere-2-norwegian-university-develops-autonomous-electric-ferry-demonstrator/> ·
<https://newatlas.com/marine/estelle-autonomous-electric-ferry-remote-supervision-trial/?amp=true>

- ***MilliAmpere 2*** (Trondheim): 8.5 × 3.5 m, room for 20 (12 in the trial), four azimuthing
  thrusters on batteries, dynamic positioning, and collision avoidance from radar, lidar and cameras. A
  control room ashore can take over. The public rode it for three weeks from late September 2022.
- ***Estelle*** (Stockholm, Kungsholmen – Södermalm across Riddarfjärden): a 12 m electric catamaran with
  four thrusters and 188 kWh of batteries, for up to 15 hours a day at 6 knots and 25 passengers, in
  commercial service since 2023. A crossing was started and supervised from Trondheim, 600 km away, with
  a deck officer on board.
- **The earlier plan** for Trondheim's channel (search records of NTNU's 2017 presentation): a crossing
  of just under 100 m in about a minute, called by pushing a button.

*Bearing:* "public automated" boats exist at the scale of a river crossing, with a person supervising
remotely or on board. Their control is dynamic positioning: a controller on position and heading
through thrusters, which is `game-ai-ecosystems.md` §5's PD law for docking. An on-demand call (a bell,
the pad) is the night service.

### 3.3 Simulating them

A crossing is a two-stop line run by frequency (GTFS route type 4 with `frequencies.txt`), and a river line
is an ordinary line with stops. The far tier needs nothing new: runs, queues, boarding by capacity (§1.6).
The arithmetic of a crossing, all *(estimate)*:
- **One way:** a 300 m crossing at 3 m/s (6 knots, Estelle's cruise) is 100 s, plus about 30 s to dock
  and 45–60 s at the landing (people and bikes over one ramp): about 3 real minutes. A 100 m crossing is
  about 1.8.
- **Frequency:** two boats crossing in opposite directions give a departure from each bank about every
  3 real minutes, which is 12 game minutes.
- **Capacity:** with 25 seats, about 470 people a real hour each way, or 120 a game hour. With 60 seats,
  1 100 and 280. With 300 places (an IJ ferry), 5 600 and 1 400.
- **At night:** one boat, or on demand from the pad or a bell.

The clock of §1.5 applies to boats as to trams: a crossing that would be comfortable in life with
25-seat boats needs two to four times the places at k = 4.

### 3.4 Driving them near the player

What the engine already has:
- **The lab's boat** (`--lab sea`, #138): Kerner's per-triangle buoyancy on the rendered sea's waves,
  sampled on the CPU from the same spectrum. It has pressure drag, skin friction and radiation damping, a
  2.6 kN outboard that pushes only while the propeller is under water, and a rudder, driven by the
  `Steer` command, so it records, replays and goes through `--net`.
- **The island's rivers** (#177): a floater sees a water plane fitted every tick, with the river's level
  along its course and its current. The current is 1.2 times the drawn speed mid-channel and 0.6 at the
  banks, fading into lakes; still water on lakes; the sea past the mouths. The wakes and bow spray take
  the bodies' velocities. A thousand barrels tick in 1.1 ms.
- **Rivers as ribbons with flow maps** (D-038, D-041), so the current a boat feels is the current the
  water shows.

What a passenger boat adds:
- **Thrusters,** not an outboard. Two to four thrust points with directions (azimuth) give the
  turn-on-the-spot behaviour of the Batobus trimarans and the autonomous ferries.
- **A docking controller.** A PD law on position and heading to a berth pose, with speed and approach
  limits. Within a few metres and below a speed it snaps to a mooring constraint (`World::join_distance`
  or a fixed joint) for the dwell.
- **A deck the player walks on:** a static mesh in the boat's frame, carried as §6.1 describes.
- **Wake budget:** a crossing boat's wake is the lab's wake; Wave Particles (`physics-fluids.md` §3) are
  the upgrade for many boats.

---

## 4. Pedestrians and cyclists as the main traffic

### 4.1 Mode shares in transit-first cities

**Wikipedia, "Modal share" (tables by city); the EU Urban Mobility Observatory on Ghent (19 October
2017); Streetfilms via eBikes International (9 September 2020); Nantes and Strasbourg (search
records).** [web] (Wikipedia, the Observatory and eBikes read)
<https://en.wikipedia.org/wiki/Modal_share> ·
<https://urban-mobility-observatory.transport.ec.europa.eu/resources/case-studies/gents-traffic-circulation-plan-belgium_en> ·
<https://ebikes-international.com/the-innovative-way-ghent-belgium-removed-cars-from-the-city>

| City (year) | Walking | Cycling | Public transport | Car | Source |
|---|---|---|---|---|---|
| Utrecht (2018) | 25.3 % | 48.4 % | 5.4 % | 18.7 % | Wikipedia table (read) |
| Copenhagen (2021) | 30 % | 26 % | 18 % | 26 % | Wikipedia table (read) |
| Vienna (2024) | 30 % | 11 % | 34 % | 25 % | Wikipedia table (read) |
| Zürich (2015) | 33 % | 12 % | 32 % | 21 % | Wikipedia table (read) |
| Paris (2020) | 40 % | 2 % | 22 % | 34 % | Wikipedia table (read) |
| Amsterdam (2020) | 5 % | 30 % | 19 % | 42 % | Wikipedia table (read; metro area, walking defined narrowly) |
| Ghent (2021) | 15.6 % | 33.8 % | 11.2 % | 39 % | Wikipedia table (read) |
| Ghent (2012 → 2030 target) | 15 → 18 % | 22 → 30 % | 9 → 20 % | 55 → 27 % | EU Observatory (read); cycling 35 % "now" in 2020 per Streetfilms (read) |
| Nantes métropole (2015) | 26 % | 3 % | 15 % | 43 % driver + 12 % passenger | EDGT 2015 via Auran (search records) |
| Strasbourg Eurométropole (2019) | 33 % | 8 % | not found | 37 % | PAMA (Jan 2019) and Conseil de développement (search records) |
| Pontevedra centre | 65 % on foot | | | | `city-generation.md` §12 |

*Bearing:* in the best transit-first cities, walking and cycling together carry 41–74 % of trips and
public transport 5–34 % (Utrecht's 5 % is the cycling city's), with cars at 19–26 %. A car-free city redistributes that car quarter among
bikes, transit and the few taxis. The tram cities the owner named (Nantes, Strasbourg) still have high
car shares. The cycling cities (Utrecht, Copenhagen, Ghent after 2017) show where a car-free city's trips
go.

### 4.2 How mode choice works

**Wikipedia, "Discrete choice".** [web] (read)
<https://en.wikipedia.org/wiki/Discrete_choice>

- **The model:** each alternative's utility is an observed part plus an independent extreme-value
  (Gumbel) error, so the probability of choosing i is exp(V_i) / Σ exp(V_j), the multinomial logit.
- **Its limits:** the "Independence of Irrelevant Alternatives" property gives unrealistic substitution
  between similar alternatives. Nested logit fixes it by grouping them.
- **Its origin:** McFadden's theory won the 2000 Nobel prize; mode choice to work (car, bus, rail) is the
  classic example.

**PTV Visum help, "Skims of time" (2025); Wardman, "Public transport values of time", *Transport
Policy* 11(4), 2004.** [docs] [paper] (Visum page and the White Rose record read; the multipliers as
search records)
<https://cgi.ptvgroup.com/vision-help/VISUM_2025_ENG/Content/1_Benutzermodell%20OeV/1_6_Kenngroessen%20der%20Zeit.htm> ·
<https://eprints.whiterose.ac.uk/id/eprint/3393/>

- **Visum:** perceived journey time combines access and egress, origin wait, transfer wait, the number of
  transfers, in-vehicle time and walking, with weighting factors set by the user.
- **Wardman:** a meta-analysis of British evidence on the value of walk, wait and headway time relative
  to in-vehicle time.
- **The multipliers** (search records): Wardman's later updates find walk and wait multipliers
  "substantially lower than the commonly used value of two"; a New Zealand model calibrates both at
  1.5–2.0.

*Bearing,* Forge's mode choice, one logit per trip at planning time, over generalised costs in game
minutes:
- **Walk:** 1.0 × time.
- **Bike:** 1.3 × riding time + 2 minutes of parking + an effort term growing with distance, nought for
  people without a bike or unable to ride, and a weather term.
- **Tram or boat:**
  - walking weighted 1.5–2;
  - the first wait at half the headway, weighted 1.5–2;
  - riding 1.0, plus a crowding factor when standing;
  - a transfer penalty of about 5 minutes;
  - each transfer's wait.
- **Taxi:** the dispatcher's quoted wait weighted 1.5, riding 1.0, a fare term if any. The taxi is
  available only to flagged trips at night and in the core.
- **Each person's traits** (age band, mobility, owns a bike, personality's patience) scale the terms.
  The choice is habitual: a person keeps their mode unless the costs move past a threshold. Nested logit
  (walk and bike in one nest, transit modes in another) avoids the "red bus, blue bus" substitution.

**Door-to-door times by mode, in real minutes** *(estimate: walking 5 km/h; bike 16 km/h plus 2 minutes;
tram 20 km/h with a 400 m walk at each end and a 2.5-minute headway; taxi 18 km/h with a 3-minute wait
and a minute of stops)*:

| Distance | Walk | Bike | Tram | Taxi |
|---|---|---|---|---|
| 1 km | 12 | 6 | 14 | 7 |
| 2 km | 24 | 10 | 17 | 11 |
| 3 km | 36 | 13 | 20 | 14 |
| 5 km | 60 | 21 | 26 | 21 |
| 8 km | 96 | 32 | 35 | 31 |

Multiply by four for game minutes. For a person who cycles, the bike beats the tram at every distance in
a city 10 km across. The tram's market is people who do not cycle, trips with luggage or children, rain,
and trips beyond about 4 km with a stop near both ends. That is why the cycling cities above have low
transit shares, and why the clock of §1.5 pushes a city toward bikes.

### 4.3 Bikes and bike share

**Wikipedia, "Vélib' Métropole"; Nantes Métropole on its bikes (July 2025); ITDP's Bikeshare Planning
Guide.** [web] (Vélib' read; Nantes and ITDP's numbers as search records; ITDP's introduction read)
<https://en.wikipedia.org/wiki/V%C3%A9lib%27_M%C3%A9tropole> · <https://bikeshare.itdp.org/guide/introduction/>

- **Vélib':** about 18 200 bikes at 1 465 stations, about one every 300 m in the centre, 390 000
  subscribers (2022), 85 811 trips a day in 2011.
- **Nantes:** 1 230 bikes at 125 stations and 41 000 subscribers in 2025.
- **ITDP's benchmarks:** 10–16 stations per km², 10–30 bikes per 1 000 residents, 4–8 trips per bike per
  day.

**Cycle-track capacity** (search records only): a Dutch study gives 3 000–3 500 cyclists an hour for a
0.78 m-wide cyclist lane; a US analysis 4 500 an hour on a 2.43 m path; reviews report 2 600–8 100 an
hour. `city-generation.md` §12 has Copenhagen's network.
*Bearing:*
- **Bike share:** a city of a million needs 10 000–30 000 shared bikes and docks every 300 m.
- **Cycle tracks become the busiest links** if bikes carry a third of trips: at k = 4, a 2–2.5 m track
  reaches a few thousand an hour each way at the peak *(estimate)*.
- **The bike network** needs the same load check as the trams (What to measure). Bikes need parking at
  every stop and every door.
- **In the simulation,** a cyclist is a walker with another speed on the bike graph (T3), and a body with
  a bike near players (T1); `people.md` has the animation side.

### 4.4 Crowds at stops and on platforms

**Dirk Helbing, Péter Molnár, "Social force model for pedestrian dynamics", *Physical Review E* 51,
4282–4286 (1995).** [paper] (arXiv record read)
<https://arxiv.org/abs/cond-mat/9805244>

Pedestrians move as if driven by "social forces", measures of their internal motivations rather than
physical pushes. The model has an acceleration toward a desired velocity, repulsion from other people
and from walls, and attraction. Its simulations reproduce observed collective effects (lanes, oscillation
at bottlenecks).
*Bearing:* the crowd at a door is the one place where people press together; ORCA (`game-ai-ecosystems.md`
§2) keeps its agents apart and does not press. For a platform's boarding crowd:
- **Agents:** T1 agents with ORCA and a density term.
- **Order:** boarding in door order with slot reservations (people's smart objects).
- **Far tier:** counts only.

The ceilings are §1.4's: Fruin's queuing levels for the platform, the Green Guide's flows for the doors
and stairs.

---

## 5. Vehicle simulation near players

### 5.1 Trams on rails

**Jorrit Rouwé, Jolt Physics, `PathConstraint.h` and `PathConstraintPathHermite.h` (vendored in
`third_party/jolt`, MIT).** [repo] (read in the repository; GitHub's architecture page refused with 503)
<https://github.com/jrouwe/JoltPhysics>

- **What it does:** a constraint "used to constrain the degrees of freedom between two bodies to a path",
  with a Hermite-spline path whose points carry a position, a tangent and a normal.
- **Rotation:** the body can be left free, constrained around the tangent, normal or binormal, made to
  follow the path's frame, or fully constrained.
- **Motion:** a motor along the path, or a friction force.
- **The path:** must be continuous without sharp corners; the constraint acts as a slider along the
  tangent.

*Bearing:* a tram does not need a dynamic constraint, but Jolt has one if a physics-driven rail vehicle is
ever wanted (a runaway, a crash). The service tram is kinematic, which is cheaper and exact.

**Wikipedia, "Bogie" and "Low-floor tram".** [web] (read)
<https://en.wikipedia.org/wiki/Bogie> · <https://en.wikipedia.org/wiki/Low-floor_tram>

- **Bogies:** some articulated trams carry a Jacobs bogie under each articulation. Low-floor trams often
  have non-pivoting bogies, which wear track and wheels and lower curve speeds.
- **Multi-articulated trams** (Citadis, Combino) use short sections over the wheels with longer sections
  between them.

*Bearing,* the tram near a player, every number deterministic:
1. **The run's distance along its track,** s(t), comes from the speed profile (accelerate 1.2 m/s²,
   cruise at the segment's limit, brake 1.3 m/s² to the stop's mark), the run's departure and its delay.
   The same integer-tick function runs on the server, the clients and the far tier.
2. **Each bogie** sits at a fixed arc-length offset behind the front: points on the track spline.
3. **Each carbody section with its own bogie(s)** is placed by its bogie points: position between them,
   heading from the vector joining them, up from the track's normal (cant). This avoids the flips that
   averaging rotations gives, which animation forums report.
4. **Each suspended section** (no bogie) hangs between its neighbours' joints: its ends at the
   articulation points, so it overhangs the curve's inside, as the real ones do.
5. **The sections** are kinematic Jolt bodies near players, so people, crates and balls collide with
   them and stand inside them. Doors and interior are a static mesh in each section's frame.

**Wheels and pantographs** are animation (wheel roll from s, bogie yaw from the spline). There is no
pantograph if the line is wire-free (`city-generation.md` §12).

### 5.2 Taxis

**Wikipedia, "Intelligent driver model".** [web] (read)
<https://en.wikipedia.org/wiki/Intelligent_driver_model>

Treiber, Hennecke and Helbing (2000). A car-following law:
- **Acceleration:** dv/dt = a [1 − (v/v₀)^δ − (s*/s)²], with the desired gap s* = s₀ + vT +
  vΔv / (2√(ab)).
- **Typical values:** v₀ = 30 m/s, T = 1.5 s, a = 0.73 m/s², b = 1.67 m/s², δ = 4, s₀ = 2 m.

*Bearing,* a taxi near a player is kinematic on its lane polyline (`game-ai-ecosystems.md` §2's `Lane`):
- **Speed:** the IDM gives its speed behind the vehicle ahead, the stop line, a booked tram gap (§2.5) or
  a person in a shared street (a virtual obstacle).
- **Body:** the chassis is a kinematic body, with a visual suspension (pitch and roll from the
  accelerations, wheels from the speed).
- **No dynamic vehicle:** the dynamic Jolt vehicle (the lab's `drive`) is for player-driven or crashed
  cars. An automated taxi that follows its path exactly is what an automated vehicle does.
- **Determinism:** kinematic taxis are as deterministic as trams. Their path is a list of lanes, their
  speed an integer-tick IDM step, and their obstacles the server's.

### 5.3 Boats

Section 3.4. A crossing boat near a player is a dynamic Jolt body, because water moves it and it bumps
pontoons. Its thrusters are driven by the docking controller or by a pilot. Far away it is a run on a
frequency, whose position along its crossing is the same speed-profile function as a tram's. At the
hand-off the body starts at the profile's position and velocity, and the controller holds it to the
profile.

### 5.4 Flying craft, later

The lab already flies:
- **an aeroplane** on lifting surfaces (`--lab fly`);
- **a rocket** with thrust vectoring and roll jets (`--lab rocket`);
- **a spaceship** with a flight computer that turns stick deflection into a commanded rate (`--lab space`).

A helicopter is the rocket's engine (a thrust along the rotor axis) with the stick tilting that axis
(cyclic), a collective for the thrust, a tail-rotor yaw, ground effect near the ground, and the
spaceship's rate controller for playability. A drone is the same with four fixed thrusts and the rate
controller always on. None of this exists yet; §8 has the airspace side.

### 5.5 The hand-off between the coarse simulation and the physical vehicle

The population note's rule, applied to vehicles:
- **One source of truth.** A run's or a fleet vehicle's record holds where it is: run + schedule + delay,
  or lane path + entered tick + speed state. Its body near a player holds only how.
- **Promotion places the vehicle where the record says,** at the profile's position and velocity, its
  passengers at their hashed slots.
- **Demotion writes back only a delay** (a tram held by a player's interference) or a new path (a taxi
  re-routed). There is no teleport to the schedule: the delay persists and the headway control absorbs
  it.
- **Hysteresis and sight:** a tram's sections become bodies within about 300 m of a player and stay while
  someone rides or watches. A taxi becomes a body within about 150 m.
- **Kinematic vehicles do not drift.** The body follows the same function as the record, so promotion
  and demotion are exact for trams, boats on profiles and taxis on paths. Only dynamic vehicles (boats
  near the player, piloted craft) can drift, and they write their position back on demotion.

`smart-systems.md` §3 has Take-Two's navigation patents (routes on a coarse graph); [Patents](#patents)
has what was searched for vehicle hand-off.

### 5.6 What physics-lab already gives

| Lab scene | What it has | What transit and vehicles reuse | What is missing |
|---|---|---|---|
| `walk` (#139) | Jolt `CharacterVirtual`: stairs to 40 cm, slopes to 45°, stands on a platform moving at 2 m/s and is carried by it (test: 1 m at 1 m/s), pushes light crates, commands `Walk`/`Jump`, record and replay | the player's controller; standing in a moving tram or boat | the vehicle's own frame for the controller (§6.1); seats; doors |
| `drive` (#140) | Jolt `VehicleConstraint`, cylinder-cast wheels on 1.6 Hz springs, 320 N·m engine with gearbox, brakes, `Steer`/`Handbrake` commands, 0.06 ms a tick | player-driven or crashed road vehicles; service vehicles if ever driven | kinematic lane-following with IDM (§5.2) |
| `sea` (#138) | Kerner buoyancy on the rendered waves, a 5 m boat with an outboard and rudder, `Steer` command, 1.8 ms a tick for 147 bodies | the boat's physics, its replay and `--net` | thrusters, docking controller, a walkable deck |
| `island` afloat (#177) | rivers' level and current per floater, wakes and spray from velocities, height-field banks | boats on the river | the river current read by a boat's hull (same call) |
| `fly` (#141), `rocket` (#148), `space` (#150) | lifting surfaces; thrust vectoring; a rate-commanding flight computer; momentum kept to 10⁻⁶ | planes, helicopters, drones later | a rotor model; airspace rules (§8) |
| `drop --net` (#137) | commands stamped with ticks, recordings, server and predicting clients, reconciliation by digest | the protocol under every vehicle | quantised snapshots and interest (Phase 5) |
| `materials` (#203) | friction and restitution from the material row | rail on steel, tyres on paving | — |

---

## 6. The player

### 6.1 Walk, board, sit, ride and get off

**Cloud Imperium Games, "Monthly Report — August 2014" (the Santa Monica section, by producer Travis
Day), reprinted on the Star Citizen Wiki.** [web] (read; the later "BTS – Multicrew & Physics Grids"
Comm-Link of March 2016 loaded only its introduction)
<https://starcitizen.tools/Comm-Link:Monthly_Report_-_August_2014>

With one global physics grid, a ship that pitches and yaws leaves the player behind, and the collisions
"fling you around like a pinball". Localised physics grids are smaller grids inside the global one; each
tracks its objects relative to what it is attached to. A grid attached to a ship keeps the player's
position relative to the ship, so the player moves with it while it manoeuvres.
*Bearing:* the rule for walking inside vehicles that move fast or turn (trams at 50 km/h, boats rolling,
later aircraft). The player's controller runs in the vehicle's frame:
- **The cabin's geometry** is a small static world in vehicle coordinates. Jolt's `CharacterVirtual`
  supports an arbitrary up and its own coordinate system (`physics-fluids.md` §3), and the lab carries it
  on platforms.
- **The vehicle's motion** is added as a frame transform, plus the felt acceleration as a gentle push
  (standing passengers sway, never fall unless the vehicle brakes in an emergency).
- **Inside or out:** a person is in the vehicle's grid when standing on its floor inside its volume.
  Doors are the seams, as for places (`population-simulation.md` §1.3).

**The sequence, for player and NPC alike** (people's smart objects, `people.md` §4.2):
1. **Wait** in a platform zone.
2. **Board** through a door when the vehicle's doors are open (a door slot, a reservation).
3. **Inside the grid,** stand in a standing slot or walk to a seat.
4. **Sit:** the seat is a smart object with a reservation, and sitting attaches the body to the seat's
   transform with the seated clip.
5. **Ride:** the camera follows the seat (§6.2).
6. **Get off:** stand when the next stop is announced, walk to a door, leave when it opens.

Boarding and alighting are server-validated interactions, never client positions.

### 6.2 Camera comfort in moving vehicles

**FAAC Realtime Technologies, "Simulator Sickness: Design Implications of the Rest-Frame Hypothesis" (17
February 2021), after Prothero et al. (1999).** [web] (read; Prothero's paper as a search record)
<https://www.faac.com/blog/2021/02/17/simulator-sickness-design-implications-of-the-rest-frame-hypothesis/>

Prothero's hypothesis: sickness comes from "conflicting rest frames deduced from those motion signals",
not from the motion cues themselves. The design answer is to give the brain a clear stationary reference,
an "independent visual background". The cues are the horizon and earth-fixed clouds, or the room itself,
seen past or through the screen.
*Bearing:* in a vehicle the cabin is the rest frame. Star Citizen's eye-then-head stabilisation is in
`people.md` §4.5.
- **Seated:** the camera is rigid to the seat, with no head bob, the cabin filling the edges of the view.
- **Standing:** the head is stabilised in the cabin's frame, not the world's; the body sways, the eyes
  hold.
- **Accelerations** show as a small, damped lean (a degree or two) and the passengers' sway, never as
  camera shake.
- **Windows** keep the horizon visible: the outside rest frame agrees.
- **The field of view** never changes with speed.
- **An option** reduces sway and lean to nothing.

### 6.3 Piloting: a boat first, flying later

A player-piloted boat is the lab's boat with the passenger boat's thrusters (§3.4). Its controls:
- **Throttle and rudder:** as `--lab sea`.
- **A docking assist:** the same PD controller as the automated boats, engaged near a berth on request.
- **Mooring:** a joint for the dwell.

Flying craft come after (§8), with the spaceship's rate controller as the playable layer. Who may pilot
what is a permission on the vehicle (a city's boats are automated; a hired boat or a private one is
piloted).

### 6.4 Netcode for vehicles shared by several players

D-010 already says the client predicts its own avatar and the physics group it touches, and interpolates
everything else 100 ms behind. For vehicles:

**Epic Games, "Chaos Modular Vehicles overview", Unreal Engine 5.8.** [docs] (read)
<https://dev.epicgames.com/documentation/en-us/unreal-engine/chaos-modular-vehicles-overview>

The modular vehicle system "comes with native support for the Network Physics component and resimulation
replication mode, which provides client predicted server authoritative physics (with rewind
resimulation)". Custom modules can add their own serialisation for resimulation.
*Bearing:* the shipped answer for player-driven vehicles is Forge's own (#137's reconciliation by
digest): the driver's client predicts the vehicle, the server is the authority, and corrections rewind
and replay.

Forge's rules:
- **Automated vehicles cost almost nothing.** A tram, a crossing boat or a taxi on a path is a function
  of replicated data: run + delay, or path + entered tick + speed state. Each client computes its
  position, as the far crowd's whereabouts are events (`population-simulation.md` §5.3). Only changes are
  sent: a delay of a few bytes per run per stop, a new path when re-routed. Many players riding the same
  tram see the same tram to the bit.
- **A piloted vehicle has one pilot.** The helm is a seat whose occupant holds the control token,
  arbitrated by the server; the pilot's client predicts the vehicle and its contact group, and the others
  interpolate it.
- **Passengers predict themselves in the vehicle's frame.** Their own movement inside the cabin is
  predicted in vehicle coordinates and replicated relative to the vehicle: 2 × 10 bits at 1 cm within a
  ±5 m cabin *(estimate)*. The vehicle's own lag (interpolated or predicted) then moves the whole cabin
  smoothly, and walking inside never jitters.
- **Projective velocity blending** for other players' piloted vehicles, when interpolation's 100 ms
  shows: Watch Dogs 2's talk in `game-ai-ecosystems.md` §2.
- **Interest follows the vehicle:** a rider's interest set is centred on the vehicle, with the next stops
  ahead preloaded.

**Bandwidth** *(estimate)*:
- **Runs and taxis:** a client near a tram line receives delay updates of about 4 bytes per run per stop,
  under 0.1 KB/s. A taxi on a path costs one path event per re-route.
- **A piloted boat** at 30 Hz, about 20 bytes a snapshot (position at 1 cm in its cell, a smallest-three
  quaternion, velocities), is 0.6 KB/s per boat seen.
- **Passengers** cost about 0.5 KB/s per rider seen.
- All of it fits inside D-010's 24 KB/s beside the people (`people.md` §7).

---

## 7. The smart pad and live displays

### 7.1 Routing

**Daniel Delling, Thomas Pajor, Renato F. Werneck, "Round-Based Public Transit Routing" (RAPTOR),
ALENEX 2012 and *Transportation Science* 49(3), 2015.** [paper] (extended abstract read at SoCS)
<https://ojs.aaai.org/index.php/SOCS/article/view/18270>

All Pareto-optimal journeys by arrival time and number of transfers, computed in rounds (round k = up to
k trips). It needs no Dijkstra and no preprocessing, so it handles a timetable that changes ("fully
dynamic"). On London it is "an order of magnitude faster" than earlier methods, and it extends to
flexible departure times and other criteria such as fare zones. Microsoft's patent on the method has
lapsed ([Patents](#patents)).

**Julian Dibbelt, Thomas Pajor, Ben Strasser, Dorothea Wagner, "Connection Scan Algorithm",
arXiv:1703.05997 (2017).** [paper] (abstract read)
<https://arxiv.org/abs/1703.05997>

Earliest-arrival and profile queries, with a Pareto variant for transfers, which "can adjust to
timetable changes and known vehicle delays". It also defines a Minimum Expected Arrival Time problem for
uncertain delays. No patent was found.

**Moritz Baum, Valentin Buchhold, Jonas Sauer, Dorothea Wagner, Tobias Zündorf, "UnLimited TRAnsfers for
Multi-Modal Route Planning" (ULTRA), ESA 2019, arXiv:1906.04832; `kit-algo/ULTRA` (MIT).** [paper] [repo]
(abstract read; the repository's metadata through `gh api`)
<https://arxiv.org/abs/1906.04832> · <https://github.com/kit-algo/ULTRA>

Transit plus an unrestricted transfer mode (walking, bike, e-scooter, car) is slow for existing
algorithms. ULTRA precomputes a small set of shortcut edges "provably sufficient for computing a Pareto
set of optimal journeys", which RAPTOR or CSA then use unchanged. It is about an order of magnitude faster
than the previous state of the art.

**Rust crates: `vulture` (RAPTOR, version 0.24.0; docs.rs lists Apache-2.0, GitHub's licence detection
reports none); `gtfs-structures` (0.50.1, MIT, rust-transit); `gtfs-rt` (0.5.0, MIT and Apache-2.0).**
[repo] (docs.rs pages read; `gh api` for the repositories)
<https://docs.rs/vulture> · <https://docs.rs/gtfs-structures> · <https://docs.rs/gtfs-rt>

`vulture` implements RAPTOR over a `Timetable` trait, with a GTFS adapter and multi-criteria labels
(arrival against walking time or fare). `gtfs-structures` reads GTFS feeds into serde structures.
`gtfs-rt` provides the Realtime message types.

**OpenTripPlanner documentation.** [docs] (read)
<https://docs.opentripplanner.org/en/latest/>

An open journey planner (LGPL) built "primarily" on GTFS and OpenStreetMap, applying real-time updates
and alerts. The page read does not name its router.

**Google's "transfer patterns"** (Bast et al., ESA 2010; Google's patents, lapsed, [Patents](#patents)):
precomputed transfer patterns for very large networks. A city-scale network does not need them.

*Bearing,* the router Forge needs is small:
- **The network's size:** a city of a million has a few hundred stops and a few thousand runs a game day.
  RAPTOR over it is well under a millisecond a query *(estimate)*.
- **Forge's own implementation:** a few hundred lines over Forge's own timetable, integer game ticks,
  deterministic tie-breaking. `vulture` is a reference to read, and its licence is unclear on GitHub.
- **Uses:**
  1. the stop-to-stop tables of §1.6, built once per timetable change;
  2. the pad's queries with live delays;
  3. re-planning after a disruption.
- **Later:** walking and bike legs need ULTRA's shortcuts, or simply the walking graph's
  footpaths between nearby stops. A taxi leg is a transfer edge whose cost is the dispatcher's quote.
- **The test:** a GTFS export of the network checked against `gtfs-structures` and, offline,
  OpenTripPlanner.

### 7.2 Live feed from the simulation

The server's transit layer writes a feed shaped like GTFS Realtime (§1.2), but as D-010 events, not HTTP:
- **Trip updates:** per run, a delay (seconds) at its next stops, sent when it changes by more than a
  threshold (15 real seconds).
- **Vehicle state:** per run, its occupancy class (GTFS-RT's enumeration, from §1.4's densities), sent
  at each stop.
- **Alerts:** a disruption with the lines and stops it touches (a blocked track, a closed landing),
  with text ids for localisation.
- **Fleet:** per request, the assigned vehicle and its ETA, to the requester only.

Clients hold the static timetable (it is data from the city's seed, D-053) and apply the deltas. Every
display and pad therefore shows numbers that agree with what the trams will do, because both come from
the same function.

### 7.3 Hailing and ETAs

The pad's taxi app is a client of §2's dispatcher:
1. **The request:** origin (the nearest stand or, for flagged trips, the door), destination, party size,
   needs (a ramp), willingness to share.
2. **The quote:** an ETA and a fare if any, for shared and private, from the dispatcher's current state.
3. **Booking:** accepting books it. The ETA updates as the vehicle approaches, and a notice comes when it
   arrives.
4. **Expiry:** a booking expires if the person does not come (the taxi waits up to a limit, a smart-object
   reservation like people's).

The quote is honest: it is the dispatcher's own estimate, and the error between quoted and actual wait
is measured. Hailing in the street (raising a hand at a stand) is the same request without the pad.

### 7.4 Stop displays

A stop display is a world-space screen whose UI renders to a texture (the pad's technique, `people.md`
§5.6). It shows:
- **Departures:** the next two or three per line, in game minutes, with their occupancy.
- **Messages:** the current alert.
- **The city's live data**, for the brief's integrated technology.

It reads the client's copy of the timetable and the feed, so it costs the server nothing. Its countdown
ticks every 15 real seconds at k = 4 (§1.5).

---

## 8. Later vehicles: ships, helicopters, planes and drones

**SKYbrary, "Regulation 2021/664 — U-space Regulatory Framework".** [web] (read)
<https://skybrary.aero/articles/regulation-2021664-u-space-regulatory-framework>

- **The airspace:** U-space airspace is designated by EU member states after a risk assessment; the
  regulation applies from 26 January 2023, with information-security rules added from 22 February 2026.
- **Four mandatory services:** network identification (continuous remote ID), geo-awareness (constraints
  and conditions), flight authorisation for each flight, and traffic information about nearby manned
  aircraft. Weather and conformance monitoring are optional.

**Heise on Volocopter (7 May 2025).** [web] (read)
<https://heise.de/-10374839>

Volocopter's plan to fly passengers at the Paris 2024 Olympics failed for lack of approval. Its design
and production were approved, but type certification for passenger flights was still pending, now aimed
at 2026, after its insolvency and takeover by Wanfeng.

*Bearing,* short, for later:
- **Ships** (issue #228) are boats with more mass, slower turning and a pilot or AI. In the far tier they
  are scheduled runs like boats; near players they are dynamic hulls. River ships need locks or weirs
  as interlockings, and harbour pilots as a docking controller.
- **Helicopters and air taxis** fly scheduled corridors between pads: a 3D lane graph in the far tier, a
  physical craft near players (§5.4). Their airspace is a set of volumes with permissions per vehicle
  class.
- **Drones** (deliveries, inspection) are U-space's four services as game rules:
  - every drone has an id;
  - geo-zones it may not enter (parks, the nightlife quarter at night);
  - a flight plan approved before take-off;
  - traffic information for the player's craft.
  In the far tier a drone is a flight plan with times; near players, a kinematic body on that plan.
- **Simulation tiers** are the same as for ground vehicles: a record with a path and times, a kinematic
  body near players, a dynamic body only when piloted or upset.

---

## 9. Taxi depots: how car parks keep their flows apart (2026-10-15)

The owner asked whether the first depot's cabs could drive in at all, and how car parks
organise the flow of the cars going in and those going out. Automated cabs relax the human
constraints, but people still need access for maintenance and repairs. A short search; the
standards are paywalled, so only the published summaries were read.

**What the sources say:**
- **Flow.** Car parks are laid out so that a car going in passes as many bays as possible, and
  one going out as few as possible. The steel-construction guide gives a one-way aisle of 6.0 m
  and a two-way one of 6.95 m for bays at 90°.
  ([steelconstruction.info, car parks](https://steelconstruction.info/sectors/retail-buildings/car-parks/))
- **Standards.** AS/NZS 2890.1 covers parking modules, circulation roadways and ramps, and the
  widths, gradients and queuing areas of driveways.
  ([Standards Australia](https://store.standards.org.au/product/as-nzs-2890-1-2004))
- **Ramps.** A ramp too steep or too narrow for the standard works as a single lane. Without
  firm controls, cars meet head-on on it.
  ([Dunedin City Council](https://www.dunedin.govt.nz/__data/assets/pdf_file/0007/603934/Andy-Carrs-powerpoint-presentation.pdf))
- **Headroom.** One planning condition asks for 2.2 m, measured as AS 2890.1 sets.
  ([NSW planning portal](https://apps.planningportal.nsw.gov.au/prweb/PRRestService/DocMgmt/v1/PublicDocuments/DATA-WORKATTACH-FILE%20PEC-DPE-EP-WORK%20PPS-2017SWC113!20190621T113955.203%20GMT))
- **Older guidance.** It treats entry, control and exit as separate design points, and raises the
  conflicts between pedestrians and vehicles. ([TRID](https://trid.trb.org/View/33276))
- **Automated valet parking** (ISO 23374-1:2023) sets requirements for the facility's
  environment, for remote assistance and recovery, and for stopping the vehicles in response to
  the other users of the facility.
  ([ISO 23374-1 listing](https://www.sis.se/produkter/informationsteknik-kontorsutrustning/ittillampningar/ittillampningar-inom-transport/iso-23374-12023))
- **Bays.** A trade article notes that bays can narrow when no doors have to open.
  ([ADAS & Autonomous Vehicle International](https://adas.mydigitalpublication.com/adas-autonomous-vehicle-international-september-2022/page-26))

**What Forge took** (`forge_geom::depot`):
- **Split levels.** The back half's floors stand half a storey above the front's, a common
  efficient layout (the Car Park Designers' Handbook compares it with sloping decks).
- **Ramps.** Short ramps of 1.5 m over 10 m (1 in 6.7, inside the usual maxima) run in the side
  strips.
- **One way round.** The front aisle runs east and the back aisle west. Each strip has an up lane
  and a down lane, so no ramp is shared both ways.
- **Ways in and out apart.** The way in at the front's middle passes the floors' bays going up.
  The way out leaves at the corner at the foot of the last down lane.
- **Clearances.** Each level has 2.7 m clear, and the way in's fascia 2.3 m.
- **Clear ways.** Columns stand only between bays and at the strips' edges.
- **Bays.** They stay 2.5 m wide: 1.8 m cabs are narrow, and staff walk between them.
- **People.** A stair and lift core opens onto the pavement for the staff. The fleet's stop
  command on a floor where they work (ISO 23374's) is noted, not simulated.
- **Departures.** Cabs leave 4 s apart and drive through the deck in 45 s.

*Not read:* the standards' full texts and the Handbook (paywalled). Its numbers are taken from
the summaries above, and are *(estimates)* for a spec.

## 10. Tram depots, crossovers and termini (2026-10-15)

The owner asked for one or more tram depots (maintenance, stabling), and for sidings or loops
where a broken-down tram can wait without stopping its line, as real networks have. A research
agent searched the web. Tags: [O] read on the page, [S] seen in a search result only, [D] the
agent's arithmetic.

**Depots.**
- **Nice, Las Planas** [O]: 2.8 ha for 28 five-section trams, by line 1's terminus. It has pit
  tracks under roof gantries, a wash, a paint booth and a short test track
  ([Railway Gazette](https://railwaygazette.com/32375.article)).
- **Luxembourg, Kirchberg** [O]: 32 trams on about 3.3 ha. It has 8 stabling tracks, 3
  service tracks (wash, sanding, a pit wheel lathe), 4 pit tracks, a test track and about 4 km
  of track
  ([gouvernement.lu](https://gouvernement.lu/fr/actualites/toutes_actualites/articles/2015/09-septembre/21-tramsschapp-kirchberg.html)).
- **Lyon, Saint-Fons (T10)** [O]: 2.6 ha for 30 trams of 43 m. Its four zones: stabling,
  routine maintenance, service and wash, arrival and departure
  ([Lyon Capitale](https://www.lyoncapitale.fr/?p=507366)).
- **Lyon, Meyzieu** [O]: two new stabling tracks of 200 m hold four 43 m trams each. The
  connecting curve has a 25 m radius
  ([ETF](https://www.etf.fr/en/agences-grands-projets-transports-urbains-en/le-depot-de-tramway-de-meyzieu-vient-de-sagrandir-pour-accueillir-plus-de-rames)).
- **Vitoria-Gasteiz, Betoño** [O]: 226 × 108 m for 22 trams of 45 m. It has 11 stabling tracks
  of two places each and an outdoor wash; the main workshop is elsewhere
  ([Euskotren](https://euskotren.eus/en/node/14273)).
- **Per network** [O]: Bordeaux has 3 depots for 130 trams on about 82 km; Strasbourg 3 (shared
  with buses) for 114 trams on 6 lines; Lyon 3 and a storage site. Montpellier placed its third
  in the east to balance the two western ones, on land no housing could use.
- **Where:** near line ends, on cheap or industrial land, joined by a double-track service line
  (single for small depots) ([Montpellier's consultation](https://participer.montpellier.fr/sites/default/files/2022-06/CEMG%20-%20D%C3%A9lib%C3%A9ration%203M%20-%20Bilan%20de%20la%20concertation.pdf)).

**Rules of thumb:**
- about 0.9–1.1 ha for 10 trams [D];
- 100–150 m of depot track per tram [D];
- a stabling place the tram's length and 5–7 m [D];
- curves of 18–25 m in a depot and 25 m or more on the line (Alstom Citadis; Polish rules:
  50 m on the line, 25 m at junctions and termini, 20 m in depots
  ([TechTrans](https://reference-global.com/download/article/10.37705/TechTrans/e2021015.pdf)));
- an R50 tram turnout of 16.9 m overall ([Extrica](https://www.extrica.com/article/23203)).
- Track spacing inside a depot: no source found; 4.5–5 m with walkways *(estimate)*.

**Crossovers and termini.**
- **Spacing:** the agent counted crossover tracks in OpenStreetMap [D, a minimum]: one every
  1.2–2.0 km, a pair or a scissors every 2.5–4 km. Bordeaux has 52 on about 75 km, Strasbourg
  43 on 52 km, Montpellier 32 on 64 km. No published spacing rule was found.
- **Short turns:** Bordeaux had 11 partial termini on its 3 lines in 2015, and added switches
  in 2024 so trams can turn back during incidents [O]
  ([Bordeaux Métropole](https://www.bordeaux-metropole.fr/sites/MET-BXMETRO-DRUPAL/files/export/dp_terminus_partiels_21-01-2015.pdf)).
- **Termini:**
  - Paris T2's Porte de Versailles was rebuilt with side platforms and tail tracks that store 2
    trams [O].
  - van Oort and van Nes (2010) find a double crossover after the platforms the most reliable,
    and a single tail track only fit for low frequencies [O]
    ([World Transit Research](https://www.worldtransitresearch.info/research/3623)).

**Patents.** None found on procedural depot or yard layout (Google Patents and the web only).
The nearest:
- Alstom's US8781803B2, a track plan from the network's topology for signalling, live until
  about 2032;
- KRRI's US12296875B2, a track-layout graph for interlocking.
Neither covers the geometry.

**What Forge took** (2026-10-15, `demos/city-plan`):
- A crossover of 30 m before each end's platform: a tram coming in crosses to the track it
  leaves by and turns where it stands, the two platforms facing each other.
- A crossover pair about every 3 km, on straight track on land, clear of the junctions and the
  stops.
- Depots (`city::tram_depot`): about one for three lines, near a line's end, industrial land
  first, 1.5 km apart.
  - Stabling places of a tram and 5 m, tracks 4.6 m apart.
  - Curves of 20 m in the yard and 25 m on the link.
  - A 45° ladder for the deeper tracks, and a 50 m hall over the three deepest.
  - A single-track link, as small depots have, rising up the verge so the footway stays level.
- *Next:*
  - trams going in and out of the depots at the service's ends;
  - a double-track link for the larger depots;
  - tail tracks past busy termini;
  - trams taking the crossover pairs round a broken-down tram.

## What professional engines do

| Game or system | Transit model | Schedule or headway | Passengers | Capacity and crowding | Riding along | What users noticed |
|---|---|---|---|---|---|---|
| Cities: Skylines (2015) | lines of stops; vehicles per line from length and budget; taxis from depots and stands | holding ("unbunching") at stops, tuned by mods | citizens as agents with routes | per vehicle: tram 90, ferry 80, metro 150 | no | bunching; holds of minutes at shared stops; vehicles leaving empty; despawns in jams |
| Cities: Skylines II (2023) | lines with vehicles per line | — | citizens routing on time, comfort and money (`game-ai-ecosystems.md` §2) | — | no | vehicles stuck boarding a passenger who never comes (2024); loops worse than point-to-point |
| Transport Fever 2 (2019) | lines with frequency from vehicles | frequency | persons with homes and jobs | — | cameras ride vehicles | waiting time not in the choice (players); the first game counted ¼ of the headway |
| Workers & Resources | lines and stops | — | citizens with travel-time limits | — | — | workers take the closest job; long trips cut work; forced disembarking misfires |
| OpenTTD, JGR's patch pack | orders and timetables | timetables with autofill; separation and scheduled dispatch in the patch pack | amounts | full-load orders | no | spacing needs start-date offsets; departure boards wanted |
| MATSim | schedule of lines, routes, departures; transit vehicles in the queue simulation; DRT for fleets | schedule; RAPTOR router (SwissRailRaptor) | agents with day plans | vehicle capacity; stop time 4 s/board, 2 s/alight, 15 s doors (simple handler) | — | research standard |
| SUMO | stops on lanes, stop durations and `until`, flows with periods; taxi device | both | persons with intermodal trips | person capacity, dwell extension | GUI | research standard |
| OpenTripPlanner | GTFS and GTFS-RT | both | — | — | — | the open journey planner |
| Nivalis Nights (2026) | automated taxis and trains between districts (`population-simulation.md` §1.2) | — | NPCs with schedules | — | — | — |
| Star Citizen | ships with crews | — | — | — | physics grids: walking inside moving ships | players left behind as the ship turned, before grids |
| Unreal Engine 5.8 | modular vehicles | — | — | — | — | client-predicted, server-authoritative vehicle physics with rewind |

---

## Recommendation for Forge

**Verdict.** Transit is cheap to simulate and expensive to size. The engine work is:
- a timetable layer with exact, deterministic vehicle motion;
- a fleet dispatcher with fixed stands;
- mode choice in the population's planner;
- vehicle-frame riding for the player.

The capacity and fleet sizes depend on the clock (§1.5) and must be checked by the engine against the
population's own plans before anything is built. None of it needs a GPU, a learned model or a live
patent if the methods below are kept.

**The crates and systems** (names to be argued in the code):
- **`forge-transit` (new, CPU, deterministic):**
  - **the network:** stops, lines, patterns (stop sequences with run times from the track geometry and
    the speed profile), timetables (exact or headway windows, GTFS's `exact_times`), runs, consists,
    depots and deadheads;
  - **the run motion function:** s(tick) from departure, delay and the profile;
  - **headway control** at control points (Daganzo);
  - **junctions:** priority requests and shared-segment bookings (§2.5);
  - **the router:** RAPTOR and the stop-to-stop tables;
  - **the live feed** (§7.2);
  - **GTFS export** for checking with outside tools.
- **`forge-transit::fleet`:**
  - requests, quotes, batched insertion (SUMO's greedy and route-extension kind, with detour limits);
  - stands as queues with chargers;
  - LP rebalancing between stands per time band (Pavone);
  - charging scheduling;
  - capped pick-up distance and quoted waits;
  - accessible-vehicle matching.
- **`forge-sim::population` (from the population note):**
  - trip legs and the `Waiting`, `OnRun`, `InVehicle` whereabouts;
  - stop queues;
  - boarding by capacity;
  - the nested logit mode choice at planning time;
  - habitual modes;
  - the load-per-section check at world generation.
- **`forge-procgen::city`:** the `CityPlan` already has `tram_lines` (edges, points, stops) and
  `crossings`. It gains stands, depots, docks, the shared-segment rules per road edge and the
  timetable parameters. `forge-transit::Network::from_plan` builds the network from them (D-053:
  generated content as data).
- **`forge-physics`:**
  - a kinematic move per tick for trams' and taxis' bodies (Jolt's `MoveKinematic`, or the existing
    velocity setter on kinematic bodies);
  - a character controller in a vehicle's frame;
  - the mooring joint;
  - thrusters on the buoyancy boat.
  `PathConstraint` only if a physical rail vehicle is ever needed.
- **Client:** the articulated tram renderer from bogie points; stop displays and the pad's transit apps
  as UI rendered to textures; the cabin-frame camera.
- **Netcode (D-010):** run delays and fleet paths as reliable events; piloted vehicles as predicted
  snapshots; riders in vehicle space.

**How the transit layer plugs in:**
- **The `CityPlan`** gives the geometry.
- **`forge-transit`** gives the network, the runs, the router's tables and the fleet.
- **The population** reads the tables to plan, joins queues and boards runs by events, and calls the
  fleet for taxi legs.
- **The near tier** asks `forge-transit` where a run or a vehicle is, and promotes it to bodies.
- **The client** computes the same positions from the same data.

One timetable, one motion function, one source of truth.

**The order to build, each step ending in a measured, visible result:**
1. **One tram line, headless.** The line from a `CityPlan`, its pattern and timetable, runs moving by the
   profile. *Measure:* run positions identical at 1 and 6 workers and on two machines (D-016 digest); µs
   per run update; a stop's departure board for a game hour printed and checked against the timetable.
2. **The first proof of concept: one tram line with live stop displays and a ride.**
   - **What runs:** an articulated three-to-five-section tram, kinematic from bogie points, with
     interior and doors, on one line through the city scene; stop displays showing the next departures
     from the server's runs.
   - **The player:** waits, boards, sits, rides three stops and gets off; a second client rides the same
     tram.
   - *Measure:*
     - the display's countdown against the actual arrival (0 s when undisturbed);
     - the tram's sections against the track (no flip, overhang correct on the tightest curve);
     - tram tick and frame cost in the F1 overlay;
     - the two clients' tram poses (equal to the bit);
     - bytes a second for the line (under 0.1 KB/s);
     - the comfort captures (seated and standing), which the owner judges.
3. **People on board.** The population's queues, boarding by capacity, loads and occupancy on the
   displays, denied boardings, the holding rule. *Measure:* headway spread with and without holding;
   denied boardings at the peak; loads per section against capacity per game hour (§1.5); no boarding
   state longer than the maximum dwell.
4. **The second proof of concept: taxis hailed from the pad.**
   - **What runs:** the dispatcher, stands with charging, kinematic taxis on lanes with the IDM, one
     shared tram segment with booked gaps, accessible shuttles; the pad's hail, quote and ETA.
   - *Measure:*
     - the wait distribution against the fleet size (§2.4's table, from the simulation);
     - the quoted against the actual wait;
     - tram–taxi conflicts on shared track (zero);
     - empty-km share;
     - charging time per vehicle;
     - the wild goose chase threshold found by raising demand.
5. **The third proof of concept: a boat crossing.**
   - **What runs:** two automated electric boats on a frequency across the river, docking under PD
     control at pontoons with bikes aboard. Then the player pilots a boat on the river with its current,
     and two players ride one boat.
   - *Measure:* the docking error and time; the crossing's capacity per game hour; the piloted boat's
     corrections over a lossy link; passenger jitter inside a rolling boat (none).
6. **The pad's router.** RAPTOR with walking, tram, boat and taxi legs; live delays; re-planning after a
   blocked track. *Measure:* µs per query; the pad's ETA against arrival; agreement with a GTFS export run
   through OpenTripPlanner offline.
7. **The whole network at a million.** All lines, the fleet, the boats, the population's mode choice.
   *Measure:* mode shares against the target; loads per section, crossing and cycle track against
   capacity at the peak game hour; the fleet's waits; server milliseconds per real second; bandwidth per
   client.

**For a decision later (D-0xx 🟡, when the transit crate starts):**
1. **Time units:** timetables written in game time with capacity checked in real time (this note's
   view), or the clock's effect handled some other way (shorter trips, a different k for travel).
2. **The vehicle tiers and radii:** runs as functions everywhere; bodies near players.
3. **Taxis on tram track:** per segment by city rule, with booked gaps (§2.5), or not at all.
4. **The dispatcher's method:** insertion and LP rebalancing (open), not the patented pooling and
   dispatch methods ([Patents](#patents)).
5. **Mode choice:** nested logit at planning time with habitual modes; the weights calibrated to a target
   share table.
6. **Clients deriving runs from the timetable** (this note's view) against receiving positions.
7. **Riding in the vehicle's frame** for the controller and the netcode.
8. **The owner's go** before building on any patent in the table below.

---

## What to measure

- **Transit core:** µs per run update and per stop event; the run-position digest at 1 and 6 workers;
  the display's countdown error against arrivals (0 s undisturbed); the headway spread with and without
  holding; the longest dwell (never above the cap).
- **Capacity:** per section, crossing and cycle track, the peak game-hour load against capacity per game
  hour (§1.5), computed from the population's plans before the first run, and measured afterwards;
  denied boardings; standing density at the peak (never above 6 a m²).
- **Fleet:**
  - the wait distribution (p50, p90) at peak and off-peak against the fleet size;
  - the quoted against the actual wait;
  - the busy, idle and charging shares;
  - empty-km;
  - pooled occupancy;
  - accessible trips' waits;
  - the conflicts on shared track (zero);
  - the demand at which pick-up distance runs away.
- **Boats:** docking time and position error; crossing capacity; boarding time with bikes; energy per
  crossing.
- **Mode choice:** shares by mode against the target table; shares by distance band; the habitual
  share; sensitivity to a headway change.
- **Near tier:** promotion and demotion of trams and taxis (exact positions, no pops: ꟻLIP of a capture
  across a promotion); tram section placement on the tightest curve; frame milliseconds with 10 trams and
  50 taxis in view.
- **Player:** jitter of the camera and the body inside a moving tram and a rolling boat (none); time to
  board and sit; the owner's comfort judgement on captures.
- **Network:** bytes a second per client by stream (runs, fleet, piloted vehicles, passengers); two
  clients' tram poses equal to the bit; the piloted boat's corrections over 100 ms and 2 % loss.
- **Router:** µs per query; the agreement of a GTFS export with OpenTripPlanner's answers.
- **Server:** transit and fleet milliseconds per real second at a million people, on the server PC.

---

## Patents

Checked briefly on 2026-10-10 on Google Patents (pages read where marked) and through search records.
Claims were not read in full (Google's pages were truncated before them), so relevance is from titles,
abstracts and descriptions. This is a flag list for the owner, not legal advice. Status and expiry are
Google's estimates, which it calls assumptions; "Expired – Fee Related" means maintenance fees were not
paid.

| Patent | Holder | Jurisdiction | Status | Expiry (estimate) | Touches | Alternative |
|---|---|---|---|---|---|---|
| US 10,438,137 B2, "System for real-time optimal matching of ride sharing requests" (priority 6 Oct 2014, granted 8 Oct 2019) (read) | Massachusetts Institute of Technology (Ratti, Santi) | US | active | 5 Aug 2035 | testing pairs of trips for sharing, building a shareability network, maximising shared trips, dispatching | greedy insertion within detour limits (SUMO's `greedyShared`/`routeExtension`, dial-a-ride insertion heuristics), no shareability graph |
| US 11,614,751 B2, "System for on-demand high-capacity ride-sharing via dynamic trip-vehicle assignment…" (priority 23 Jan 2017, granted 28 Mar 2023) (read); US 11,619,951 B2 "…with future requests" (search record) | MIT (Alonso-Mora, Samaranayake, Wallar, Rus) | US | active | 11 Jul 2038; the second not checked | trip-vehicle assignment for high-capacity pooling | as above; pool at most two parties per vehicle |
| US 10,235,888 B2, "Ride chaining" (priority 22 Jul 2014, granted 19 Mar 2019) (read) | Lyft, Inc. | US | active | 27 May 2035 | adding a new rider's pick-up and drop-off to a driver's existing route when shareable | broad; check the claims before pooling is built. Prior art in dial-a-ride insertion (decades older) and SUMO's open algorithms; or no pooling at first |
| US 10,775,806 B2, "Autonomous-vehicle dispatch based on fleet-level target objectives" (priority 22 Dec 2017, granted 15 Sep 2020) (read) | Lyft, Inc. | US | active | 18 Aug 2038 | dispatching AVs while pursuing fleet goals (maintenance, rebalancing) through no-passenger, destination and incentive modes | Pavone et al.'s LP rebalancing (2011, published before the priority date) between fixed stands; SUMO's `taxistand` idle return |
| US 10,731,998 B2, "Network computer system to arrange pooled transport services" (priority 5 Nov 2017, granted 4 Aug 2020) (read) | Uber Technologies, Inc. | US | active | 2 Nov 2038 | grouping requests in a time window, choosing shared pick-up points, walking directions, assigning a driver | fixed stands and stops chosen by the city plan (as bus stops are), no per-request meeting points |
| US 9,939,279 and US 10,928,210 B2, "Method and system for shared transport" (search records) | Uber (per Justia's listing) | US | granted | not checked | rider-pool requests, choosing a driver by predicted completion time | as above |
| US 11,631,333 B2, "Multi-agent reinforcement learning for order-dispatching via order-vehicle distribution matching" (priority 26 Feb 2019, granted 18 Apr 2023) (read) | Beijing Didi Infinity Technology and Development | US | active | 7 Sep 2041 | learned dispatch policies | no learning in dispatch (D-016 and the population note's class table): deterministic insertion and assignment |
| US 10,591,916 B2, "Arranging passenger pickups for autonomous vehicles" (fly-by pick-ups; priority 24 Mar 2016, granted 17 Mar 2020) (read) | Waymo LLC | US | Expired – Fee Related | — | picking up a passenger outside the assigned area while approaching | free |
| US 11,780,462 (driveway pullovers), US 12,425,822 (early boarding), pull-over and suggested-location families (search records) | Waymo LLC | US | granted (per records) | not checked | choosing and assessing pull-over spots, early boarding | fixed stands and bays from the city plan; check the claims if dynamic pull-over spots are ever wanted |
| US 7,840,427 (and four others asserted against Uber in 2025, search record) | Carma Technology (formerly Avego) | US | granted 2010 | not checked (filed 2007) | matching riders or goods to spare vehicle capacity | public fleet without spare private capacity; check expiry |
| US 8,494,771 B2, "Journey planning in public transportation networks" (RAPTOR-like rounds; priority 7 Sep 2011, granted 23 Jul 2013) (read) | Microsoft (Delling, Goldberg, Pajor, Werneck) | US | Expired – Fee Related | — (would have run to 28 Nov 2031) | round-based Pareto journeys by arrival and transfers | free to use |
| US 8,738,286 B2, "Transit routing system for public transportation trip planning" (transfer patterns; priority 11 Nov 2009) (read); US 8,700,328 B1, "Better diversity for transit routing" (read) | Google LLC | US | both Expired – Fee Related | — | precomputed transfer patterns | free; not needed at city scale |
| Connection Scan Algorithm, ULTRA | — | — | no patent found | — | — | MIT-licensed ULTRA code exists |
| US 10,217,356 B2, "Timing submission of transit signal priority requests to reduce transit vehicle stop times" (priority 22 Sep 2016, granted 26 Feb 2019) (read) | Global Traffic Technologies, now Miovision Technologies US (per Google) | US | active | 28 Mar 2037 | estimating a transit vehicle's arrival at a junction and timing its priority request so the controller shortens red or extends green | a request at a fixed detection point (Manchester's loops, §1.1) with the controller's own green extension and early green; or junction slots reserved from the timetable. Check the claims before building predictive priority |
| US 6,243,026 B1, GPS-based signal pre-emption (priority 5 May 1995) (read) | 3M, now Global Traffic Technologies | US | Expired – Lifetime | 2015 | pre-emption from tracked vehicle positions | free |
| US 11,084,512 and US 11,958,516, "Autonomous rail or off rail vehicle movement and system among a group of vehicles" (search records) | not checked | US | granted (per records) | not checked | autonomous vehicles on dedicated tracks physically barred to others | not Forge's case (shared paved tram lanes, no barrier); no patent found on AVs sharing tram or bus lanes |
| GB 2354417 (B), LOD by the number of moving objects (priority 13 Apr 1999) (read) | Namco | GB | Expired – Lifetime | 12 Apr 2020 | draw detail by object count | free |
| US 11,071,916 B2 and US 11,684,855 B2 (Take-Two, navigation on a coarse road graph), Improbable's handover family | — | US | active | 2039; 2036–2037 | in `population-simulation.md`'s table | as there |

Not found or not checked: a patent on promoting game vehicles from a coarse simulation to physics near a
player (none found; the technique is Brom's and Chenney's simulation LOD); tram signal-priority patents
by Siemens, Alstom or Thales (not searched); patents on wire-free trams are in `city-generation.md`'s
table; EP, CN and GB equivalents of the US patents above (not searched); Uber's and Waymo's families
beyond the records listed; the claims of every active patent above.

---

## Checked and left out

- **A tram's energy per km:** no readable source gave a per-vehicle-km figure; studies found were about
  regenerative braking shares (search records), so the note gives none.
- **Cities: Skylines II's public-transport developer diary:** the Paradox forum returned an error; the
  CS2 wiki page was not found. Players' threads stand in.
- **Sea of Thieves on networking players aboard ships:** no talk found. Rocket League's GDC 2018 talk:
  the page read had no detail.
- **Unreal's "based movement" for characters on moving bases:** only the API listing as search records
  (`bFastAttachedMove`).
- **The TCQSM itself, Fruin's original tables, the Nantes dwell paper's full text, Wardman's multipliers:**
  PDFs or refused pages; secondary search records only, marked where used.
- **ITF's Lisbon and curb reports, the PNAS papers:** refused (403); press and university pages used
  instead.
- **Official fleet sizes:** Waymo publishes none; Zoox's and Apollo Go's were not found.
- **French and Belgian traffic law texts:** only search records (the Légifrance URL tried returned 404).
- **Nantes's tram signal priority:** not found; French guidance (Cerema) only as search records.
- **Cycle-track capacity from CROW's manual:** not found; research figures only as search records.
- **Helicopter flight models for games, drone simulation libraries:** not researched; §5.4 and §8 are
  pointers for later.
- **The "20-minute rule" of Train Fever** and the route-cost formula of Transport Fever: players' claims,
  unconfirmed by the developer.
- **Bunching in Cities: Skylines' first game:** the workshop discussions refused (429); kept as search
  records.

---

## Verification notes

Checked on 2026-10-10 with WebFetch, WebSearch and `gh api` only; no browser pane, nothing downloaded,
the repository not modified. "Read" means the page or file was returned and the facts are from it;
"search record" means only a search engine's extract was seen.

- **Read:**
  - **Standards and simulators:** GTFS Schedule and Realtime references (gtfs.org); SUMO's "Public
    Transport" and "Taxi" pages; MATSim's 13.0 release news and the Doxygen source of
    `SimpleTransitStopHandler`.
  - **Transit operations:** Daganzo's abstract (Berkeley ITS); Wikipedia's "Transit signal priority",
    "Crush load", "Bombardier CR4000", "Siemens-Duewag Supertram", "Bogie", "Low-floor tram", "Discrete
    choice", "Modal share", "Intelligent driver model" and "Vélib' Métropole"; German Wikipedia's
    "Fahren auf Sicht" and "Busfahrstreifen"; the LRTA's Manchester page; Urban Transport Magazine on
    Munich.
  - **Boats:** French Wikipedia's "Navibus" and "Voguéo"; Wikipedia's "Batobus", "Vaporetto" and
    "Copenhagen Harbour Buses"; NH Nieuws (6 September 2024); Baird Maritime on *NZK Pont 103* and
    *MilliAmpere 2*; New Atlas on *Estelle*; Ships Monthly on Candela's *Nova*.
  - **Games:** the Paradox wiki's Cities: Skylines page; the Improved Public Transport 2 workshop page;
    Steam threads on Cities: Skylines (one), Cities: Skylines II (two), Transport Fever (one), Transport
    Fever 2 (one), Workers & Resources (one); the OpenTTD wiki's timetable page; JGRennison's README
    through `gh api`.
  - **Fleets:** MIT News (January 2017); PubMed Central's copy of Santi et al.; the RSS 2011 abstract
    (Pavone et al.); Microsoft Research's record of Castillo and Knoepfle; CoMSES's abstract and IEEE
    Spectrum on Fagnant and Kockelman; Claims Journal (12 February 2026), TechCrunch (24 February 2026)
    and the Internet Info Agency (9 July 2026) on Waymo; CnEVPost (28 July 2026) on Apollo Go; Insurance
    Journal (25 March 2026) and Automotive World (6 August 2026) on Zoox; Singularity Hub (2 February
    2026) on waits; Electrive (25 June 2020) on Oslo.
  - **Mode choice and crowds:** PTV Visum's "Skims of time"; White Rose's record of Wardman (2004); the
    EU Urban Mobility Observatory and eBikes International on Ghent; G. Keith Still's thesis chapter 3;
    the arXiv record of Helbing and Molnár.
  - **Routing:** the SoCS extended abstract of RAPTOR; arXiv 1703.05997 (CSA) and 1906.04832 (ULTRA);
    docs.rs for `vulture`, `gtfs-structures` and `gtfs-rt`; the OpenTripPlanner documentation; `gh api`
    metadata for `kit-algo/ULTRA`, `urschrei/vulture`, `rust-transit/gtfs-structure`.
  - **Vehicles and the player:** arXiv 2102.04034 (Siemens's tram); the Star Citizen Wiki's August 2014
    monthly report; Epic's Chaos Modular Vehicles page (5.8); FAAC's rest-frame article; SKYbrary on
    U-space; Heise on Volocopter.
  - **Patents (Google Patents):** US 10,438,137; US 11,614,751; US 10,235,888; US 10,775,806;
    US 10,731,998; US 11,631,333; US 10,591,916; US 8,494,771; US 8,738,286; US 8,700,328;
    US 10,217,356; US 6,243,026; GB 2354417.
- **Search record only:** MATSim's DVRP/DRT papers and hEART 2025 short paper; SwissRailRaptor's source
  and the SBB showcase; MATSim's `VehicleUtils`; the TCQSM's dwell parameters; the Nantes dwell study;
  French tram-priority guidance (Cerema, Aximum); Manchester's doubles; Fruin's levels of service; the
  Mauritius and Sydney crowding figures; Wardman's later multipliers and the New Zealand calibration;
  Nantes's EDGT 2015 and Strasbourg's 2019 shares; ITDP's bikeshare benchmarks; Nantes's bike share
  (2025); cycle-track capacities; the ITF Lisbon and curb reports; France's R412-7 and R110-2; Belgium's
  *site spécial franchissable*; HOLON's approval (Benteler); the I-Pace's specifications; Prothero et al.
  (1999); the Unreal character movement API; the Cities: Skylines workshop discussions on holding; the
  Waymo pick-up patent family beyond US 10,591,916; Uber's US 9,939,279 family; Carma's patents;
  US 11,084,512 and 11,958,516.
- **Refused or empty:** pnas.org (403), csail.mit.edu (403), itf-oecd.org (403), oecd.org (403),
  fleetnews.co.uk (403), lasvegassun.com (402), link.springer.com (login redirect),
  eprints.soton.ac.uk (403), paperity.org (403), css.umich.edu (403), the Paradox forum's CS2 diary
  (500), cs2.paradoxwikis.com (404), Wikipedia's "Line-of-sight operation" and "Fruin level of service"
  (404), "Copenhagen_harbour_buses" in lower case (404), "Ferries in Amsterdam" (404), the Légifrance URL
  tried (404), uber.com's research blog (404), a Steam workshop discussion (429), GitHub's Jolt
  architecture page (503), Stadler's point-control page and PC Gamer's FOV article (no text), the RSI
  Comm-Link (introduction only), the Game Developer page on Rocket League (no detail), MIT's Urban
  Operations Research book (a different section): each is marked where its fact is used.
- **From Forge, read in the repository:** `CLAUDE.md`; `docs/research/galaxy-sky.md` (format),
  `city-generation.md` §12–§13 and its extension, `population-simulation.md`, `game-ai-ecosystems.md`
  §2 and §5, `physics-fluids.md` §3, `netcode.md`'s recommendation, `smart-systems.md` §3,
  `procedural-content.md`'s vehicle generator; the scratchpad's `people.md`; `docs/DECISIONS.md` (D-010,
  D-016, D-038, D-041); `docs/demos/physics-lab.md`; `crates/forge-physics/src/lib.rs`;
  `crates/forge-procgen/src/city/transit.rs` and `mod.rs` (`CityPlan`, `TramLine`, `BoatCrossing`);
  Jolt's `PathConstraint.h`, `PathConstraintPathHermite.h`, `CharacterVirtual.h` in `third_party/jolt`;
  issues #177 and #228 through `gh`.
- **Numbers to re-check before a spec:** every *(estimate)*. That includes §1.5's load table (its
  peak-hour share and section factor are assumptions), §2.4's fleet table (trip length, speed, occupancy,
  the imbalance factor of two), §3.3's crossing times, §4.2's door-to-door table, §2.8's energy, and the
  router's sub-millisecond query time.
