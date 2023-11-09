use rust_cc::*;

#[derive(Trace, Finalize)]
#[rust_cc()]
struct MyStruct {
    #[rust_cc()]
    a: (),
}

#[derive(Trace, Finalize)]
#[rust_cc()]
enum MyEnum {
    #[rust_cc()]
    A(#[rust_cc()] i32),
    #[rust_cc()]
    B {
        #[rust_cc()]
        b: i32,
    }
}

fn main() {
}
