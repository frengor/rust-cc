use rust_cc::*;

#[derive(Trace, Finalize)]
#[rust_cc]
#[rust_cc = ""]
struct MyStruct1 {
}

#[derive(Trace, Finalize)]
struct MyStruct2 {
    #[rust_cc]
    #[rust_cc = ""]
    a: (),
}

#[derive(Trace, Finalize)]
#[rust_cc]
#[rust_cc = ""]
enum MyEnum1 {
}

#[derive(Trace, Finalize)]
enum MyEnum3 {
    #[rust_cc]
    #[rust_cc = ""]
    A(#[rust_cc] #[rust_cc = ""] i32),
    #[rust_cc]
    #[rust_cc = ""]
    B {
        #[rust_cc]
        #[rust_cc = ""]
        b: i32,
    }
}

fn main() {
}
