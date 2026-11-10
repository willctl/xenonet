//! What must not compile (tests/ui): Information<T> can't become a request,
//! an authorization or a token, and an Authorized can't be made or changed
//! outside the crate.

#[test]
fn what_must_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
