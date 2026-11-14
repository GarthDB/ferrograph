use std::collections::HashMap;

mod one;
mod two;

// Each positive class has a method on `A` reached through the named shape and a same-named method on
// `B` that never is (`dead_b_*`). Each negative class never gives a project type: both stay (`neg_*`).
pub struct A;
pub struct B;

impl A {
    fn tuple_for(&self) {} // live_a_tuple_for
    fn tuple_if_let(&self) {} // live_a_tuple_if_let
    fn tuple_let(&self) {} // live_a_tuple_let
    fn map_pairs(&self) {} // live_a_map_pairs
    fn map_field(&self) {} // live_a_map_field
    fn and_then(&self) {} // live_a_and_then
    fn opt_map(&self) {} // live_a_opt_map
    fn then(&self) {} // live_a_then
    fn fn_param(&self) {} // live_a_fn_param
    fn fn_field(&self) {} // live_a_fn_field
    fn let_else(&self) {} // live_a_let_else
    fn and_then_untyped(&self) {} // live_a_and_then_untyped
    fn fn_alias(&self) {} // live_a_fn_alias
    fn maker_path(&self) {} // live_a_maker_path
    fn fn_alias_generic(&self) {} // neg_a_fn_alias_generic
    fn maker_ext(&self) {} // neg_a_maker_ext
    fn tuple_pos(&self) {} // neg_a_tuple_pos
    fn tuple_rest(&self) {} // neg_a_tuple_rest
    fn map_key(&self) {} // neg_a_map_key
    fn and_then_ext(&self) {} // neg_a_and_then_ext
    fn then_flag(&self) {} // neg_a_then_flag
    fn fn_ext(&self) {} // neg_a_fn_ext
    fn tuple_let_ext(&self) {} // neg_a_tuple_let_ext
    fn collect_map(&self) {} // live_a_collect_map
    fn collect_ext(&self) {} // neg_a_collect_ext
}

impl B {
    fn tuple_for(&self) {} // dead_b_tuple_for
    fn tuple_if_let(&self) {} // dead_b_tuple_if_let
    fn tuple_let(&self) {} // dead_b_tuple_let
    fn map_pairs(&self) {} // dead_b_map_pairs
    fn map_field(&self) {} // dead_b_map_field
    fn and_then(&self) {} // dead_b_and_then
    fn opt_map(&self) {} // dead_b_opt_map
    fn then(&self) {} // dead_b_then
    fn fn_param(&self) {} // dead_b_fn_param
    fn fn_field(&self) {} // dead_b_fn_field
    fn let_else(&self) {} // dead_b_let_else
    fn and_then_untyped(&self) {} // dead_b_and_then_untyped
    fn fn_alias(&self) {} // dead_b_fn_alias
    fn maker_path(&self) {} // dead_b_maker_path
    fn fn_alias_generic(&self) {} // neg_b_fn_alias_generic
    fn maker_ext(&self) {} // neg_b_maker_ext
    fn tuple_pos(&self) {} // neg_b_tuple_pos
    fn tuple_rest(&self) {} // neg_b_tuple_rest
    fn map_key(&self) {} // neg_b_map_key
    fn and_then_ext(&self) {} // neg_b_and_then_ext
    fn then_flag(&self) {} // neg_b_then_flag
    fn fn_ext(&self) {} // neg_b_fn_ext
    fn tuple_let_ext(&self) {} // neg_b_tuple_let_ext
    fn collect_map(&self) {} // dead_b_collect_map
    fn collect_ext(&self) {} // neg_b_collect_ext
}

fn make() -> A {
    A
}
fn lookup(_: u8) -> Option<A> {
    None
}
fn pick(_: u8) -> A {
    A
}
fn external(_: u8) -> Option<std::net::IpAddr> {
    None
}
fn unknown() -> impl Sized {
    A
}

pub struct Native {
    shadow: Option<A>,
}
pub struct Layer {
    native: Native,
}
type Access = fn(&mut u8) -> &mut A;
type Pick<T> = fn(u8) -> T;
// Cyclic aliases do not compile; indexing must still finish.
type Cyc1 = Cyc2;
type Cyc2 = Cyc1;

pub struct Ctl {
    cyc: Option<Cyc1>,
    alias: Option<Access>,
    generic: Option<Pick<A>>,
    access: Option<fn(&mut u8) -> &mut A>,
    bad: Option<fn(&mut u8) -> u8>,
}

pub struct Host {
    pairs: Vec<(u8, A)>,
    pair: Option<(A, u8)>,
    wide: Vec<(u8, u8, A)>,
    by_name: HashMap<String, A>,
    win: Option<Layer>,
    gpu: Option<u8>,
    ctl: Ctl,
}

impl Host {
    pub fn tuples(&mut self, pairs: &[(u8, A)]) {
        for (_, a) in pairs {
            a.tuple_for();
        }
        if let Some((a, _)) = self.pair.as_mut() {
            a.tuple_if_let();
        }
        let (a, _n) = (make(), 1);
        a.tuple_let();
        for (n, _) in &self.pairs {
            n.tuple_pos();
        }
        for (.., a) in &self.wide {
            a.tuple_rest();
        }
        let (a, _n) = (unknown(), 1);
        a.tuple_let_ext();
    }

    pub fn collected(&self, ns: Vec<u8>) {
        let typed: Vec<_> = ns.into_iter().map(|_| A {}).collect();
        typed.into_iter().for_each(|a| a.collect_map());
        let untyped: Vec<_> = ns.into_iter().map(|_| unknown()).collect();
        untyped.into_iter().for_each(|a| a.collect_ext());
    }

    pub fn maps(&self) {
        for (_, a) in &self.by_name {
            a.map_pairs();
        }
        for (_, a) in self.by_name.iter() {
            a.map_field();
        }
        for (k, _) in &self.by_name {
            k.map_key();
        }
    }

    pub fn adapters(&self, id: Option<u8>, flag: bool, n: u8) {
        if let Some(a) = id.and_then(|i| lookup(i)) {
            a.and_then();
        }
        if let Some(a) = id.map(|i| pick(i)) {
            a.opt_map();
        }
        if let Some(a) = (flag && n > 1).then(|| make()) {
            a.then();
        }
        if let Some(a) = id.and_then(|i| external(i)) {
            a.and_then_ext();
        }
        if let Some(a) = flag.then(|| make()) {
            a.then_flag();
        }
        if let Some(a) = 1u8.checked_sub(n).and_then(|i| lookup(i)) {
            a.and_then_untyped();
        }
    }

    pub fn pointers(&mut self, f: fn(u8) -> A) {
        f(1).fn_param();
        let mut x = 0u8;
        let Some(access) = self.ctl.access else { return };
        access(&mut x).fn_field();
        let Some(bad) = self.ctl.bad else { return };
        bad(&mut x).fn_ext();
        let Some(alias) = self.ctl.alias else { return };
        alias(&mut x).fn_alias();
        let Some(generic) = self.ctl.generic else { return };
        generic(1).fn_alias_generic();
        one::Maker::new().maker_path();
        ext::Maker::new().maker_ext();
    }

    pub fn let_else(&mut self) {
        let (Some(l), Some(_gpu)) = (self.win.as_mut(), self.gpu.as_mut()) else {
            return;
        };
        if let Some(shadow) = &l.native.shadow {
            shadow.let_else();
        }
    }
}

