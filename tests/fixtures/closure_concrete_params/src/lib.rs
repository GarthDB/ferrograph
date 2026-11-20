// Like egui's `Context::write(|ctx: &mut ContextImpl| ..)`: the closure parameter's type comes from the
// callee's concrete signature, and the private `Impl` methods have same-named public twins on `Ctx`.
pub struct Impl;
pub struct Ctx {
    inner: Impl,
}
pub struct Other;
pub struct Holder {
    inner: Other,
}

impl Impl {
    fn foo(&self) {} // live_impl_foo
    fn val(&self) -> u32 {
        0
    } // live_impl_val
    fn bar(&self) {} // dead_impl_bar
    fn baz(&self) {} // dead_impl_baz
}

impl Other {
    fn bar(&self) {} // dead_other_bar
    fn baz(&self) {} // dead_other_baz
}

impl Ctx {
    pub fn foo(&self) {}
    pub fn val(&self) -> u32 {
        0
    }
    pub fn bar(&self) {}
    pub fn baz(&self) {}

    fn write<R>(&self, writer: impl FnOnce(&Impl) -> R) -> R {
        writer(&self.inner)
    }

    fn read<F>(&self, reader: F) -> u32
    where
        F: Fn(&Impl) -> u32,
    {
        reader(&self.inner)
    }

    pub fn run(&self) -> u32 {
        self.write(|c| c.foo());
        self.read(|c| c.val())
    }

    fn modify(&self, f: impl FnOnce(&Impl)) {
        f(&self.inner);
    }

    // `Holder::modify` takes an `Other`: the two signatures disagree, so `c` stays untyped.
    pub fn conflicting(&self) {
        self.modify(|c| c.bar());
    }
}

impl Holder {
    fn modify(&self, f: impl FnOnce(&Other)) {
        f(&self.inner);
    }

    pub fn conflicting(&self) {
        self.modify(|c| c.baz());
    }
}
