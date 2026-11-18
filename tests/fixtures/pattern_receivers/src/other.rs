// Same variant name as `E::A` in lib.rs, with another payload: a bare `A(a)` in lib.rs is `E::A`.
pub enum F {
    A(crate::B),
}

pub struct Pod<N>(N);
pub type AnyPod = Pod<u8>;
impl AnyPod {
    pub fn generic_alias(&self) {}
}
