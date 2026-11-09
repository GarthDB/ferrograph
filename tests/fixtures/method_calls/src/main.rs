struct V;
struct Chain;
struct A;
struct B;

impl V {
    fn public_entry(&self) -> u8 {
        self.helper() + self.outer() + Self::assoc::<u8>()
    }
    fn helper(&self) -> u8 {
        1
    }
    fn outer(&self) -> u8 {
        self.inner()
    }
    fn inner(&self) -> u8 {
        free_from_inner()
    }
    fn assoc<T: Default>() -> u8 {
        0
    }
    fn truly_unused_method(&self) -> u8 {
        orphan_free()
    }
}

impl Chain {
    fn first(&self) -> Chain {
        Chain
    }
    fn second(&self) -> u8 {
        2
    }
}

impl A {
    fn run(&self) -> u8 {
        1
    }
}

impl B {
    fn run(&self) -> u8 {
        2
    }
}

struct Coll;

impl Coll {
    fn gen_collect<T: Default>(&self) -> T {
        T::default()
    }
    fn in_macro(&self) -> u8 {
        3
    }
    fn in_format(&self) -> u8 {
        4
    }
}

fn free_from_inner() -> u8 {
    5
}

fn orphan_free() -> u8 {
    6
}

fn parse<T: Default>() -> T {
    T::default()
}

fn main() {
    let v = V;
    let c = Chain;
    let _ = v.public_entry();
    let _ = c.first().second();
    let _ = parse::<u8>();
    let _ = Coll.gen_collect::<u8>();
    println!("{}", Coll.in_macro());
    let _ = format!("{}", Coll.in_format());
    let _ = A.run();
}
