#![feature(bench_black_box)]

use std::cell::RefCell;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use rust_cc::*;

fn benchmark(c: &mut Criterion) {
    c.bench_function("finalized", |b| {
        b.iter(|| finalized(black_box(1), black_box(1)))
    });
    c.bench_function("not_finalized", |b| {
        b.iter(|| not_finalized(black_box(2), black_box(2)))
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = benchmark
}
criterion_main!(benches);

macro_rules! define_test {
    (fn $build_fn:ident, $A:ident, $B:ident, $C:ident, $D:ident, $E:ident) => {
        #[inline(always)]
        fn $build_fn(d_: u64, e_: i64) -> Cc<$A> {
            let root1 = Cc::new($A {
                c: RefCell::new(Some(Cc::new($C {
                    d: Cc::new($D { _value: d_ }),
                    b: Cc::new($B {
                        e: Cc::new($E { _value: e_ }),
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

        struct $A {
            c: RefCell<Option<Cc<$C>>>,
            b: RefCell<Option<Cc<$B>>>,
        }
        struct $B {
            e: Cc<$E>,
            b: RefCell<Option<Cc<$B>>>,
            a: RefCell<Option<Cc<$A>>>,
        }
        struct $C {
            d: Cc<$D>,
            b: Cc<$B>,
        }
        struct $D {
            _value: u64,
        }
        struct $E {
            _value: i64,
        }

        unsafe impl Trace for $A {
            fn trace(&self, ctx: &mut Context<'_>) {
                if let Some(ref c) = *self.c.borrow() {
                    c.trace(ctx);
                }
                if let Some(ref b) = *self.b.borrow() {
                    b.trace(ctx);
                }
            }
        }

        unsafe impl Trace for $B {
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

        unsafe impl Trace for $C {
            fn trace(&self, ctx: &mut Context<'_>) {
                self.d.trace(ctx);
                self.b.trace(ctx);
            }
        }

        unsafe impl Trace for $D {
            fn trace(&self, _: &mut Context<'_>) {}
        }

        unsafe impl Trace for $E {
            fn trace(&self, _: &mut Context<'_>) {}
        }
    };
    (with_finalizers fn $build_fn:ident, $A:ident, $B:ident, $C:ident, $D:ident, $E:ident) => {
        define_test!(fn $build_fn, $A, $B, $C, $D, $E);

        impl Finalize for $A {
            fn finalize(&mut self) {}
        }

        impl Finalize for $B {
            fn finalize(&mut self) {}
        }

        impl Finalize for $C {
            fn finalize(&mut self) {}
        }

        impl Finalize for $D {
            fn finalize(&mut self) {}
        }

        impl Finalize for $E {
            fn finalize(&mut self) {}
        }
    };
}

#[inline(never)]
fn finalized(d_: u64, e_: i64) {
    define_test!(with_finalizers fn build, A, B, C, D, E);

    {
        let root1 = build(d_, e_);
        collect_cycles();
        let _root2 = root1.clone();
        collect_cycles();
        *root1.c.borrow_mut() = None;
        collect_cycles();
    }
    collect_cycles();
}

#[inline(never)]
fn not_finalized(d_: u64, e_: i64) {
    define_test!(fn build, A, B, C, D, E);

    {
        let root1 = build(d_, e_);
        collect_cycles();
        let _root2 = root1.clone();
        collect_cycles();
        *root1.b.borrow_mut() = None;
        collect_cycles();
    }
    collect_cycles();
}
