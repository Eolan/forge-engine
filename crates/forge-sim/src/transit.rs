//! Public transport as timetabled runs (D-059's step 7, #253; GTFS's model of routes, trips,
//! stop times and frequencies). A run's place on its line is a pure function of the time, so
//! the server, every client, every stop display and the smart pad compute the same vehicle
//! from a few numbers, with no state to send.
//!
//! A line is a track of known length with its stops at distances along it. A service runs
//! vehicles out along it and back, each leg from stop to stop on a trapezoidal speed profile
//! (accelerate, cruise, brake), dwelling at each stop and turning at the ends; vehicles leave the
//! first stop one headway apart. Delays and holding to the headway come with the population's
//! boarding (#250).

/// How a line's vehicles run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Service {
    /// Cruising speed, m/s.
    pub cruise: f64,
    /// Acceleration and braking, m/s².
    pub accel: f64,
    /// Time at each stop, seconds.
    pub dwell: f64,
    /// Time at each end before the run back, seconds.
    pub turnaround: f64,
    /// Seconds between vehicles.
    pub headway: f64,
}

impl Default for Service {
    /// A modern tram in a city centre: 50 km/h at most, 1.1 m/s², 25 s at a stop, 3 minutes at
    /// each end, every 6 minutes.
    fn default() -> Self {
        Self {
            cruise: 13.9,
            accel: 1.1,
            dwell: 25.0,
            turnaround: 180.0,
            headway: 360.0,
        }
    }
}

/// Where a vehicle is at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunState {
    /// Its distance along the track from the line's start, metres.
    pub s: f64,
    /// Whether it runs from the start towards the end.
    pub outbound: bool,
    /// Its speed, m/s.
    pub speed: f64,
    /// The stop it stands at, if it stands at one.
    pub at_stop: Option<usize>,
}

/// One step of a vehicle's cycle.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// Standing at a stop.
    Stand { stop: usize, outbound: bool },
    /// Moving from one distance to another.
    Move { from: f64, to: f64, outbound: bool },
}

/// A line's timetable: its vehicles' cycle, out and back.
#[derive(Clone, Debug, PartialEq)]
pub struct Timetable {
    stops: Vec<f64>,
    service: Service,
    /// Each step and the time it starts, from the cycle's start.
    steps: Vec<(f64, f64, Step)>,
    cycle: f64,
}

impl Timetable {
    /// The timetable of a line whose stops stand at `stops` metres along its track (in order,
    /// at least two).
    ///
    /// # Panics
    /// With fewer than two stops, stops out of order, or a service that cannot move.
    pub fn new(stops: &[f64], service: Service) -> Self {
        assert!(stops.len() >= 2, "a line has two stops at least");
        assert!(
            stops.windows(2).all(|w| w[1] > w[0]),
            "stops in order along the track"
        );
        assert!(service.cruise > 0.0 && service.accel > 0.0 && service.headway > 0.0);
        let n = stops.len();
        let mut steps = Vec::new();
        let mut t = 0.0;
        let mut push = |step: Step, length: f64, t: &mut f64| {
            steps.push((*t, length, step));
            *t += length;
        };
        // Out: stand at each stop but the last, move to the next; turn; and back.
        for (outbound, order) in [
            (true, (0..n).collect::<Vec<_>>()),
            (false, (0..n).rev().collect()),
        ] {
            for (k, &i) in order.iter().enumerate() {
                let last = k + 1 == order.len();
                let stand = if k == 0 {
                    service.turnaround
                } else {
                    service.dwell
                };
                if !last || k == 0 {
                    push(Step::Stand { stop: i, outbound }, stand, &mut t);
                }
                if !last {
                    let j = order[k + 1];
                    let (from, to) = (stops[i], stops[j]);
                    let length = leg_time((to - from).abs(), &service);
                    push(Step::Move { from, to, outbound }, length, &mut t);
                }
            }
        }
        Self {
            stops: stops.to_vec(),
            service,
            steps,
            cycle: t,
        }
    }

    /// The same service over other stops (as [`Timetable::new`]).
    pub fn with_stops(&self, stops: &[f64]) -> Self {
        Self::new(stops, self.service)
    }

    /// The stops' distances along the track, metres.
    pub fn stops(&self) -> &[f64] {
        &self.stops
    }

    /// Seconds a vehicle takes out and back, turns included.
    pub fn cycle(&self) -> f64 {
        self.cycle
    }

    /// The vehicles the line needs to keep its headway.
    pub fn vehicles(&self) -> usize {
        (self.cycle / self.service.headway).ceil() as usize
    }

