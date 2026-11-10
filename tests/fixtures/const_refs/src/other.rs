use super::*;
fn run(f: fn() -> u8) -> u8 { f() }
pub fn go() -> u8 { run(dv) + dc() }

// A const from another file, imported by path.
use crate::consts::REG;
pub fn go_reg() -> u8 { let mut n = 0; for f in REG { n += f(); } n }
