//! Arrival schedules for open-loop phases.
//!
//! [`Arrivals`] lazily yields the offset (from phase start) of each new
//! virtual user. Open-model semantics: a user is created on schedule
//! regardless of whether earlier users finished, so slow responses never
//! hide the load the generator was meant to apply (no coordinated
//! omission at the scheduler).

use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::profile::{Phase, PhaseKind};

/// Inter-arrival randomness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Jitter {
    /// Deterministic even spacing (Artillery's behaviour).
    #[default]
    None,
    /// Exponential inter-arrival times with the phase's instantaneous rate
    /// as the mean rate (a Poisson process).
    Poisson,
}

/// Lazily computed arrival offsets for one phase.
pub struct Arrivals {
    rate: f64,
    ramp_to: f64,
    duration: Duration,
    total: u64,
    next_k: u64,
    jitter: Jitter,
    rng: StdRng,
    last: f64,
}

impl Arrivals {
    /// Arrivals for `phase`; `Pause` and `ConstantVus` phases yield nothing.
    #[must_use]
    pub fn new(phase: &Phase, jitter: Jitter, seed: u64) -> Self {
        let d = phase.duration.as_secs_f64();
        let (rate, ramp_to, total) = match &phase.kind {
            PhaseKind::ArrivalRate { rate, ramp_to } => {
                let to = ramp_to.unwrap_or(*rate);
                // Integral of the linear rate over the phase.
                let total = ((*rate + to) / 2.0 * d + 1e-9).floor();
                (*rate, to, total.max(0.0) as u64)
            }
            PhaseKind::ArrivalCount { count } => {
                let r = *count as f64 / d;
                (r, r, *count)
            }
            PhaseKind::Pause | PhaseKind::ConstantVus { .. } => (0.0, 0.0, 0),
        };
        Self {
            rate,
            ramp_to,
            duration: phase.duration,
            total,
            next_k: 0,
            jitter,
            rng: StdRng::seed_from_u64(seed),
            last: 0.0,
        }
    }

    /// Arrivals the deterministic schedule will produce (an estimate under
    /// [`Jitter::Poisson`]).
    #[must_use]
    pub fn expected_total(&self) -> u64 {
        self.total
    }

    /// Instantaneous rate at `t` seconds into the phase.
    fn rate_at(&self, t: f64) -> f64 {
        let d = self.duration.as_secs_f64();
        if d <= 0.0 {
            return self.rate;
        }
        self.rate + (self.ramp_to - self.rate) * (t / d).clamp(0.0, 1.0)
    }

    /// Time at which the cumulative arrival count reaches `k` under the
    /// linear ramp: solves `r0·t + (r1−r0)·t²/(2d) = k`.
    fn time_of(&self, k: f64) -> f64 {
        let d = self.duration.as_secs_f64();
        let r0 = self.rate;
        let a = (self.ramp_to - r0) / (2.0 * d);
        if a.abs() < 1e-12 {
            if r0 <= 0.0 { d } else { k / r0 }
        } else {
            let disc = r0 * r0 + 4.0 * a * k;
            if disc < 0.0 {
                return d;
            }
            (-r0 + disc.sqrt()) / (2.0 * a)
        }
    }
}

