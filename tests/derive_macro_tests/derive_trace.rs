use rust_cc::*;

#[derive(Trace)]
struct MyStruct {
    a: (),
}

impl Finalize for MyStruct {
}

#[derive(Trace)]
enum MyEnum {
    A(),
    B(),
}

impl Finalize for MyEnum {
}

fn main() {
    fn test<T: Trace>(_t: T) {
    }

    test(MyStruct {
        a: (),
    });
    test(MyEnum::A());
}
