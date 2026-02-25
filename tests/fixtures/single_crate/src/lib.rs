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
