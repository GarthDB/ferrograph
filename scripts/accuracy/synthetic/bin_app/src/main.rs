mod util;

struct S;
struct T;

impl S {
    fn run(&self) -> u32 {
        live_b()
    }
    fn dead_m(&self) -> u32 {
        1
    }
}

impl T {
    fn run(&self) -> u32 {
        2
    }
}

trait Speak {
    fn speak(&self) -> u32;
}

impl Speak for S {
    fn speak(&self) -> u32 {
        3
    }
}

fn live_a() -> u32 {
    live_b()
}

fn live_b() -> u32 {
    4
}

fn dead_c() -> u32 {
    dead_d()
}

fn dead_d() -> u32 {
    5
}

fn cleanup() {
    println!("cleanup");
}

fn teardown() {
    println!("teardown");
}

impl std::fmt::Display for T {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", greeting_helper())
    }
}

fn greeting_helper() -> u32 {
    8
}

fn table_entry() -> u32 {
    9
}

const TABLE: &[fn() -> u32] = &[
    table_entry,
];

fn main() {
    let s = S;
    let _t = T;
    println!("{}", TABLE.len());
    println!("{} {} {}", live_a(), s.run(), s.speak());
    util::used();
    cleanup();
    teardown();
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        assert_eq!(test_only_helper(), 7);
    }

    fn test_only_helper() -> u32 {
        7
    }
}
