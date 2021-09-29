// #[test]
fn derive_trybuild() {
    let t = trybuild::TestCases::new();
    t.pass("tests/derive_compile/*.rs");
    t.compile_fail("tests/derive_compile_fail/*.rs");
}
