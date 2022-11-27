use std::cell::{BorrowMutError, RefCell};
use std::ops::DerefMut;
use std::thread::AccessError;
use thiserror::Error;

use crate::state::State;

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
    // adjustment_percent <= (allocated_bytes / bytes_threshold) <= trigger_percent
    bytes_threshold: usize,
    trigger_percent: f64,
    adjustment_percent: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bytes_threshold: 100,
            trigger_percent: 0.7,
            adjustment_percent: 0.25,
        }
    }
}

impl Config {
    #[inline]
    pub fn collection_trigger_percent(&self) -> f64 {
        self.trigger_percent
    }

    #[inline]
    #[track_caller]
    pub fn set_collection_trigger_percent(&mut self, percent: f64) {
        assert!(
            percent > 0f64 && percent < 1f64,
            "percent must be between 0 and 1 (excluded)"
        );
        assert!(
            percent > self.adjustment_percent,
            "percent must be greater than adjustment percent"
        );
        self.trigger_percent = percent;
    }

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
        assert!(
            percent < self.trigger_percent,
            "percent must be less than collection trigger percent"
        );
        self.adjustment_percent = percent;
    }

    #[inline(always)]
    pub(crate) fn should_collect(&mut self, state: &State) -> bool {
        state.allocated_bytes() as f64 > self.trigger_percent * self.bytes_threshold as f64
    }

    #[inline(always)]
    pub(crate) fn adjust(&mut self, state: &State) {
        if (state.allocated_bytes() as f64) < self.adjustment_percent * self.bytes_threshold as f64
        {
            self.bytes_threshold = (state.allocated_bytes() as f64 / self.trigger_percent) as usize;
        }
    }
}
