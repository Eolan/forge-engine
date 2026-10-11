# Population — a city's people as records advanced by events (`forge-sim::population`)

D-059's step 8 begins (#250, `docs/research/population-simulation.md`, steps 1 and 2): everyone a
city's homes hold, each with a home, a role, work or a school, a personality and a seed, carried
through game days by a calendar of events, headless, on the CPU (D-016).

```
cargo run --release -p population-bench -- --days 2
```

## How it works

| Part | How |
|---|---|
| Places | from a city plan's masses: homes and jobs by their net floor area (`city::structure`, D-060: two thirds homes at INSEE's 32.4 m² net a person, jobs at 25 m² net; it was 40 and 30 m² gross), a school in every 120 buildings, active ground floors (on the avenues and the lanes, and facing a square, a park or a quay: `Mass::lively`) as shops, cafés and bars in turn, the parks (`places_from_buildings`) |
| People | everyone the homes hold; roles by a mix (45 % day workers, 3 % night workers, 9 % students, 15 % children, 28 % retired or at home); five personality traits of four bits; work at a distance drawn from a log-normal (median 6 km, σ 0.8) in a random direction, drawn again when it falls outside the city, so that an 8 km city by the sea commutes a median 3.3 km straight, 23 % under 2 km, near French dense centres' (`docs/research/trip-distances.md`); school the nearest (`generate`) |
| Records | a structure of arrays, 36 bytes a person: home, anchor, role, personality, seed, where (a place or a trip, its origin and destination), since when, day, step |
| Days | never stored: each person's plan for a day is made again from their seed, role, personality and the day's number whenever an event needs it; weekdays and weekends differ; the conscientious rise earlier, the extraverted go out more and later; an afternoon out goes mostly to the cafés and tea rooms, the shops and the pubs, a park now and then (the parks took everyone out before; the owner, 2026-10-11) |
| Events | a calendar ordered by (game second, person): an activity ends, a trip arrives; nobody is updated in between |
| Trips | timed by distance and mode: on foot under 1.2 km, by bike under 4 km for the open, by tram beyond (with a 5-minute wait); free flow, no congestion yet |

## Numbers (the example city, 4 km, release, one thread)

| | |
|---|---|
| Places | 28 518 |
| People | 348 999, 36 bytes each (12.6 MB): 307 283 before net areas and the storeys by era |
| Generation | 0.3 s |
| Events | 6.7 a person a day; 1.65 µs each (an outing weighs 32 places and more: see "Outings") |
| Two game days | 7.7 s |
| A game hour of 20 real minutes asks | 81 events a real second: 0.013 % of one core |
| Six workers (`population::Shards`) | the two days in 1.51 s, 5.1 times one, the same digest to the bit |
| A snapshot (`Sim::snapshot`, `Sim::restore`) | 8.0 MB, 23 bytes a person, saved in 2 ms, restored in 8 ms, the same digest |

Where everyone is, by game hour (weekday, %):

| Hour | Home | Work or school | Shops | Cafés | Bars | Parks | Travelling |
|---|---|---|---|---|---|---|---|
| 03 | 100.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| 08 | 42.3 | 40.8 | 0.0 | 0.0 | 0.0 | 0.0 | 16.9 |
| 11 | 25.8 | 68.9 | 4.4 | 0.0 | 0.0 | 0.0 | 0.9 |
| 13 | 31.1 | 55.5 | 0.0 | 13.4 | 0.0 | 0.0 | 0.0 |
| 16 | 16.8 | 68.9 | 4.5 | 5.3 | 1.7 | 1.6 | 1.2 |
| 17 | 29.3 | 44.8 | 5.9 | 7.9 | 3.6 | 3.8 | 4.7 |
| 18 | 55.3 | 15.0 | 8.2 | 7.7 | 3.8 | 2.7 | 7.4 |
| 20 | 87.5 | 0.0 | 0.0 | 7.0 | 5.5 | 0.0 | 0.0 |
| 22 | 87.7 | 3.0 | 0.0 | 3.3 | 5.3 | 0.0 | 0.6 |

