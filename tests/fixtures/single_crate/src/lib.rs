mod utils;

pub fn greet() -> &'static str {
    "hello"
}

pub fn unused() -> i32 {
    42
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
