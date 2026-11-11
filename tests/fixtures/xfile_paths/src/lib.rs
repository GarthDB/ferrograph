mod a;
mod derives;
mod util;

fn helper() {}
fn sup() {}
fn unused_in_lib() {}

struct S;
impl S {
    fn new() -> S {
        S
    }
    fn made_via_alias() {}
}

pub fn entry() {
    a::run();
}