Night workers (3 %) are at work at night. The same seed gives the same day to the bit (a test
compares the digests).

**Outings** (`World::choose`, the owner's notes of 2026-10-11, `docs/research/trip-distances.md`):
- **Choosing the place.** A shop, a café, a bar or a park is chosen among the 32 nearest and
  the 8 nearest with some appeal. Each is weighed by `exp(−β (km − appeal × bonus))`:
  - β is 1.2 a km for shopping, 1.5 for a café or tea room, 0.7 for an evening out, 1.0 for a
    park. With places spread evenly, the median trip is about 1.7/β.
  - Shops, cafés and bars facing a square, a park or a quay have appeal 1, and parks 0.5. The
    bonus is 1.5 km on weekdays and 3 km at weekends.
  - So people go mostly near home, and now and then further, to the places that draw them.
  - Before, it was one of the four nearest: trips of 100–300 m, and nobody left their block.
- **On the way home.** About one worker in seven stops at the shops near work.
- **Lunch** stays near work, chosen among the four nearest.
- **Together** (the owner: people go out with friends, family, sometimes coworkers). Plans stay
  each person's own pure function, so a group is a shared key, not a message:
  - a household (`population::household`: one of its home's households, 2.2 residents each,
    the French mean, theirs by their seed) goes out together two weekend days in seven, to a
    park, a café or the shops, every member with the same place, leaving time and return;
  - a workplace goes for a drink one workday in ten, and its more outgoing workers (about half)
    leave at the same time for the same bar near work.
  The drawn crowd sets each person up to 0.6 m to one side of the line by their seed, so a group
  on one trip walks side by side. Friends across the city need a social graph: later.

**On worker threads** (`population::Shards`): the people are split into contiguous runs, each
with its own calendar on its own thread. Nobody acts on anyone else yet, so the shards' events
depend on their own people alone. A test checks that the digest, the events, the census and the
positions are equal at one, three and six workers, to the bit. The bench (six workers) and a
1 048 538-person city (the private sandbox's). The sandbox is all adults (`Mix::adults()`): the
default mix without its 15 % of children, the other roles scaled to fill their place.

| | One worker | Six workers |
|---|---|---|
| The example, 348 999 people, two days | 7.7 s | 1.51 s (5.1 times) |
| The private sandbox's city, 1 301 827 people (1 048 538 before net areas and the storeys by era), two days | 36.1 s | 6.7 s (5.3 times) |
| 4 226 650 people (`population-bench --scale 13.5`: the example's floors 13.5 times over, before net areas), two days | 108.6 s | 22.6 s (4.8 times) |

The sandbox's 1.3 M need 46.9 MB, 311 events a real second at one game hour per 20 minutes, and 0.06 %
of one core. That is #250's first test, with the digests equal at one and six workers. At 4 M:
152.2 MB, generated in 4 s (on one thread), 982 events a real second, 0.19 % of one core; the
same digest at six workers.

![The example city's people at 08:00, 13:00 and 21:00](images/population-day.png)

*Everyone as a dot (`Sim::positions`, `city::map::write_points`): blue at home, orange at work or
school (the bright dots are schools), cyan shopping, magenta in cafés and bars, green in parks,
white on the way, along the straight line of their trip for now. The morning's travellers, the
lunch hour's cafés, the evening out.*

## Next (#250)

- Places' capacities and opening hours; appointments between friends (the night out).
- Routes on the streets and transit with link queues (#253).
- The spatial index of who is where by cell, and the near tiers round a player.
- Persistence: the snapshot exists (what changes as people live, over the base generated again
  from the seed; a test restores it mid-day and runs on to the same digest). Next: the input log
  of what players change, compression (LZ4, a dependency to agree first), and snapshots of the
  shards.
- The cost per event: plans are made again on every event, 1.7 µs with the outings' weighed
  choice; caching a person's day would cut it, if a fast-forward needs it.
