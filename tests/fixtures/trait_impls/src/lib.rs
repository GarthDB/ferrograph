pub trait Shape { fn area(&self) -> u32; }
pub struct Sq(pub u32);
impl Shape for Sq { fn area(&self) -> u32 { self.0 } }
impl std::fmt::Display for Sq {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { fmt_helper(self, f) }
}
fn fmt_helper(s: &Sq, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", s.0) }
impl std::ops::Add for Sq { type Output = Sq; fn add(self, o: Sq) -> Sq { Sq(self.0 + o.0) } }
pub fn total(s: &dyn Shape) -> u32 { s.area() }
pub fn show(s: &Sq) -> String { format!("{s}") }
pub fn sum(a: Sq, b: Sq) -> Sq { a + b }

struct Guard;
impl Drop for Guard { fn drop(&mut self) { drop_helper() } }
fn drop_helper() {}

struct Alloc;
unsafe impl std::alloc::GlobalAlloc for Alloc {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 { alloc_helper(l) }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) { dealloc_helper(p, l) }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, n: usize) -> *mut u8 { realloc_helper(p, l, n) }
}
fn alloc_helper(_l: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }
fn dealloc_helper(_p: *mut u8, _l: std::alloc::Layout) {}
fn realloc_helper(_p: *mut u8, _l: std::alloc::Layout, _n: usize) -> *mut u8 { std::ptr::null_mut() }
#[global_allocator]
static A: Alloc = Alloc;

trait WithDefault { fn dflt(&self) -> u32 { 1 } }
impl WithDefault for Sq {}

trait NoImpl { fn orphan_default(&self) -> u32 { 2 } }

struct Plain;
impl Plain { fn truly_unused_private(&self) {} }
