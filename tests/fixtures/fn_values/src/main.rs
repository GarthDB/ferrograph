struct Handler {
    run: fn(i32) -> i32,
}

fn as_arg(x: i32) -> i32 {
    x + 1
}

fn in_field(x: i32) -> i32 {
    x + 2
}

fn in_array(x: i32) -> i32 {
    x + 3
}

fn in_map(x: i32) -> i32 {
    x + 4
}

fn in_let(x: i32) -> i32 {
    x + 5
}

fn truly_unused() {}

fn apply(f: fn(i32) -> i32, v: i32) -> i32 {
    f(v)
}

fn main() {
    let _ = apply(as_arg, 1);
    let h = Handler { run: in_field };
    let table = [in_array];
    let mapped: Vec<i32> = vec![1, 2].into_iter().map(in_map).collect();
    let f = in_let;
    println!("{} {} {:?} {}", (h.run)(1), table[0](1), mapped, f(1));
}
