// Every definition that matters carries a `// tag` comment; the integration test maps tags to lines.
fn main() {}

// #98 reproduction (verbatim)
pub struct S;
impl S {
    #[cfg(unix)]
    fn f() -> u8 { 1 } // f_unix
    #[cfg(not(unix))]
    fn f() -> u8 { 2 } // f_other
    pub fn api() -> u8 { Self::f() }
}
pub struct A;
pub struct B;
impl A { fn make() -> u8 { 1 } } // a_make
impl B { fn make() -> u8 { 2 } } // b_make
pub fn both() -> u8 { A::make() + B::make() }

// Only one of two same-named assoc fns is called: the other must stay dead.
pub struct P;
pub struct Q;
impl P { fn mk() -> u8 { 1 } } // p_mk
impl Q { fn mk() -> u8 { 2 } } // q_mk
pub fn only_q() -> u8 { Q::mk() }

// `Self::g()` from a trait impl resolves to the inherent impl of the same type.
pub trait Tr { fn t() -> u8; }
pub struct X;
pub struct Y;
impl X { fn g() -> u8 { 1 } } // x_g
impl Y { fn g() -> u8 { 2 } } // y_g
impl Tr for X { fn t() -> u8 { Self::g() } }

// A trait-impl method named like an inherent fn calls `Self::pv()`: the inherent one is the target.
pub trait Pv { fn pv() -> u8; }
pub struct V;
impl V { fn pv() -> u8 { 1 } } // v_pv
impl Pv for V { fn pv() -> u8 { Self::pv() } }
