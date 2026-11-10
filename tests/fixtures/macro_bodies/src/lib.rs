// Repro from #96 (verbatim).
fn bubble() -> u8 { 1 }
fn third() -> u8 { 3 }
pub fn api() -> Vec<u8> { vec![bubble(), third()] }

fn in_macro_def() -> u8 { 1 }
macro_rules! call_it { () => { in_macro_def() }; }
pub fn b() -> u8 { call_it!() }

// Negative controls: still dead.
fn neighbour_unused() -> u8 { 9 }
fn only_in_unused_macro() -> u8 { 2 }
macro_rules! never_invoked { () => { only_in_unused_macro() }; }
fn only_in_dead_caller() -> u8 { 4 }
fn dead_caller() -> Vec<u8> { vec![only_in_dead_caller()] }

// Bracket and brace invocations, values and nested calls.
fn observer_class() -> u8 { 1 }
fn observe() -> u8 { 2 }
fn observer_class_helper() -> u8 { observe() }
macro_rules! send { ($($t:tt)*) => { 0u8 }; }
pub fn msg() -> u8 { send![observer_class_helper(), new] }
fn value_in_vec() -> u8 { 5 }
pub fn vals() -> Vec<fn() -> u8> { vec![value_in_vec] }
pub fn braces() -> u8 { send! { observer_class() } }

// Item-level brace invocation (`stories! { "x" => f }`).
fn story_a() -> u8 { 1 }
fn story_helper() -> u8 { 2 }
fn story_b() -> u8 { story_helper() }
fn story_unlisted() -> u8 { 3 }
macro_rules! stories {
    ($($k:expr => $f:ident),* $(,)?) => { pub fn all() -> Vec<(&'static str, fn() -> u8)> { vec![$(($k, $f)),*] } };
}
stories! { "a" => story_a, "b" => story_b }

// A fn defined by a macro body is not a call; nested macros expand.
macro_rules! make_fn { ($n:ident) => { fn $n() -> u8 { body_helper() } }; }
macro_rules! outer_mac { () => { inner_mac!(inner_callee()) }; }
macro_rules! inner_mac { ($e:expr) => { $e }; }
fn body_helper() -> u8 { 1 }
fn inner_callee() -> u8 { 1 }
make_fn!(generated);
pub fn uses_outer() -> u8 { outer_mac!() }

// Same-named methods in brackets: low confidence, kept in default mode.
struct A;
struct B;
impl A { fn go(&self) -> u8 { 1 } }
impl B { fn go(&self) -> u8 { 2 } }
pub fn ambiguous(a: A) -> Vec<u8> { vec![a.go()] }

// `Type::make()` in a macro keeps its path: the unused free `make` stays dead.
struct Maker;
impl Maker { fn make() -> u8 { 1 } }
fn make() -> u8 { 2 }
pub fn made() -> Vec<u8> { vec![Maker::make()] }
fn turbo_in_macro<T>(x: u8) -> u8 { x }
pub fn tm() -> Vec<u8> { vec![turbo_in_macro::<u8>(1)] }
