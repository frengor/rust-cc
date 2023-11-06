#![cfg(feature = "auto-collect")]

use std::cell::RefCell;

use rust_cc::{Cc, collect_cycles, Context, Finalize, Trace};
use rust_cc::config::config;
use rust_cc::state::executions_count;

struct Traceable {
    inner: RefCell<Option<Cc<Traceable>>>,
    _big: Big,
}

struct Big {
    _array: [i64; 4096],
}

impl Default for Big {
    fn default() -> Self {
        Big { _array: [0; 4096] }
    }
}

unsafe impl Trace for Traceable {
    fn trace(&self, ctx: &mut Context<'_>) {
        self.inner.trace(ctx);
    }
}

impl Finalize for Traceable {}

impl Traceable {
    fn new() -> Cc<Self> {
        let traceable = Cc::new(Traceable {
            inner: RefCell::new(None),
            _big: Default::default(),
        });
        *traceable.inner.borrow_mut() = Some(Cc::new(Traceable {
            inner: RefCell::new(Some(traceable.clone())),
            _big: Default::default(),
        }));
        traceable
    }
}

#[test]
fn test_auto_collect() {
    {
        let traceable = Traceable::new();

        let executions_counter = executions_count().unwrap();
        drop(traceable);
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");

        let _ = Cc::new(Traceable {
            inner: RefCell::new(None),
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_ne!(executions_counter, executions_count().unwrap(), "Didn't collected");
    }
    collect_cycles(); // Make sure to don't leak test's memory
}

#[test]
fn test_disable_auto_collect() {
    config(|config| config.set_auto_collect(false)).expect("Couldn't disable auto-collect");

    // Always re-enable auto-collect, even with panics
    struct DropGuard;
    impl Drop for DropGuard {
        fn drop(&mut self) {
            config(|config| config.set_auto_collect(true)).expect("Couldn't re-enable auto-collect");
        }
    }
    let _drop_guard = DropGuard;

    {
        let executions_counter = executions_count().unwrap();
        let traceable = Traceable::new();

        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");
        drop(traceable);
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");

        let _ = Cc::new(Traceable {
            inner: RefCell::new(None),
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");
    }
    collect_cycles(); // Make sure to don't leak test's memory
}
