//! Configuration of the garbage collector.
//!
//! The configuration can be accessed using the [`config`][`fn@config`] function.
//!
//! # Automatic collection executions
//!
//! Collections can be automatically started if [`auto_collect`][`fn@Config::auto_collect`] is set to `true`.
//!
//! To determine whether to start a collection, a *threshold* is kept over the number of allocated bytes.
//! When calling a function which may start a collection (e.g. [`Cc::new`][`crate::Cc::new`]),
//! if the number of allocated bytes exceeds the *threshold* a collection is started.
//!
//! At the end of the automatically started collection, if the *threshold* is still lower than the number of allocated bytes
//! then it is doubled until it exceed it.
//!
//! Instead, if the number of allocated bytes exceed the *threshold* multiplied by the [`adjustment_percent`][`fn@Config::adjustment_percent`],
//! then the *threshold* is halved until the condition becomes true.
//!
//! Finally, a collection may also happen if the number of objects buffered to be processed in the next collection (see [`Cc::mark_alive`][`crate::Cc::mark_alive`])
//! exceeds the [`buffered_objects_threshold`][`fn@Config::buffered_objects_threshold`]. This parameter is disabled by default, but can be enabled by
//! using [`set_buffered_objects_threshold`][`fn@Config::set_buffered_objects_threshold`].

use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;
use core::num::NonZeroUsize;

use thiserror::Error;

use crate::lists::PossibleCycles;
use crate::state::State;
use crate::utils;

const DEFAULT_BYTES_THRESHOLD: usize = 100;

utils::rust_cc_thread_local! {
    pub(crate) static CONFIG: RefCell<Config> = const { RefCell::new(Config::new()) };
}

/// Access the configuration.
///
/// Returns [`Err`] if the configuration is already being accessed.
///
/// # Panics
///
/// Panics if the provided closure panics.
///
/// # Example
/// ```rust
///# use rust_cc::config::config;
/// let res = config(|config| {
///     // Edit the configuration
/// }).unwrap();
/// ```
pub fn config<F, R>(f: F) -> Result<R, ConfigAccessError>
where
    F: FnOnce(&mut Config) -> R,
{
    CONFIG
        .try_with(|config| {
            config
                .try_borrow_mut()
                .or(Err(ConfigAccessError::ConcurrentAccessError))
                .map(|mut config| f(&mut config))
        })
        .unwrap_or(Err(ConfigAccessError::AccessError))
}

/// An error returned by [`config`][`fn@config`].
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum ConfigAccessError {
    /// The configuration couldn't be accessed.
    #[error("couldn't access the configuration")]
    AccessError,
    /// The configuration was already being accessed.
    #[error("the configuration is already being accessed")]
    ConcurrentAccessError,
}

/// The configuration of the garbage collector.
#[derive(Debug, Clone)]
pub struct Config {
    // The invariant is:
    // bytes_threshold * adjustment_percent < allocated_bytes < bytes_threshold
    bytes_threshold: usize,
    adjustment_percent: f64,
    buffered_threshold: Option<NonZeroUsize>,
    auto_collect: bool,
    _phantom: PhantomData<Rc<()>>, // Make Config !Send and !Sync
}

impl Config {
    #[inline]
    const fn new() -> Self {
        Self {
            bytes_threshold: DEFAULT_BYTES_THRESHOLD,
            adjustment_percent: 0.1,
            buffered_threshold: None,
            auto_collect: true,
            _phantom: PhantomData,
        }
    }

    /// Returns `true` if collections can be automatically started, `false` otherwise.
    #[inline]
    pub fn auto_collect(&self) -> bool {
        self.auto_collect
    }

    /// Sets whether collections can be automatically started.
    #[inline]
    pub fn set_auto_collect(&mut self, auto_collect: bool) {
        self.auto_collect = auto_collect;
    }

    /// Returns the threshold adjustment percent.
    ///
    /// See the [module-level documentation][`mod@crate::config`] for more details.
    #[inline]
    pub fn adjustment_percent(&self) -> f64 {
        self.adjustment_percent
    }

    /// Sets the threshold adjustment percent.
    ///
    /// See the [module-level documentation][`mod@crate::config`] for more details.
    ///
    /// # Panics
    ///
    /// Panics if the provided `percent` isn't between 0 and 1 (included).
    #[inline]
    #[track_caller]
    pub fn set_adjustment_percent(&mut self, percent: f64) {
        assert!(
            (0f64..=1f64).contains(&percent),
            "percent must be between 0 and 1"
        );
        self.adjustment_percent = percent;
    }

    /// Returns the buffered-objects threshold (see [`Cc::mark_alive`][`crate::Cc::mark_alive`]).
    ///
    /// Returns [`None`] if this parameter isn't used to start a collection.
    ///
    /// See the [module-level documentation][`mod@crate::config`] for more details.
    #[inline]
    pub fn buffered_objects_threshold(&self) -> Option<NonZeroUsize> {
        self.buffered_threshold
    }

    /// Sets the buffered-objects threshold (see [`Cc::mark_alive`][`crate::Cc::mark_alive`]).
    ///
    /// If the provided `threshold` is [`None`], then this parameter will not be used to start a collection.
    ///
    /// See the [module-level documentation][`mod@crate::config`] for more details.
    #[inline]
    #[track_caller]
    pub fn set_buffered_objects_threshold(&mut self, threshold: Option<NonZeroUsize>) {
        self.buffered_threshold = threshold;
    }

    #[inline(always)]
    pub(super) fn should_collect(
        &mut self,
        state: &State,
        possible_cycles: &PossibleCycles,
    ) -> bool {
        if !self.auto_collect {
            return false;
        }

        if state.allocated_bytes() > self.bytes_threshold {
            return true;
        }

        if let Some(buffered_threshold) = self.buffered_threshold {
            possible_cycles.size() > buffered_threshold.get()
        } else {
            false
        }
    }

    #[inline(always)]
    pub(super) fn adjust(&mut self, state: &State) {
        // First case: the threshold might have to be increased
        if state.allocated_bytes() >= self.bytes_threshold {
            loop {
                let Some(new_threshold) = self.bytes_threshold.checked_shl(1) else {
                    break;
                };
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
