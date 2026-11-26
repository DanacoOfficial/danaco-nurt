// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Statistical tests of the analysis: exact McNemar test, Wilcoxon
//! signed-rank test and percentile bootstrap.
//!
//! The procedures follow the usual textbook definitions; the tests of this
//! module check them against values computed independently.

/// Coefficients of the Lanczos approximation of the gamma function
/// (g = 7, n = 9).
const LANCZOS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];

/// Largest number of non-zero differences for which the Wilcoxon test uses
/// the exact distribution; larger samples use the normal approximation.
pub const WILCOXON_EXACT_MAX: usize = 50;

/// Natural logarithm of the gamma function for `x > 0`.
#[must_use]
pub fn ln_gamma(x: f64) -> f64 {
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = LANCZOS[0];
    let t = x + 7.5;
    for (index, coefficient) in LANCZOS.iter().enumerate().skip(1) {
        sum += coefficient / (x + index as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

fn ln_choose(n: u64, k: u64) -> f64 {
    ln_gamma(n as f64 + 1.0) - ln_gamma(k as f64 + 1.0) - ln_gamma((n - k) as f64 + 1.0)
}

/// Probability that a binomial variable with `n` trials and probability
/// one half is at most `k`.
#[must_use]
pub fn binomial_half_cdf(k: u64, n: u64) -> f64 {
    let half = (0.5_f64).ln() * n as f64;
    (0..=k.min(n))
        .map(|i| (ln_choose(n, i) + half).exp())
        .sum::<f64>()
        .min(1.0)
}

/// Two-sided p-value of the exact McNemar test for paired binary outcomes.
///
/// `b` counts pairs where only the first condition succeeded, `c` pairs
/// where only the second succeeded.
///
/// ```
/// use nurt_bench::stats::mcnemar_exact;
///
/// assert!((mcnemar_exact(10, 2) - 158.0 / 4096.0).abs() < 1e-12);
/// assert_eq!(mcnemar_exact(0, 0), 1.0);
/// ```
#[must_use]
pub fn mcnemar_exact(b: u64, c: u64) -> f64 {
    let n = b + c;
    if n == 0 {
        return 1.0;
    }
    (2.0 * binomial_half_cdf(b.min(c), n)).min(1.0)
}

/// Complementary error function, with a relative error below 1.2e-7.
#[must_use]
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let polynomial = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let value = t * polynomial.exp();
    if x >= 0.0 { value } else { 2.0 - value }
}

/// Distribution function of the standard normal distribution.
#[must_use]
pub fn normal_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Result of a Wilcoxon signed-rank test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wilcoxon {
    /// Number of pairs with a non-zero difference.
    pub n: usize,
    /// Sum of the ranks of the positive differences.
    pub w_plus: f64,
    /// Two-sided p-value.
    pub p_value: f64,
    /// Whether the exact distribution was used.
    pub exact: bool,
}

/// Ranks of values, with the average rank for ties; also returns the sum of
/// `t³ − t` over the groups of ties.
fn average_ranks(values: &[f64]) -> (Vec<f64>, f64) {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut ties = 0.0;
    let mut start = 0;
    while start < order.len() {
        let mut end = start;
        while end + 1 < order.len() && values[order[end + 1]] == values[order[start]] {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0 + 1.0;
        for position in &order[start..=end] {
            ranks[*position] = rank;
        }
        let size = (end - start + 1) as f64;
        ties += size * size * size - size;
        start = end + 1;
    }
    (ranks, ties)
}

/// Number of subsets of `{1, …, n}` for every possible sum.
fn signed_rank_counts(n: usize) -> Vec<f64> {
    let total = n * (n + 1) / 2;
    let mut counts = vec![0.0; total + 1];
    counts[0] = 1.0;
    for rank in 1..=n {
        for sum in (rank..=total).rev() {
            counts[sum] += counts[sum - rank];
        }
    }
    counts
}

/// Wilcoxon signed-rank test of paired samples `x` and `y`.
///
/// Zero differences are dropped. Without ties and with at most
/// [`WILCOXON_EXACT_MAX`] non-zero differences the exact distribution is
/// used; otherwise the normal approximation with the tie correction of the
/// variance and a continuity correction of 0.5.
///
/// # Panics
///
/// Panics when the samples have different lengths.
#[must_use]
pub fn wilcoxon_signed_rank(x: &[f64], y: &[f64]) -> Wilcoxon {
    assert_eq!(x.len(), y.len(), "paired samples must have equal lengths");
    let differences: Vec<f64> = x
        .iter()
        .zip(y)
        .map(|(a, b)| a - b)
        .filter(|difference| *difference != 0.0)
        .collect();
    let n = differences.len();
    if n == 0 {
        return Wilcoxon {
            n,
            w_plus: 0.0,
            p_value: 1.0,
            exact: true,
        };
    }
    let magnitudes: Vec<f64> = differences.iter().map(|d| d.abs()).collect();
    let (ranks, ties) = average_ranks(&magnitudes);
    let w_plus: f64 = ranks
        .iter()
        .zip(&differences)
        .filter(|(_, d)| **d > 0.0)
        .map(|(rank, _)| rank)
        .sum();
    let nf = n as f64;
    if ties == 0.0 && n <= WILCOXON_EXACT_MAX {
        let counts = signed_rank_counts(n);
        let total: f64 = counts.iter().sum();
        let observed = w_plus.round() as usize;
        let lower: f64 = counts[..=observed].iter().sum::<f64>() / total;
        let upper: f64 = counts[observed..].iter().sum::<f64>() / total;
        return Wilcoxon {
            n,
            w_plus,
            p_value: (2.0 * lower.min(upper)).min(1.0),
            exact: true,
        };
    }
    let mean = nf * (nf + 1.0) / 4.0;
    let variance = nf * (nf + 1.0) * (2.0 * nf + 1.0) / 24.0 - ties / 48.0;
    let deviation = w_plus - mean;
    let corrected = deviation.abs() - 0.5;
    let p_value = if variance <= 0.0 || corrected <= 0.0 {
        1.0
    } else {
        (2.0 * (1.0 - normal_cdf(corrected / variance.sqrt()))).min(1.0)
    };
    Wilcoxon {
        n,
        w_plus,
        p_value,
        exact: false,
    }
}

/// Median of values; `NaN` for an empty slice.
#[must_use]
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

/// Pseudo-random generator SplitMix64: fixed and documented, so that
/// bootstrap intervals are reproducible from the seed on every platform.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform index below `bound` (`bound > 0`), by rejection of the biased
    /// range.
    pub fn below(&mut self, bound: usize) -> usize {
        let bound = bound as u64;
        let zone = u64::MAX - u64::MAX % bound;
        loop {
            let value = self.next_u64();
            if value < zone {
                return (value % bound) as usize;
            }
        }
    }
}

/// Settings of a bootstrap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bootstrap {
    /// Number of resamples.
    pub resamples: usize,
    /// Seed of the generator.
    pub seed: u64,
    /// Confidence level, for example 0.95.
    pub level: f64,
}

/// Confidence interval.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Interval {
    /// Lower bound.
    pub low: f64,
    /// Upper bound.
    pub high: f64,
}

