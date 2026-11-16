//! Interval and test statistics, ported from `fgacc.py`.

/// Percentage floored to one decimal (never rounded up); `n/a` for an empty denominator.
pub fn floor_pct(n: u64, d: u64) -> String {
    if d == 0 {
        return "n/a".into();
    }
    format!("{:.1}%", (1000.0 * n as f64 / d as f64).floor() / 10.0)
}

pub fn frac(n: u64, d: u64) -> String {
    format!("{} ({n}/{d})", floor_pct(n, d))
}

/// Wilson score interval (lo, hi) in [0, 1] for `k` successes of `n`; (0, 1) for `n == 0`.
pub fn wilson(k: u64, n: u64, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let den = 1.0 + z * z / n;
    let mid = (p + z * z / (2.0 * n)) / den;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / den;
    ((mid - half).max(0.0), (mid + half).min(1.0))
}

/// `frac` plus the Wilson 95% interval, e.g. `34.0% (17/50) [22.4-47.8]`.
pub fn frac_ci(k: u64, n: u64) -> String {
    if n == 0 {
        return "n/a".into();
    }
    let (lo, hi) = wilson(k, n, 1.96);
    format!("{} [{:.1}-{:.1}]", frac(k, n), 100.0 * lo, 100.0 * hi)
}

pub const Z_ALPHA: f64 = 1.96;
pub const Z_BETA: f64 = 0.8416;

/// Items per arm for a two-sided 5% test at 80% power to detect a change `d` from proportion `p`
/// with two independent samples.
pub fn n_unpaired(p: f64, d: f64) -> u64 {
    let p2 = (p + d).clamp(0.0, 1.0);
    ((Z_ALPHA + Z_BETA).powi(2) * (p * (1.0 - p) + p2 * (1.0 - p2)) / (d * d)).ceil() as u64
}

/// Items for a paired (`McNemar`) comparison when a fraction `q` is classified differently by the
/// two arms and the net difference is `d` (Connor 1987 normal approximation).
pub fn n_paired(q: f64, d: f64) -> Option<u64> {
    if q <= d {
        return None;
    }
    Some(((Z_ALPHA * q.sqrt() + Z_BETA * (q - d * d).sqrt()).powi(2) / (d * d)).ceil() as u64)
}

/// Exact two-sided `McNemar` (sign test) p-value for `b` vs `c` discordant items.
pub fn mcnemar_p(b: u64, c: u64) -> f64 {
    let n = b + c;
    if n == 0 {
        return 1.0;
    }
    // Sum of C(n, i) / 2^n for i <= min(b, c), accumulated in f64 via the running binomial ratio
    // (exact enough for the sample sizes here; Python used integers).
    let k = b.min(c);
    let mut term = 0.5f64.powi(i32::try_from(n).unwrap_or(i32::MAX));
    let mut tail = term;
    for i in 1..=k {
        term *= (n - i + 1) as f64 / i as f64;
        tail += term;
    }
    (2.0 * tail).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn floor_pct_floors_and_handles_empty() {
        assert_eq!(floor_pct(1, 3), "33.3%");
        assert_eq!(floor_pct(2, 3), "66.6%");
        assert_eq!(floor_pct(0, 0), "n/a");
        assert_eq!(frac(17, 50), "34.0% (17/50)");
    }

    #[test]
    fn wilson_known_values() {
        let (lo, hi) = wilson(17, 50, 1.96);
        assert!(close(lo, 0.224_367_696_350_610_6), "{lo}");
        assert!(close(hi, 0.478_464_314_585_171_47), "{hi}");
        assert_eq!(wilson(0, 0, 1.96), (0.0, 1.0));
        assert_eq!(frac_ci(17, 50), "34.0% (17/50) [22.4-47.8]");
        assert_eq!(frac_ci(0, 0), "n/a");
    }

    #[test]
    fn mcnemar_exact() {
        assert!(close(mcnemar_p(0, 0), 1.0));
        assert!(close(mcnemar_p(5, 5), 1.0));
        assert!(close(mcnemar_p(0, 10), 2.0 / 1024.0));
        assert!(close(mcnemar_p(1, 9), 0.021_484_375));
        assert!((mcnemar_p(30, 70) / 7.850_139_645_593_67e-5 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sample_sizes() {
        assert_eq!(n_paired(0.05, 0.05), None);
        assert_eq!(n_paired(0.2, 0.05), Some(626));
        assert_eq!(n_unpaired(0.5, 0.05), 1562);
    }
}
