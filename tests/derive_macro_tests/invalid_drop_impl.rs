use rust_cc::*;

#[derive(Trace)]
struct MyStruct {
    a: (),
}

impl Finalize for MyStruct {
}

impl Drop for MyStruct {
    fn drop(&mut self) {
    }
}

#[derive(Trace)]
enum MyEnum {
    A(),
    B(),
}

impl Finalize for MyEnum {
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
