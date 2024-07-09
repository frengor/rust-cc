use rust_cc::*;

#[derive(Trace, Finalize)]
#[rust_cc(ignore)]
struct MyStruct {
}

#[derive(Trace, Finalize)]
#[rust_cc(ignore)]
enum MyEnum {
    A(),
}

fn main() {
}
