pub fn open_fn() {}

mod inner {
    pub fn inner_dead() {}
}

pub mod deep {
    pub fn deep_fn() {}
}
pub const OPEN_CONST: u8 = 4;

pub fn leak() -> crate::hidden::Leaked {
    crate::hidden::Leaked
}

pub struct Open;
impl Open {
    pub fn open_method() {}
}

pub struct Wrapper;
impl std::ops::Deref for Wrapper {
    type Target = crate::hidden::Inner;
    fn deref(&self) -> &Self::Target {
        &crate::hidden::Inner
    }
}

pub static TOKEN: crate::hidden::Token = crate::hidden::Token;
