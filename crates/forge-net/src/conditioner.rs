//! The link conditioner: what a bad network does to packets, decided from a seed (D-010).
//!
//! [`LinkConditioner`] holds no socket and reads no clock: it is told when each packet is sent
//! and how long it is, and answers when the packet (and its duplicate, if any) arrives, or that
//! it is lost. The same seed and the same packets give the same answers, so a test that drives
//! a link through it sees the same losses each run. [`crate::socket::ConditionedSocket`] puts
//! it under quinn's UDP socket.
//!
//! What it models, in this order for each packet:
//! - **loss**: the packet is dropped with probability [`ConditionerConfig::loss`];
//! - **bandwidth**: with a rate, the packet waits for the link to be free and takes its length
//!   over the rate to send; a packet that would wait longer than [`ConditionerConfig::queue`]
//!   is dropped (a full router queue);
//! - **latency and jitter**: it then takes the latency plus a uniform share of the jitter;
//!   jitter alone keeps the packets in their order, as one path through a queue does;
//! - **reordering**: with probability [`ConditionerConfig::reorder`] a packet is held back a
//!   further [`ConditionerConfig::reorder_hold`], and the packets after it may overtake it;
//! - **duplication**: with probability [`ConditionerConfig::duplicate`] a second copy arrives
//!   up to one jitter after the first.

use std::time::Duration;

use forge_core::SplitMix64;

/// What the link does to packets. All probabilities are in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConditionerConfig {
    /// The one-way delay every packet takes.
    pub latency: Duration,
    /// Added to the latency, uniform in `0..jitter`.
    pub jitter: Duration,
    /// The share of packets lost.
    pub loss: f64,
    /// The share of packets that arrive twice.
    pub duplicate: f64,
    /// The share of packets held back so that later ones overtake them.
    pub reorder: f64,
    /// How long a reordered packet is held back.
    pub reorder_hold: Duration,
    /// The link's rate in bytes a second; `None` for no limit.
    pub bandwidth: Option<u64>,
    /// The longest a packet waits for the rate before it is dropped.
    pub queue: Duration,
    /// The generator's seed: the same seed and packets, the same fates.
    pub seed: u64,
}

impl ConditionerConfig {
    /// A link that delivers every packet at once.
    pub const fn perfect(seed: u64) -> Self {
        Self {
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            loss: 0.0,
            duplicate: 0.0,
            reorder: 0.0,
            reorder_hold: Duration::ZERO,
            bandwidth: None,
            queue: Duration::from_millis(250),
            seed,
        }
    }

    /// A link of `latency_ms` one way and `loss` of the packets lost, with a tenth of the
    /// latency as jitter.
    pub fn new(latency_ms: u64, loss: f64, seed: u64) -> Self {
        Self {
            latency: Duration::from_millis(latency_ms),
            jitter: Duration::from_micros(latency_ms * 100),
            loss,
            ..Self::perfect(seed)
        }
    }

    /// The same link with this jitter.
    pub fn with_jitter(self, jitter: Duration) -> Self {
        Self { jitter, ..self }
    }

    /// The same link with this share of packets duplicated.
    pub fn with_duplicate(self, duplicate: f64) -> Self {
        Self { duplicate, ..self }
    }

    /// The same link with this share of packets held back by `hold`.
    pub fn with_reorder(self, reorder: f64, hold: Duration) -> Self {
        Self {
            reorder,
            reorder_hold: hold,
            ..self
        }
    }

    /// The same link limited to `bytes_per_second`, dropping packets that would wait longer
    /// than `queue` for it.
    pub fn with_bandwidth(self, bytes_per_second: u64, queue: Duration) -> Self {
        Self {
            bandwidth: Some(bytes_per_second),
            queue,
            ..self
        }
    }
}

/// What a conditioner did so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConditionerStats {
    /// Packets submitted.
    pub sent: u64,
    /// Bytes submitted.
    pub bytes: u64,
    /// Packets dropped by the loss.
    pub lost: u64,
    /// Packets dropped because the rate's queue was full.
    pub overflowed: u64,
    /// Packets held back to be overtaken.
    pub reordered: u64,
    /// Packets sent twice.
    pub duplicated: u64,
}

/// When a packet arrives: its first copy and its duplicate, as times since the conditioner's
/// start; `[None, None]` when it is lost.
pub type Fate = [Option<Duration>; 2];

/// One direction of a conditioned link, without I/O (see the module's documentation).
#[derive(Clone, Debug)]
pub struct LinkConditioner {
    config: ConditionerConfig,
    random: SplitMix64,
    /// When the rate-limited link is free again.
    link_free: Duration,
    /// The latest arrival of a packet that was not held back: jitter does not reorder.
    last_arrival: Duration,
    stats: ConditionerStats,
}

impl LinkConditioner {
    /// A conditioner that has seen no packet.
    pub fn new(config: ConditionerConfig) -> Self {
        Self {
            config,
            random: SplitMix64::new(config.seed),
            link_free: Duration::ZERO,
            last_arrival: Duration::ZERO,
            stats: ConditionerStats::default(),
        }
    }

    /// The link's settings.
    pub fn config(&self) -> ConditionerConfig {
        self.config
    }

