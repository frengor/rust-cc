use std::mem::MaybeUninit;

use super::*;
use crate::*;

struct Circular {
    cc: Cell<Option<Cc<Droppable<Circular>>>>,
}

unsafe impl Trace for Circular {
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Some(cc) = unsafe { &*self.cc.as_ptr() } {
            cc.trace(ctx);
        }
    }
}

impl Finalize for Circular {}

#[test]
fn test_simple() {
    reset_state();

    let (droppable, checker) = Droppable::new(56);
    let cc = Cc::new(droppable);

    checker.assert_not_finalized();
    checker.assert_not_dropped();
    collect_cycles();
    checker.assert_not_finalized();
    checker.assert_not_dropped();
    assert_eq!(cc.strong_count(), 1);
    let cloned = cc.clone();
    assert_eq!(cc.strong_count(), 2);
    drop(cloned);
    assert_eq!(cc.strong_count(), 1);
    drop(cc);
    assert_empty();
    checker.assert_finalized();
    checker.assert_dropped();
    collect_cycles();
    checker.assert_finalized();
    checker.assert_dropped();
}

#[test]
fn test_less_simple() {
    reset_state();

    let (droppable1, checker1) = Droppable::new(Circular {
        cc: Cell::new(None),
    });
    let cc1 = Cc::new(droppable1);

    let (droppable2, checker2) = Droppable::new(Circular {
        cc: Cell::new(None),
    });
    let cc2 = Cc::new(droppable2);

    cc1.cc.set(Some(cc2.clone()));
    cc2.cc.set(Some(cc1.clone()));

    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    collect_cycles();
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    let cloned = cc1.clone();
    drop(cloned);
    collect_cycles();
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    drop(cc1);
    drop(cc2);
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    collect_cycles();
    checker1.assert_finalized();
    checker2.assert_finalized();
    checker1.assert_dropped();
    checker2.assert_dropped();
}

#[test]
fn test_cc() {
    reset_state();

    let (droppable1, checker1) = Droppable::new(Circular {
        cc: Cell::new(None),
    });
    let cc1 = Cc::new(droppable1);

    let (droppable2, checker2) = Droppable::new(Circular {
        cc: Cell::new(None),
    });
    let cc2 = Cc::new(droppable2);

    cc1.cc.set(Some(cc2.clone()));
    cc2.cc.set(Some(cc1.clone()));

    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    collect_cycles();
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    let cloned = cc1.clone();
    drop(cloned);
    collect_cycles();
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    drop(cc1);
    drop(cc2);
    checker1.assert_not_finalized();
    checker2.assert_not_finalized();
    checker1.assert_not_dropped();
    checker2.assert_not_dropped();
    collect_cycles();
    checker1.assert_finalized();
    checker2.assert_finalized();
    checker1.assert_dropped();
    checker2.assert_dropped();
}

#[cfg(feature = "nightly")]
#[test]
fn test_trait_object() {
    reset_state();

    thread_local! {
        static CALLED: Cell<bool> = Cell::new(false);
        static FINALIZED: Cell<bool> = Cell::new(false);
        static DROPPED: Cell<bool> = Cell::new(false);
        static TRACED: Cell<bool> = Cell::new(false);
    }

    struct MyTraitObject(u8);

    unsafe impl Trace for MyTraitObject {
        fn trace(&self, _: &mut Context<'_>) {
            TRACED.with(|traced| traced.set(true));
        }
    }

    impl Finalize for MyTraitObject {
        fn finalize(&self) {
            FINALIZED.with(|finalized| finalized.set(true));
        }
    }

    impl Drop for MyTraitObject {
        fn drop(&mut self) {
            DROPPED.with(|dropped| dropped.set(true));
        }
    }

    trait TestTrait: Trace {
        fn hello(&self);
    }

    impl TestTrait for MyTraitObject {
        fn hello(&self) {
            CALLED.with(|called| called.set(true));
        }
    }

    {
        let cc = Cc::new(MyTraitObject(5)) as Cc<dyn TestTrait>;

        assert_eq!(cc.strong_count(), 1);

        let cc_cloned = cc.clone();
        assert_eq!(cc_cloned.strong_count(), 2);
        assert_eq!(cc.strong_count(), 2);

        drop(cc_cloned);
        assert_eq!(cc.strong_count(), 1);

        let inner = cc.deref();
        inner.hello();

        let mut l1 = List::new();
        let mut l2 = List::new();

        cc.trace(&mut Context::new(ContextInner::Counting {
            root_list: &mut l1,
            non_root_list: &mut l2,
        }));
    }

    assert!(
        CALLED.with(|called| called.get()),
        "<MyTraitObject as TestTrait>::hello hasn't been called"
    );
    assert!(
        DROPPED.with(|dropped| dropped.get()),
        "MyTraitObject hasn't been dropped"
    );
    assert!(
        TRACED.with(|dropped| dropped.get()),
        "MyTraitObject hasn't been traced"
    );

    #[cfg(feature = "finalization")]
    assert!(
        FINALIZED.with(|dropped| dropped.get()),
        "MyTraitObject hasn't been traced"
    );
}

