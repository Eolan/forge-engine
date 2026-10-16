//! Variation for sounds played again and again: a set of takes in an order that never repeats
//! the last few, each played at its own pitch and level.
//!
//! This is the random container of Wwise and the multi instrument of FMOD: an ear notices the
//! same sample played twice in a row long before it notices which of five it heard. A
//! [`SoundSet`] holds the takes of one sound (five door slams, eight pebbles); [`SoundSet::play`]
//! picks one uniformly among those not played in its last `avoid` picks, and scales the
//! voice's pitch and gain by a draw within [`Variation`]'s ranges. The sounds made in code vary
//! by themselves ([`crate::foley`]: every step is synthesised anew).

use std::collections::VecDeque;
use std::sync::Arc;

use forge_core::SplitMix64;

use crate::{AudioEngine, PlayParams, SampleBuffer, VoiceId, db_to_gain};

/// How far each play strays from the take as recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variation {
    /// The playback rate spreads by this share either way (0.03: ±3 %, about ±half a
    /// semitone).
    pub pitch: f32,
    /// The gain spreads by this many decibels either way.
    pub gain_db: f32,
}

impl Default for Variation {
    /// ±3 % in pitch, ±1.5 dB in level.
    fn default() -> Self {
        Self {
            pitch: 0.03,
            gain_db: 1.5,
        }
    }
}

/// One play's take, pitch and gain.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    /// Which take, by its index in the set.
    pub index: usize,
    /// The take.
    pub take: Arc<SampleBuffer>,
    /// The playback rate factor.
    pub pitch: f32,
    /// The linear gain.
    pub gain: f32,
}

/// A uniform pick in `0..count` that is not in `recent`, then remembered there (which keeps
/// at most `avoid` entries, and fewer than `count`).
pub fn pick_avoiding(
    rng: &mut SplitMix64,
    count: usize,
    recent: &mut VecDeque<usize>,
    avoid: usize,
) -> usize {
    assert!(count > 0, "nothing to pick from");
    let avoid = avoid.min(count - 1);
    while recent.len() > avoid {
        recent.pop_front();
    }
    let free = count - recent.len();
    let mut k = rng.below(free as u64) as usize;
    let mut index = 0;
    loop {
        if !recent.contains(&index) {
            if k == 0 {
                break;
            }
            k -= 1;
        }
        index += 1;
    }
    if avoid > 0 {
        if recent.len() == avoid {
            recent.pop_front();
        }
        recent.push_back(index);
    }
    index
}

/// The takes of one sound and what was played last.
#[derive(Clone, Debug)]
pub struct SoundSet {
    takes: Vec<Arc<SampleBuffer>>,
    variation: Variation,
    avoid: usize,
    recent: VecDeque<usize>,
    rng: SplitMix64,
}

impl SoundSet {
    /// A set of `takes` (at least one), varied by `variation` from `seed`, avoiding the last
    /// half of the takes (at most three) on each pick.
    pub fn new(takes: Vec<Arc<SampleBuffer>>, variation: Variation, seed: u64) -> Self {
        assert!(!takes.is_empty(), "a sound set needs a take");
        let avoid = (takes.len() / 2).min(3);
        Self {
            takes,
            variation,
            avoid,
            recent: VecDeque::with_capacity(avoid + 1),
            rng: SplitMix64::new(seed),
        }
    }

    /// Avoids the last `avoid` picks (at most one less than the takes).
    pub fn avoiding(mut self, avoid: usize) -> Self {
        self.avoid = avoid.min(self.takes.len() - 1);
        self
    }

    /// The takes.
    pub fn takes(&self) -> &[Arc<SampleBuffer>] {
        &self.takes
    }

    /// The next play's take, pitch and gain.
    pub fn next_pick(&mut self) -> Pick {
        let index = pick_avoiding(
            &mut self.rng,
            self.takes.len(),
            &mut self.recent,
            self.avoid,
        );
        let v = self.variation;
        Pick {
            index,
            take: self.takes[index].clone(),
            pitch: 1.0 + self.rng.range_f32(-v.pitch, v.pitch),
            gain: db_to_gain(self.rng.range_f32(-v.gain_db, v.gain_db)),
        }
    }

    /// Plays the next pick with `params`, its pitch and gain multiplied by the pick's.
    pub fn play(&mut self, engine: &mut AudioEngine, mut params: PlayParams) -> VoiceId {
        let pick = self.next_pick();
        params.pitch *= pick.pitch;
        params.gain *= pick.gain;
        engine.play(&pick.take, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::sine;

    fn set(takes: usize, seed: u64) -> SoundSet {
        let takes = (0..takes)
            .map(|i| sine(200.0 * (i + 1) as f64, 0.5, 0.01, 48_000).shared())
            .collect();
        SoundSet::new(takes, Variation::default(), seed)
    }

    #[test]
    fn no_take_comes_back_within_the_last_three() {
        let mut s = set(6, 1);
        let picks: Vec<usize> = (0..6000).map(|_| s.next_pick().index).collect();
        for w in picks.windows(4) {
            assert!(!w[..3].contains(&w[3]), "{w:?}");
        }
        // Every take is played, about as often as the others.
        for take in 0..6 {
            let n = picks.iter().filter(|&&p| p == take).count();
            assert!((800..1200).contains(&n), "take {take}: {n}");
        }
    }

    #[test]
    fn pitch_and_gain_stay_within_their_ranges_and_spread() {
        let mut s = set(4, 2);
        let picks: Vec<Pick> = (0..1000).map(|_| s.next_pick()).collect();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in &picks {
            assert!((0.97..=1.03).contains(&p.pitch));
            let db = crate::gain_to_db(p.gain);
            assert!((-1.5..=1.5).contains(&db));
            lo = lo.min(p.pitch);
            hi = hi.max(p.pitch);
        }
        assert!(hi - lo > 0.05);
        // The same seed, the same picks.
        let mut again = set(4, 2);
        assert!(picks.iter().all(|p| *p == again.next_pick()));
    }

    #[test]
    fn two_takes_alternate_and_one_take_repeats() {
        let mut two = set(2, 3);
        let picks: Vec<usize> = (0..20).map(|_| two.next_pick().index).collect();
        assert!(picks.windows(2).all(|w| w[0] != w[1]), "{picks:?}");
        let mut one = set(1, 4);
        assert!((0..5).all(|_| one.next_pick().index == 0));
    }
}
