pub fn args() -> crate::A {
    crate::A { v: 0 }
}

pub struct Error;

impl Error {
    fn describe(&self) {} // describe_error
}
