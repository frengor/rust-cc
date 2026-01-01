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

    struct MyTraitObject(u8, RefCell<Option<Cc<Self>>>);

    unsafe impl Trace for MyTraitObject {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.0.trace(ctx);
            self.1.trace(ctx);
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
        let cc = Cc::new(MyTraitObject(5, RefCell::new(None)));
        *cc.1.borrow_mut() = Some(cc.clone());

        let cc: Cc<dyn TestTrait> = cc;

        assert_eq!(cc.strong_count(), 2);

        let cc_cloned = cc.clone();
        assert_eq!(cc_cloned.strong_count(), 3);
        assert_eq!(cc.strong_count(), 3);

        drop(cc_cloned);
        assert_eq!(cc.strong_count(), 2);

        let inner = cc.deref();
        inner.hello();

        drop(cc);
        collect_cycles();
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
        "MyTraitObject hasn't been finalized"
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
            assert!(
                self.cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .strong_count()
                    >= 0
            );
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
            assert!(
                self.cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .cc
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .strong_count()
                    >= 0
            );
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
fn no_cyclic_finalization_ends() {
    reset_state();

    struct ToFinalize;

    unsafe impl Trace for ToFinalize {
        fn trace(&self, _: &mut Context<'_>) {
            panic!("Trace shouldn't have been called on ToFinalize.");
        }
    }

    impl Finalize for ToFinalize {
        fn finalize(&self) {
            let _cc = Cc::new(ToFinalize);

            #[cfg(feature = "finalization")]
            assert!(_cc.already_finalized());
        }
    }

    let _ = Cc::new(ToFinalize);
}

#[test]
fn cyclic_finalization_ends() {
    reset_state();

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
            let _cc = Cyclic::new();

            #[cfg(feature = "finalization")]
            assert!(_cc.already_finalized());
        }
    }

    let _ = Cyclic::new();
    collect_cycles();
}

#[test]
fn buffered_objects_count_test() {
    reset_state();

    struct Cyclic {
        cyclic: RefCell<Option<Cc<Cyclic>>>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cyclic.trace(ctx);
        }
    }

    impl Finalize for Cyclic {}

    assert_eq!(0, state::buffered_objects_count().unwrap());

    let cc = {
        let cc = Cc::new(Cyclic {
            cyclic: RefCell::new(None),
        });
        *cc.cyclic.borrow_mut() = Some(cc.clone());
        cc.clone()
    };

    assert_eq!(1, state::buffered_objects_count().unwrap());

    cc.mark_alive();

    assert_eq!(0, state::buffered_objects_count().unwrap());

    drop(cc);
    collect_cycles();
}

#[test]
fn try_unwrap_test() {
    reset_state();

    let cc = Cc::new(5u32);

    #[cfg(feature = "weak-ptrs")]
    let weak = cc.downgrade();

    let unwrapped = cc.try_unwrap();
    assert_eq!(5, unwrapped.unwrap());

    #[cfg(feature = "weak-ptrs")]
    assert!(weak.upgrade().is_none());
}

#[test]
fn fail_try_unwrap_test() {
    reset_state();

    let cc = Cc::new(5u32);
    let copy = cc.clone();

    #[cfg(feature = "weak-ptrs")]
    let weak = cc.downgrade();

    assert!(cc.try_unwrap().is_err()); // cc dropped here

    #[cfg(feature = "weak-ptrs")]
    assert!(weak.upgrade().is_some());

    let unwrapped = copy.try_unwrap();
    assert_eq!(5, unwrapped.unwrap());

    #[cfg(feature = "weak-ptrs")]
    assert!(weak.upgrade().is_none());
}

#[cfg(feature = "finalization")]
#[test]
fn finalization_try_unwrap_test() {
    reset_state();

    struct Finalizable {
        other: RefCell<Option<Cc<u32>>>,
    }

    unsafe impl Trace for Finalizable {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.other.trace(ctx);
        }
    }

    impl Finalize for Finalizable {
        fn finalize(&self) {
            let res = self.other.take().unwrap().try_unwrap();
            match res {
                Err(cc) => assert_eq!(5, *cc),
                _ => panic!("try_unwrap returned an Ok(...) value during finalization."),
            }
        }
    }

    let _ = Cc::new(Finalizable {
        other: RefCell::new(Some(Cc::new(5u32))),
    });
}

#[cfg(feature = "finalization")]
#[test]
fn cyclic_finalization_try_unwrap_test() {
    reset_state();

    thread_local! {
        static FINALIZED: Cell<bool> = Cell::new(false);
    }

    struct Cyclic {
        cyclic: RefCell<Option<Cc<Self>>>,
        other: RefCell<Option<Cc<u32>>>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.cyclic.trace(ctx);
            self.other.trace(ctx);
        }
    }

    impl Finalize for Cyclic {
        fn finalize(&self) {
            FINALIZED.with(|fin| fin.set(true));
            state(|state| assert!(state.is_collecting()));

            let res = self.other.take().unwrap().try_unwrap();
            match res {
                Err(cc) => assert_eq!(5, *cc),
                _ => panic!("try_unwrap returned an Ok(...) value during collection."),
            }
        }
    }

    let cc = Cc::new(Cyclic {
        cyclic: RefCell::new(None),
        other: RefCell::new(Some(Cc::new(5u32))),
    });
    *cc.cyclic.borrow_mut() = Some(cc.clone());
    drop(cc);
    collect_cycles();

    FINALIZED.with(|fin| assert!(fin.get()));
}
