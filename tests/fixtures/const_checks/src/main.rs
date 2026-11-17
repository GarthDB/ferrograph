struct Ctx;

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Ctx>();
};

const CHECK: () = {
    check_helper();
};

const fn check_helper() {}

const VALUE: u8 = value_helper();

const fn value_helper() -> u8 {
    1
}

fn main() {}
