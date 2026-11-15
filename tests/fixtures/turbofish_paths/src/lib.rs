mod grid;

pub fn build() {
    let _ = grid::PatternGrid::<2, 4>::new(1);
    let _ = grid::Wrapper::<Vec<u8>, 3>::make();
    let _ = grid::FnHolder::<Box<dyn Fn(u8) -> u8>>::hold();
}

fn make() {}

struct Other;

impl Other {
    fn hold() {}
}
