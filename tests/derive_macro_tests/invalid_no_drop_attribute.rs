use rust_cc::*;

#[derive(Trace, Finalize)]
struct MyStruct {
    #[rust_cc(unsafe_no_drop)]
    a: (),
}

#[derive(Trace, Finalize)]
enum MyEnum1 {
    #[rust_cc(unsafe_no_drop)]
    A(),
}

#[derive(Trace, Finalize)]
enum MyEnum2 {
    A {
        #[rust_cc(unsafe_no_drop)]
        a: (),
    },
}

fn main() {
}
