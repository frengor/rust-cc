use std::cell::RefCell;

use crate::tests::reset_state;
use crate::{collect_cycles, Cc, Context, Finalize, Trace};

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
        if let Some((first, second)) = str.split_once('-') {
            assert_eq!(first.len(), 2);
            assert_eq!(second.len(), 3);
            assert!(first.contains('C'));
            assert!(first.contains('D'));
            assert!(second.contains('A'));
            assert!(second.contains('B'));
            assert!(second.contains('E'));
        } else {
            panic!("String doesn't contains a -.");
        }
        str.clear();
    });
    two();
    FREED_LIST.with(|str| {
        let mut str = str.borrow_mut();
        if let Some((first, second)) = str.split_once('-') {
            assert_eq!(first.len(), 0);
            if let Some((second, third)) = second.split_once('-') {
                assert_eq!(second.len(), 5);
                assert_eq!(third.len(), 0);
                assert!(second.contains('A'));
                assert!(second.contains('B'));
                assert!(second.contains('C'));
                assert!(second.contains('D'));
                assert!(second.contains('E'));
                //assert!(third.contains('D'));
            } else {
                panic!("String doesn't contains a second -.");
            }
        } else {
            panic!("String doesn't contains a -.");
        }
        str.clear();
    });
}

fn one() {
    GLOBAL_CC.with(|global| {
        // Don't make <C as Drop>::drop move D into GLOBAL_CC!
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
        global.take()
    });
}

fn build() -> Cc<A> {
    let root1 = Cc::new(A {
        c: RefCell::new(Some(Cc::new(C {
            d: Some(Cc::new(D { _value: 0xD })),
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
    d: Option<Cc<D>>,
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
        if let Some(d) = &self.d {
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

impl Finalize for A {
    fn finalize(&mut self) {
        FREED_LIST.with(|str| {
            str.borrow_mut().push('A');
        });
    }
}

impl Finalize for B {
    fn finalize(&mut self) {
        FREED_LIST.with(|str| {
            str.borrow_mut().push('B');
        });
    }
}

impl Finalize for C {
    fn finalize(&mut self) {
        FREED_LIST.with(|str| {
            str.borrow_mut().push('C');
        });
        GLOBAL_CC.with(|global| {
            if let Ok(mut global) = global.try_borrow_mut() {
                *global = self.d.take();
            }
        });
    }
}

impl Finalize for D {
    fn finalize(&mut self) {
        FREED_LIST.with(|str| {
            str.borrow_mut().push('D');
        });
    }
}

impl Finalize for E {
    fn finalize(&mut self) {
        FREED_LIST.with(|str| {
            str.borrow_mut().push('E');
        });
    }
}
