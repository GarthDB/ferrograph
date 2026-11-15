mod a;
mod b;

pub fn entry() {
    let scene = |x: i32| x + 1;
    let _ = scene(1);
    a::run();
}
