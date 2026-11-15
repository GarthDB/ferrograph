pub struct A;

impl A {
    pub fn shared(&self) {}
    pub(crate) fn hidden(&self) {}
    fn private(&self) {}
}

pub fn api(a: &A) {
    a.private();
}
