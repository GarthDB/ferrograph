const USED: u8 = 1;
const UNUSED: u8 = 2;
static UNUSED_STATIC: u8 = 3;
const ONLY_DEAD_FN: u8 = 4;
pub const PUB_ROOT: u8 = 5;
const _: () = assert!(USED == 1);

trait Config {
    const LEVEL: u8 = 1;
}

struct Settings;

impl Config for Settings {}

fn dead_fn() -> u8 {
    ONLY_DEAD_FN
}

fn main() {
    let _ = USED;
}
