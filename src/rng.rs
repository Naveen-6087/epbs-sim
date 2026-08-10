/// xoshiro256** seeded through SplitMix64. Implemented here so results are
/// identical on every platform and dependency version.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        Self { s: [splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm)] }
    }

    /// An independent stream for `label`, so adding draws in one part of the
    /// simulation does not shift the randomness of another.
    pub fn fork(&self, label: u64) -> Self {
        let mut sm = self.s[0] ^ label.wrapping_mul(0xd1b5_4a32_d192_ed03);
        Self { s: [splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm), splitmix64(&mut sm)] }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        // Lemire's multiply-shift with rejection for an unbiased result.
        loop {
            let m = u128::from(self.next_u64()) * u128::from(n);
            if (m as u64) >= n.wrapping_neg() % n {
                return (m >> 64) as u64;
            }
        }
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Standard normal via Box-Muller.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit().max(f64::MIN_POSITIVE);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Log-normal with the given median and log-space standard deviation.
    pub fn log_normal(&mut self, median: f64, sigma: f64) -> f64 {
        median * (sigma * self.normal()).exp()
    }

    /// `k` distinct indices from `0..n` (partial Fisher-Yates).
    pub fn sample(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut pool: Vec<usize> = (0..n).collect();
        let k = k.min(n);
        for i in 0..k {
            let j = i + self.below((n - i) as u64) as usize;
            pool.swap(i, j);
        }
        pool.truncate(k);
        pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_forkable() {
        let a: Vec<u64> = (0..5)
            .map({
                let mut r = Rng::new(7);
                move |_| r.next_u64()
            })
            .collect();
        let b: Vec<u64> = (0..5)
            .map({
                let mut r = Rng::new(7);
                move |_| r.next_u64()
            })
            .collect();
        assert_eq!(a, b);
        let root = Rng::new(7);
        assert_ne!(root.fork(1).next_u64(), root.fork(2).next_u64());
        assert_eq!(root.fork(1).next_u64(), root.clone().fork(1).next_u64());
    }

    #[test]
    fn matches_xoshiro_reference() {
        // From state [1, 2, 3, 4] the first xoshiro256** output is
        // rotl(2 * 5, 7) * 9 = 11520, and the second is 0.
        let mut r = Rng { s: [1, 2, 3, 4] };
        assert_eq!(r.next_u64(), 11520);
        assert_eq!(r.next_u64(), 0);
        let mut sm = 0u64;
        assert_eq!(splitmix64(&mut sm), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn distributions_are_sane() {
        let mut r = Rng::new(42);
        let n = 100_000;
        let mean = (0..n).map(|_| r.unit()).sum::<f64>() / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
        let normal_mean = (0..n).map(|_| r.normal()).sum::<f64>() / n as f64;
        assert!(normal_mean.abs() < 0.02, "{normal_mean}");
        let mut medians: Vec<f64> = (0..n).map(|_| r.log_normal(100.0, 0.5)).collect();
        medians.sort_by(f64::total_cmp);
        assert!((medians[n / 2] - 100.0).abs() < 3.0, "{}", medians[n / 2]);
        let counts = (0..n).fold([0usize; 4], |mut c, _| {
            c[r.below(4) as usize] += 1;
            c
        });
        assert!(counts.iter().all(|&c| (c as f64 - n as f64 / 4.0).abs() < n as f64 * 0.01), "{counts:?}");
    }

    #[test]
    fn sampling_is_distinct() {
        let mut r = Rng::new(3);
        let mut s = r.sample(100, 30);
        s.sort_unstable();
        s.dedup();
        assert_eq!(s.len(), 30);
        assert_eq!(r.sample(5, 10).len(), 5);
    }
}
