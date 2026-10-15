//! The residents' lights (D-060 step 3): who is at home and awake at the hour, household by
//! household, so each flat's windows light by what its people are doing, not by a share of the
//! hour (the owner: "lit windows from the people living there").

use forge_procgen::city::CityPlan;
use forge_procgen::city::geom;
use forge_procgen::city::mass::Mass;
use forge_procgen::city::structure::Structure;
use forge_procgen::city::units::Use;
use forge_sim::population::{self, Mix};
use glam::DVec2;

use crate::crowd::{People, people};

/// The day the windows are lit for: a Tuesday, its Monday evening before counted (the late
/// nights out, the night shifts).
const DAY: u16 = 1;

/// The share of a household's windows lit while it is up after dark: the rooms in use, the
/// others dark.
const AFTER_DARK: f64 = 0.55;
/// The same by day: a lamp in a dark room, a kitchen at the back.
const BY_DAY: f64 = 0.15;
/// The share of the windows of a household out or asleep that are lit: a lamp left on, a
/// hall light, a sleepless night.
const LEFT_ON: f64 = 0.02;

/// Who is at home and awake at an hour, by mass and household.
pub struct Residents {
    /// Per mass, per household, whether one of its people is at home and awake (empty where
    /// nobody lives or the mass was not asked for).
    awake: Vec<Vec<bool>>,
    /// The share of an awake household's windows lit at the hour.
    lamps: f64,
    /// How long it took, seconds.
    pub seconds: f64,
    /// The households asked for, and how many were at home and awake.
    pub households: (usize, usize),
}

impl Residents {
    /// The people of `plan`'s `masses` (`nets`: each mass's net floor area) in `mix`'s roles, at
    /// home and awake or not at `hour` (0 to 24): those of the masses `near`.
    pub fn new(
        plan: &CityPlan,
        masses: &[Mass],
        nets: &[f64],
        mix: &Mix,
        hour: f64,
        near: &[usize],
    ) -> Self {
        let start = std::time::Instant::now();
        let People {
            world,
            population: pop,
            homes,
        } = people(plan, masses, nets, mix, Some(near));
        // The people of each home: one run in the population, home by home.
        let mut first = vec![(usize::MAX, 0); world.places.len()];
        for p in 0..pop.len() {
            let h = &mut first[pop.home[p] as usize];
            if h.0 == usize::MAX {
                h.0 = p;
            }
            h.1 += 1;
        }
        let t = (hour.rem_euclid(24.0) * 3600.0) as u32;
        let mut awake = vec![Vec::new(); masses.len()];
        let mut households = (0, 0);
        for &m in near {
            let Some(h) = homes[m] else {
                continue;
            };
            let mut up = vec![false; population::households(&world, h) as usize];
            let (from, count) = first[h as usize];
            for p in (from..from + count).filter(|_| from != usize::MAX) {
                let k = population::household_index(&world, &pop, p) as usize;
                if !up[k] && population::home_awake(&world, &pop, p, DAY, t) {
                    up[k] = true;
                }
            }
            households.0 += up.len();
            households.1 += up.iter().filter(|&&a| a).count();
            awake[m] = up;
        }
        Self {
            awake,
            lamps: lamps(hour),
            seconds: start.elapsed().as_secs_f64(),
            households,
        }
    }

    /// Whether a window of mass `mass` is lit: by its flat's household (`flat`: the mass's
    /// homes in order, a household to each in turn), or in the share of the mass's households
    /// awake where the window is in no flat; `spot` is the window's own draw. `None` where
    /// nobody lives in the mass.
    pub fn lit(&self, mass: usize, flat: Option<usize>, spot: u64) -> Option<bool> {
        let awake = &self.awake[mass];
        if awake.is_empty() {
            return None;
        }
        let draw = |shift: u32| ((spot >> shift) % 1000) as f64 / 1000.0;
        let up = match flat {
            Some(k) => awake[k % awake.len()],
            None => draw(8) < awake.iter().filter(|&&a| a).count() as f64 / awake.len() as f64,
        };
        Some(if up {
            draw(20) < self.lamps
        } else {
            draw(32) < LEFT_ON
        })
    }
}

