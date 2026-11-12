// Same variant name as `E::A` in lib.rs, with another payload: a bare `A(a)` in lib.rs is `E::A`.
pub enum F {
    A(crate::B),
}
