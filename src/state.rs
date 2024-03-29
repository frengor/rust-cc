use std::alloc::Layout;
use std::cell::Cell;
use std::thread::AccessError;
use thiserror::Error;

thread_local! {
    static STATE: State = State::default();
}

#[inline]
pub(crate) fn state<R>(f: impl FnOnce(&State) -> R) -> R {
    // Use try_state instead of state.with(...) since with is not marked as inline
    try_state(f).unwrap_or_else(|err| panic!("Couldn't access state: {}", err))
}

#[inline]
pub(crate) fn try_state<R>(f: impl FnOnce(&State) -> R) -> Result<R, StateAccessError> {
    STATE.try_with(|state| Ok(f(state)))?
}

#[non_exhaustive]
#[derive(Error, Debug)]
pub enum StateAccessError {
    #[error(transparent)]
    AccessError(#[from] AccessError),
}

#[cfg(test)] // Used in tests
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

#[derive(Default)]
pub(crate) struct State {
    collecting: Cell<bool>,

    #[cfg(feature = "finalization")]
    finalizing: Cell<bool>,

    dropping: Cell<bool>,
    allocated_bytes: Cell<usize>,
    executions_counter: Cell<usize>,
}

impl State {
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

#[inline]
pub fn allocated_bytes() -> Result<usize, StateAccessError> {
    STATE.try_with(|state| Ok(state.allocated_bytes()))?
}

#[inline]
pub fn executions_count() -> Result<usize, StateAccessError> {
    STATE.try_with(|state| Ok(state.executions_count()))?
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

            impl<'a> ::std::ops::Drop for DropGuard<'a> {
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
    use crate::state::{state};

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
