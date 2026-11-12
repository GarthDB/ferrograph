mod mac;
mod other;
mod types;

// One name renamed to two modules (`#[cfg]` variants) names neither: no certain edge to either.
use mac as backend;
use other as backend;

mod elsewhere {
    fn make() {} // neg_ext_make
}

mod same {
    pub(crate) struct E;
    pub(crate) struct F;
    impl E {
        fn new() -> E { E } // live_e_new
    }
    impl F {
        fn new() -> F { F } // live_f_new
    }
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

fn third() {
    use crate::same::E as Alias;
    let _ = Alias::new();
}

fn fourth() {
    use crate::same::F as Alias;
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
    third();
    fourth();
    external();
    backend::probe();
}
