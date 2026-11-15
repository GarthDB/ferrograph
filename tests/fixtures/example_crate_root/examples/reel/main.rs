mod frame;
mod output;
use frame::{load_bench, Reel};
use output::bench;

fn main() {
    let r = Reel;
    let n = load_bench();
    bench(&r, n);
}
