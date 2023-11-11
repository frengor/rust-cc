#![cfg(feature = "derive")]

#[cfg(not(miri))]
#[test]
fn macro_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/derive_macro_tests/derive_finalize.rs");
    t.pass("tests/derive_macro_tests/derive_trace.rs");
    t.pass("tests/derive_macro_tests/traced_fields_struct.rs");
    t.pass("tests/derive_macro_tests/traced_fields_enum.rs");
    t.pass("tests/derive_macro_tests/ignored_variant.rs");
    t.pass("tests/derive_macro_tests/no_drop.rs");
    t.pass("tests/derive_macro_tests/empty_attribute.rs");
    t.compile_fail("tests/derive_macro_tests/invalid_attributes.rs");
    t.compile_fail("tests/derive_macro_tests/invalid_ignore_attribute.rs");
    t.compile_fail("tests/derive_macro_tests/invalid_no_drop_attribute.rs");
    t.compile_fail("tests/derive_macro_tests/invalid_drop_impl.rs");
}
