pub fn exported_fn() {}
pub fn reexp_sibling_dead() {}
pub const EXPORTED_CONST: u8 = 3;
pub struct Reexported;
impl Reexported {
    pub fn reexp_method() {}
}
