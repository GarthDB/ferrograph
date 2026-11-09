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
fn unused_private() {}
