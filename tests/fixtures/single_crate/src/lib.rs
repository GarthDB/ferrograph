mod utils;

pub const MAX: i32 = 100;
pub type Coordinate = (i32, i32);

pub fn greet() -> &'static str {
    "hello"
}

pub fn unused() -> i32 {
    42
}

fn private_unused() -> i32 {
    0
}

pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub enum Color {
    Red,
    Green,
    Blue,
}

pub trait Draw {
    fn draw(&self);
}

impl Draw for Point {
    fn draw(&self) {}
}

/// Struct with owned and borrowed fields (exercises owns/borrows edges).
pub struct Container {
    pub owned_data: Point,
    pub borrowed_ref: &'static str,
}

/// Function that returns Point by value (exercises return-type owns edge).
pub fn create_point() -> Point {
    Point { x: 0, y: 0 }
}

pub fn take_ownership(p: Point) -> Point {
    p
}

pub fn borrow_ref(p: &Point) -> &Point {
    p
}

/// Struct with explicit lifetime (exercises lifetime_scope edge).
pub struct Wrapper<'a> {
    pub inner: &'a str,
}

pub fn with_lifetime<'a>(s: &'a str) -> &'a str {
    s
}

/// Tuple struct (exercises ordered_field_declaration_list owns edges).
pub struct Pair(pub Point, pub i32);

pub fn use_add() -> i32 {
    utils::add(1, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_integration_test() {
        assert_eq!(greet(), "hello");
    }
}
