use std::cell::{BorrowMutError, RefCell};
use std::ops::DerefMut;
use std::thread::AccessError;

use thiserror::Error;

use crate::state::State;

const DEFAULT_BYTES_THRESHOLD: usize = 100;

thread_local! {
    pub(crate) static CONFIG: RefCell<Config> = RefCell::new(Config::default());
}

pub fn config<F, R>(f: F) -> Result<R, ConfigAccessError>
where
    F: FnOnce(&mut Config) -> R,
{
    CONFIG.try_with(|config| Ok(f(config.try_borrow_mut()?.deref_mut())))?
}

#[derive(Error, Debug)]
pub enum ConfigAccessError {
    #[error(transparent)]
    AccessError(#[from] AccessError),
    #[error(transparent)]
    BorrowMutError(#[from] BorrowMutError),
}

#[derive(Debug, Clone)]
pub struct Config {
    // The invariant is:
    // bytes_threshold * adjustment_percent < allocated_bytes < bytes_threshold
    bytes_threshold: usize,
    adjustment_percent: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bytes_threshold: DEFAULT_BYTES_THRESHOLD,
            adjustment_percent: 0.25,
        }
    }
}

impl Config {
    #[inline]
    pub fn adjustment_percent(&self) -> f64 {
        self.adjustment_percent
    }

    #[inline]
    #[track_caller]
    pub fn set_adjustment_percent(&mut self, percent: f64) {
        assert!(
            percent > 0f64 && percent < 1f64,
            "percent must be between 0 and 1 (excluded)"
        );
        self.adjustment_percent = percent;
    }

    #[inline(always)]
    pub(crate) fn should_collect(&mut self, state: &State) -> bool {
        state.allocated_bytes() > self.bytes_threshold
    }

    #[inline(always)]
    pub(crate) fn adjust(&mut self, state: &State) {
        if state.allocated_bytes() >= self.bytes_threshold {
            let Some(new_threshold) = self.bytes_threshold.checked_mul(2) else { return; };
            self.bytes_threshold = new_threshold;
            while state.allocated_bytes() >= self.bytes_threshold {
                let Some(new_threshold) = self.bytes_threshold.checked_mul(2) else { return; };
                self.bytes_threshold = new_threshold;
            }
            return;
        }

        let allocated = state.allocated_bytes() as f64;
        let mut bytes_threshold = self.bytes_threshold;
        if allocated <= ((self.bytes_threshold as f64) * self.adjustment_percent) {
            bytes_threshold <<= 1;
            if bytes_threshold <= DEFAULT_BYTES_THRESHOLD {
                self.bytes_threshold = DEFAULT_BYTES_THRESHOLD;
                return;
            }
            while allocated <= ((self.bytes_threshold as f64) * self.adjustment_percent) {
                bytes_threshold <<= 1;
                if bytes_threshold <= DEFAULT_BYTES_THRESHOLD {
                    self.bytes_threshold = DEFAULT_BYTES_THRESHOLD;
                    return;
                }
            }
            self.bytes_threshold = bytes_threshold;
        }
    }
}
