pub struct Ignore {
    dir: String,
    parent: Option<Box<Ignore>>,
}

#[derive(Clone, Copy)]
pub(crate) struct IgnoreRef<'a> {
    inner: &'a Ignore,
}

impl IgnoreRef<'_> {
    pub(crate) fn path(&self) -> &str { // ref_path
        &self.inner.dir
    }
}

pub(crate) struct Parents<'a>(Option<IgnoreRef<'a>>);

impl<'a> Iterator for Parents<'a> {
    type Item = IgnoreRef<'a>;

    fn next(&mut self) -> Option<IgnoreRef<'a>> {
        let cur = self.0.take()?;
        self.0 = cur.inner.parent.as_deref().map(|inner| IgnoreRef { inner });
        Some(cur)
    }
}

impl Ignore {
    pub(crate) fn parents(&self) -> Parents<'_> {
        Parents(Some(IgnoreRef { inner: self }))
    }
}
