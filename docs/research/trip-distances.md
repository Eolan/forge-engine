# Where people work and go out: real distances for the population model

Research note, 2026-10-11, for the owner's remarks on the population (#250):
- the places people go should follow where they live and work;
- people look for jobs not too far from home;
- sometimes they go further, to nice places: good restaurants, views, the waterfront;
- they go out with friends, family, and now and then coworkers.

Figures are sourced unless marked **[est.]**. Sources marked * in the list were opened; the
others come from search results.

## 1. Home to work

- **France, 2019 census** [1]*: the median commute is 7.8 km nationally and 5.4 km in dense
  urban areas. 35 % of workers work in their own commune (43 % in dense areas). Medians differ by
  job: in rural areas 18 km for managers, 11 km for employees.
- **Paris residents, 2017** [2][3]*:
  - median 4.8 km; more than half under 5 km, 79 % under 10 km, 4 % over 25 km;
  - in the centre most residents travel under 2 km;
  - these are road distances between centroids; a straight line is about 0.75 times that
    [est.].
- **Munich residents (MiD 2017)** [4]*: work trips average 12.4 km, median 6.3 km. Germany as a
  whole: 16.0 and 8.1 km.
- **Shape:**
  - A log-normal with a 4.8 km median and σ ≈ 0.9 reproduces the Paris figures: 52 % under
    5 km, 79 % under 10, 3 % over 25, about 17 % under 2 km [est.].
  - An exponential with the same median puts 25 % under 2 km.
  - Long-range tails fall off as a power law (search result only), which is irrelevant inside
    an 8 km city.

## 2. Distances and shares by purpose

| Munich residents, MiD 2017 [4] | Mean km | Median km |
|---|---|---|
| Work | 12.4 | 6.3 |
| Education | 6.3 | 2.0 |
| Shopping | 3.7 | 1.5 |
| Errands | 9.2 | 2.9 |
| Leisure | 17.6 | 3.9 |
| Accompanying | 8.4 | 2.4 |
| Walking trips | 1.4 | 0.8 |

Leisure's mean is about 4.5 times its median, shopping's about 2.5 times: that long tail is
people going to particular places.

**French figures:**
- **Dense communes:** 22 % of trips are under 1 km [5]*.
- **Lyon Métropole, 2015:** 57 % of trips are under 3 km [6]*.
- **Aix-Marseille-Provence (EMC² 2019–20):** 3.4 trips a day [7]*. Purposes: leisure 23 %,
  shopping 21 %, work 20 %, accompanying 15 %, school 13 %, errands about 7 %. French surveys
  count eating out inside leisure.
- **Walking:** Lyon-Villeurbanne residents make 44.7 % of their trips on foot, and in that
  centre walking carries 65 % of shopping trips and 56 % of leisure trips. Marseille 38–39 %
  [8]*.
- **The nearest real analogue** to the sandbox city is Lyon-Villeurbanne: 0.67 M people in about
  63 km².

## 3. Chaining

- **Lyon:** 20 % of trips neither start nor end at home [6].
- **Aix-Marseille:** 24 % are secondary trips [7].
- **France (EMP 2019):** 15 % of weekday home-work journeys include another stop, mostly to drop
  children off or to shop [9] (search result only).
- **England (NTS)** [10]*: 88 % of morning trips to work are direct. About 17 % of trip chains
  start at work, most often work, then shops, then home.
- **Lunch:** in Marseille, going home for lunch fell 30 % between 2009 and 2019 [7]. Lunch near
  work is right.

## 4. Time of day

- **Marseille:** peaks at 7–9 h and 16–19 h. At 16–19 h the trips are for leisure 24 %, shopping
  20 %, accompanying 19 % and work 15 % [7].
- **Île-de-France at night:** 5–9 % of daily trips; about three quarters of weeknight night trips
  fall between 21 h and midnight, mostly for leisure [11] (search result only).
- **Weekends** (Nord Franche-Comté 2024) [12]*:
  - Saturday has 15 % fewer trips than a weekday, Sunday 35 % fewer than Saturday.
  - Saturdays are mostly shopping, Sundays leisure, visits and shopping.
  - At home all day: 15 % on a weekday, 23 % on Saturday, 38 % on Sunday.
- **People out at 18 h and at 21 h:** no source found. About 35–45 % out at 18:00 and 10–15 %
  at 21:00 on weekdays, later in Mediterranean cities [est.].

