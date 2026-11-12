pub(crate) struct A;
pub(crate) struct B;
pub(crate) struct D;
pub(crate) struct C;

impl A {
    fn new() -> A { A } // live_a_new
    fn twice() {} // neg_a_twice
}
impl B {
    fn new() -> B { B } // live_b_new
    fn only_b() {} // dead_only_b
    fn probe2() {} // neg_b_probe2
}
impl D {
    fn new() -> D { D } // dead_d_new
}
impl C {
    fn make() -> C { C } // dead_c_make
}
