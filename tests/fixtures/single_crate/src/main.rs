use crate::greet;

fn bar() {}

fn main() {
    bar();
    greet();
    println!("{}", fixture_single::greet());
    let _ = fixture_single::use_add();
}
