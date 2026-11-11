pub fn used() {}

fn unused_helper() {}

pub fn pub_dead() -> u32 {
    6
}

pub(crate) fn crate_dead() {}
