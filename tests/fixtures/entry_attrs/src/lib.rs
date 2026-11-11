pub fn api() {}
#[unsafe(no_mangle)]
extern "C" fn exported_to_c() {}
#[export_name = "renamed"]
extern "C" fn exported_renamed() {}
#[cfg(test)]
mod tests {
    #[test] // trailing comment
    fn a() {}
    #[test]
    // line comment
    fn b() {}
    /// doc
    #[test]
    /// doc between
    fn c() {}
}
#[no_mangle]
extern "C" fn plain_no_mangle() {}
#[cfg(test)]
mod more {
    #[test]
    /* block */
    fn d() {}
}
#[cfg(test)]
mod frameworks {
    use test_case::test_case;

    #[test_case(1 ; "one")]
    #[test_case(2 ; "two")]
    fn parametrized(_n: u32) {}

    #[rstest::rstest]
    fn with_rstest() {}

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn wasm() {}
}
fn unused_private() {}
