#[cfg(test)]
mod tests {
    // `a` is a dev-dependency of `d`, which `#[cfg(test)]` code in `src/` may use.
    fn from_unit_test<T>(x: T) {
        x.shared();
    }
}
