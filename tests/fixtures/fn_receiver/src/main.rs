trait Parse {
    fn map<T>(self, f: fn(u8) -> u8) -> u8;
}

impl<F: Fn(&str) -> u8> Parse for F {
    fn map<T>(self, f: fn(u8) -> u8) -> u8 {
        f(self("1"))
    }
}

fn point(_s: &str) -> u8 {
    1
}

fn turbofish_parser(_s: &str) -> u8 {
    3
}

fn unused_parser(_s: &str) -> u8 {
    2
}

fn double(x: u8) -> u8 {
    x * 2
}

fn main() {
    let _ = point.map(double);
    let _ = turbofish_parser.map::<u8>(double);
}
