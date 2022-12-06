#![cfg(feature = "auto-collect")]

use std::mem::size_of;

use rust_cc::state::execution_count;
use rust_cc::{collect_cycles, Cc, Context, Finalize, Trace};

#[test]
/// Useful to debug triggering.
/// To be called with `cargo test print_triggers -- --nocapture`
fn print_triggers() {
    const LENGTH: usize = 50;

    struct Traceable {
        inner: Cc<Traceable>,
        _elem: [u8; LENGTH],
    }

    unsafe impl Trace for Traceable {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.inner.trace(ctx);
        }
    }

    impl Finalize for Traceable {}

    fn new() -> Cc<Traceable> {
        Cc::<Traceable>::new_cyclic(|cc| Traceable {
            inner: cc.clone(),
            _elem: [0u8; LENGTH],
        })
    }

    {
        {
            println!("Size: {}", size_of::<Traceable>() + 32);
            let _ = new();
            println!("{}", execution_count());
            let _ = Cc::new(());
            println!("{}", execution_count());
            let _a = new();
            let _b = new();
            let _c = new();
            let _d = new();
        }
        println!("{}", execution_count());
        let _ = new();
        println!("{}", execution_count());
    }
    collect_cycles(); // Make sure to don't leak test's memory
}

#[test]
fn test_trigger() {
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

        let executions_count = execution_count();
        drop(_traceable);
        assert_eq!(executions_count, execution_count(), "Collected but shouldn't have collected.");

        let _ = Cc::new(Traceable {
            inner: None,
            _big: Default::default(),
        }); // Collection should be triggered by allocations
        assert_ne!(executions_count, execution_count(), "Didn't collected");
    }
    collect_cycles(); // Make sure to don't leak test's memory
}
