use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use rust_cc::*;

#[test]
fn test_complex() {
    struct A {
        b: Cc<B>,
    }

    struct B {
        c: Cc<C>,
    }

    struct C {
        a: RefCell<Option<Cc<A>>>,
        b: Cc<B>,
    }

    struct D {
        c: Cc<C>,
    }

    unsafe impl Trace for A {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.b.trace(ctx);
        }
    }

    impl Finalize for A {
    }

    unsafe impl Trace for B {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.c.trace(ctx);
        }
    }

    impl Finalize for B {
    }

    unsafe impl Trace for D {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.c.trace(ctx);
        }
    }

    impl Finalize for D {
    }

    unsafe impl Trace for C {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.b.trace(ctx);
            if let Some(cc) = &*self.a.borrow() {
                cc.trace(ctx);
            }
        }
    }

    impl Finalize for C {
    }

    let a = Cc::<A>::new_cyclic(|a| A {
        b: Cc::<B>::new_cyclic(|b| B {
            c: Cc::new(C {
                a: RefCell::new(Some(a.clone())),
                b: b.clone(),
            }),
        }),
    });
    let d = Cc::new(D { c: a.b.c.clone() });
    drop(a);
    collect_cycles();
    if let Some(a) = &*d.c.a.borrow() {
        let _count = a.strong_count();
    }
    drop(d);
    collect_cycles();
}

#[test]
fn useless_cyclic() {
    let _cc = Cc::<u32>::new_cyclic(|_| {
        collect_cycles();
        {
            let _ = Cc::new(42);
        }
        collect_cycles();
        10
    });
    collect_cycles();
    drop(_cc);
    collect_cycles();
}

#[test]
fn test_finalization() {
    thread_local! {
        static FINALIZED: Cell<bool> = Cell::new(false);
        static DROPPED: Cell<bool> = Cell::new(false);
        static FINALIZEDB: Cell<bool> = Cell::new(false);
        static DROPPEDB: Cell<bool> = Cell::new(false);
    }

    fn assert_not_dropped() {
        assert!(!DROPPED.with(|cell| cell.get()));
        assert!(!DROPPEDB.with(|cell| cell.get()));
    }

    // C doesn't impl Trace, so it cannot be put inside a Cc
    struct C {
        a: RefCell<Option<A>>,
    }

    struct A {
        dropped: Cell<bool>,
        c: Weak<C>,
        b: RefCell<Option<Cc<B>>>,
    }

    struct B {
        dropped: Cell<bool>,
        a: Cc<A>,
    }

    unsafe impl Trace for A {
        fn trace(&self, ctx: &mut Context<'_>) {
            if let Some(b) = &*self.b.borrow() {
                b.trace(ctx);
            }
        }
    }

    unsafe impl Trace for B {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.a.trace(ctx);
        }
    }

    impl Finalize for B {
        fn finalize(&mut self) {
            FINALIZEDB.with(|cell| cell.set(true));
        }
    }

    impl Finalize for A {
        fn finalize(&mut self) {
            FINALIZED.with(|cell| cell.set(true));
            if let Some(c) = self.c.upgrade() {
                *c.a.borrow_mut() = Some(A {
                    dropped: Cell::new(false),
                    c: self.c.clone(),
                    b: RefCell::new(self.b.borrow_mut().take()),
                });
            }
        }
    }

    impl Drop for A {
        fn drop(&mut self) {
            assert!(!self.dropped.get());
            self.dropped.set(true);
            DROPPED.with(|cell| cell.set(true));
        }
    }

    impl Drop for B {
        fn drop(&mut self) {
            assert!(!self.dropped.get());
            self.dropped.set(true);
            DROPPEDB.with(|cell| cell.set(true));
        }
    }

    {
        let c1 = Rc::new(C {
            a: RefCell::new(None),
        });

        let a = Cc::<A>::new_cyclic(|a| A {
            dropped: Cell::new(false),
            c: Rc::downgrade(&c1),
            b: RefCell::new(Some(Cc::new(B {
                dropped: Cell::new(false),
                a: a.clone(),
            }))),
        });

        assert_eq!(a.strong_count(), 2);

        assert!(!FINALIZED.with(|cell| cell.get()));
        assert!(!FINALIZEDB.with(|cell| cell.get()));
        assert_not_dropped();
        collect_cycles();
        assert!(!FINALIZED.with(|cell| cell.get()));
        assert!(!FINALIZEDB.with(|cell| cell.get()));
        assert_not_dropped();

        //let _c = a.c.clone();

        assert!(!FINALIZED.with(|cell| cell.get()));
        assert!(!FINALIZEDB.with(|cell| cell.get()));
        assert_not_dropped();
        drop(a);
        assert!(!FINALIZED.with(|cell| cell.get()));
        assert!(!FINALIZEDB.with(|cell| cell.get()));
        assert_not_dropped();
        collect_cycles();
        // a dropped here
        assert!(FINALIZED.with(|cell| cell.get()));
        assert!(FINALIZEDB.with(|cell| cell.get()));
        assert_not_dropped();

        // Reset DROPPED
        FINALIZED.with(|cell| cell.set(false));
        FINALIZEDB.with(|cell| cell.set(false));

        match &*c1.a.borrow() {
            Some(a) => {
                assert!(!a.dropped.get());
                a.b.borrow().iter().for_each(|b| {
                    assert!(!b.dropped.get());
                    let _ = b.strong_count();
                    let _ = b.a.strong_count();
                    assert!(Weak::ptr_eq(&b.a.c, &a.c));
                    b.a.b.borrow().iter().for_each(|b| {
                        assert!(!b.dropped.get());
                        let _ = b.strong_count();
                    });
                });
            },
            None => panic!("None"),
        };

        assert_not_dropped();
    }
    collect_cycles();

    assert!(FINALIZED.with(|cell| cell.get()));
    assert!(FINALIZEDB.with(|cell| cell.get()));
    assert!(DROPPED.with(|cell| cell.get()));
    assert!(DROPPEDB.with(|cell| cell.get()));
}

