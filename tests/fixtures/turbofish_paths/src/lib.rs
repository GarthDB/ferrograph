mod grid;

pub fn build() {
    let _ = grid::PatternGrid::<2, 4>::new(1);
    let _ = grid::Wrapper::<Vec<u8>, 3>::make();
}

fn make() {}
