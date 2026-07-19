//! The one progress primitive: a monotonic meter over a phase's work units.
//!
//! Long-running builders accept a [`Progress`] sink and, once they know their
//! total, open a [`Meter`] and tick it as units complete — one identical
//! parameter and a one-line tick per site, whatever the unit (GPU chunks,
//! voxels, leaves, export tiles). All *policy* lives here, once: emission
//! throttling (~`MAX_EMISSIONS` evenly spaced updates plus both endpoints),
//! monotonicity, saturation at the total, and the disconnected no-op.
//!
//! The sink is a dumb `(done, total)` callback: callees never know what the
//! work is *for*. The orchestrator (e.g. the web front end's mesh build)
//! curries its own phase label into the sink, so phases stay labels on one
//! stream rather than four bespoke channels.

/// Cap on emissions per meter (plus the endpoints): enough for a smooth bar,
/// cheap enough that a per-voxel `add(1)` is an integer compare when not due.
const MAX_EMISSIONS: u64 = 64;

/// An injectable progress sink — [`none`](Progress::none) for callers that
/// don't observe progress (ticks become no-ops).
pub struct Progress<'a> {
    sink: Option<&'a mut dyn FnMut(u64, u64)>,
}

impl<'a> Progress<'a> {
    /// A connected sink: `report(done, total)` fires on the meter's schedule.
    pub fn new(report: &'a mut dyn FnMut(u64, u64)) -> Self {
        Self { sink: Some(report) }
    }

    /// The disconnected sink: every meter opened from it is a no-op.
    #[must_use]
    pub fn none() -> Progress<'static> {
        Progress { sink: None }
    }

    /// Opens a meter over `total` units, emitting the `(0, total)` start
    /// immediately (a `total` of `0` marks an indeterminate phase: the start
    /// is emitted and every tick is a no-op).
    pub fn meter<'p>(&'p mut self, total: u64) -> Meter<'p, 'a> {
        if let Some(sink) = self.sink.as_deref_mut() {
            sink(0, total);
        }
        let stride = (total / MAX_EMISSIONS).max(1);
        Meter {
            progress: self,
            total,
            done: 0,
            stride,
            next: stride,
        }
    }
}

/// A monotonic counter toward a known total; see [`Progress::meter`].
// Borrows the whole `Progress` (not a reborrow of its `dyn` sink): the trait
// object's lifetime is invariant behind `&mut`, so a shortened reborrow would
// not coerce — the extra indirection sidesteps that entirely.
pub struct Meter<'p, 'a> {
    progress: &'p mut Progress<'a>,
    total: u64,
    done: u64,
    stride: u64,
    next: u64,
}

impl Meter<'_, '_> {
    /// Records `n` completed units, emitting when a stride boundary or the
    /// total is crossed. Saturates at the total (over-ticking is a caller bug
    /// but must not report `done > total`).
    pub fn add(&mut self, n: u64) {
        let Some(sink) = self.progress.sink.as_deref_mut() else {
            return;
        };
        if self.done == self.total {
            return; // already finished (and 0-total meters are indeterminate)
        }
        self.done = self.done.saturating_add(n).min(self.total);
        // Emit on stride boundaries — and always at the total, so the bar
        // never finishes short of 100% however the strides landed.
        if self.done >= self.next || self.done == self.total {
            sink(self.done, self.total);
            self.next = self.done.saturating_add(self.stride);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(total: u64, ticks: u64, per: u64) -> Vec<(u64, u64)> {
        let mut got = Vec::new();
        let mut sink = |done, tot| got.push((done, tot));
        let mut progress = Progress::new(&mut sink);
        let mut meter = progress.meter(total);
        for _ in 0..ticks {
            meter.add(per);
        }
        got
    }

    #[test]
    fn emits_endpoints_and_stays_monotone_and_bounded() {
        let got = run(100_000, 100_000, 1);
        assert_eq!(*got.first().expect("start emission"), (0, 100_000));
        assert_eq!(*got.last().expect("final emission"), (100_000, 100_000));
        assert!(
            got.windows(2).all(|w| w[0].0 < w[1].0),
            "emissions must be strictly increasing"
        );
        assert!(
            got.len() as u64 <= MAX_EMISSIONS + 2,
            "throttle failed: {} emissions",
            got.len()
        );
    }

    #[test]
    fn small_totals_emit_every_unit() {
        let got = run(3, 3, 1);
        assert_eq!(got, vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
    }

    #[test]
    fn over_ticking_saturates_at_the_total() {
        let got = run(10, 5, 4); // 20 units ticked against a total of 10
        assert_eq!(*got.last().expect("final"), (10, 10));
        assert!(got.iter().all(|&(done, _)| done <= 10));
    }

    #[test]
    fn zero_total_is_indeterminate_start_only() {
        let got = run(0, 5, 1);
        assert_eq!(got, vec![(0, 0)], "start marker only; ticks are no-ops");
    }

    #[test]
    fn disconnected_progress_is_a_no_op() {
        let mut progress = Progress::none();
        let mut meter = progress.meter(1_000);
        meter.add(1_000); // must not panic, must not observe anything
    }
}
