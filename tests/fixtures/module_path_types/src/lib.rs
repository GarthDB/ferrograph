use std::io;

// Types named through a module path (`parent::A`, as a sibling module's `use` makes them): each
// `A` method is called through such a field, parameter or local, and the same-named `B` method never is.
struct A;
struct B;
struct Error;

impl A {
    fn field(&self) {} // live_a_field
    fn param(&self) {} // live_a_param
    fn local(&self) {} // live_a_local
    fn wrapped(&self) {} // live_a_wrapped
}

impl B {
    fn field(&self) {} // dead_b_field
    fn param(&self) {} // dead_b_param
    fn local(&self) {} // dead_b_local
    fn wrapped(&self) {} // dead_b_wrapped
    fn raw(&self) {} // dead_b_raw
    fn ext(&self) {} // dead_b_ext
}

// `io::Error` is the standard library's: it must not make this `raw` certain.
impl Error {
    fn raw(&self) {} // dead_error_raw
    fn ext(&self) {} // dead_error_ext
}

mod user {
    use super as parent;

    pub struct Holder {
        a: parent::A,
        maybe: Option<parent::A>,
    }

    impl Holder {
        pub fn field(&self) {
            self.a.field();
            if let Some(a) = &self.maybe {
                a.wrapped();
            }
        }
    }

    pub fn param(a: &parent::A) {
        a.param();
    }

    pub fn local(h: &Holder) {
        let a: &parent::A = &h.a;
        a.local();
    }
}

pub fn io_typed(e: io::Error) {
    e.raw();
}

pub fn run(h: &user::Holder) {
    h.field();
    user::local(h);
}

// External-crate and std-module paths (`anyhow::Error`, `error::Error`) are not the project's types.
pub fn external(e: &anyhow::Error, f: &error::Error) {
    e.ext();
    f.ext();
}
