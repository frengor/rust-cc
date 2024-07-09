use rust_cc::*;

struct DoesNotImplementTrace;

#[derive(Trace, Finalize)]
struct MyStruct1 {
    field: DoesNotImplementTrace,
}

#[derive(Trace, Finalize)]
enum MyEnum3 {
    A(DoesNotImplementTrace),
}

fn main() {
}
