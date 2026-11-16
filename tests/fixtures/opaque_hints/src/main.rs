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

struct Runner {
    cmds: Vec<u8>,
}

impl Runner {
    // The shape of fd's `execute_batch`: a `_` hole in the declared type, then a match arm and a loop.
    fn batch(&self) {
        let built: io::Result<Vec<_>> = self.cmds.iter().map(|c| S::new(*c)).collect();
        match built {
            Ok(mut built) => {
                for b in &mut built {
                    b.go();
                }
            }
            Err(_) => {}
        }
    }
}

fn main() {
    let _ = other::T;
    collected(&[]);
}
