mod util;
use util::helper;

pub struct S;

impl S {
    fn helper(&self) {} // dead_method
}

pub fn api() {
    helper();
}
