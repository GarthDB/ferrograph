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

fn main() {
    let s = S;
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