/// Value of the empirical quantile `q` of sorted values (linear
/// interpolation between order statistics).
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let position = q * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let weight = position - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

impl Bootstrap {
    /// Percentile interval of a statistic of `n` units, resampled with
    /// replacement. The statistic receives the indices of a resample;
    /// resamples where it is not finite are dropped. Returns `None` when
    /// `n` is 0 or no resample gives a finite value.
    pub fn interval(&self, n: usize, statistic: impl Fn(&[usize]) -> f64) -> Option<Interval> {
        if n == 0 || self.resamples == 0 {
            return None;
        }
        let mut generator = SplitMix64::new(self.seed);
        let mut indices = vec![0; n];
        let mut values = Vec::with_capacity(self.resamples);
        for _ in 0..self.resamples {
            for slot in &mut indices {
                *slot = generator.below(n);
            }
            let value = statistic(&indices);
            if value.is_finite() {
                values.push(value);
            }
        }
        if values.is_empty() {
            return None;
        }
        values.sort_by(f64::total_cmp);
        let tail = (1.0 - self.level) / 2.0;
        Some(Interval {
            low: quantile(&values, tail),
            high: quantile(&values, 1.0 - tail),
        })
    }

    /// Interval of a proportion of successes.
    #[must_use]
    pub fn proportion(&self, outcomes: &[bool]) -> Option<Interval> {
        self.interval(outcomes.len(), |sample| {
            sample.iter().filter(|index| outcomes[**index]).count() as f64 / sample.len() as f64
        })
    }

