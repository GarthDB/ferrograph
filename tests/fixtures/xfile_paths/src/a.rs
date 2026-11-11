use crate::derives::{from_mod, generate_two};
use crate::S;
use crate::S as Alias;

pub(crate) fn run() {
    crate::helper();
    super::sup();
    let _ = S::new();
    crate::util::tool();
    util::other();
    crate::derives::generate();
    generate_two();
    from_mod();
    Alias::made_via_alias();
}
