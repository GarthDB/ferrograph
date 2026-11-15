mod a;
mod b;

pub fn entry() {
    let scene = |x: i32| x + 1;
    let _ = scene(1);
    a::run();
}

pub fn entry_param<F: Fn(i32) -> i32>(scene: F) -> i32 {
    scene(1)
}

pub fn entry_self_shadow() {
    let scene = scene(1);
    let _ = scene;
}
