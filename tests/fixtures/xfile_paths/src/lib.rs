mod a;
mod util;

fn helper() {}
fn sup() {}
fn unused_in_lib() {}

struct S;
impl S {
    fn new() -> S {
        S
    }
}

pub fn entry() {
    a::run();
}
