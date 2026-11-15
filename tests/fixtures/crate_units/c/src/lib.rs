// `c` depends on `a`: it can call `shared` but not `hidden`.
pub fn go<T>(x: T) {
    x.shared();
    x.hidden();
}
