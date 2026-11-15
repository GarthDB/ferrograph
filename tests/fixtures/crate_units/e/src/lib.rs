// `renamed` is `a` under another name in the root's `[workspace.dependencies]`.
pub fn through_rename<T>(x: T) {
    x.shared();
}
