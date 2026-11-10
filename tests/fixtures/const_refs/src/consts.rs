pub(crate) const REG: &[fn() -> u8] = &[reg_fn];
fn reg_fn() -> u8 { 1 }
pub(crate) const UNUSED_REG: &[fn() -> u8] = &[unused_reg_fn];
fn unused_reg_fn() -> u8 { 2 }
