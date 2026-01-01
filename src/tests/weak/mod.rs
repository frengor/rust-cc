use std::panic::catch_unwind;

use super::*;
use crate::weak::Weak;
use crate::*;

#[test]
fn weak_test() {
    reset_state();

    let (cc, weak) = weak_test_common();
    drop(cc);
    assert_eq!(1, weak.weak_count());
    assert_eq!(0, weak.strong_count());
    assert!(weak.upgrade().is_none());
    assert_eq!(1, weak.weak_count());
    assert_eq!(0, weak.strong_count());
    let weak3 = weak.clone();
    assert!(Weak::ptr_eq(&weak, &weak3));
    assert_eq!(2, weak.weak_count());
    assert_eq!(0, weak.strong_count());
    drop(weak3);
    assert_eq!(1, weak.weak_count());
    assert_eq!(0, weak.strong_count());
    drop(weak);
}

#[test]
fn weak_test2() {
    reset_state();

    let (cc, weak) = weak_test_common();
    drop(weak);
    assert_eq!(1, cc.strong_count());
    assert_eq!(0, cc.weak_count());
    let weak2 = cc.downgrade();
    assert_eq!(1, cc.strong_count());
    assert_eq!(1, weak2.weak_count());
    assert_eq!(cc.strong_count(), weak2.strong_count());
    assert_eq!(weak2.weak_count(), cc.weak_count());
    drop(weak2);
    drop(cc);
    collect_cycles();
}

