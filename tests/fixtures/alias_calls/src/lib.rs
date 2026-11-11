mod chain;
mod dup_a;
mod dup_b;
mod ext;
mod flags;
mod other;
mod util;

pub fn api_bash() -> String {
    flags::gen_bash()
}

pub fn api_fish() -> String {
    flags::gen_fish()
}

pub fn api_nested() -> u32 {
    util::helper_alias()
}

pub fn api_chain() -> u32 {
    chain::outer()
}

pub fn api_negative() -> String {
    other::gen_bash()
}

pub fn api_two_stems() -> String {
    flags::gen_dup()
}

pub fn api_external_path() -> u32 {
    somecrate::util::ext_alias()
}
