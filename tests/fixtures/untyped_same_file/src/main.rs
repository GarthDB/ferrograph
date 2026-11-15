mod other;

struct S;

impl S {
    fn go(&self) {}
}

fn run() {
    let (x, _) = (other::T, 1);
    x.go();
}

fn main() {
    let _ = S;
    run();
}
