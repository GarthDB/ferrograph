use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// #133: tokio `lock().await` and `parking_lot` `try_lock()`. Each positive class has a method on `A`
// reached through the guard and a same-named method on `B` that never is (`dead_b_*`). `neg_*` sits
// behind a guard of a non-project type or an `.await` that is not a lock's: both twins stay unresolved.
pub struct A;
pub struct B;

impl A {
    fn await_mutex(&self) {} // live_a_await_mutex
    fn await_read(&self) {} // live_a_await_read
    fn await_write(&self) {} // live_a_await_write
    fn await_let(&self) {} // live_a_await_let
    fn try_if_let(&self) {} // live_a_try_if_let
    fn try_ok(&self) {} // live_a_try_ok
    fn try_unwrap(&self) {} // live_a_try_unwrap
    fn try_expect(&self) {} // live_a_try_expect
    fn try_read(&self) {} // live_a_try_read
    fn try_write(&self) {} // live_a_try_write
    fn try_question(&self) {} // live_a_try_question
    fn push_str(&self) {} // neg_a_ext
    fn own(&self) {} // neg_a_own
}

impl B {
    fn await_mutex(&self) {} // dead_b_await_mutex
    fn await_read(&self) {} // dead_b_await_read
    fn await_write(&self) {} // dead_b_await_write
    fn await_let(&self) {} // dead_b_await_let
    fn try_if_let(&self) {} // dead_b_try_if_let
    fn try_ok(&self) {} // dead_b_try_ok
    fn try_unwrap(&self) {} // dead_b_try_unwrap
    fn try_expect(&self) {} // dead_b_try_expect
    fn try_read(&self) {} // dead_b_try_read
    fn try_write(&self) {} // dead_b_try_write
    fn try_question(&self) {} // dead_b_try_question
    fn push_str(&self) {} // neg_b_ext
    fn own(&self) {} // neg_b_own
}

// A project type with its own `lock()`: its `.await` is not a lock guard's.
pub struct Slot;

impl Slot {
    async fn lock(&self) -> B {
        B
    }
}

pub struct H {
    m: Arc<Mutex<A>>,
    rw: RwLock<A>,
    text: Mutex<String>,
    slot: Slot,
}

impl H {
    pub async fn run(&self) {
        self.m.lock().await.await_mutex();
        self.rw.read().await.await_read();
        self.rw.write().await.await_write();
        let g = self.m.lock().await;
        g.await_let();
        self.text.lock().await.push_str();
        self.slot.lock().await.own();
    }

    pub fn run_try(&self) {
        if let Some(g) = self.m.try_lock() {
            g.try_if_let();
        }
        if let Ok(g) = self.m.try_lock() {
            g.try_ok();
        }
        self.m.try_lock().unwrap().try_unwrap();
        self.m.try_lock().expect("free").try_expect();
        self.rw.try_read().unwrap().try_read();
        self.rw.try_write().unwrap().try_write();
        self.text.try_lock().unwrap().push_str();
        let _ = self.question();
    }

    fn question(&self) -> Option<()> {
        self.m.try_lock()?.try_question();
        None
    }
}
