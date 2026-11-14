mod q;
use crate::q::ser;

// `tests` imports a name `q` does not define, which leaves a module-file import of `q/mod.rs`
// next to the item import of `ser`.
pub fn api_twice_imported() -> u8 {
    ser()
}

#[cfg(test)]
mod tests {
    use crate::q::missing;
}
