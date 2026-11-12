use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

mod tape {
    pub struct Tape;
    impl Tape {
        fn scoped(&self) {} // live_a_scoped
    }
    pub struct Other;
    impl Other {
        fn scoped(&self) {} // dead_b_scoped
    }
}

// Each positive class has a method on `A` reached through a lock or `RefCell` guard and a same-named
// method on `B` that never is (`dead_b_*`). Each negative class has a guard around a non-project type:
// both twins stay unresolved (`neg_*`).
pub struct A;
pub struct B;

impl A {
    fn refcell(&self) {} // live_a_refcell
    fn refcell_mut(&self) {} // live_a_refcell_mut
    fn mutex(&self) {} // live_a_mutex
    fn rwlock_read(&self) {} // live_a_rwlock_read
    fn rwlock_write(&self) {} // live_a_rwlock_write
    fn guard_ret(&self) {} // live_a_guard_ret
    fn let_guard(&self) {} // live_a_let_guard
    fn poisoned(&self) {} // live_a_poisoned
    fn cell_get_mut(&self) {} // live_a_cell_get_mut
    fn mutex_get_mut(&self) {} // live_a_mutex_get_mut
    fn push_str(&self) {} // neg_a_ext
}

impl B {
    fn refcell(&self) {} // dead_b_refcell
    fn refcell_mut(&self) {} // dead_b_refcell_mut
    fn mutex(&self) {} // dead_b_mutex
    fn rwlock_read(&self) {} // dead_b_rwlock_read
    fn rwlock_write(&self) {} // dead_b_rwlock_write
    fn guard_ret(&self) {} // dead_b_guard_ret
    fn let_guard(&self) {} // dead_b_let_guard
    fn poisoned(&self) {} // dead_b_poisoned
    fn cell_get_mut(&self) {} // dead_b_cell_get_mut
    fn mutex_get_mut(&self) {} // dead_b_mutex_get_mut
    fn push_str(&self) {} // neg_b_ext
}

pub struct H {
    cell: Rc<RefCell<A>>,
    cell_owned: RefCell<A>,
    mutex: Arc<Mutex<A>>,
    rw: RwLock<A>,
    text: Mutex<String>,
    tape: Mutex<tape::Tape>,
}

impl H {
    fn lock(&self) -> MutexGuard<'_, A> {
        self.mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn tape(&self) -> MutexGuard<'_, tape::Tape> {
        self.tape.lock().unwrap()
    }

    pub fn run_mut(&mut self) {
        self.cell_owned.get_mut().cell_get_mut();
        self.mutex.get_mut().unwrap().mutex_get_mut();
        self.text.get_mut().unwrap().push_str();
    }

    pub fn run(&self) {
        self.cell.borrow().refcell();
        self.cell.borrow_mut().refcell_mut();
        self.mutex.lock().unwrap().mutex();
        self.rw.read().unwrap().rwlock_read();
        self.rw.write().expect("lock").rwlock_write();
        self.lock().guard_ret();
        let g = self.mutex.lock().unwrap();
        g.let_guard();
        self.mutex.lock().unwrap_or_else(|e| e.into_inner()).poisoned();
        self.text.lock().unwrap().push_str();
        self.tape().scoped();
    }
}
