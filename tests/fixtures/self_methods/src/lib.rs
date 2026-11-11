pub struct A;
pub struct B;

impl A {
    fn find(&self) -> u32 {
        1
    }
    fn via_macro(&self) -> u32 {
        2
    }
    pub fn api(&self) -> u32 {
        self.find() + dbg!(self.via_macro())
    }
}

impl B {
    fn find(&self) -> u32 { // dead_b_find
        3
    }
    fn via_macro(&self) -> u32 { // dead_b_macro
        4
    }
}

impl A {
    fn other_impl_block(&self) -> u32 {
        5
    }
    pub fn api2(&self) -> u32 {
        self.other_impl_block()
    }
}
