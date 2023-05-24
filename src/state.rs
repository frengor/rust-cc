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
        state.execution_counter.set(0);
    });
}

#[derive(Default)]
pub(crate) struct State {
    collecting: Cell<bool>,

    #[cfg(feature = "finalization")]
    finalizing: Cell<bool>,

    dropping: Cell<bool>,
    allocated_bytes: Cell<usize>,
    execution_counter: Cell<usize>,
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
    pub(crate) fn execution_count(&self) -> usize {
        self.execution_counter.get()
    }

    #[inline]
    pub(super) fn increment_execution_count(&self) {
        self.execution_counter.set(self.execution_counter.get() + 1);
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
pub fn execution_count() -> Result<usize, StateAccessError> {
    STATE.try_with(|state| Ok(state.execution_count()))?
}

/// Utility macro used internally to implement drop guards that accesses the state
macro_rules! replace_state_field {
    (dropping, $value:expr) => {
        $crate::state::replace_state_field!(__internal is_dropping, set_dropping, bool, $value)
    };
    (finalizing, $value:expr) => {
        $crate::state::replace_state_field!(__internal is_finalizing, set_finalizing, bool, $value)
    };
    (__internal $is_name:ident, $set_name:ident, $field_type:ty, $value:expr) => {
        $crate::state::state(|state| {
            let old_value: $field_type = $crate::state::State::$is_name(state);
            $crate::state::State::$set_name(state, $value);

            #[must_use = "the drop guard shouldn't be dropped instantly"]
            struct DropGuard {
                old_value: $field_type,
            }

            impl ::std::ops::Drop for DropGuard {
                #[inline]
                fn drop(&mut self) {
                    $crate::state::replace_state_field!(__drop_impl DropGuard, $set_name, self.old_value);
                }
            }

            DropGuard { old_value }
        })
    };
    (__drop_impl $struct_name:ident, $set_name:ident, $old_value:expr) => {
        {
            let res = $crate::state::try_state(|state| {
                $crate::state::State::$set_name(state, $old_value);
            });

            if ::std::result::Result::is_err(&res) {
                // If we cannot reset the internal state then abort, since continuing may lead to UB
                // Note that this should never happen though!

                #[cold]
                #[inline(always)]
                fn cannot_reset() {
                    // Use catch_unwind to avoid panicking if writing to stderr fails, since we want to always abort here
                    let _ = ::std::panic::catch_unwind(|| ::std::eprintln!("Couldn't reset state. This is (probably) a rust-cc bug, please report it at https://github.com/frengor/rust-cc/issues."));
                    ::std::process::abort();
                }

                cannot_reset();
            }
        };
    };
}

// This makes replace_state_field macro usable across modules
pub(crate) use replace_state_field;

#[cfg(test)]
mod tests {
    use crate::state::{state};

    #[test]
    fn test_replace_state_field() {
        // Test state.dropping = true
        state(|state| state.set_dropping(true));
        {
            let _finalizing_guard = replace_state_field!(dropping, false);
            state(|state| assert!(!state.is_dropping()));
        }
        state(|state| assert!(state.is_dropping()));

        // Test state.dropping = false
        state(|state| state.set_dropping(false));
        {
            let _dropping_guard = replace_state_field!(dropping, true);
            state(|state| assert!(state.is_dropping()));
        }
        state(|state| assert!(!state.is_dropping()));

        #[cfg(feature = "finalization")]
        {
            // Test state.finalizing = true
            state(|state| state.set_finalizing(true));
            {
                let _finalizing_guard = replace_state_field!(finalizing, false);
                state(|state| assert!(!state.is_finalizing()));
            }
            state(|state| assert!(state.is_finalizing()));

            // Test state.finalizing = false
            state(|state| state.set_finalizing(false));
            {
                let _finalizing_guard = replace_state_field!(finalizing, true);
                state(|state| assert!(state.is_finalizing()));
            }
            state(|state| assert!(!state.is_finalizing()));
        }
    }
}
