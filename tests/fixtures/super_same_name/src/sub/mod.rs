mod n;
mod unused;

pub fn find_closest_pair(p: u8, m: i16) -> (i16, i16) {
    (p as i16, m)
}

pub fn wrapper(p: u8) {
    n::find_closest_pair(p);
}