## 5. Destination attractiveness

- **The usual model:** a logit, or Huff's gravity model: U = −β·cost + γ·ln(1 + A) + bonuses.
  mobiTopp, estimated on Stuttgart's survey [13]*, has:
  - a size coefficient γ of 0.27–0.47;
  - time coefficients per minute: daily shopping −0.111, other shopping −0.060, indoor leisure
    −0.036, outdoor leisure −0.068, visits −0.055, strolling −0.176;
  - a cost that includes the detour to the next fixed place.
- **Restaurants:** an area's chance of being chosen rises with its count of restaurants [14]
  (Tokyo, search result only).
- **Blue space:** across 18 countries, weekly visits fall off exponentially with distance: people
  travel up to about 50 km for the coast, 5 km for lakes, 2.5 km for rivers [15] (search result
  only).
- **Weekend leisure:** in Aix-Marseille it is 57 % of the distance travelled at weekends, against
  20 % on weekdays [16] (search summary).

## 6. For the model

1. **Workplaces.**
   - Fill jobs where the jobs are, distance mattering gently: P ∝ jobs × exp(−β d).
   - A straight-line median of 3–4 km in an 8 km city, about 20 % under 2 km, 60–65 % under 5 km.
   - In an 8 km square a random pairing already gives a median of about 4 km, so the job map
     shapes most of it.
   - A simpler step is to draw the distance from a log-normal (median 3.5 km, σ ≈ 0.8) in place
     of the exponential of mean 2.5 km (median 1.7 km, 55 % under 2 km).
2. **Outings.**
   - Weigh about 20–50 candidates by exp(−β d + γ ln(1 + A) + bonus), in place of "one of the
     four nearest" (trips of 100–300 m, nobody leaving their block).
   - With evenly spread places the median is about 1.68 / β. Starting values: daily shopping
     β ≈ 2.5 /km (median 0.6 km), a weekday café 1.5 (1 km), other shopping 0.8 (2 km), an
     evening out 0.7 (2–3 km), weekend leisure 0.4 (4 km).
   - A bonus b is worth b / β km. For the waterfront, the old town or a lively square, take
     1–2 km on weekdays and 3–5 km at weekends.
3. **Chaining:** about 15 % of workers stop to shop or run an errand on the way home, chosen by
   the detour d(work, j) + d(j, home) − d(work, home); lunch near work.
4. **Schedules:**
   - shopping at 10–13 h and 16–19 h;
   - leisure at 17–20 h;
   - dinners at 20:00–21:30;
   - 5–10 % of adults out after 21:00 on weeknights, more on Fridays and Saturdays;
   - Saturdays for shopping, Sundays for leisure, many at home.

No patents met.

## Sources

1. \* https://www.insee.fr/fr/statistiques/7622203
2. \* https://www.insee.fr/fr/statistiques/5012380
3. \* https://www.insee.fr/fr/statistiques/5425974
4. \* https://muenchenunterwegs.de/content/657/download/infas-grossraummuenchen-regionalbericht-mid5431-20201204.pdf
5. \* https://www.statistiques.developpement-durable.gouv.fr/les-pratiques-de-mobilite-des-francais-varient-selon-la-densite-des-territoires
6. \* https://www.urbalyon.org/sites/default/files/productions/publications/imports/2019-11/m-emd-agglo.pdf
7. \* https://www.agam.org/wp-content/uploads/2022/04/Chiffres-cles-EMD.pdf
8. \* https://www.statistiques.developpement-durable.gouv.fr/comment-les-francais-se-deplacent-ils-en-2019-resultats-de-lenquete-mobilite-des-personnes
9. https://www.statistiques.developpement-durable.gouv.fr/mobilite-des-personnes
10. \* https://www.gov.uk/government/statistics/nts-factsheets/trip-chaining-2024
11. https://omnil.fr/medias/omnil/75047185-e91e-4acf-9fec-ea0018ee1cf0_fiche_egt_nuit_mel.pdf
12. \* https://www.cerema.fr/sites/default/files/inline-files/emc2-nfc2024_lesindicateurscleswe.pdf
13. \* https://arxiv.org/pdf/1707.05050
14. https://arxiv.org/pdf/2302.11400
15. https://bluehealth2020.eu/publications/residential-distance/
16. https://agam.org/focus-mobilite-loisirs-metropole-actu
