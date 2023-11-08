use rust_cc::*;

#[derive(Trace, Finalize)] // Finalize is required by Trace
#[rust_cc(unsafe_no_drop)]
struct MyStruct {
    a: (),
}

impl Drop for MyStruct {
    fn drop(&mut self) {
    }
}

#[derive(Trace, Finalize)] // Finalize is required by Trace
#[rust_cc(unsafe_no_drop)]
enum MyEnum {
    A(),
    B(),
}

impl Drop for MyEnum {
    fn drop(&mut self) {
    }
}

fn main() {
    fn test<T: Trace>(_t: T) {
    }

    test(MyStruct {
        a: (),
    });
    test(MyEnum::A());
}
