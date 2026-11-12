use std::collections::HashMap;

// Each class has a method on `A` that is called through a pattern, loop or closure binding, and a
// same-named method on `B` that never is (`dead_b_*`): the name alone cannot pick the type.
pub struct A;
pub struct B;

impl A {
    fn for_field(&self) {} // live_a_for_field
    fn for_slice(&self) {} // live_a_for_slice
    fn for_iter(&self) {} // live_a_for_iter
    fn for_option(&self) {} // live_a_for_option
    fn variant_if_let(&self) {} // live_a_variant_if_let
    fn variant_match(&self) {} // live_a_variant_match
    fn variant_bare(&self) {} // live_a_variant_bare
    fn variant_struct(&self) {} // live_a_variant_struct
    fn variant_self(&self) {} // live_a_variant_self
    fn some_pattern(&self) {} // live_a_some_pattern
    fn while_let(&self) {} // live_a_while_let
    fn try_option(&self) {} // live_a_try_option
    fn try_result(&self) {} // live_a_try_result
    fn closure_map(&self) {} // live_a_closure_map
    fn closure_typed(&self) {} // live_a_closure_typed
    fn closure_option(&self) {} // live_a_closure_option
    fn closure_for_each(&self) {} // live_a_closure_for_each
    fn closure_fold(&self) {} // live_a_closure_fold
    fn index_chain(&self) {} // live_a_index_chain
    fn boxed(&self) {} // live_a_boxed
    fn map_values(&self) {} // live_a_map_values
    fn shadowed(&self) {} // shadowed_a
}

impl B {
    fn for_field(&self) {} // dead_b_for_field
    fn for_slice(&self) {} // dead_b_for_slice
    fn for_iter(&self) {} // dead_b_for_iter
    fn for_option(&self) {} // dead_b_for_option
    fn variant_if_let(&self) {} // dead_b_variant_if_let
    fn variant_match(&self) {} // dead_b_variant_match
    fn variant_bare(&self) {} // dead_b_variant_bare
    fn variant_struct(&self) {} // dead_b_variant_struct
    fn variant_self(&self) {} // dead_b_variant_self
    fn some_pattern(&self) {} // dead_b_some_pattern
    fn while_let(&self) {} // dead_b_while_let
    fn try_option(&self) {} // dead_b_try_option
    fn try_result(&self) {} // dead_b_try_result
    fn closure_map(&self) {} // dead_b_closure_map
    fn closure_typed(&self) {} // dead_b_closure_typed
    fn closure_option(&self) {} // dead_b_closure_option
    fn closure_for_each(&self) {} // dead_b_closure_for_each
    fn closure_fold(&self) {} // dead_b_closure_fold
    fn index_chain(&self) {} // dead_b_index_chain
    fn boxed(&self) {} // dead_b_boxed
    fn map_values(&self) {} // dead_b_map_values
    fn shadowed(&self) {} // shadowed_b
}

pub enum E {
    A(A),
    B(B),
}
pub struct Wrap(A);
pub struct Other(B);
use E::*;

pub struct Holder {
    items: Vec<A>,
    maybe: Option<A>,
    boxed: Vec<Box<A>>,
    by_name: HashMap<String, A>,
}

impl Holder {
    pub fn for_loops(&self, slice: &[A]) {
        for a in &self.items {
            a.for_field();
        }
        for a in slice {
            a.for_slice();
        }
        for a in self.items.iter() {
            a.for_iter();
        }
        for a in &self.maybe {
            a.for_option();
        }
        for a in &self.boxed {
            a.boxed();
        }
    }

    pub fn patterns(&self, e: &E, w: &Wrap) {
        if let E::A(a) = e {
            a.variant_if_let();
        }
        match e {
            E::A(a) => a.variant_match(),
            E::B(_) => {}
        }
        if let A(a) = e {
            a.variant_bare();
        }
        let Wrap(a) = w;
        a.variant_struct();
        if let Some(a) = &self.maybe {
            a.some_pattern();
        }
        let mut stack = self.items.iter().collect::<Vec<_>>();
        let _ = &mut stack;
    }

    pub fn tries(&self) -> Option<()> {
        let a = self.first()?;
        a.try_option();
        let b = self.parse().ok()?;
        let _ = b;
        let c = self.checked().unwrap();
        c.try_result();
        Some(())
    }

    fn first(&self) -> Option<A> {
        None
    }
    fn parse(&self) -> Result<A, ()> {
        Err(())
    }
    fn checked(&self) -> Result<A, ()> {
        Err(())
    }

    pub fn closures(&self) {
        self.items.iter().for_each(|a| a.closure_for_each());
        let _: Vec<()> = self.items.iter().map(|a| a.closure_map()).collect();
        let _: Vec<()> = self.items.iter().map(|a: &A| a.closure_typed()).collect();
        self.maybe.as_ref().map(|a| a.closure_option());
        let _ = self.items.iter().fold(0, |n, a| {
            a.closure_fold();
            n
        });
    }

    pub fn chains(&self) {
        self.items[0].index_chain();
        for a in self.by_name.values() {
            a.map_values();
        }
    }

    pub fn shadowing(&self, items: &[A]) {
        for a in items {
            let _ = a;
        }
        let a = unknown();
        a.shadowed();
    }
}

fn unknown() -> impl Sized {
    A
}

pub fn variant_self_payload(e: &E) {
    impl E {
        pub fn run(&self) {
            if let Self::A(a) = self {
                a.variant_self();
            }
        }
    }
    e.run();
}

pub fn while_let(mut v: Vec<A>) {
    while let Some(a) = v.pop() {
        a.while_let();
    }
}
