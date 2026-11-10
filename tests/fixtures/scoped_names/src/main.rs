// Every definition that matters carries a `// tag` comment; the integration test maps tags to lines.
fn main() {}

fn run(f: fn() -> u8) -> u8 {
    f()
}

// #88 reproduction (verbatim, one fn per line)
fn view() -> u8 { 1 } // outer_view
pub fn a() -> u8 { run(view) }
pub fn b() -> u8 { fn view() -> u8 { 2 } /* b_view */ run(view) }

// An inner fn shadows an outer one in one fn only.
fn shadowed() -> u8 { 0 } // outer_shadowed
pub fn s1() -> u8 { fn shadowed() -> u8 { 1 } /* s1_shadowed */ run(shadowed) }
pub fn s2() -> u8 { fn shadowed() -> u8 { 2 } /* s2_shadowed */ 0 }

// Same name in two mods of one file.
pub mod m1 {
    fn helper() -> u8 { 1 } // m1_helper
    pub fn go() -> u8 { helper() }
}
pub mod m2 {
    fn helper() -> u8 { 2 } // m2_helper
    pub fn go() -> u8 { 0 }
}

// Nested fn inside a closure.
fn inner() -> u8 { 9 } // outer_inner
pub fn c() -> u8 { let f = || { fn inner() -> u8 { 3 } /* c_inner */ run(inner) }; f() }

// Three same-named fns, only two used.
pub fn n1() -> u8 { fn tri() -> u8 { 1 } /* tri1 */ run(tri) }
pub fn n2() -> u8 { fn tri() -> u8 { 2 } /* tri2 */ run(tri) }
pub fn n3() -> u8 { fn tri() -> u8 { 3 } /* tri3 */ 0 }

// Plain calls resolve by scope too.
pub fn d() -> u8 { fn h() -> u8 { 1 } /* d_h */ h() }
pub fn e() -> u8 { fn h() -> u8 { 2 } /* e_h */ 0 }
