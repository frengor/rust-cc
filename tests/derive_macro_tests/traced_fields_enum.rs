use std::cell::{Cell, RefCell};
use rust_cc::*;

#[derive(Finalize)]
struct ToTrace {
    has_been_traced: Cell<bool>,
}

unsafe impl Trace for ToTrace {
    fn trace(&self, _: &mut Context<'_>) {
        self.has_been_traced.set(true);
    }
}

impl ToTrace {
    fn new() -> Cc<ToTrace> {
        Cc::new(ToTrace {
            has_been_traced: Cell::new(false),
        })
    }
}

#[derive(Trace, Finalize)] // Finalize is required by Trace
enum MyEnum {
    A {
        cyclic: RefCell<Option<Cc<MyEnum>>>,
        traced: Cc<ToTrace>,
        #[rust_cc(ignore)]
        ignored: Cc<ToTrace>,
    }
}

fn main() {
    let my_struct = Cc::new(MyEnum::A {
        cyclic: RefCell::new(None),
        traced: ToTrace::new(),
        ignored: ToTrace::new(),
    });

    let MyEnum::A {cyclic, traced, ignored} = &*my_struct;

    *cyclic.borrow_mut() = Some(my_struct.clone());

    // Drop an instance and collect
    let _ = my_struct.clone();
    collect_cycles();

    assert!(traced.has_been_traced.get());
    assert!(!ignored.has_been_traced.get());
}
