use std::io;

mod other;

struct S;

impl S {
    fn new(_: u8) -> io::Result<S> {
        Ok(S)
    }
    fn go(&self) {}
}

struct Holder<W> {
    w: W,
}

impl<W> Holder<W> {
    fn generic_field(&self) {
        self.w.go();
    }
}

fn collected(items: &[u8]) {
    let built: io::Result<Vec<_>> = items.iter().map(|c| S::new(*c)).collect();
    if let Ok(mut built) = built {
        for b in &mut built {
            b.go();
        }
    }
}

fn main() {
    let _ = other::T;
    collected(&[]);
}
