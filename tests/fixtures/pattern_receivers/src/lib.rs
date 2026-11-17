mod other;
use std::collections::HashMap;
use std::rc::Rc;

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
    fn if_value(&self) {} // live_a_if_value
    fn if_chain(&self) {} // live_a_if_chain
    fn if_comment(&self) {} // live_a_if_comment
    fn field_variant(&self) {} // live_a_field_variant
    fn field_bare(&self) {} // live_a_field_bare
    fn field_self(&self) {} // live_a_field_self
    fn field_struct(&self) {} // live_a_field_struct
    fn field_renamed(&self) {} // live_a_field_renamed
    fn field_twin(&self) {} // live_a_field_twin
    fn twin_bare(&self) {} // twin_bare_a
    fn int_map(&self) {} // live_a_int_map
    fn some_pattern(&self) {} // live_a_some_pattern
    fn while_let(&self) {} // live_a_while_let
    fn try_option(&self) {} // live_a_try_option
    fn try_result(&self) {} // live_a_try_result
    fn closure_map(&self) {} // live_a_closure_map
    fn impl_iter(&self) {} // live_a_impl_iter
    fn impl_iter_adapter(&self) {} // live_a_impl_iter_adapter
    fn impl_iter_next(&self) {} // live_a_impl_iter_next
    fn assoc_state(&self) {} // live_a_assoc_state
    fn enum_map(&self) {} // live_a_enum_map
    fn enum_for(&self) {} // live_a_enum_for
    fn inherent_first(&self) {} // live_a_inherent_first
    fn inherent_path(&self) {} // live_a_inherent_path
    fn inherent_self(&self) {} // live_a_inherent_self
    fn closure_typed(&self) {} // live_a_closure_typed
    fn closure_option(&self) {} // live_a_closure_option
    fn closure_for_each(&self) {} // live_a_closure_for_each
    fn closure_fold(&self) {} // live_a_closure_fold
    fn zip_closure(&self) {} // live_a_zip_closure
    fn zip_for(&self) {} // live_a_zip_for
    fn zip_nested(&self) {} // live_a_zip_nested
    fn zip_untyped_right(&self) {} // live_a_zip_untyped_right
    fn zip_untyped_left(&self) {} // live_a_zip_untyped_left
    fn zip_map(&self) {} // live_a_zip_map
    fn zip_option(&self) {} // live_a_zip_option
    fn index_chain(&self) {} // live_a_index_chain
    fn boxed(&self) {} // live_a_boxed
    fn map_values(&self) {} // live_a_map_values
    fn map_index(&self) {} // live_a_map_index
    fn clone(&self) -> A { A } // rc_clone_a
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
    fn if_value(&self) {} // dead_b_if_value
    fn if_chain(&self) {} // dead_b_if_chain
    fn if_comment(&self) {} // dead_b_if_comment
    fn field_variant(&self) {} // dead_b_field_variant
    fn field_bare(&self) {} // dead_b_field_bare
    fn field_self(&self) {} // dead_b_field_self
    fn field_struct(&self) {} // dead_b_field_struct
    fn field_renamed(&self) {} // dead_b_field_renamed
    fn field_twin(&self) {} // dead_b_field_twin
    fn twin_bare(&self) {} // twin_bare_b
    fn int_map(&self) {} // dead_b_int_map
    fn some_pattern(&self) {} // dead_b_some_pattern
    fn while_let(&self) {} // dead_b_while_let
    fn try_option(&self) {} // dead_b_try_option
    fn try_result(&self) {} // dead_b_try_result
    fn closure_map(&self) {} // dead_b_closure_map
    fn impl_iter(&self) {} // dead_b_impl_iter
    fn impl_iter_adapter(&self) {} // dead_b_impl_iter_adapter
    fn impl_iter_next(&self) {} // dead_b_impl_iter_next
    fn assoc_state(&self) {} // dead_b_assoc_state
    fn enum_map(&self) {} // dead_b_enum_map
    fn enum_for(&self) {} // dead_b_enum_for
    fn inherent_first(&self) {} // dead_b_inherent_first
    fn inherent_path(&self) {} // dead_b_inherent_path
    fn inherent_self(&self) {} // dead_b_inherent_self
    fn closure_typed(&self) {} // dead_b_closure_typed
    fn closure_option(&self) {} // dead_b_closure_option
    fn closure_for_each(&self) {} // dead_b_closure_for_each
    fn closure_fold(&self) {} // dead_b_closure_fold
    fn zip_closure(&self) {} // dead_b_zip_closure
    fn zip_for(&self) {} // dead_b_zip_for
    fn zip_nested(&self) {} // dead_b_zip_nested
    fn zip_untyped_right(&self) {} // dead_b_zip_untyped_right
    fn zip_untyped_left(&self) {} // dead_b_zip_untyped_left
    fn zip_map(&self) {} // dead_b_zip_map
    fn zip_option(&self) {} // dead_b_zip_option
    fn index_chain(&self) {} // dead_b_index_chain
    fn boxed(&self) {} // dead_b_boxed
    fn map_values(&self) {} // dead_b_map_values
    fn map_index(&self) {} // dead_b_map_index
    fn clone(&self) -> B { B } // rc_clone_b
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
    ids: Vec<u32>,
    maybe_b: Option<B>,
    maybe: Option<A>,
    boxed: Vec<Box<A>>,
    shared: Rc<A>,
    by_name: HashMap<String, A>,
    keyed: IntMap<u32, A>,
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

    pub fn zips(&self) {
        let _ = self
            .ids
            .iter()
            .zip(&self.items)
            .find_map(|(_id, a)| Some(a.zip_closure()));
        for (_id, a) in self.ids.iter().zip(self.items.iter()) {
            a.zip_for();
        }
        let _ = self
            .ids
            .iter()
            .zip(&self.items)
            .zip(self.ids.iter())
            .find_map(|((_id, a), _other)| Some(a.zip_nested()));
        // Neither side typed: `_` keeps the positions.
        for (a, _n) in self.items.iter().zip(0..3) {
            a.zip_untyped_right();
        }
        for (_n, a) in (0..3).zip(self.items.iter()) {
            a.zip_untyped_left();
        }
        for ((_k, a), _id) in self.by_name.iter().zip(self.ids.iter()) {
            a.zip_map();
        }
        if let Some((_b, a)) = self.maybe_b.as_ref().zip(self.maybe.as_ref()) {
            a.zip_option();
        }
    }

    pub fn int_maps(&mut self) {
        for a in self.keyed.values_mut() {
            a.int_map();
        }
    }

    pub fn enumerated(&self) {
        let _: Vec<()> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, a)| a.is_some_flag())
            .map(|(_i, a)| a.enum_map())
            .collect();
        for (_i, a) in self.items.iter().enumerate() {
            a.enum_for();
        }
    }

    pub fn chains(&self) {
        self.items[0].index_chain();
        for a in self.by_name.values() {
            a.map_values();
        }
    }

    pub fn map_and_rc(&self) {
        self.by_name["k"].map_index();
        let _ = self.shared.clone();
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

impl A {
    fn variant_dup(&self) {} // live_a_variant_dup
    fn map_path(&self) {} // live_a_map_path
    pub fn make(_s: String) -> A { A }
    pub fn make_self(&self) -> A { A }
}
impl B {
    fn variant_dup(&self) {} // dead_b_variant_dup
    fn map_path(&self) {} // dead_b_map_path
}

// `Pair` names a variant of two enums in this file: the payload type comes from the enclosing impl.
pub enum E2 {
    Pair(A),
    Z(B),
}
pub enum Noise {
    Pair(String),
}
impl E2 {
    pub fn run(&self) {
        use self::E2::*;
        match *self {
            Pair(ref a) => a.variant_dup(),
            Z(_) => {}
        }
    }
}

pub fn map_path(r: Result<String, ()>) -> Result<(), ()> {
    let a = r.map(A::make)?;
    a.map_path();
    Ok(())
}

// Struct-like patterns: `Variant { f }`, `Self { f }`, `Struct { f: x }`.
pub enum Shape {
    Named { first: A, second: B },
}
pub struct Pair {
    left: A,
    right: B,
}
impl Pair {
    pub fn fields(&self, s: &Shape) {
        let Shape::Named { first, .. } = s else { return };
        first.field_variant();
        let Self { left, .. } = self;
        left.field_self();
    }
}
pub fn field_patterns(p: &Pair) {
    use Shape::*;
    let Named { first, .. } = Shape::Named { first: A, second: B };
    first.field_bare();
    let Pair { left, .. } = p;
    left.field_struct();
    let Pair { left: l, .. } = p;
    l.field_renamed();
}

// Two enums share a variant name: the qualified path picks the enum.
pub enum Left {
    Same { v: A },
}
pub enum Right {
    Same { v: B },
}
pub fn twin_variants(l: &Left) {
    let Left::Same { v } = l;
    v.field_twin();
}
// The bare form cannot tell the two enums apart: no type, both candidates stay low-confidence.
pub fn twin_variants_bare(l: &Left) {
    use Left::*;
    let Same { v } = l;
    v.twin_bare();

// An `if` has the type of its branches.
pub fn if_values(flag: bool, a: A) {
    let a = if flag { a.make_self() } else { a };
    a.if_value();
}
pub fn if_chain(flag: bool, a: A) {
    let a = if flag {
        a /* the value, then a comment */
    } else if !flag {
        a.make_self()
    } else {
        a
    };
    a.if_chain();
}
pub fn if_comment(flag: bool, a: A) {
    let a = if flag {
        a /* the value, then a comment */
    } else {
        todo!()
    };
    a.if_comment();
}

// `Self::State` is the `type State = A;` of the same impl.
pub trait Program {
    type State;
    fn run(&self, state: &Self::State);
}
pub struct Attach;
impl Program for Attach {
    type State = A;
    fn run(&self, state: &Self::State) {
        state.assoc_state();
    }
}
// `a.f()` and `A::f(..)` call the inherent `f` before a trait's `f` of the same name.
pub trait Both {
    fn inherent_first(&self);
    fn inherent_path(&self);
    fn inherent_self(&self);
}
impl Both for A {
    fn inherent_first(&self) {}
    fn inherent_path(&self) {
        A::inherent_path(self);
    }
    fn inherent_self(&self) {
        self.inherent_self();
    }
}
pub fn inherent_calls(a: &A) {
    a.inherent_first();
}

// `impl Iterator<Item = (&A, u8)>` yields its item type.
pub fn impl_iter(group: impl Iterator<Item = (&'static A, u8)>) {
    for (a, _n) in group {
        a.impl_iter();
    }
}

// Adapters and `next` on an `impl Iterator` parameter keep its item type.
pub fn impl_iter_adapters(group: impl Iterator<Item = &'static A>, mut more: impl Iterator<Item = &'static A>) {
    group.filter(|a| a.is_some_flag()).for_each(|a| a.impl_iter_adapter());
    if let Some(a) = more.next() {
        a.impl_iter_next();
    }
}
