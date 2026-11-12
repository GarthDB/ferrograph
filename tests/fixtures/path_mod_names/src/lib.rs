#[path = "hidden/real.rs"]
mod renamed;
#[path = "hidden/decoy.rs"]
mod decoy;

pub fn api_path_mod() {
    renamed::work();
}
