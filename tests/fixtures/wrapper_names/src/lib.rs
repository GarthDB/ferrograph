use std::cell::RefCell;

// Project types named like the std lock and guard wrappers are not those wrappers: `Mutex::lock`
// returns an `A`, so `go` is `A::go`; the `Mutex<B>` payload never is the receiver.
pub struct A;
pub struct B;

impl A {
    fn go(&self) {} // live_a_go
    fn cell(&self) {} // live_a_cell
}

impl B {
    fn go(&self) {} // dead_b_go
    fn cell(&self) {} // dead_b_cell
}

pub struct Mutex<T> {
    value: T,
}

impl<T> Mutex<T> {
    fn lock(&self) -> A {
        A
    }
}

pub struct Ref<T> {
    value: T,
}

impl<T> Ref<T> {
    fn unwrap(&self) -> A {
        A
    }
}

pub struct H {
    m: Mutex<B>,
    r: Ref<B>,
    c: RefCell<A>,
}

impl H {
    pub fn run(&self) {
        self.m.lock().go();
        self.r.unwrap().go();
        // `borrow()` is std's even though the project has its own `Ref`.
        self.c.borrow().cell();
    }
}
