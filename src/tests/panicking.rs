use std::cell::{Cell, RefCell};
use std::mem;

use crate::tests::{assert_state_not_collecting, reset_state};
use crate::{Cc, Context, Finalize, Trace, collect_cycles};

fn register_panicking<T: Trace>(#[allow(unused_variables)] cc: &Cc<T>) {
    // The unused_variables warning is triggered when not running on Miri
    #[cfg(miri)]
    unsafe extern "Rust" {
        /// From Miri documentation:<br>
        /// _Miri-provided extern function to mark the block `ptr` points to as a "root"
        /// for some static memory. This memory and everything reachable by it is not
        /// considered leaking even if it still exists when the program terminates._<br>
        fn miri_static_root(ptr: *const u8);
    }

    #[cfg(miri)]
    // Use miri_static_root to avoid failures caused by leaks. Leaks are expected,
    // since this module tests panics (which leaks memory to prevent UB)
    unsafe {
        use core::mem::transmute;

        miri_static_root(*transmute::<_, &*const u8>(cc));
    }
}

fn panicking_collect_cycles(on_panic: impl FnOnce()) {
    struct DropGuard<F>
    where
        F: FnOnce(),
    {
        on_panic: Option<F>,
    }

    impl<F: FnOnce()> Drop for DropGuard<F> {
        fn drop(&mut self) {
            if let Some(f) = self.on_panic.take() {
                f();
            }
        }
    }

    let drop_guard = DropGuard {
        on_panic: Some(on_panic),
    };

    collect_cycles();

    mem::forget(drop_guard); // Don't execute on_panic when collect_cycles doesn't panic
}

struct Panicking {
    trace_counter: Cell<usize>,
    panic_on_trace: Cell<bool>,
    panic_on_finalize: Cell<bool>,
    panic_on_drop: Cell<bool>,
    cc: RefCell<Option<Cc<Panicking>>>,
}

unsafe impl Trace for Panicking {
    fn trace(&self, ctx: &mut Context<'_>) {
        let counter = self.trace_counter.get();
        if self.panic_on_trace.get() {
            if counter == 0 {
                panic!("Test panic on Trace");
            } else {
                self.trace_counter.set(counter - 1);
            }
        }
        self.cc.trace(ctx);
    }
}

impl Finalize for Panicking {
    fn finalize(&self) {
        if self.panic_on_finalize.get() {
            panic!("Test panic on Finalize");
        }
    }
}

impl Drop for Panicking {
    fn drop(&mut self) {
        if self.panic_on_drop.get() {
            panic!("Test panic on Drop");
        }
    }
}

