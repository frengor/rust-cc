#![cfg(test)]

use std::cell::Cell;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::trace::Trace;
use crate::{state, Cc, Context, Finalize, List, POSSIBLE_CYCLES};
use crate::state::state;

mod bench_code;
mod cc;
mod list;
mod panicking;
mod counter_marker;

#[cfg(feature = "weak-ptr")]
mod weak;

#[cfg(feature = "cleaners")]
mod cleaners;

pub(crate) fn reset_state() {
    POSSIBLE_CYCLES.with(|pc| {
        pc.replace(List::new());
    });
    state::reset_state();

    #[cfg(feature = "auto-collect")]
    {
        use super::config::{config, Config};
        config(|config| *config = Config::default()).expect("Couldn't reset the config.");
    }
}

pub(crate) struct Droppable<T: Trace> {
    inner: T,
    #[allow(unused)]
    finalize: Arc<Cell<bool>>,
    drop: Arc<Cell<bool>>,
}

impl<T: Trace> Droppable<T> {
    pub(crate) fn new(t: T) -> (Droppable<T>, DropChecker) {
        let finalize = Arc::new(Cell::new(false));
        let drop = Arc::new(Cell::new(false));
        (
            Droppable {
                inner: t,
                finalize: finalize.clone(),
                drop: drop.clone(),
            },
            DropChecker { finalize, drop },
        )
    }
}

impl<T: Trace> Deref for Droppable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: Trace> DerefMut for Droppable<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

unsafe impl<T: Trace> Trace for Droppable<T> {
    fn trace(&self, ctx: &mut Context<'_>) {
        assert_collecting();
        assert_tracing();
        self.inner.trace(ctx);
    }
}

impl<T: Trace> Finalize for Droppable<T> {
    fn finalize(&self) {
        #[cfg(not(feature = "finalization"))]
        panic!("Finalized called with finalization feature disabled!");

        #[cfg(feature = "finalization")]
        {
            assert_finalizing();
            self.finalize.set(true);
        }
    }
}

impl<T: Trace> Drop for Droppable<T> {
    fn drop(&mut self) {
        assert_dropping();
        // Set arc value to true
        self.drop.set(true);
    }
}

pub(crate) struct DropChecker {
    #[allow(unused)]
    finalize: Arc<Cell<bool>>,
    drop: Arc<Cell<bool>>,
}

impl DropChecker {
    pub(crate) fn assert_finalized(&self) {
        #[cfg(feature = "finalization")]
        assert!(self.finalize.get(), "Expected finalized!");
    }

    pub(crate) fn assert_not_finalized(&self) {
        #[cfg(feature = "finalization")]
        assert!(!self.finalize.get(), "Expected not finalized!");
    }

    pub(crate) fn assert_dropped(&self) {
        assert!(self.drop.get(), "Expected dropped!");
    }

    pub(crate) fn assert_not_dropped(&self) {
        assert!(!self.drop.get(), "Expected not dropped!");
    }
}

pub(crate) fn assert_empty() {
    let list = POSSIBLE_CYCLES.with(|pc| pc.borrow().first());
    assert!(list.is_none());
}

pub(crate) fn assert_collecting() {
    state(|state| {
        assert!(state.is_collecting());
    });
}

pub(crate) fn assert_tracing() {
    state(|state| {
        assert!(state.is_tracing());

        #[cfg(feature = "finalization")]
        assert!(!state.is_finalizing());

        assert!(!state.is_dropping());
    });
}

#[cfg(feature = "finalization")]
pub(crate) fn assert_finalizing() {
    state(|state| {
        assert!(!state.is_tracing());
        assert!(state.is_finalizing());
        assert!(!state.is_dropping());
    });
}

pub(crate) fn assert_dropping() {
    state(|state| {
        assert!(!state.is_tracing());

        #[cfg(feature = "finalization")]
        assert!(!state.is_finalizing());

        assert!(state.is_dropping());
    });
}

pub(crate) fn assert_state_not_collecting() {
    state(|state| {
        assert!(!state.is_collecting());
        assert!(!state.is_tracing());

        #[cfg(feature = "finalization")]
        assert!(!state.is_finalizing());

        assert!(!state.is_dropping());
    });
}

#[test]
fn make_sure_droppable_is_finalizable() {
    reset_state();

    let (droppable, checker) = Droppable::new(());

    {
        let _ = Cc::new(droppable);
    }

    #[cfg(feature = "finalization")]
    checker.assert_finalized();

    checker.assert_dropped();
}