    /// Interval of the difference of two paired proportions, `a − b`.
    #[must_use]
    pub fn paired_proportion_difference(&self, a: &[bool], b: &[bool]) -> Option<Interval> {
        self.interval(a.len().min(b.len()), |sample| {
            let total = sample.len() as f64;
            let first = sample.iter().filter(|index| a[**index]).count() as f64;
            let second = sample.iter().filter(|index| b[**index]).count() as f64;
            (first - second) / total
        })
    }

    /// Interval of the difference of the medians of paired samples,
    /// `median(a) − median(b)`.
    #[must_use]
    pub fn paired_median_difference(&self, a: &[f64], b: &[f64]) -> Option<Interval> {
        self.interval(a.len().min(b.len()), |sample| {
            let first: Vec<f64> = sample.iter().map(|index| a[*index]).collect();
            let second: Vec<f64> = sample.iter().map(|index| b[*index]).collect();
            median(&first) - median(&second)
        })
    }

    /// Percentile interval of a statistic by the bootstrap of clusters:
    /// `clusters[i]` names the cluster of observation `i`, for example its
    /// task. A resample draws as many clusters as there are, with
    /// replacement, and takes every observation of a drawn cluster, so that
    /// the runs of one task stay together. The statistic receives the indices
    /// of the observations. Returns `None` when there are no observations or
    /// no resample gives a finite value.
    pub fn interval_by_cluster(
        &self,
        clusters: &[usize],
        statistic: impl Fn(&[usize]) -> f64,
    ) -> Option<Interval> {
        if clusters.is_empty() || self.resamples == 0 {
            return None;
        }
        let mut members: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (index, cluster) in clusters.iter().enumerate() {
            members.entry(*cluster).or_default().push(index);
        }
        let groups: Vec<&Vec<usize>> = members.values().collect();
        let mut generator = SplitMix64::new(self.seed);
        let mut sample = Vec::with_capacity(clusters.len());
        let mut values = Vec::with_capacity(self.resamples);
        for _ in 0..self.resamples {
            sample.clear();
            for _ in 0..groups.len() {
                sample.extend_from_slice(groups[generator.below(groups.len())]);
            }
            let value = statistic(&sample);
            if value.is_finite() {
                values.push(value);
            }
        }
        if values.is_empty() {
            return None;
        }
        values.sort_by(f64::total_cmp);
        let tail = (1.0 - self.level) / 2.0;
        Some(Interval {
            low: quantile(&values, tail),
            high: quantile(&values, 1.0 - tail),
        })
    }

    /// Interval of a proportion of successes, resampled by cluster.
    #[must_use]
    pub fn proportion_by_cluster(&self, outcomes: &[bool], clusters: &[usize]) -> Option<Interval> {
        self.interval_by_cluster(&clusters[..clusters.len().min(outcomes.len())], |sample| {
            sample.iter().filter(|index| outcomes[**index]).count() as f64 / sample.len() as f64
        })
    }

    /// Interval of the difference of two paired proportions, `a − b`,
    /// resampled by cluster.
    #[must_use]
    pub fn paired_proportion_difference_by_cluster(
        &self,
        a: &[bool],
        b: &[bool],
        clusters: &[usize],
    ) -> Option<Interval> {
        let n = a.len().min(b.len()).min(clusters.len());
        self.interval_by_cluster(&clusters[..n], |sample| {
            let total = sample.len() as f64;
            let first = sample.iter().filter(|index| a[**index]).count() as f64;
            let second = sample.iter().filter(|index| b[**index]).count() as f64;
            (first - second) / total
        })
    }

