pub fn open_fn() {}

mod inner {
    pub fn inner_dead() {}
}

pub mod deep {
    pub fn deep_fn() {}
}
pub const OPEN_CONST: u8 = 4;