/*#[test]
fn test_cyclic() {
    reset_state();

    struct Circular {
        cc: Cc<Circular>,
    }

    unsafe impl Trace for Circular {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cc.trace(ctx);
        }
    }

    impl Finalize for Circular {}

    let cc = Cc::<Circular>::new_cyclic(|cc| {
        assert!(!cc.is_valid());

        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _ = cc.deref();
            }))
            .is_err(),
            "Didn't panicked on deref."
        );

        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                cc.trace(&mut Context::new(ContextInner::Counting {
                    root_list: &mut List::new(),
                    non_root_list: &mut List::new(),
                }));
            }))
            .is_err(),
            "Didn't panicked on trace."
        );

        assert_eq!(cc.strong_count(), 1);

        drop(cc.clone());
        collect_cycles();

        Circular { cc: cc.clone() }
    });

    assert!(cc.is_valid());
    assert_eq!(cc.strong_count(), 2);

    assert!(Cc::ptr_eq(&cc, &cc.cc));

    {
        drop(cc.clone());
        // Test that we don't stuck in a loop while tracing
        collect_cycles();
    }

    drop(cc);
    collect_cycles();
}*/

#[test]
fn test_cyclic_finalization_aliasing() {
    reset_state();

    struct Circular {
        cc: RefCell<Option<Cc<Circular>>>,
    }

    unsafe impl Trace for Circular {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cc.trace(ctx);
        }
    }

    impl Finalize for Circular {
        // See comment below
        #[allow(clippy::absurd_extreme_comparisons)]
        #[allow(unused_comparisons)]
        fn finalize(&self) {
            // The scope of this comparison is to recursively access the same allocation during finalization
            assert!(self.cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().strong_count() >= 0);
        }
    }

    {
        let cc = Cc::new(Circular {
            cc: RefCell::new(None),
        });

        *cc.cc.borrow_mut() = Some(Cc::new(Circular {
            cc: RefCell::new(Some(cc.clone())),
        }));
    }

    collect_cycles();
}

#[test]
fn test_self_loop_finalization_aliasing() {
    reset_state();

    struct Circular {
        cc: RefCell<Option<Cc<Circular>>>,
    }

    unsafe impl Trace for Circular {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cc.trace(ctx);
        }
    }

    impl Finalize for Circular {
        // See comment below
        #[allow(clippy::absurd_extreme_comparisons)]
        #[allow(unused_comparisons)]
        fn finalize(&self) {
            // The scope of this comparison is to recursively access the same allocation during finalization
            assert!(self.cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().cc.borrow().as_ref().unwrap().strong_count() >= 0);
        }
    }

    {
        let cc = Cc::new(Circular {
            cc: RefCell::new(None),
        });
        *cc.cc.borrow_mut() = Some(cc.clone());
    }

    collect_cycles();
}

#[test]
fn test_assume_init() {
    reset_state();

    let cc = Cc::new(MaybeUninit::<u32>::new(42));
    // SAFETY: cc is already initialized
    let cc = unsafe { cc.assume_init() };
    assert_eq!(*cc, 42);
}

#[test]
fn test_init() {
    reset_state();

    let cc = Cc::new(MaybeUninit::<u32>::uninit());
    let cc = cc.init(42);
    assert_eq!(*cc, 42);
}

#[test]
fn no_cyclic_finalization_ends() {
    struct ToFinalize;

    unsafe impl Trace for ToFinalize {
        fn trace(&self, _: &mut Context<'_>) {
            panic!("Trace shouldn't have been called on ToFinalize.");
        }
    }

    impl Finalize for ToFinalize {
        fn finalize(&self) {
            let _ = Cc::new(ToFinalize);
        }
    }

    let _ = Cc::new(ToFinalize);
}

#[test]
fn cyclic_finalization_ends() {
    struct Cyclic {
        cyclic: RefCell<Option<Cc<Cyclic>>>,
    }

    impl Cyclic {
        fn new() -> Cc<Cyclic> {
            let cc = Cc::new(Cyclic {
                cyclic: RefCell::new(None),
            });
            *cc.cyclic.borrow_mut() = Some(cc.clone());
            cc
        }
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cyclic.trace(ctx);
        }
    }

    impl Finalize for Cyclic {
        fn finalize(&self) {
            let _ = Cyclic::new();
        }
    }

    let _ = Cyclic::new();
    collect_cycles();
}
