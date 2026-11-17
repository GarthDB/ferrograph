trait Parse {
    fn map(self, f: fn(u8) -> u8) -> u8;
}

impl<F: Fn(&str) -> u8> Parse for F {
    fn map(self, f: fn(u8) -> u8) -> u8 {
        f(self("1"))
    }
}

fn point(_s: &str) -> u8 {
    1
}

fn unused_parser(_s: &str) -> u8 {
    2
}

fn double(x: u8) -> u8 {
    x * 2
}

fn main() {
    let _ = point.map(double);
}
