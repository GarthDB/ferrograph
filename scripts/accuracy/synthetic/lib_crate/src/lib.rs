pub fn api() -> u32 {
    helper()
}

fn helper() -> u32 {
    1
}

fn lib_dead() -> u32 {
    2
}

pub fn pub_unused() -> u32 {
    3
}

pub fn entry() -> u32 {
    only_by_entry()
}

fn only_by_entry() -> u32 {
    11
}
