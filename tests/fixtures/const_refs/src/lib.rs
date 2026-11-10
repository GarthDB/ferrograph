// Repro from #97 (verbatim).
mod other;
mod consts;
fn dv() -> u8 { 1 }
fn dc() -> u8 { 2 }
fn adapter<S>(x: u8) -> u8 { x }
fn handler() {}
fn init() -> u8 { 1 }
pub static ONE: fn() = handler;
pub static VAL: std::sync::LazyLock<u8> = std::sync::LazyLock::new(init);
pub fn api() -> u8 { other::go() }
pub fn api2() -> Vec<u8> { vec![1u8].into_iter().map(adapter::<u8>).collect() }

// Private static table read by a live fn.
fn table_fn() -> u8 { 1 }
static TABLE: [(&str, fn() -> u8); 1] = [("a", table_fn)];
pub fn lookup() -> u8 { (TABLE[0].1)() }

// Const calling a fn, used from live code; lazy static through a deref.
fn const_helper() -> u8 { 3 }
const FROM_FN: u8 = const_helper();
pub fn uses_const() -> u8 { FROM_FN }
fn lazy_init() -> u8 { 4 }
static LAZY: std::sync::LazyLock<u8> = std::sync::LazyLock::new(lazy_init);
pub fn uses_lazy() -> u8 { *LAZY }

// Associated consts: `pub` is a root, a private one needs a user.
pub struct Prop(u32);
impl Prop {
    const fn builtin(n: u32) -> Prop { Prop(n) }
    const fn hidden_builtin(n: u32) -> Prop { Prop(n) }
    const fn used_builtin(n: u32) -> Prop { Prop(n) }
    pub const OPACITY: Prop = Prop::builtin(0);
    const HIDDEN: Prop = Prop::hidden_builtin(1);
    const USED: Prop = Prop::used_builtin(2);
    pub fn used(&self) -> u32 { Self::USED.0 }
}

// Trait impl consts and #[used] statics are roots.
trait Tr { const ID: u8; }
fn trait_const_init() -> u8 { 5 }
impl Tr for Prop { const ID: u8 = trait_const_init(); }
fn kept_by_used() {}
#[used]
static KEEP: fn() = kept_by_used;

// Turbofish as a value in a struct field.
struct H { f: fn(u8) -> u8 }
fn adapter_field<S>(x: u8) -> u8 { x }
pub fn make_h() -> H { H { f: adapter_field::<u8> } }

// Negative controls: still dead.
fn unref_fn() {}
static UNREFERENCED: [fn(); 1] = [unref_fn];
fn unref_const_fn() -> u8 { 6 }
const UNREFERENCED_CONST: u8 = unref_const_fn();
fn max_init() -> u8 { 7 }
const MAX: u8 = max_init();
pub fn std_max() -> u32 { u32::MAX }
fn dead_turbofish<S>(x: u8) -> u8 { x }
