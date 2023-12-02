use core::cell::RefCell;
use core::num::NonZeroUsize;

use thiserror::Error;
use crate::list::CountedList;

use crate::state::State;
use crate::utils;

const DEFAULT_BYTES_THRESHOLD: usize = 100;

utils::rust_cc_thread_local! {
    pub(crate) static CONFIG: RefCell<Config> = const { RefCell::new(Config::new()) };
}

pub fn config<F, R>(f: F) -> Result<R, ConfigAccessError>
where
    F: FnOnce(&mut Config) -> R,
{
    CONFIG.try_with(|config| {
        config
        .try_borrow_mut()
        .or(Err(ConfigAccessError::BorrowMutError))
        .map(|mut config| f(&mut config))
    }).unwrap_or(Err(ConfigAccessError::AccessError))
}

#[non_exhaustive]
#[derive(Error, Debug)]
pub enum ConfigAccessError {
    #[error("couldn't access the configuration")]
    AccessError,
    #[error("couldn't borrow the configuration mutably")]
    BorrowMutError,
}

#[derive(Debug, Clone)]
pub struct Config {
    // The invariant is:
    // bytes_threshold * adjustment_percent < allocated_bytes < bytes_threshold
    bytes_threshold: usize,
    adjustment_percent: f64,
    buffered_threshold: Option<NonZeroUsize>,
    auto_collect: bool,
}

impl Config {
    #[inline]
    const fn new() -> Self {
        Self {
            bytes_threshold: DEFAULT_BYTES_THRESHOLD,
            adjustment_percent: 0.1,
            buffered_threshold: None,
            auto_collect: true,
        }
    }

    #[inline]
    pub fn auto_collect(&self) -> bool {
        self.auto_collect
    }

    #[inline]
    pub fn set_auto_collect(&mut self, auto_collect: bool) {
        self.auto_collect = auto_collect;
    }

    #[inline]
    pub fn adjustment_percent(&self) -> f64 {
        self.adjustment_percent
    }

    #[inline]
    #[track_caller]
    pub fn set_adjustment_percent(&mut self, percent: f64) {
        assert!(
            (0f64..=1f64).contains(&percent),
            "percent must be between 0 and 1"
        );
        self.adjustment_percent = percent;
    }

    #[inline]
    pub fn buffered_objects_threshold(&self) -> Option<NonZeroUsize> {
        self.buffered_threshold
    }

    #[inline]
    #[track_caller]
    pub fn set_buffered_objects_threshold(&mut self, threshold: Option<NonZeroUsize>) {
        self.buffered_threshold = threshold;
    }

    #[inline(always)]
    pub(super) fn should_collect(&mut self, state: &State, possible_cycles: &RefCell<CountedList>) -> bool {
        if !self.auto_collect {
            return false;
        }

        if state.allocated_bytes() > self.bytes_threshold {
            return true;
        }

        return if let Some(buffered_threshold) = self.buffered_threshold {
            possible_cycles.try_borrow().map_or(false, |pc| pc.size() > buffered_threshold.get())
        } else {
            false
        }
    }

    #[inline(always)]
    pub(super) fn adjust(&mut self, state: &State) {
        // First case: the threshold might have to be increased
        if state.allocated_bytes() >= self.bytes_threshold {

            loop {
                let Some(new_threshold) = self.bytes_threshold.checked_shl(1) else { break; };
                self.bytes_threshold = new_threshold;
                if state.allocated_bytes() < self.bytes_threshold {
                    break;
                }
            }

            return; // Skip the other case
        }

        // Second case: the threshold might have to be decreased
        let allocated = state.allocated_bytes() as f64;

        // If adjustment_percent or the result of the multiplication is 0 do nothing
        if ((self.bytes_threshold as f64) * self.adjustment_percent) == 0.0 {
            return;
        }

        // No more cases after this, there's no need to use an additional if as above
        while allocated <= ((self.bytes_threshold as f64) * self.adjustment_percent) {
            let new_threshold = self.bytes_threshold >> 1;
            if state.allocated_bytes() >= new_threshold {
                break; // If the shift produces a threshold <= allocated, then don't update bytes_threshold to maintain the invariant
            }
            if new_threshold <= DEFAULT_BYTES_THRESHOLD {
                self.bytes_threshold = DEFAULT_BYTES_THRESHOLD;
                break;
            }
            self.bytes_threshold = new_threshold;
        }
    }
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
