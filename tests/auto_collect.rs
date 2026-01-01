#![cfg(feature = "auto-collect")]

use std::cell::RefCell;
use std::num::NonZeroUsize;

use rust_cc::config::config;
use rust_cc::state::executions_count;
use rust_cc::{Cc, Context, Finalize, Trace, collect_cycles};

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
        assert_eq!(
            executions_counter,
            executions_count().unwrap(),
            "Collected but shouldn't have collected."
        );

        let _ = Cc::new(Traceable {
            inner: RefCell::new(None),
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_ne!(
            executions_counter,
            executions_count().unwrap(),
            "Didn't collected"
        );
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
            config(|config| config.set_auto_collect(true))
                .expect("Couldn't re-enable auto-collect");
        }
    }
    let _drop_guard = DropGuard;

    {
        let executions_counter = executions_count().unwrap();
        let traceable = Traceable::new();

        assert_eq!(
            executions_counter,
            executions_count().unwrap(),
            "Collected but shouldn't have collected."
        );
        drop(traceable);
        assert_eq!(
            executions_counter,
            executions_count().unwrap(),
            "Collected but shouldn't have collected."
        );

        let _ = Cc::new(Traceable {
            inner: RefCell::new(None),
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_eq!(
            executions_counter,
            executions_count().unwrap(),
            "Collected but shouldn't have collected."
        );
    }
    collect_cycles(); // Make sure to don't leak test's memory
}

#[test]
fn test_buffered_threshold_auto_collect() {
    const MAX_BUFFERED_OBJS: usize = 4;

    // Always reset buffered objs threshold and adjustment percent, even with panics
    struct DropGuard(f64, Option<NonZeroUsize>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            config(|config| {
                config.set_adjustment_percent(self.0);
                config.set_buffered_objects_threshold(self.1);
            })
            .expect("Couldn't reset buffered objs threshold and adjustment percent");
        }
    }
    let _drop_guard = config(|config| {
        let guard = DropGuard(
            config.adjustment_percent(),
            config.buffered_objects_threshold(),
        );
        config.set_adjustment_percent(0.0);
        config.set_buffered_objects_threshold(Some(NonZeroUsize::new(3).unwrap()));
        guard
    })
    .expect("Couldn't set buffered objs threshold and adjustment percent");

    struct Cyclic<T: 'static> {
        cyclic: RefCell<Option<Cc<Cyclic<T>>>>,
        _t: T,
    }

    unsafe impl<T> Trace for Cyclic<T> {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cyclic.trace(ctx);
        }
    }

    impl<T> Finalize for Cyclic<T> {}

    fn new<T: Default>() -> Cc<Cyclic<T>> {
        let cc = Cc::new(Cyclic {
            cyclic: RefCell::new(None),
            _t: Default::default(),
        });
        *cc.cyclic.borrow_mut() = Some(cc.clone());
        cc
    }

    // Increase bytes_threshold
    {
        let _big = new::<Big>();
        collect_cycles();
    }
    collect_cycles();

    let executions_counter = executions_count().unwrap();

    assert_eq!(0, rust_cc::state::buffered_objects_count().unwrap());

    for _ in 0..MAX_BUFFERED_OBJS {
        let _ = new::<()>();

        assert_eq!(
            executions_counter,
            executions_count().unwrap(),
            "Collected but shouldn't have collected."
        );
    }

    let _ = new::<()>();

    assert_eq!(
        executions_counter + 1,
        executions_count().unwrap(),
        "Didn't collected"
    );
    collect_cycles(); // Make sure to don't leak test's memory
}