/// The share of an awake household's windows lit at `hour` (0 to 24), the sun up from 6 to 18
/// ([`crate::hour_of_day`]): [`AFTER_DARK`] from dusk to dawn, [`BY_DAY`] in full day, between
/// them over the two hours after sunrise and before sunset.
pub fn lamps(hour: f64) -> f64 {
    let day = ((hour - 6.0) / 2.0)
        .clamp(0.0, 1.0)
        .min(((18.0 - hour) / 2.0).clamp(0.0, 1.0));
    AFTER_DARK + (BY_DAY - AFTER_DARK) * day
}

/// The home of `s` (its flats in order, [`Use::Home`]) on `storey` holding `p` (on the plan): a
/// window's flat, from a point just inside it.
pub fn flat_at(s: &Structure, storey: usize, p: DVec2) -> Option<usize> {
    s.units
        .iter()
        .filter(|u| matches!(u.use_, Use::Home(_)))
        .position(|u| u.storey == storey && u.pieces.iter().any(|q| geom::contains(q, p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_procgen::city::{example, plan, structure};

    #[test]
    fn the_residents_light_their_flats_in_the_evening_and_sleep_in_the_small_hours() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = forge_procgen::city::mass::masses(
            &city,
            &forge_procgen::city::bible::StyleBible::default(),
            7,
        );
        let structures = structure::structures(&city, &masses);
        let nets: Vec<f64> = structures.iter().map(|s| s.net).collect();
        let near: Vec<usize> = (0..masses.len()).step_by(7).collect();
        // The share of households at home and awake, and of a flat's windows lit.
        let at = |hour: f64| {
            let r = Residents::new(&city, &masses, &nets, &Mix::adults(), hour, &near);
            let (all, up) = r.households;
            assert!(all > 500, "{all} households");
            let lit = near
                .iter()
                .flat_map(|&m| (0..40_u64).map(move |k| (m, k)))
                .filter_map(|(m, k)| {
                    r.lit(m, Some(k as usize), k.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                })
                .collect::<Vec<bool>>();
            let windows = lit.iter().filter(|&&l| l).count() as f64 / lit.len() as f64;
            (up as f64 / all as f64, windows)
        };
        let (evening, evening_lit) = at(21.0);
        let (noon, _) = at(12.0);
        let (night, night_lit) = at(3.0);
        println!("awake at home: 21:00 {evening:.2}, 12:00 {noon:.2}, 3:00 {night:.2}");
        assert!(evening > 0.7, "21:00 {evening}");
        assert!(noon < evening, "12:00 {noon}");
        assert!(night < 0.1, "3:00 {night}");
        // About half the windows lit at 21:00, one in twenty or fewer at 3:00.
        assert!((0.35..0.6).contains(&evening_lit), "21:00 {evening_lit}");
        assert!(night_lit < 0.06, "3:00 {night_lit}");
        // Each home is found from a point inside it, as a window finds its flat.
        let (mut homes, mut found) = (0, 0);
        for s in &structures {
            let ordinals = s.units.iter().filter(|u| matches!(u.use_, Use::Home(_)));
            for (k, u) in ordinals.enumerate() {
                let Some(piece) = u.pieces.iter().max_by(|a, b| {
                    geom::signed_area(a)
                        .abs()
                        .total_cmp(&geom::signed_area(b).abs())
                }) else {
                    continue;
                };
                let p = geom::centroid(piece);
                if !geom::contains(piece, p) {
                    continue;
                }
                homes += 1;
                found += usize::from(flat_at(s, u.storey, p) == Some(k));
            }
        }
        assert!(
            homes > 1000 && found == homes,
            "{found} of {homes} homes found"
        );
    }
}