#[test]
fn test_finalize_drop() {
    thread_local! {
        static FINALIZED: Cell<bool> = Cell::new(false);
        static DROPPED: Cell<bool> = Cell::new(false);
        static FINALIZEDB: Cell<bool> = Cell::new(false);
        static DROPPEDB: Cell<bool> = Cell::new(false);
    }

    fn assert_not_dropped() {
        assert!(!DROPPED.with(|cell| cell.get()));
        assert!(!DROPPEDB.with(|cell| cell.get()));
    }

    // C doesn't impl Trace, so it cannot be put inside a Cc
    struct C {
        _a: RefCell<Option<A>>,
    }

    struct A {
        dropped: Cell<bool>,
        _c: Weak<C>,
        b: RefCell<Option<Cc<B>>>,
    }

    struct B {
        dropped: Cell<bool>,
        a: Cc<A>,
    }

    unsafe impl Trace for A {
        fn trace(&self, ctx: &mut Context<'_>) {
            if let Some(b) = &*self.b.borrow() {
                b.trace(ctx);
            }
        }
    }

    unsafe impl Trace for B {
        fn trace(&self, ctx: &mut Context<'_>) {
            self.a.trace(ctx);
        }
    }

    impl Finalize for A {
        fn finalize(&mut self) {
            FINALIZED.with(|cell| cell.set(true));
        }
    }

    impl Finalize for B {
        fn finalize(&mut self) {
            FINALIZEDB.with(|cell| cell.set(true));
        }
    }

    impl Drop for A {
        fn drop(&mut self) {
            assert!(!self.dropped.get());
            self.dropped.set(true);
            DROPPED.with(|cell| cell.set(true));
        }
    }

    impl Drop for B {
        fn drop(&mut self) {
            assert!(!self.dropped.get());
            self.dropped.set(true);
            DROPPEDB.with(|cell| cell.set(true));
        }
    }

    let cc = Rc::new_cyclic(|weak| C {
        _a: RefCell::new(Some(A {
            dropped: Cell::new(false),
            _c: weak.clone(),
            b: RefCell::new(Some(Cc::new_cyclic(|_| B {
                dropped: Cell::new(false),
                a: Cc::new(A {
                    dropped: Cell::new(false),
                    _c: weak.clone(),
                    b: RefCell::new(None),
                }),
            }))),
        })),
    });

    assert!(!FINALIZED.with(|cell| cell.get()));
    assert!(!FINALIZEDB.with(|cell| cell.get()));
    assert_not_dropped();

    drop(cc);

    collect_cycles();
    assert!(FINALIZED.with(|cell| cell.get()));
    assert!(FINALIZEDB.with(|cell| cell.get()));
    assert!(DROPPED.with(|cell| cell.get()));
    assert!(DROPPEDB.with(|cell| cell.get()));
}