    /// Interval of the difference of the medians of paired samples,
    /// `median(a) − median(b)`, resampled by cluster.
    #[must_use]
    pub fn paired_median_difference_by_cluster(
        &self,
        a: &[f64],
        b: &[f64],
        clusters: &[usize],
    ) -> Option<Interval> {
        let n = a.len().min(b.len()).min(clusters.len());
        self.interval_by_cluster(&clusters[..n], |sample| {
            let first: Vec<f64> = sample.iter().map(|index| a[*index]).collect();
            let second: Vec<f64> = sample.iter().map(|index| b[*index]).collect();
            median(&first) - median(&second)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn gamma_matches_factorials() {
        assert!(close(ln_gamma(1.0), 0.0, 1e-12));
        assert!(close(ln_gamma(6.0), 120_f64.ln(), 1e-10));
        assert!(close(
            ln_gamma(0.5),
            std::f64::consts::PI.sqrt().ln(),
            1e-10
        ));
        assert!(close(ln_gamma(101.0), 363.739_375_555_563_5, 1e-8));
    }

    #[test]
    fn exact_mcnemar_matches_binomial_tail() {
        assert!(close(mcnemar_exact(10, 2), 0.038_574_218_75, 1e-12));
        assert!(close(mcnemar_exact(2, 10), 0.038_574_218_75, 1e-12));
        assert!(close(mcnemar_exact(5, 5), 1.0, 1e-12));
        assert!(close(mcnemar_exact(0, 6), 0.031_25, 1e-12));
        // 2 · P(X ≤ 40 | n = 100, p = 1/2), computed independently.
        assert!(close(mcnemar_exact(40, 60), 0.056_887_933_640_980_79, 1e-9));
    }

    #[test]
    fn normal_distribution_function() {
        assert!(close(normal_cdf(0.0), 0.5, 1e-7));
        assert!(close(normal_cdf(1.959_963_985), 0.975, 1e-6));
        assert!(close(normal_cdf(-1.0), 0.158_655_253_9, 1e-6));
        assert!(close(erfc(-1.0) + erfc(1.0), 2.0, 1e-12));
    }

    #[test]
    #[allow(
        clippy::approx_constant,
        reason = "3.14 is a measurement of the reference data set, not an approximation of pi"
    )]
    fn exact_wilcoxon_matches_the_reference_example() {
        let x = [1.83, 0.50, 1.62, 2.48, 1.68, 1.88, 1.55, 3.06, 1.30];
        let y = [0.878, 0.647, 0.598, 2.05, 1.06, 1.29, 1.06, 3.14, 1.29];
        let result = wilcoxon_signed_rank(&x, &y);
        assert_eq!(result.n, 9);
        assert!(close(result.w_plus, 40.0, 1e-12));
        assert!(result.exact);
        assert!(close(result.p_value, 0.039_062_5, 1e-12));
    }

    #[test]
    fn exact_wilcoxon_of_all_positive_differences() {
        let x: Vec<f64> = (1..=10).map(f64::from).collect();
        let y = vec![0.0; 10];
        let result = wilcoxon_signed_rank(&x, &y);
        assert!(close(result.p_value, 2.0 / 1024.0, 1e-12));
        let same = wilcoxon_signed_rank(&x, &x);
        assert_eq!((same.n, same.p_value), (0, 1.0));
    }

    #[test]
    fn normal_wilcoxon_with_ties() {
        let x = [1.0, 1.0, 1.0, 1.0, -1.0];
        let result = wilcoxon_signed_rank(&x, &[0.0; 5]);
        assert!(!result.exact);
        assert!(close(result.w_plus, 12.0, 1e-12));
        assert!(close(result.p_value, 0.233_038, 1e-5));
        let large: Vec<f64> = (1..=60).map(f64::from).collect();
        let result = wilcoxon_signed_rank(&large, &[0.0; 60]);
        assert!(!result.exact && result.p_value < 1e-9);
        let balanced: Vec<f64> = (1..=60)
            .map(|v| {
                if v % 2 == 0 {
                    f64::from(v)
                } else {
                    -f64::from(v)
                }
            })
            .collect();
        assert!(wilcoxon_signed_rank(&balanced, &[0.0; 60]).p_value > 0.5);
    }

    #[test]
    fn medians() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert!(median(&[]).is_nan());
    }