fn weak_test_common() -> (Cc<i32>, Weak<i32>) {
    reset_state();

    let cc: Cc<i32> = Cc::new(0i32);

    assert!(!cc.inner().counter_marker().has_allocated_for_metadata());
    assert_eq!(0, cc.weak_count());
    assert_eq!(1, cc.strong_count());

    let cc1 = cc.clone();

    assert!(!cc.inner().counter_marker().has_allocated_for_metadata());

    let weak = cc.downgrade();

    assert!(cc.inner().counter_marker().has_allocated_for_metadata());

    assert_eq!(2, cc.strong_count());
    assert_eq!(1, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    drop(cc1);
    assert_eq!(1, cc.strong_count());
    assert_eq!(1, weak.weak_count());
    collect_cycles();
    assert_eq!(1, cc.strong_count());
    assert_eq!(1, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    let weak2 = weak.clone();
    assert!(Weak::ptr_eq(&weak, &weak2));
    assert_eq!(1, cc.strong_count());
    assert_eq!(2, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    let upgraded = weak.upgrade().expect("Couldn't upgrade");
    assert!(Cc::ptr_eq(&cc, &upgraded));
    assert_eq!(2, cc.strong_count());
    assert_eq!(2, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    drop(weak2);
    assert_eq!(2, cc.strong_count());
    assert_eq!(1, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    drop(upgraded);
    assert_eq!(1, cc.strong_count());
    assert_eq!(1, weak.weak_count());
    assert_eq!(cc.strong_count(), weak.strong_count());
    assert_eq!(weak.weak_count(), cc.weak_count());
    (cc, weak)
}

#[cfg(feature = "nightly")]
#[test]
fn weak_dst() {
    reset_state();

    let cc = Cc::new(0i32);
    let cc1: Cc<dyn Trace> = cc.clone();
    let _weak: Weak<dyn Trace> = cc.downgrade();
    let _weak1: Weak<dyn Trace> = cc1.downgrade();
}

#[test]
fn test_new_cyclic() {
    reset_state();

    struct Cyclic {
        weak: Weak<Cyclic>,
        int: i32,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {}

    let cyclic = Cc::new_cyclic(|weak| {
        assert_eq!(1, weak.weak_count());
        assert_eq!(0, weak.strong_count());
        assert!(weak.upgrade().is_none());
        Cyclic {
            weak: weak.clone(),
            int: 5,
        }
    });

    assert!(cyclic.inner().counter_marker().has_allocated_for_metadata());

    assert_eq!(1, cyclic.weak_count());
    assert_eq!(1, cyclic.strong_count());

    assert_eq!(5, cyclic.int);
    assert!(Cc::ptr_eq(&cyclic.weak.upgrade().unwrap(), &cyclic));
}

#[test]
#[should_panic(expected = "Expected panic during panicking_new_cyclic1!")]
fn panicking_new_cyclic1() {
    reset_state();

    #[allow(clippy::unused_unit)] // Unit type needed to fix never type fallback error
    let _cc = Cc::new_cyclic(|_| -> () {
        panic!("Expected panic during panicking_new_cyclic1!");
    });
}

#[test]
#[should_panic(expected = "Expected panic during panicking_new_cyclic2!")]
fn panicking_new_cyclic2() {
    reset_state();

    #[allow(clippy::unused_unit)] // Unit type needed to fix never type fallback error
    let _cc = Cc::new_cyclic(|weak| -> () {
        let _weak = weak.clone();
        panic!("Expected panic during panicking_new_cyclic2!");
    });
}

#[test]
fn panicking_saving_new_cyclic() {
    reset_state();

    thread_local! {
        static SAVED: RefCell<Option<Weak<Cyclic>>> = RefCell::new(None);
    }

    struct Cyclic {
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {}

    assert!(
        catch_unwind(|| {
            let _cc = Cc::new_cyclic(|weak| {
                SAVED.with(|saved| {
                    *saved.borrow_mut() = Some(weak.clone());
                });
                panic!();
            });
        })
        .is_err()
    );

    SAVED.with(|saved| {
        let weak = &*saved.borrow();
        assert!(weak.is_some());
        let weak = weak.as_ref().unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(1, weak.weak_count());
        assert_eq!(0, weak.strong_count());
        let _weak = weak.clone();
        assert_eq!(2, weak.weak_count());
        assert_eq!(0, weak.strong_count());
    });
}

#[test]
fn try_upgrade_in_finalize_and_drop() {
    reset_state();

    thread_local! {
        static TRACED: Cell<bool> = Cell::new(false);
        static FINALIZED: Cell<bool> = Cell::new(false);
        static DROPPED: Cell<bool> = Cell::new(false);
    }

    struct Cyclic {
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            TRACED.with(|traced| traced.set(true));
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {
        fn finalize(&self) {
            FINALIZED.with(|finalized| finalized.set(true));
            assert!(self.weak.upgrade().is_some());
        }
    }

    impl Drop for Cyclic {
        fn drop(&mut self) {
            DROPPED.with(|dropped| dropped.set(true));
            assert!(self.weak.upgrade().is_none());
        }
    }

    let weak = {
        let cc = Cc::new_cyclic(|weak| Cyclic { weak: weak.clone() });
        assert_eq!(1, cc.strong_count());
        cc.downgrade()
        // cc is dropped and collected automatically
    };

    assert!(weak.upgrade().is_none());
    assert_eq!(0, weak.strong_count());
    assert_eq!(1, weak.weak_count());

    // Shouldn't have traced
    assert!(!TRACED.with(|traced| traced.get()));
    if cfg!(feature = "finalization") {
        assert!(FINALIZED.with(|finalized| finalized.get()));
    }
    assert!(DROPPED.with(|dropped| dropped.get()));
}

#[cfg(feature = "finalization")]
#[test]
fn try_upgrade_and_resurrect_in_finalize_and_drop() {
    reset_state();

    thread_local! {
        static RESURRECTED: Cell<Option<Cc<Cyclic>>> = Cell::new(None);
    }

    struct Cyclic {
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {
        fn finalize(&self) {
            RESURRECTED.with(|r| r.set(Some(self.weak.upgrade().unwrap())));
        }
    }

    impl Drop for Cyclic {
        fn drop(&mut self) {
            assert!(self.weak.upgrade().is_none());
        }
    }

    {
        let cc = Cc::new_cyclic(|weak| Cyclic { weak: weak.clone() });
        assert_eq!(1, cc.strong_count());
        // cc is dropped and collected automatically
    }

    RESURRECTED.with(|r| {
        let cc = r.replace(None).unwrap();
        assert_eq!(1, cc.weak_count());
        assert_eq!(1, cc.strong_count());
        // cc is dropped here, finally freeing the allocation
    });
}

#[test]
fn try_upgrade_in_cyclic_finalize_and_drop() {
    reset_state();

    thread_local! {
        static TRACED: Cell<bool> = Cell::new(false);
        static FINALIZED: Cell<bool> = Cell::new(false);
        static DROPPED: Cell<bool> = Cell::new(false);
    }

    struct Cyclic {
        cyclic: RefCell<Option<Cc<Cyclic>>>,
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            TRACED.with(|traced| traced.set(true));
            self.cyclic.trace(ctx);
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {
        fn finalize(&self) {
            FINALIZED.with(|finalized| finalized.set(true));
            assert!(self.weak.upgrade().is_some());
        }
    }

    impl Drop for Cyclic {
        fn drop(&mut self) {
            DROPPED.with(|dropped| dropped.set(true));
            assert!(self.weak.upgrade().is_none());
        }
    }

    let weak: Weak<Cyclic> = {
        let cc: Cc<Cyclic> = Cc::new_cyclic(|weak| Cyclic {
            cyclic: RefCell::new(None),
            weak: weak.clone(),
        });
        *cc.cyclic.borrow_mut() = Some(cc.clone());
        assert_eq!(2, cc.strong_count());
        cc.downgrade()
    };

    assert_eq!(1, weak.strong_count());
    assert_eq!(2, weak.weak_count());

    collect_cycles();

    assert!(weak.upgrade().is_none());
    assert_eq!(0, weak.strong_count());
    assert_eq!(1, weak.weak_count());

    assert!(TRACED.with(|traced| traced.get()));
    if cfg!(feature = "finalization") {
        assert!(FINALIZED.with(|finalized| finalized.get()));
    }
    assert!(DROPPED.with(|dropped| dropped.get()));
}

#[test]
fn weak_new() {
    reset_state();

    let new: Weak<u32> = Weak::new();

    assert_eq!(0, new.weak_count());
    assert_eq!(0, new.strong_count());
    assert!(new.upgrade().is_none());

    let other = new.clone();

    assert_eq!(0, other.weak_count());
    assert_eq!(0, other.strong_count());
    assert!(other.upgrade().is_none());
}

#[test]
fn weak_new_ptr_eq() {
    reset_state();

    let new: Weak<u32> = Weak::new();
    let other_new: Weak<u32> = Weak::new();

    let cc = Cc::new(5u32);
    let other_cc = Cc::new(6u32);

    assert_eq!(0, new.weak_count());
    assert_eq!(0, new.strong_count());

    assert!(Weak::ptr_eq(&new, &new));
    assert!(Weak::ptr_eq(&new, &other_new));
    assert!(Weak::ptr_eq(&new, &new.clone()));
    assert!(!Weak::ptr_eq(&new, &cc.downgrade()));
    assert!(Weak::ptr_eq(&cc.downgrade(), &cc.downgrade()));
    assert!(!Weak::ptr_eq(&cc.downgrade(), &other_cc.downgrade()));
}

#[test]
fn drop_zero_weak_counter() {
    reset_state();

    let cc = Cc::new(5u32);
    let _ = cc.downgrade();
    drop(cc);
}

#[test]
fn cyclic_drop_zero_weak_counter() {
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

    let cc = Cc::new(Cyclic {
        cyclic: RefCell::new(None),
    });
    *cc.cyclic.borrow_mut() = Some(cc.clone());

    let _ = cc.downgrade();
    drop(cc);
    collect_cycles();
}
