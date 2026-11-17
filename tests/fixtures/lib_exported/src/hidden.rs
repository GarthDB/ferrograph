pub fn hidden_dead() {}

#[registered_elsewhere::export]
pub fn registered_fn() {}
pub fn ext_fn() {}
pub const HIDDEN_CONST: u8 = 1;
pub static HIDDEN_STATIC: u8 = 2;

pub struct Hidden;
impl Hidden {
    pub fn hidden_method() {}
}
impl std::fmt::Display for Hidden {
    fn fmt(&self, _: &mut std::fmt::Formatter) -> std::fmt::Result {
        Ok(())
    }
}

pub struct Leaked;
impl Leaked {
    pub fn leaked_method() {}
}

pub struct Inner;
impl Inner {
    pub fn inner_method() {}
}

pub struct Token;
impl Token {
    pub fn token_method() {}
}
