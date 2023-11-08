use rust_cc::*;

#[derive(Trace, Finalize)] // Finalize is required by Trace
struct MyStruct {
    a: (),
}

#[derive(Trace, Finalize)] // Finalize is required by Trace
enum MyEnum {
    A(),
    B(),
}

fn main() {
    fn test<T: Trace>(_t: T) {
    }

    test(MyStruct {
        a: (),
    });
    test(MyEnum::A());
}
