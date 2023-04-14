use std::alloc::Layout;
use std::cell::{BorrowMutError, RefCell};
use std::ops::DerefMut;
use std::thread::AccessError;
use thiserror::Error;

thread_local! {
    pub(crate) static STATE: RefCell<State> = RefCell::new(State::default());
}

#[track_caller]
#[inline]
pub(crate) fn state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // Use try_state instead of state.with(...) since with is not marked as inline
    try_state(f).unwrap_or_else(|err| panic!("Couldn't access state: {}", err))
}

#[track_caller]
#[inline]
pub(crate) fn try_state<R>(f: impl FnOnce(&mut State) -> R) -> Result<R, StateAccessError> {
    STATE.try_with(|state| Ok(f(state.try_borrow_mut()?.deref_mut())))?
}

#[derive(Error, Debug)]
pub(crate) enum StateAccessError {
    #[error(transparent)]
    AccessError(#[from] AccessError),
    #[error(transparent)]
    BorrowMutError(#[from] BorrowMutError),
}

#[derive(Default)]
pub(crate) struct State {
    collecting: bool,
    finalizing: bool,
    dropping: bool,
    allocated_bytes: usize,
    execution_counter: usize,
}

impl State {
    #[inline]
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }

    #[inline]
    pub(crate) fn record_allocation(&mut self, layout: Layout) {
        self.allocated_bytes += layout.size();
    }

    #[inline]
    pub(crate) fn record_deallocation(&mut self, layout: Layout) {
        self.allocated_bytes -= layout.size();
    }

    #[inline]
    pub(super) fn increment_execution_count(&mut self) {
        self.execution_counter += 1;
    }

    #[inline]
    pub(crate) fn is_collecting(&self) -> bool {
        self.collecting
    }

    #[inline]
    pub(crate) fn set_collecting(&mut self, value: bool) {
        self.collecting = value;
    }

    #[inline]
    pub(crate) fn is_finalizing(&self) -> bool {
        self.finalizing
    }

    #[inline]
    pub(crate) fn set_finalizing(&mut self, value: bool) {
        self.finalizing = value;
    }

    #[inline]
    pub(crate) fn is_dropping(&self) -> bool {
        self.dropping
    }

    #[inline]
    pub(crate) fn set_dropping(&mut self, value: bool) {
        self.dropping = value;
    }

    #[inline]
    pub(crate) fn is_tracing(&self) -> bool {
        self.collecting && !self.finalizing && !self.dropping
    }
}

#[inline]
#[track_caller]
pub fn allocated_bytes() -> usize {
    state(|state| state.allocated_bytes())
}

#[inline]
#[track_caller]
pub fn execution_count() -> usize {
    state(|state| state.execution_counter)
}

/// Utility macro used internally to implement drop guards that accesses the state
macro_rules! replace_state_field {
    (dropping, $value:expr) => {
        $crate::state::replace_state_field!(dropping, $value, |_state| {})
    };
    (dropping, $value:expr, |$state:ident| $arg:block) => {
        $crate::state::replace_state_field!(__internal is_dropping, set_dropping, bool, $value, |$state| $arg)
    };
    (finalizing, $value:expr) => {
        $crate::state::replace_state_field!(finalizing, $value, |_state| {})
    };
    (finalizing, $value:expr, |$state:ident| $arg:block) => {
        $crate::state::replace_state_field!(__internal is_finalizing, set_finalizing, bool, $value, |$state| $arg)
    };
    (__internal $is_name:ident, $set_name:ident, $field_type:ty, $value:expr, |$state:ident| $arg:block) => {
        $crate::state::state(|state| {
            let old_value: $field_type = $crate::state::State::$is_name(state);
            $crate::state::State::$set_name(state, $value);

            |$state: &mut $crate::state::State| -> () { $arg; } (state);

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
            // Cannot use state function here since it may panic
            let res = $crate::state::try_state(|state| {
                $crate::state::State::$set_name(state, $old_value);
            });

            if ::std::result::Result::is_err(&res) {
                // If we cannot reset the internal state then abort, since continuing may lead to UB
                // Note that this should never happen though!

                #[cold]
                fn cannot_reset() {
                    ::std::println!("Couldn't reset state. This is a Cc<_> bug, please report it."); // TODO Improve error message
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
    use crate::state;

    #[test]
    fn test_replace_state_field() {
        // Test state.dropping = true
        state(|state| state.dropping = true);
        {
            let _finalizing_guard = replace_state_field!(dropping, false);
            state(|state| assert!(!state.dropping));
        }
        state(|state| assert!(state.dropping));

        // Test state.dropping = false
        state(|state| state.dropping = false);
        {
            let _dropping_guard = replace_state_field!(dropping, true);
            state(|state| assert!(state.dropping));
        }
        state(|state| assert!(!state.dropping));

        // Test state.finalizing = true
        state(|state| state.finalizing = true);
        {
            let _finalizing_guard = replace_state_field!(finalizing, false);
            state(|state| assert!(!state.finalizing));
        }
        state(|state| assert!(state.finalizing));

        // Test state.finalizing = false
        state(|state| state.finalizing = false);
        {
            let _finalizing_guard = replace_state_field!(finalizing, true);
            state(|state| assert!(state.finalizing));
        }
        state(|state| assert!(!state.finalizing));
    }

    #[test]
    fn test_other_code() {
        state(|state| state.execution_counter = 0);
        let _drop_guard =
            replace_state_field!(dropping, true, |state| { state.execution_counter = 1 });
        state(|state| assert_eq!(state.execution_counter, 1));
        state(|state| state.execution_counter = 0); // Reset execution_counter
    }
}