impl Iterator for Arrivals {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        let d = self.duration.as_secs_f64();
        match self.jitter {
            Jitter::None => {
                if self.next_k >= self.total {
                    return None;
                }
                let t = self.time_of(self.next_k as f64).clamp(0.0, d);
                self.next_k += 1;
                Some(Duration::from_secs_f64(t))
            }
            Jitter::Poisson => {
                let r = self.rate_at(self.last);
                if r <= 0.0 {
                    return None;
                }
                // First arrival at t=0 like the even schedule; afterwards
                // exponential gaps with mean 1/r(t).
                let t = if self.next_k == 0 {
                    0.0
                } else {
                    let u: f64 = self.rng.r#gen::<f64>().max(f64::MIN_POSITIVE);
                    self.last + (-u.ln()) / r
                };
                if t > d {
                    return None;
                }
                self.last = t;
                self.next_k += 1;
                Some(Duration::from_secs_f64(t))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(kind: PhaseKind, secs: f64) -> Phase {
        Phase {
            name: None,
            kind,
            duration: Duration::from_secs_f64(secs),
            max_vusers: None,
            range: 0..0,
        }
    }

    #[test]
    fn constant_rate_is_evenly_spaced() {
        let p = phase(
            PhaseKind::ArrivalRate {
                rate: 10.0,
                ramp_to: None,
            },
            5.0,
        );
        let offsets: Vec<Duration> = Arrivals::new(&p, Jitter::None, 0).collect();
        assert_eq!(offsets.len(), 50);
        assert_eq!(offsets[0], Duration::ZERO);
        assert!((offsets[1].as_secs_f64() - 0.1).abs() < 1e-9);
        assert!((offsets[49].as_secs_f64() - 4.9).abs() < 1e-9);
    }

    #[test]
    fn ramp_is_linear_and_totals_the_integral() {
        // 1 → 50 over 100s: (1+50)/2 * 100 = 2550 arrivals.
        let p = phase(
            PhaseKind::ArrivalRate {
                rate: 1.0,
                ramp_to: Some(50.0),
            },
            100.0,
        );
        let offsets: Vec<f64> = Arrivals::new(&p, Jitter::None, 0)
            .map(|d| d.as_secs_f64())
            .collect();
        assert_eq!(offsets.len(), 2550);
        assert!(offsets.windows(2).all(|w| w[1] >= w[0]), "monotonic");
        // Arrivals per fifth grow with time.
        let per_fifth: Vec<usize> = (0..5)
            .map(|i| {
                offsets
                    .iter()
                    .filter(|t| **t >= i as f64 * 20.0 && **t < (i + 1) as f64 * 20.0)
                    .count()
            })
            .collect();
        assert!(per_fifth.windows(2).all(|w| w[1] > w[0]), "{per_fifth:?}");
        // First fifth: rate 1→10.8, mean ≈ 5.9/s × 20s ≈ 118.
        assert!((100..=140).contains(&per_fifth[0]), "{per_fifth:?}");
        // Last fifth: rate 40.2→50, mean ≈ 45.1/s × 20s ≈ 902.
        assert!((880..=925).contains(&per_fifth[4]), "{per_fifth:?}");
    }

    #[test]
    fn ramp_down_works_too() {
        let p = phase(
            PhaseKind::ArrivalRate {
                rate: 20.0,
                ramp_to: Some(0.0),
            },
            10.0,
        );
        let offsets: Vec<f64> = Arrivals::new(&p, Jitter::None, 0)
            .map(|d| d.as_secs_f64())
            .collect();
        assert_eq!(offsets.len(), 100);
        assert!(offsets.windows(2).all(|w| w[1] >= w[0]));
        assert!(offsets.iter().all(|t| *t <= 10.0));
    }

    #[test]
    fn arrival_count_spacing_and_pause() {
        let p = phase(PhaseKind::ArrivalCount { count: 20 }, 60.0);
        let offsets: Vec<f64> = Arrivals::new(&p, Jitter::None, 0)
            .map(|d| d.as_secs_f64())
            .collect();
        assert_eq!(offsets.len(), 20);
        assert!((offsets[1] - 3.0).abs() < 1e-9);
        assert!((offsets[19] - 57.0).abs() < 1e-9);

        let pause = phase(PhaseKind::Pause, 30.0);
        assert_eq!(Arrivals::new(&pause, Jitter::None, 0).count(), 0);
        let vus = phase(
            PhaseKind::ConstantVus {
                vus: 4,
                iterations: None,
            },
            30.0,
        );
        assert_eq!(Arrivals::new(&vus, Jitter::None, 0).count(), 0);
    }

    #[test]
    fn poisson_is_seeded_and_roughly_right() {
        let p = phase(
            PhaseKind::ArrivalRate {
                rate: 100.0,
                ramp_to: None,
            },
            60.0,
        );
        let a: Vec<Duration> = Arrivals::new(&p, Jitter::Poisson, 7).collect();
        let b: Vec<Duration> = Arrivals::new(&p, Jitter::Poisson, 7).collect();
        assert_eq!(a, b, "same seed, same schedule");
        // 6000 expected; a Poisson count's sd is ~77, so ±5% is generous.
        assert!((5700..=6300).contains(&a.len()), "{}", a.len());
        assert!(a.iter().all(|t| *t <= Duration::from_secs(60)));
    }
}
