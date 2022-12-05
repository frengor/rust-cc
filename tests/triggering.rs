use std::cell::RefCell;
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
        fn trace<'a, 'b: 'a>(&self, ctx: &'a mut Context<'b>) {
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
    thread_local! {
        static TRACE: RefCell<bool> = RefCell::new(false);
    }

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
        fn trace<'a, 'b: 'a>(&self, ctx: &'a mut Context<'b>) {
            TRACE.with(|trace| *trace.borrow_mut() = true);
            if let Some(cc) = &self.inner {
                cc.trace(ctx);
            }
        }
    }

    impl Finalize for Traceable {}

    {
        let _traceable = Cc::<Traceable>::new_cyclic(|cc| Traceable {
            inner: Some(Cc::new(Traceable {
                inner: Some(Cc::new(Traceable {
                    inner: Some(cc.clone()),
                    _big: Default::default(),
                })),
                _big: Default::default(),
            })),
            _big: Default::default(),
        });

        assert!(
            !TRACE.with(|trace| *trace.borrow()),
            "Collected but shouldn't have collected."
        );
        drop(_traceable);
        assert!(
            !TRACE.with(|trace| *trace.borrow()),
            "Collected but shouldn't have collected."
        );

        let _ = Cc::new(0); // Collection should be triggered by allocations
        assert!(TRACE.with(|trace| *trace.borrow()), "Didn't collected");
    }
    collect_cycles(); // Make sure to don't leak test's memory
}
