use crate::*;
use crate::state::reset_state;
use crate::weak::{Weak, Weakable, WeakableCc};

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

fn weak_test_common() -> (WeakableCc<i32>, Weak<i32>) {
    reset_state();

    let cc: Cc<Weakable<i32>> = WeakableCc::new_weakable(0i32);
    let cc1 = cc.clone();
    let weak = cc.downgrade();
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

    let cc = Cc::new_weakable(0i32);
    let cc1: WeakableCc<dyn Trace> = cc.clone();
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

    assert_eq!(1, cyclic.weak_count());
    assert_eq!(1, cyclic.strong_count());

    assert_eq!(5, cyclic.int);
    assert!(Cc::ptr_eq(&cyclic.weak.upgrade().unwrap(), &cyclic));
}

#[test]
#[should_panic(expected = "Expected panic during new_cyclic!")]
fn panicking_new_cyclic1() {
    reset_state();

    struct Cyclic {
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {}

    let _cc = Cc::new_cyclic(|_| {
        panic!("Expected panic during new_cyclic!");
    });
}

#[test]
#[should_panic(expected = "Expected panic during new_cyclic!")]
fn panicking_new_cyclic2() {
    reset_state();

    struct Cyclic {
        weak: Weak<Cyclic>,
    }

    unsafe impl Trace for Cyclic {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.weak.trace(ctx);
        }
    }

    impl Finalize for Cyclic {}

    let _cc = Cc::new_cyclic(|weak| {
        let _weak = weak.clone();
        panic!("Expected panic during new_cyclic!");
    });
}
