//! Information about the garbage collector state.

use alloc::alloc::Layout;
use alloc::rc::Rc;
use core::cell::Cell;
use core::marker::PhantomData;

use thiserror::Error;

use crate::utils;

utils::rust_cc_thread_local! {
    static STATE: State = const { State::new() };
}

#[inline]
pub(crate) fn state<R>(f: impl FnOnce(&State) -> R) -> R {
    // Use try_state instead of state.with(...) since with is not marked as inline
    try_state(f).unwrap_or_else(|_| panic!("Couldn't access the state"))
}

#[inline]
pub(crate) fn try_state<R>(f: impl FnOnce(&State) -> R) -> Result<R, StateAccessError> {
    STATE.try_with(|state| Ok(f(state))).unwrap_or(Err(StateAccessError::AccessError))
}

/// An error returned by functions accessing the garbage collector state.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum StateAccessError {
    /// The garbage collector state couldn't be accessed.
    #[error("couldn't access the state")]
    AccessError,
}

#[cfg(all(test, feature = "std"))] // Only used in unit tests
pub(crate) fn reset_state() {
    state(|state| {
        state.collecting.set(false);

        #[cfg(feature = "finalization")]
        state.finalizing.set(false);

        state.dropping.set(false);
        state.allocated_bytes.set(0);
        state.executions_counter.set(0);
    });
}

pub(crate) struct State {
    collecting: Cell<bool>,

    #[cfg(feature = "finalization")]
    finalizing: Cell<bool>,

    dropping: Cell<bool>,
    allocated_bytes: Cell<usize>,
    executions_counter: Cell<usize>,

    _phantom: PhantomData<Rc<()>>, // Make State !Send and !Sync
}

impl State {
    #[inline]
    const fn new() -> Self {
        Self {
            collecting: Cell::new(false),

            #[cfg(feature = "finalization")]
            finalizing: Cell::new(false),

            dropping: Cell::new(false),
            allocated_bytes: Cell::new(0),
            executions_counter: Cell::new(0),

            _phantom: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.allocated_bytes.get()
    }

    #[inline]
    pub(crate) fn record_allocation(&self, layout: Layout) {
        self.allocated_bytes.set(self.allocated_bytes.get() + layout.size());
    }

    #[inline]
    pub(crate) fn record_deallocation(&self, layout: Layout) {
        self.allocated_bytes.set(self.allocated_bytes.get() - layout.size());
    }

    #[inline]
    pub(crate) fn executions_count(&self) -> usize {
        self.executions_counter.get()
    }

    #[inline]
    pub(super) fn increment_executions_count(&self) {
        self.executions_counter.set(self.executions_counter.get() + 1);
    }

    #[inline]
    pub(crate) fn is_collecting(&self) -> bool {
        self.collecting.get()
    }

    #[inline]
    pub(crate) fn set_collecting(&self, value: bool) {
        self.collecting.set(value);
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn is_finalizing(&self) -> bool {
        self.finalizing.get()
    }

    #[cfg(feature = "finalization")]
    #[inline]
    pub(crate) fn set_finalizing(&self, value: bool) {
        self.finalizing.set(value);
    }

    #[inline]
    pub(crate) fn is_dropping(&self) -> bool {
        self.dropping.get()
    }

    #[inline]
    pub(crate) fn set_dropping(&self, value: bool) {
        self.dropping.set(value);
    }

    #[inline]
    #[allow(dead_code)] // Currently used only inside #[cfg(debug_assertions)], but always keep it
    pub(crate) fn is_tracing(&self) -> bool {
        #[cfg(feature = "finalization")]
        {
            self.collecting.get() && !self.finalizing.get() && !self.dropping.get()
        }

        #[cfg(not(feature = "finalization"))]
        {
            self.collecting.get() && !self.dropping.get()
        }
    }
}

impl Default for State {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the number of objects buffered to be processed in the next collection.
///
/// See [`Cc::mark_alive`][`crate::Cc::mark_alive`] for more details.
#[inline]
pub fn buffered_objects_count() -> Result<usize, StateAccessError> {
    // Expose this in state module even though the count is kept inside POSSIBLE_CYCLES
    // The error returned in case of failed access is a generic StateAccessError::AccessError
    crate::POSSIBLE_CYCLES
        .try_with(|pc| Ok(pc.size()))
        .unwrap_or(Err(StateAccessError::AccessError))
}

/// Returns the number of allocated bytes managed by the garbage collector.
#[inline]
pub fn allocated_bytes() -> Result<usize, StateAccessError> {
    try_state(|state| Ok(state.allocated_bytes()))?
}

/// Returns the total number of executed collections.
#[inline]
pub fn executions_count() -> Result<usize, StateAccessError> {
    try_state(|state| Ok(state.executions_count()))?
}

/// Returns `true` if the garbage collector is in a tracing phase, `false` otherwise.
///
/// See [`Trace`][`trait@crate::Trace`] for more details.
#[inline]
pub fn is_tracing() -> Result<bool, StateAccessError> {
    try_state(|state| Ok(state.is_tracing()))?
}

/// Utility macro used internally to implement drop guards that accesses the state
macro_rules! replace_state_field {
    (dropping, $value:expr, $state:ident) => {
        $crate::state::replace_state_field!(__internal is_dropping, set_dropping, bool, $value, $state)
    };
    (finalizing, $value:expr, $state:ident) => {
        $crate::state::replace_state_field!(__internal is_finalizing, set_finalizing, bool, $value, $state)
    };
    (__internal $is_name:ident, $set_name:ident, $field_type:ty, $value:expr, $state:ident) => {
        {
            let old_value: $field_type = $crate::state::State::$is_name($state);
            $crate::state::State::$set_name($state, $value);

            #[must_use = "the drop guard shouldn't be dropped instantly"]
            struct DropGuard<'a> {
                state: &'a $crate::state::State,
                old_value: $field_type,
            }

            impl<'a> ::core::ops::Drop for DropGuard<'a> {
                #[inline]
                fn drop(&mut self) {
                    $crate::state::State::$set_name(self.state, self.old_value);
                }
            }

            #[allow(clippy::redundant_field_names)]
            DropGuard { state: $state, old_value }
        }
    };
}

// This makes replace_state_field macro usable across modules
pub(crate) use replace_state_field;

#[cfg(test)]
mod tests {
    use crate::state::state;

    #[test]
    fn test_replace_state_field() {
        state(|state| {
            // Test state.dropping = true
            state.set_dropping(true);
            {
                let _finalizing_guard = replace_state_field!(dropping, false, state);
                assert!(!state.is_dropping());
            }
            assert!(state.is_dropping());

            // Test state.dropping = false
            state.set_dropping(false);
            {
                let _dropping_guard = replace_state_field!(dropping, true, state);
                assert!(state.is_dropping());
            }
            assert!(!state.is_dropping());

            #[cfg(feature = "finalization")]
            {
                // Test state.finalizing = true
                state.set_finalizing(true);
                {
                    let _finalizing_guard = replace_state_field!(finalizing, false, state);
                    assert!(!state.is_finalizing());
                }
                assert!(state.is_finalizing());

                // Test state.finalizing = false
                state.set_finalizing(false);
                {
                    let _finalizing_guard = replace_state_field!(finalizing, true, state);
                    assert!(state.is_finalizing());
                }
                assert!(!state.is_finalizing());
            }
        });
    }
}
