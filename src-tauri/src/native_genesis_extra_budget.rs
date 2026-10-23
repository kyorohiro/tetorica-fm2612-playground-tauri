//! Aggregate reservations for independently created Native sound chips.
//!
//! Admission and final release belong to the control thread. A reservation must
//! stay with the retained instance/prepared allocation until the last callback
//! reference has been retired and its control-side owner is dropped. Replacing
//! a prepared payload reserves the new payload before releasing the old one.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub(crate) const DEFAULT_LIMIT: usize = 64 * 1024 * 1024;
// Bounded 65,536-entry retired-Arc queue plus conservative core/rack headroom.
pub(crate) const INSTANCE_BYTES: usize = 1024 * 1024 + 64 * 1024;
pub(crate) const YM2608_MEMORY_BYTES: usize = 0x200000 + 0x2000;
pub(crate) const PAYLOAD_OVERHEAD: usize = 256;

#[derive(Debug)]
pub(crate) struct Budget {
    limit: usize,
    used: AtomicUsize,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}
impl Budget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: AtomicUsize::new(0),
        }
    }
    pub(crate) fn limit(&self) -> usize {
        self.limit
    }
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
    pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Permit, String> {
        let mut used = self.used();
        loop {
            let next = used
                .checked_add(bytes)
                .filter(|value| *value <= self.limit)
                .ok_or_else(|| {
                    format!(
                        "Additional Native sound-chip memory budget exceeded ({} bytes)",
                        self.limit
                    )
                })?;
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    return Ok(Permit {
                        budget: Arc::clone(self),
                        bytes,
                    })
                }
                Err(value) => used = value,
            }
        }
    }
}

// Intentionally not Clone: there is exactly one accounting release per charge.
// Share the containing immutable payload/instance through Arc instead.
#[derive(Debug)]
pub(crate) struct Permit {
    budget: Arc<Budget>,
    bytes: usize,
}
impl Permit {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let previous = self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes, "Native sound-chip budget underflow");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn default_and_fixed_counts() {
        let budget = Arc::new(Budget::default());
        assert_eq!(budget.limit(), 64 * 1024 * 1024);
        let mut cores = Vec::new();
        for _ in 0..16 {
            cores.push(
                budget
                    .reserve(INSTANCE_BYTES + YM2608_MEMORY_BYTES)
                    .unwrap(),
            );
        }
        assert_eq!(budget.used(), 16 * (INSTANCE_BYTES + YM2608_MEMORY_BYTES));
        assert!(budget.used() < budget.limit());
        drop(cores);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn exact_bound_and_failed_admission_do_not_leak() {
        let budget = Arc::new(Budget::new(100));
        let first = budget.reserve(60).unwrap();
        assert_eq!(first.bytes(), 60);
        assert!(budget.reserve(41).is_err());
        assert_eq!(budget.used(), 60);
        let second = budget.reserve(40).unwrap();
        assert!(budget.reserve(1).is_err());
        drop(first);
        assert_eq!(budget.used(), 40);
        drop(second);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn overflow_and_zero_size_remain_bounded() {
        let budget = Arc::new(Budget::new(usize::MAX));
        let full = budget.reserve(usize::MAX).unwrap();
        assert!(budget.reserve(1).is_err());
        let zero = budget.reserve(0).unwrap();
        drop(zero);
        assert_eq!(budget.used(), usize::MAX);
        drop(full);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn old_new_overlap_and_stalled_retirement_keep_both_charges() {
        let budget = Arc::new(Budget::new(1000));
        let old = Arc::new(budget.reserve(400 + PAYLOAD_OVERHEAD).unwrap());
        let callback = Arc::clone(&old);
        assert!(budget.reserve(400 + PAYLOAD_OVERHEAD).is_err());
        drop(old); // Control registry replacement alone cannot release storage.
        assert_eq!(budget.used(), 656);
        let retired_on_control = callback;
        drop(retired_on_control);
        let replacement = budget.reserve(400 + PAYLOAD_OVERHEAD).unwrap();
        assert_eq!(budget.used(), replacement.bytes());
    }

    #[test]
    fn partial_batch_failure_rolls_back_prior_guards() {
        let budget = Arc::new(Budget::new(100));
        let admission = (|| -> Result<Vec<Permit>, String> {
            let mut values = vec![budget.reserve(30)?, budget.reserve(40)?];
            values.push(budget.reserve(40)?);
            Ok(values)
        })();
        assert!(admission.is_err());
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn concurrent_reservations_never_oversubscribe() {
        let budget = Arc::new(Budget::new(10));
        let start = Arc::new(Barrier::new(32));
        let hold = Arc::new(Barrier::new(32));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let (budget, start, hold) =
                    (Arc::clone(&budget), Arc::clone(&start), Arc::clone(&hold));
                std::thread::spawn(move || {
                    start.wait();
                    let permit = budget.reserve(1).ok();
                    hold.wait();
                    assert!(budget.used() <= budget.limit());
                    permit
                })
            })
            .collect();
        let admitted: Vec<_> = threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(admitted.len(), 10);
        assert_eq!(budget.used(), 10);
        drop(admitted);
        assert_eq!(budget.used(), 0);
    }
}