    #[test]
    fn generator_is_reproducible_and_uniform() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(1);
        assert_eq!(a.next_u64(), b.next_u64());
        assert_eq!(SplitMix64::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);
        let mut generator = SplitMix64::new(7);
        let mut counts = [0_u32; 4];
        for _ in 0..40_000 {
            counts[generator.below(4)] += 1;
        }
        assert!(counts.iter().all(|count| (9_000..11_000).contains(count)));
    }

    const BOOT: Bootstrap = Bootstrap {
        resamples: 10_000,
        seed: 1,
        level: 0.95,
    };

    #[test]
    fn bootstrap_interval_of_a_proportion() {
        let outcomes: Vec<bool> = (0..100).map(|i| i % 2 == 0).collect();
        let interval = BOOT.proportion(&outcomes).unwrap();
        assert!(interval.low < 0.5 && interval.high > 0.5);
        assert!(close(interval.high - interval.low, 0.196, 0.03));
        assert_eq!(BOOT.proportion(&outcomes), BOOT.proportion(&outcomes));
        assert_eq!(BOOT.proportion(&[]), None);
        let constant = BOOT.proportion(&[true; 20]).unwrap();
        assert_eq!((constant.low, constant.high), (1.0, 1.0));
    }

    #[test]
    fn bootstrap_intervals_of_paired_differences() {
        let a: Vec<bool> = (0..200).map(|i| i % 10 < 8).collect();
        let b: Vec<bool> = (0..200).map(|i| i % 10 < 5).collect();
        let difference = BOOT.paired_proportion_difference(&a, &b).unwrap();
        assert!(difference.low > 0.2 && difference.high < 0.4);
        let x: Vec<f64> = (0..50).map(|i| f64::from(i) + 10.0).collect();
        let y: Vec<f64> = (0..50).map(f64::from).collect();
        let shift = BOOT.paired_median_difference(&x, &y).unwrap();
        assert!(shift.low <= 10.0 && shift.high >= 10.0);
        let nothing = BOOT.interval(5, |_| f64::NAN);
        assert_eq!(nothing, None);
    }

    #[test]
    fn clusters_stay_together_in_a_resample() {
        // Two tasks with three runs each, all passed or all failed: a
        // resample of tasks gives a proportion of 0, 0.5 or 1 only.
        let outcomes = [true, true, true, false, false, false];
        let clusters = [0, 0, 0, 1, 1, 1];
        let by_task = BOOT.proportion_by_cluster(&outcomes, &clusters).unwrap();
        assert_eq!((by_task.low, by_task.high), (0.0, 1.0));
        let by_run = BOOT.proportion(&outcomes).unwrap();
        assert!(by_run.high < 1.0 && by_run.low > 0.0);
        assert_eq!(BOOT.proportion_by_cluster(&[], &[]), None);
        let a = [true, true, false, false];
        let b = [false, false, false, false];
        let difference = BOOT
            .paired_proportion_difference_by_cluster(&a, &b, &[0, 0, 1, 1])
            .unwrap();
        assert_eq!((difference.low, difference.high), (0.0, 1.0));
        let x = [3.0, 3.0, 5.0, 5.0];
        let y = [1.0, 1.0, 1.0, 1.0];
        let shift = BOOT
            .paired_median_difference_by_cluster(&x, &y, &[0, 0, 1, 1])
            .unwrap();
        assert!(shift.low >= 2.0 && shift.high <= 4.0);
    }
}
