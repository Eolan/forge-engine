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
| Places | from a city plan's masses: homes and jobs by floor area (two thirds homes at 40 m² a person, jobs at 30 m²), a school in every 120 buildings, active ground floors as shops, cafés and bars in turn, the parks (`places_from_buildings`) |
| People | everyone the homes hold; roles by a mix (45 % day workers, 3 % night workers, 9 % students, 15 % children, 28 % retired or at home); five personality traits of four bits; work at a distance drawn from an exponential of mean 2.5 km in a random direction, school the nearest (`generate`) |
| Records | a structure of arrays, 32 bytes a person: home, anchor, role, personality, seed, where (a place or a trip and its destination), since when, day, step |
| Days | never stored: each person's plan for a day is made again from their seed, role, personality and the day's number whenever an event needs it; weekdays and weekends differ; the conscientious rise earlier, the extraverted go out more and later |
| Events | a calendar ordered by (game second, person): an activity ends, a trip arrives; nobody is updated in between |
| Trips | timed by distance and mode: on foot under 1.2 km, by bike under 4 km for the open, by tram beyond (with a 5-minute wait); free flow, no congestion yet |

## Numbers (the example city, 4 km, release, one thread)

| | |
|---|---|
| Places | 36 256 |
| People | 331 429, 32 bytes each (10.6 MB) |
| Generation | 1.5 s |
| Events | 6.4 a person a day; 1.3 µs each |
| Two game days | 5.5 s |
| A game hour of 15 real minutes asks | 99 events a real second: 0.013 % of one core |

Where everyone is, by game hour (weekday, %):

| Hour | Home | Work or school | Shops | Cafés | Bars | Parks | Travelling |
|---|---|---|---|---|---|---|---|
| 03 | 100.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.0 |
| 08 | 42.3 | 43.2 | 0.0 | 0.0 | 0.0 | 0.0 | 14.4 |
| 11 | 26.1 | 68.9 | 4.7 | 0.0 | 0.0 | 0.0 | 0.3 |
| 13 | 31.1 | 55.5 | 0.0 | 13.4 | 0.0 | 0.0 | 0.0 |
| 16 | 18.5 | 68.9 | 1.0 | 2.2 | 0.0 | 8.2 | 1.2 |
| 18 | 60.2 | 14.9 | 4.2 | 5.3 | 4.0 | 5.4 | 5.9 |
| 20 | 87.5 | 0.0 | 0.0 | 7.0 | 5.4 | 0.0 | 0.0 |
| 22 | 88.2 | 3.0 | 0.0 | 3.3 | 5.3 | 0.0 | 0.2 |

Night workers (3 %) are at work at night. The same seed gives the same day to the bit (a test
compares the digests).

## Next (#250)

- Places' capacities and opening hours; appointments between friends (the night out).
- Routes on the streets and transit with link queues (#253).
- The spatial index of who is where by cell, and the near tiers round a player.
- Persistence: snapshots of the changed columns and the input log.
- The cost per event: plans are made again on every event, 1.3 µs; caching a person's day would
  cut it, if a fast-forward needs it.
