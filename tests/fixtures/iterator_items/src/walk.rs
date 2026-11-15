use crate::dir::Ignore;

pub struct DirEntry;

impl DirEntry {
    pub fn path(&self) -> &str { // entry_path
        ""
    }
}

pub fn check_loop(ig: &Ignore) -> usize {
    let mut n = 0;
    for p in ig.parents().take_while(|p| p.path() != "x") {
        n += p.path().len();
    }
    n
}

pub fn check_next(ig: &Ignore) -> usize {
    let mut it = ig.parents();
    it.next().map_or(0, |p| p.path().len())
}
