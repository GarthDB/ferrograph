mod other;
use std::env::args;

// Every method name exists on both `A` and `B`, so a name-only lookup cannot pick one.
// `dead_b_*`: `B`'s version is never called, because the receiver is an `A`.
pub struct A {
    v: u32,
}
pub struct B;
pub struct Holder {
    a: A,
    count: u32,
}

impl A {
    fn via_field(&self) -> u32 { 1 }
    fn via_param(&self) -> u32 { 1 }
    fn via_let(&self) -> u32 { 1 }
    fn via_new(&self) -> u32 { 1 }
    fn via_literal(&self) -> u32 { 1 }
    fn via_macro(&self) -> u32 { 1 }
    fn via_factory(&self) -> u32 { 1 }
    fn via_chain(&self) -> u32 { 1 }
    fn via_mut(&self) -> u32 { 1 }
    fn me(&self) -> &Self { self }
    fn push(&mut self, _n: u32) {} // library_push_a
    fn via_args(&self) {} // args_a
    fn unknown(&self) -> u32 { 1 } // unknown_a
    fn shadowed(&self) -> u32 { 1 } // shadowed_a
    pub fn new() -> A { A { v: 0 } }
}

impl B {
    fn via_field(&self) -> u32 { 2 } // dead_b_field
    fn via_param(&self) -> u32 { 2 } // dead_b_param
    fn via_let(&self) -> u32 { 2 } // dead_b_let
    fn via_new(&self) -> u32 { 2 } // dead_b_new
    fn via_literal(&self) -> u32 { 2 } // dead_b_literal
    fn via_macro(&self) -> u32 { 2 } // dead_b_macro
    fn via_factory(&self) -> u32 { 2 } // dead_b_factory
    fn via_chain(&self) -> u32 { 2 } // dead_b_chain
    fn via_mut(&self) -> u32 { 2 } // dead_b_mut
    fn push(&mut self, _n: u32) {} // library_push_b
    fn via_args(&self) {} // args_b
    fn describe(&self) {} // describe_b
    fn unknown(&self) -> u32 { 2 } // unknown_b
    fn shadowed(&self) -> u32 { 2 } // shadowed_b
}

impl Holder {
    pub fn field(&self) -> u32 {
        self.a.via_field() + self.count
    }
}

pub fn param(x: &A) -> u32 {
    x.via_param()
}

pub fn typed_let() -> u32 {
    let x: A = A { v: 0 };
    x.via_let()
}

pub fn constructor() -> u32 {
    let x = A::new();
    x.via_new()
}

pub fn literal() -> u32 {
    let x = A { v: 1 };
    x.via_literal() + x.v
}

pub fn in_macro(x: &A, h: &Holder) -> u32 {
    println!("{}", x.via_macro());
    h.field()
}

fn make_a() -> A {
    A { v: 3 }
}

pub fn factory() -> u32 {
    let x = make_a();
    x.via_factory()
}

pub fn chain(x: &A) -> u32 {
    x.me().via_chain()
}

pub fn mutable() -> u32 {
    let mut x = A::new();
    x.via_mut()
}

// Generic receiver: the type is unknown, both `unknown`s stay candidates.
pub fn generic<T>(t: T) -> u32 {
    t.unknown()
}

// The nearest binding has no known type: the earlier `A` must not be used.
pub fn shadowed<T>(t: T) -> u32 {
    let x: A = A { v: 0 };
    let _ = x;
    let x = t;
    x.shadowed()
}

// Bindings whose initialisers refer to each other's names must not loop.
pub fn cyclic<T>(t: T) {
    let a = b.m();
    let b = a.n();
    let _ = (t, b);
}

// `Vec::new()` is a library path: it must not take the return type of this project free fn
// `new` (that would make `A::push` certain and `B::push` dead).
pub fn new() -> A {
    A { v: 0 }
}

pub fn library_call() {
    let mut v = Vec::new();
    v.push(1);
}

// An fn imported from another crate is not the project fn `other::args`.
pub fn imported() {
    let a = args();
    a.via_args();
}

// `std::io::Error` is not the project's `Error`.
pub fn std_typed(e: &std::io::Error) {
    e.describe();
}
