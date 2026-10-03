//! Small numeric helpers shared across the analysis modules.

/// Middle value (mean of the two middle values for an even count). Sorts `values`.
pub fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = values.len() / 2;
    Some(if values.len().is_multiple_of(2) { (values[m - 1] + values[m]) / 2.0 } else { values[m] })
}

/// The value at fraction `p` (0..=1) of the sorted `values`. Sorts `values`.
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[((values.len() - 1) as f64 * p).round() as usize])
}

/// Mean of the finite values.
pub fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.filter(|v| v.is_finite()).fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f64)
}

/// Largest of the finite values.
pub fn finite_max(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.filter(|v| v.is_finite()).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
}

/// Index on a resampled grid whose last index is `last` for a lap fraction.
pub fn grid_index(pct: f64, last: usize) -> usize {
    ((pct * last as f64).round() as usize).min(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(percentile(&mut [1.0, 2.0, 3.0, 4.0, 5.0], 0.98), Some(5.0));
        assert_eq!(mean([1.0, f64::NAN, 3.0].into_iter()), Some(2.0));
        assert_eq!(finite_max([1.0, f64::NAN, 3.0].into_iter()), Some(3.0));
        assert_eq!(finite_max(std::iter::empty()), None);
        assert_eq!(grid_index(0.5, 10), 5);
        assert_eq!(grid_index(1.2, 10), 10);
    }
}