#[test]
#[should_panic = "Test panic on Trace"]
fn test_panicking_tracing_counting() {
    reset_state();

    {
        let cc = Cc::new(Panicking {
            trace_counter: Cell::new(0),
            panic_on_trace: Cell::new(true),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(None),
        });
        *cc.cc.borrow_mut() = Some(cc.clone());
        register_panicking(&cc);
    }
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[should_panic = "Test panic on Trace"]
fn test_panicking_tracing_root() {
    reset_state();

    // Leave a root alive to trigger root tracing
    let _root = {
        let root = Cc::new(Panicking {
            trace_counter: Cell::new(1),
            panic_on_trace: Cell::new(true),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(None),
        });
        *root.cc.borrow_mut() = Some(Cc::new(Panicking {
            trace_counter: Cell::new(usize::MAX),
            panic_on_trace: Cell::new(false),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(Some(root.clone())),
        }));
        register_panicking(&root);
        register_panicking(root.cc.borrow().as_ref().unwrap());
        #[allow(clippy::redundant_clone)]
        root.clone()
    };
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[cfg_attr(feature = "finalization", should_panic = "Test panic on Finalize")]
fn test_panicking_finalize() {
    reset_state();

    {
        let cc = Cc::new(Panicking {
            trace_counter: Cell::new(usize::MAX),
            panic_on_trace: Cell::new(false),
            panic_on_finalize: Cell::new(true),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(None),
        });
        *cc.cc.borrow_mut() = Some(cc.clone());
        register_panicking(&cc);
    }
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[should_panic = "Test panic on Drop"]
fn test_panicking_drop() {
    reset_state();

    // Cannot use Panicking since Panicking implements Finalize
    struct DropPanicking {
        cyclic: RefCell<Option<Cc<DropPanicking>>>,
    }

    unsafe impl Trace for DropPanicking {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cyclic.trace(ctx);
        }
    }

    impl Finalize for DropPanicking {}

    impl Drop for DropPanicking {
        fn drop(&mut self) {
            panic!("Test panic on Drop");
        }
    }

    {
        let cc = Cc::new(DropPanicking {
            cyclic: RefCell::new(None),
        });
        *cc.cyclic.borrow_mut() = Some(cc.clone());
        register_panicking(&cc);
    }
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[should_panic = "Test panic on Drop"]
fn test_panicking_drop_and_finalize() {
    reset_state();

    {
        let cc = Cc::new(Panicking {
            trace_counter: Cell::new(usize::MAX),
            panic_on_trace: Cell::new(false),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(true),
            cc: RefCell::new(None),
        });
        *cc.cc.borrow_mut() = Some(cc.clone());
        register_panicking(&cc);
    }
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[cfg_attr(feature = "finalization", should_panic = "Test panic on Trace")]
fn test_panicking_tracing_drop() {
    reset_state();

    {
        // (See usage of this constant below for more context)
        // Since our objects are marked as non-roots they are traced a first time using
        // counting tracing, then they are NOT traced during root tracing and they are
        // then traced again during dropping tracing. So, 1 means that the second trace
        // call will panic, which is during dropping tracing
        const CELL_VALUE: usize = 1;

        let root = Cc::new(Panicking {
            trace_counter: Cell::new(CELL_VALUE),
            panic_on_trace: Cell::new(true),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(None),
        });
        *root.cc.borrow_mut() = Some(Cc::new(Panicking {
            trace_counter: Cell::new(CELL_VALUE),
            panic_on_trace: Cell::new(true),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(Some(root.clone())),
        }));

        register_panicking(&root);
        register_panicking(root.cc.borrow().as_ref().unwrap());
    }
    panicking_collect_cycles(assert_state_not_collecting);
}

#[test]
#[cfg_attr(feature = "finalization", should_panic = "Test panic on Trace")]
fn test_panicking_tracing_resurrecting() {
    reset_state();

    thread_local! {
        static RESURRECTED: Cell<Option<Cc<Panicking>>> = Cell::new(None);
    }

    struct DropGuard; // Used to clean up RESURRECTED

    impl Drop for DropGuard {
        fn drop(&mut self) {
            fn reset_panicking(panicking: &Panicking) {
                panicking.panic_on_trace.set(false);
                panicking.panic_on_finalize.set(false);
                panicking.panic_on_drop.set(false);
            }

            if let Some(replaced) = RESURRECTED.with(|cell| cell.replace(None)) {
                reset_panicking(&replaced);
                reset_panicking(replaced.cc.borrow().as_ref().unwrap());
                // replaced is dropped here
            }
            collect_cycles(); // Reclaim memory
        }
    }

    let _drop_guard = DropGuard;

    struct Resurrecter {
        panicking: Cc<Panicking>,
        cyclic: RefCell<Option<Cc<Resurrecter>>>,
    }

    unsafe impl Trace for Resurrecter {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.panicking.trace(ctx);
            self.cyclic.trace(ctx);
        }
    }

    impl Finalize for Resurrecter {
        fn finalize(&self) {
            RESURRECTED.with(|res| res.set(Some(self.panicking.clone())));
        }
    }

    {
        let a = Cc::new(Resurrecter {
            panicking: Cc::new(Panicking {
                trace_counter: Cell::new(2),
                panic_on_trace: Cell::new(true),
                panic_on_finalize: Cell::new(false),
                panic_on_drop: Cell::new(false),
                cc: RefCell::new(None),
            }),
            cyclic: RefCell::new(None),
        });
        *a.panicking.cc.borrow_mut() = Some(Cc::new(Panicking {
            trace_counter: Cell::new(2),
            panic_on_trace: Cell::new(true),
            panic_on_finalize: Cell::new(false),
            panic_on_drop: Cell::new(false),
            cc: RefCell::new(Some(a.panicking.clone())),
        }));
        *a.cyclic.borrow_mut() = Some(a.clone());

        register_panicking(&a);
        register_panicking(&a.panicking);
        register_panicking(a.panicking.cc.borrow().as_ref().unwrap());
        drop(a);
    }

    panicking_collect_cycles(assert_state_not_collecting);
}
