#![cfg(not(miri))] // This test leaks memory, so it can't be run by Miri
#![cfg(all(feature = "weak-ptrs", feature = "derive"))]
//! This module tests that `Weak::upgrade` returns None when called in destructors while collecting.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use rust_cc::weak::*;
use rust_cc::*;
use test_case::test_case;

struct Ignored<T: Trace + 'static> {
    weak: Weak<T>,
    should_panic: bool,
}

impl<T: Trace> Drop for Ignored<T> {
    fn drop(&mut self) {
        // This is safe to implement
        assert!(self.weak.upgrade().is_none());

        assert!(!self.should_panic, "Expected panic");
    }
}

#[test_case(false)]
#[test_case(true)]
fn acyclic_upgrade(should_panic: bool) {
    #[derive(Trace, Finalize)]
    struct Allocated {
        #[rust_cc(ignore)]
        _ignored: Ignored<Allocated>,
    }

    let cc = Cc::new_cyclic(|weak| Allocated {
        _ignored: Ignored {
            weak: weak.clone(),
            should_panic,
        },
    });

    let weak = cc.downgrade();

    let res = catch_unwind(AssertUnwindSafe(|| {
        drop(cc);
    }));

    assert_eq!(should_panic, res.is_err());

    assert!(weak.upgrade().is_none());
}

#[test_case(false)]
#[test_case(true)]
fn cyclic_upgrade(should_panic: bool) {
    #[derive(Trace, Finalize)]
    struct Allocated {
        #[rust_cc(ignore)]
        _ignored: Ignored<Allocated>,
        cyclic: RefCell<Option<Cc<Self>>>,
    }

    let cc1 = Cc::new_cyclic(|weak| Allocated {
        _ignored: Ignored {
            weak: weak.clone(),
            should_panic,
        },
        cyclic: RefCell::new(None),
    });
    let cc2 = Cc::new_cyclic(|weak| Allocated {
        _ignored: Ignored {
            weak: weak.clone(),
            should_panic,
        },
        cyclic: RefCell::new(None),
    });
    let cc3 = Cc::new_cyclic(|weak| Allocated {
        _ignored: Ignored {
            weak: weak.clone(),
            should_panic,
        },
        cyclic: RefCell::new(None),
    });

    *cc1.cyclic.borrow_mut() = Some(cc2.clone());
    *cc2.cyclic.borrow_mut() = Some(cc3.clone());
    *cc3.cyclic.borrow_mut() = Some(cc1.clone());

    let weak1 = cc1.downgrade();
    let weak2 = cc2.downgrade();
    let weak3 = cc3.downgrade();

    drop(cc1);
    drop(cc2);
    drop(cc3);

    let res = catch_unwind(AssertUnwindSafe(|| {
        collect_cycles();
    }));

    assert_eq!(should_panic, res.is_err());

    assert!(weak1.upgrade().is_none());
    assert!(weak2.upgrade().is_none());
    assert!(weak3.upgrade().is_none());
}
