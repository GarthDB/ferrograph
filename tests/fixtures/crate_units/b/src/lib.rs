// `b` does not depend on `a`.
pub fn run<T>(x: T) {
    x.shared();
    x.hidden();
}
