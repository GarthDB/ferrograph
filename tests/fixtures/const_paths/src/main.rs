mod a;
mod b;
mod c;
mod e;

use a::DUP;

macro_rules! mac {
    () => {
        crate::b::MAC
    };
}

fn main() {
    let _ = b::QUAL + crate::b::CRATE_QUAL + DUP + e::REEXP + mac!();
}
