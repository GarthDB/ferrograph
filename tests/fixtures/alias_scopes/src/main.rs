mod types {
    pub(crate) struct A;
    pub(crate) struct B;
    impl A {
        fn new() -> A { A } // live_a_new
    }
    impl B {
        fn new() -> B { B } // live_b_new
        fn only_b() {} // dead_only_b
    }
    pub(crate) struct D;
    impl D {
        fn new() -> D { D } // dead_d_new
    }
    pub(crate) struct C;
    impl C {
        fn make() -> C { C } // dead_c_make
    }
}

mod elsewhere {
    fn make() {} // neg_ext_make
}

// Two functions import different types as the same name; each call goes to its own import.
fn first() {
    use crate::types::A as Alias;
    let _ = Alias::new();
}

fn second() {
    use crate::types::B as Alias;
    let _ = Alias::new();
}

// `Alias` here is an external crate item, not a project type: `make` must not bind to any project fn.
fn external() {
    use std::collections::HashMap as Alias;
    let _ = Alias::make();
}

fn main() {
    first();
    second();
    external();
}
