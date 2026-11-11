pub fn api() {
    used_crate();
    inner::used_via_parent();
}

pub(crate) fn used_crate() {}

pub(crate) fn unused_crate() {}

pub(super) fn unused_super() {}

pub(in crate) fn unused_in_path() {}

mod inner {
    pub(super) fn used_via_parent() {}

    pub(crate) fn unused_nested() {}
}