    /// Where vehicle `k` is at time `t` seconds (any `t`: the service runs round the clock).
    pub fn at(&self, k: usize, t: f64) -> RunState {
        let local = (t - k as f64 * self.service.headway).rem_euclid(self.cycle);
        // The step holding `local`: the last one starting at or before it.
        let i = self
            .steps
            .partition_point(|&(start, _, _)| start <= local)
            .saturating_sub(1);
        let (start, length, step) = self.steps[i];
        match step {
            Step::Stand { stop, outbound } => RunState {
                s: self.stops[stop],
                outbound,
                speed: 0.0,
                at_stop: Some(stop),
            },
            Step::Move { from, to, outbound } => {
                let d = (to - from).abs();
                let (gone, speed) = leg_distance(d, local - start, length, &self.service);
                RunState {
                    s: from + (to - from).signum() * gone,
                    outbound,
                    speed,
                    at_stop: None,
                }
            }
        }
    }
}

/// Seconds to cover `d` metres from standing to standing.
fn leg_time(d: f64, s: &Service) -> f64 {
    let ramp = s.cruise * s.cruise / s.accel;
    if d >= ramp {
        d / s.cruise + s.cruise / s.accel
    } else {
        2.0 * (d / s.accel).sqrt()
    }
}

/// Metres covered and the speed `t` seconds into a leg of `d` metres taking `total` seconds.
fn leg_distance(d: f64, t: f64, total: f64, s: &Service) -> (f64, f64) {
    let t = t.clamp(0.0, total);
    // The top speed reached: the cruise, or less on a short leg.
    let top = s.cruise.min((d * s.accel).sqrt());
    let ramp_t = top / s.accel;
    let ramp_d = 0.5 * top * ramp_t;
    if t < ramp_t {
        (0.5 * s.accel * t * t, s.accel * t)
    } else if t > total - ramp_t {
        let left = total - t;
        (d - 0.5 * s.accel * left * left, s.accel * left)
    } else {
        (ramp_d + top * (t - ramp_t), top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Timetable {
        Timetable::new(&[0.0, 400.0, 900.0, 1300.0, 2000.0], Service::default())
    }

    #[test]
    fn a_leg_starts_and_ends_standing() {
        let s = Service::default();
        for d in [50.0, 120.0, 400.0, 1500.0] {
            let total = leg_time(d, &s);
            let (a, va) = leg_distance(d, 0.0, total, &s);
            let (b, vb) = leg_distance(d, total, total, &s);
            assert!(a.abs() < 1e-9 && va.abs() < 1e-9);
            assert!((b - d).abs() < 1e-6, "{d}: {b}");
            assert!(vb.abs() < 1e-6);
            // Never faster than the cruise, never backwards.
            let mut last = 0.0;
            for k in 0..=100 {
                let (x, v) = leg_distance(d, total * f64::from(k) / 100.0, total, &s);
                assert!(x + 1e-9 >= last && v <= s.cruise + 1e-9);
                last = x;
            }
        }
    }

    #[test]
    fn vehicles_run_out_and_back_a_headway_apart() {
        let t = line();
        // Five stops: four legs each way, three dwells each way between, two turns.
        assert!(t.cycle() > 2.0 * 2000.0 / 13.9);
        assert_eq!(t.vehicles(), (t.cycle() / 360.0).ceil() as usize);
        // At the cycle's start vehicle 0 stands at the first stop; vehicle 1 a headway later.
        let a = t.at(0, 0.0);
        assert_eq!(a.at_stop, Some(0));
        assert!(a.outbound);
        assert_eq!(t.at(1, 360.0), a);
        // Positions stay on the track and the run is periodic.
        for k in 0..2000 {
            let time = f64::from(k) * 7.3;
            let r = t.at(2, time);
            assert!((0.0..=2000.0).contains(&r.s), "{r:?}");
            let later = t.at(2, time + t.cycle());
            assert!((r.s - later.s).abs() < 1e-6 && r.outbound == later.outbound);
        }
        // Halfway through the cycle it is on its way back.
        let back = (0..100)
            .map(|k| t.at(0, t.cycle() * (0.55 + 0.004 * f64::from(k))))
            .filter(|r| !r.outbound)
            .count();
        assert_eq!(back, 100);
    }

    #[test]
    fn a_vehicle_stands_at_every_stop_each_way() {
        let t = line();
        let mut seen = std::collections::BTreeSet::new();
        let mut time = 0.0;
        while time < t.cycle() {
            if let Some(stop) = t.at(0, time).at_stop {
                seen.insert((stop, t.at(0, time).outbound));
            }
            time += 1.0;
        }
        // Out it stands at 0..=3 (the turn at 0 included), back at 4..=1.
        for stop in 0..4 {
            assert!(seen.contains(&(stop, true)), "{seen:?}");
        }
        for stop in 1..5 {
            assert!(seen.contains(&(stop, false)), "{seen:?}");
        }
    }
}
