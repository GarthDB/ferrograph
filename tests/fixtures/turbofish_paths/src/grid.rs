pub struct PatternGrid<const W: usize, const H: usize>;

impl<const W: usize, const H: usize> PatternGrid<W, H> {
    pub(crate) fn new(_n: usize) -> Self {
        PatternGrid
    }
}

pub struct Wrapper<T, const N: usize>(T);

impl<T, const N: usize> Wrapper<T, N> {
    pub(crate) fn make() {}
}

pub struct FnHolder<F>(F);

impl<F> FnHolder<F> {
    pub(crate) fn hold() {}
}
