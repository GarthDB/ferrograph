fn main() {}

fn _kept() {
    helper(); // callee of an `_` fn stays live
}

fn helper() {}

pub(crate) fn _kept_restricted() {}

fn really_dead() {} // dead
