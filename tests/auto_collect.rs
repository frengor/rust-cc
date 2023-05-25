#![cfg(feature = "auto-collect")]

use rust_cc::state::executions_count;
use rust_cc::{collect_cycles, Cc, Context, Finalize, Trace};
use rust_cc::config::config;

#[test]
fn test_auto_collect() {
    struct Traceable {
        inner: Option<Cc<Traceable>>,
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

    {
        let _traceable = Cc::<Traceable>::new_cyclic(|cc| Traceable {
            inner: Some(Cc::new(Traceable {
                inner: Some(cc.clone()),
                _big: Default::default(),
            })),
            _big: Default::default(),
        });

        let executions_counter = executions_count().unwrap();
        drop(_traceable);
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");

        let _ = Cc::new(Traceable {
            inner: None,
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

    struct Traceable {
        inner: Option<Cc<Traceable>>,
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

    {
        let executions_counter = executions_count().unwrap();
        let _traceable = Cc::<Traceable>::new_cyclic(|cc| Traceable {
            inner: Some(Cc::new(Traceable {
                inner: Some(cc.clone()),
                _big: Default::default(),
            })),
            _big: Default::default(),
        });

        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");
        drop(_traceable);
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");

        let _ = Cc::new(Traceable {
            inner: None,
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_eq!(executions_counter, executions_count().unwrap(), "Collected but shouldn't have collected.");
    }
    collect_cycles(); // Make sure to don't leak test's memory
}
