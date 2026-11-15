use std::fmt;

pub struct W {
    out: String,
}

impl W {
    fn write_fmt(&mut self, a: fmt::Arguments<'_>) { // live_write_fmt
        self.out.push_str(&a.to_string());
    }
    pub fn go(&mut self) {
        write!(self, "{}", 1).ok();
    }
}

struct Svc;

#[some_crate::interface(name = "x")]
impl Svc {
    fn category(&self) -> &str { // live_category
        "a"
    }
}

struct Plain;

#[allow(dead_code)]
impl Plain {
    fn plain_dead(&self) {} // dead_plain
}

struct Unattributed;

impl Unattributed {
    fn unused(&self) {} // dead_unused
}
