//! Code of an old benchmark, modified and kept for testing

use std::cell::RefCell;

use crate::tests::reset_state;
use crate::{Cc, Context, Trace, collect_cycles};

thread_local! {
   static FREED_LIST: RefCell<String> = RefCell::new(String::with_capacity(10));
}

#[test]
fn test() {
    reset_state();

    FREED_LIST.with(|str| str.borrow_mut().clear());
    one();
    FREED_LIST.with(|str| {
        let mut str = str.borrow_mut();
        let Some((first, second)) = str.split_once('-') else {
            panic!("String doesn't contains a -.");
        };
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 3);
        assert!(first.contains('C'));
        assert!(first.contains('D'));
        assert!(second.contains('A'));
        assert!(second.contains('B'));
        assert!(second.contains('E'));

        str.clear();
    });

    reset_state();

    #[cfg(feature = "finalization")]
    {
        two();
        FREED_LIST.with(|str| {
            let mut str = str.borrow_mut();
            let Some((first, second)) = str.split_once('-') else {
                panic!("String doesn't contains a -.");
            };
            assert_eq!(first.len(), 0);
            let Some((second, third)) = second.split_once('-') else {
                panic!("String doesn't contains a second -.");
            };
            assert_eq!(second.len(), 5);
            assert_eq!(third.len(), 1);
            assert!(second.contains('A'));
            assert!(second.contains('B'));
            assert!(second.contains('C'));
            assert!(second.contains('D'));
            assert!(second.contains('E'));
            assert!(third.contains('D'));
            str.clear();
        });
    }
}

fn one() {
    GLOBAL_CC.with(|global| {
        // Don't make <C as Finalize>::finalize move D into GLOBAL_CC!
        // That is tested in function two()
        let _unused = global.borrow_mut();

        {
            let root1 = build();
            collect_cycles();
            let _root2 = root1.clone();
            collect_cycles();
            *root1.c.borrow_mut() = None;
            collect_cycles();
        }
        FREED_LIST.with(|str| {
            str.borrow_mut().push('-');
        });
        collect_cycles();
    });
}

thread_local! {
    static GLOBAL_CC: RefCell<Option<Cc<D>>> = RefCell::new(None);
}

#[cfg(feature = "finalization")]
fn two() {
    {
        let root1 = build();
        collect_cycles();
        let _root2 = root1.clone();
        collect_cycles();
        *root1.b.borrow_mut() = None;
        collect_cycles();
    }
    FREED_LIST.with(|str| {
        str.borrow_mut().push('-');
        collect_cycles();
    });
    FREED_LIST.with(|str| {
        str.borrow_mut().push('-');
    });
    GLOBAL_CC.with(|global| {
        // Drop the Cc, if present
        // Note that this shouldn't require to call collect_cycles()
        if let Some(mut cc) = global.take() {
            assert!(cc.already_finalized());
            cc.finalize_again();
        }
    });
}

fn build() -> Cc<A> {
    let root1 = Cc::new(A {
        c: RefCell::new(Some(Cc::new(C {
            d: RefCell::new(Some(Cc::new(D { _value: 0xD }))),
            b: Cc::new(B {
                e: Cc::new(E { _value: 0xE }),
                b: RefCell::new(None),
                a: RefCell::new(None),
            }),
        }))),
        b: RefCell::new(None),
    });
    if let Some(ref c) = *root1.c.borrow_mut() {
        *root1.b.borrow_mut() = Some(c.b.clone());
        *c.b.b.borrow_mut() = Some(c.b.clone());
        *c.b.a.borrow_mut() = Some(root1.clone());
    }
    root1
}

struct A {
    c: RefCell<Option<Cc<C>>>,
    b: RefCell<Option<Cc<B>>>,
}
struct B {
    e: Cc<E>,
    b: RefCell<Option<Cc<B>>>,
    a: RefCell<Option<Cc<A>>>,
}
struct C {
    d: RefCell<Option<Cc<D>>>,
    b: Cc<B>,
}
struct D {
    _value: u64,
}
struct E {
    _value: i64,
}

unsafe impl Trace for A {
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Some(ref c) = *self.c.borrow() {
            c.trace(ctx);
        }
        if let Some(ref b) = *self.b.borrow() {
            b.trace(ctx);
        }
    }
}

unsafe impl Trace for B {
    fn trace(&self, ctx: &mut Context<'_>) {
        self.e.trace(ctx);
        if let Some(ref b) = *self.b.borrow() {
            b.trace(ctx);
        }
        if let Some(ref a) = *self.a.borrow() {
            a.trace(ctx);
        }
    }
}

unsafe impl Trace for C {
    fn trace(&self, ctx: &mut Context<'_>) {
        if let Some(d) = &*self.d.borrow() {
            d.trace(ctx);
        }
        self.b.trace(ctx);
    }
}

unsafe impl Trace for D {
    fn trace(&self, _: &mut Context<'_>) {}
}

unsafe impl Trace for E {
    fn trace(&self, _: &mut Context<'_>) {}
}

macro_rules! finalize_or_drop {
    (impl Finalize/Drop for $id:ident { fn finalize/drop(&$selfId:ident) $body:block }) => {
        #[cfg(feature = "finalization")]
        impl $crate::Finalize for $id {
            fn finalize(&$selfId) {
                $body
            }
        }

        #[cfg(not(feature = "finalization"))]
        impl $crate::Finalize for $id {
            fn finalize(&$selfId) {}
        }

        #[cfg(not(feature = "finalization"))]
        impl ::std::ops::Drop for $id {
            fn drop(&mut $selfId) {
                $body
            }
        }
    };
}

finalize_or_drop! {
    impl Finalize/Drop for A {
        fn finalize/drop(&self) {
            FREED_LIST.with(|str| {
                str.borrow_mut().push('A');
            });
        }
    }
}

finalize_or_drop! {
    impl Finalize/Drop for B {
        fn finalize/drop(&self) {
            FREED_LIST.with(|str| {
                str.borrow_mut().push('B');
            });
        }
    }
}

finalize_or_drop! {
    impl Finalize/Drop for C {
        fn finalize/drop(&self) {
            FREED_LIST.with(|str| {
                str.borrow_mut().push('C');
            });

            #[cfg(feature = "finalization")]
            GLOBAL_CC.with(|global| {
                if let Ok(mut global) = global.try_borrow_mut() {
                    *global = self.d.take();
                }
            });
        }
    }
}

finalize_or_drop! {
    impl Finalize/Drop for D {
       fn finalize/drop(&self) {
            FREED_LIST.with(|str| {
                str.borrow_mut().push('D');
            });
        }
    }
}

finalize_or_drop! {
    impl Finalize/Drop for E {
        fn finalize/drop(&self) {
            FREED_LIST.with(|str| {
                str.borrow_mut().push('E');
            });
        }
    }
}