    /// Changes the link's settings from the next packet on. The random stream goes on where it
    /// was (the seed is read only by [`LinkConditioner::new`]), so a run that changes the link
    /// at the same packets is still the same run.
    pub fn set_config(&mut self, config: ConditionerConfig) {
        self.config = config;
    }

    /// What it did so far.
    pub fn stats(&self) -> ConditionerStats {
        self.stats
    }

    /// A packet of `bytes` sent at `now` (since the conditioner's start): when it arrives.
    pub fn submit(&mut self, now: Duration, bytes: usize) -> Fate {
        let c = self.config;
        self.stats.sent += 1;
        self.stats.bytes += bytes as u64;
        // Five draws a packet whatever becomes of it, so one packet's fate never shifts the
        // next one's.
        let lose = self.random.next_f64();
        let jitter = self.random.next_f64();
        let reorder = self.random.next_f64();
        let duplicate = self.random.next_f64();
        let second_jitter = self.random.next_f64();
        if lose < c.loss {
            self.stats.lost += 1;
            return [None, None];
        }
        let departs = match c.bandwidth {
            Some(rate) if rate > 0 => {
                let start = self.link_free.max(now);
                let takes = Duration::from_secs_f64(bytes as f64 / rate as f64);
                if start + takes - now > c.queue {
                    self.stats.overflowed += 1;
                    return [None, None];
                }
                self.link_free = start + takes;
                self.link_free
            }
            _ => now,
        };
        let mut arrives = departs + c.latency + c.jitter.mul_f64(jitter);
        if reorder < c.reorder {
            self.stats.reordered += 1;
            arrives += c.reorder_hold;
        } else {
            arrives = arrives.max(self.last_arrival);
            self.last_arrival = arrives;
        }
        let second = (duplicate < c.duplicate).then(|| {
            self.stats.duplicated += 1;
            arrives + c.jitter.mul_f64(second_jitter)
        });
        [Some(arrives), second]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_perfect_link_delivers_everything_at_once() {
        let mut link = LinkConditioner::new(ConditionerConfig::perfect(1));
        for k in 0..100 {
            assert_eq!(link.submit(ms(k), 100), [Some(ms(k)), None]);
        }
        assert_eq!(link.stats().lost, 0);
    }

    #[test]
    fn the_same_seed_gives_the_same_fates() {
        let config = ConditionerConfig::new(50, 0.1, 7)
            .with_duplicate(0.05)
            .with_reorder(0.1, ms(30));
        let run = |config| {
            let mut link = LinkConditioner::new(config);
            (0..1000)
                .map(|k| link.submit(ms(k), 1200))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(config), run(config));
        assert_ne!(run(config), run(ConditionerConfig { seed: 8, ..config }));
    }

    #[test]
    fn loss_and_duplication_match_their_rates() {
        let config = ConditionerConfig::new(20, 0.1, 3).with_duplicate(0.05);
        let mut link = LinkConditioner::new(config);
        let n = 100_000;
        for k in 0..n {
            link.submit(Duration::from_micros(k * 100), 500);
        }
        let s = link.stats();
        let lost = s.lost as f64 / n as f64;
        let duplicated = s.duplicated as f64 / (n - s.lost) as f64;
        assert!((lost - 0.1).abs() < 0.005, "lost {lost}");
        assert!((duplicated - 0.05).abs() < 0.005, "duplicated {duplicated}");
    }

    #[test]
    fn jitter_keeps_the_order_and_reordering_breaks_it() {
        let config = ConditionerConfig::new(40, 0.0, 5).with_jitter(ms(30));
        let mut link = LinkConditioner::new(config);
        let mut last = Duration::ZERO;
        for k in 0..1000 {
            let [Some(at), None] = link.submit(ms(k), 100) else {
                panic!("a lossless link lost a packet");
            };
            assert!(at >= last && at >= ms(k + 40) && at < ms(k + 70));
            last = at;
        }
        let mut link = LinkConditioner::new(config.with_reorder(0.2, ms(25)));
        let arrivals: Vec<Duration> = (0..1000)
            .map(|k| link.submit(ms(k), 100)[0].expect("a lossless link lost a packet"))
            .collect();
        let overtaken = arrivals.windows(2).filter(|w| w[1] < w[0]).count();
        assert!(overtaken > 100, "{overtaken} packets overtaken");
        let held = link.stats().reordered;
        assert!((150..250).contains(&held), "{held} packets held back");
    }

    #[test]
    fn the_rate_spaces_packets_and_drops_what_the_queue_cannot_hold() {
        // 100 kB/s: a 1000-byte packet takes 10 ms; a 50 ms queue holds five.
        let config = ConditionerConfig::perfect(9).with_bandwidth(100_000, ms(50));
        let mut link = LinkConditioner::new(config);
        let fates: Vec<Fate> = (0..10).map(|_| link.submit(Duration::ZERO, 1000)).collect();
        for (k, fate) in fates.iter().take(5).enumerate() {
            assert_eq!(fate[0], Some(ms(10 * (k as u64 + 1))));
        }
        assert!(fates[5..].iter().all(|f| f[0].is_none()));
        assert_eq!(link.stats().overflowed, 5);
        // Once the queue has drained, packets pass again.
        assert_eq!(link.submit(ms(100), 1000)[0], Some(ms(110)));
    }
}
