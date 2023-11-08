use rust_cc::*;

#[derive(Finalize)]
struct MyStruct {
    a: (),
}

#[derive(Finalize)] // Finalize is required by Trace
enum MyEnum {
    A(),
    B(),
}

fn main() {
    fn test<T: Finalize>(_t: T) {
    }

    test(MyStruct {
        a: (),
    });

    test(MyEnum::A());
}
