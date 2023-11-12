#![cfg(feature = "weak-ptr")]

use rust_cc::*;
use rust_cc::weak::{Weak, Weakable, WeakableCc};

#[test]
fn weak_test() {
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
    let cc = Cc::new_weakable(0i32);
    let cc1: WeakableCc<dyn Trace> = cc.clone();
    let _weak: Weak<dyn Trace> = cc.downgrade();
    let _weak1: Weak<dyn Trace> = cc1.downgrade();
}
