use serde::{Deserialize, Serialize};

/// Timing constants from the Gloas validator spec, in basis points of a slot.
pub const SLOT_MS: u64 = 12_000;
pub const ATTESTATION_DUE_BPS: u64 = 2_500;
pub const PAYLOAD_DUE_BPS: u64 = 5_000;
pub const PAYLOAD_ATTESTATION_DUE_BPS: u64 = 7_500;

/// Fork-choice and state-transition constants.
pub const PTC_SIZE: usize = 512;
pub const PAYLOAD_TIMELY_THRESHOLD: usize = PTC_SIZE / 2;
pub const DATA_AVAILABILITY_TIMELY_THRESHOLD: usize = PTC_SIZE / 2;
pub const BUILDER_PAYMENT_THRESHOLD: (u64, u64) = (6, 10);
pub const PROPOSER_SCORE_BOOST_PCT: u64 = 40;
pub const REORG_HEAD_WEIGHT_THRESHOLD_PCT: u64 = 20;
pub const REORG_PARENT_WEIGHT_THRESHOLD_PCT: u64 = 160;

pub fn due_ms(bps: u64) -> u64 {
    SLOT_MS * bps / 10_000
}

/// How a builder behaves once its bid has been selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuilderStrategy {
    /// Reveals at `reveal_ms` if the block reached it before the attestation
    /// deadline, otherwise withholds (the spec's honest withholding rule).
    Honest,
    /// Never reveals the payload.
    Withhold,
    /// Reveals at `late_reveal_ms`, ignoring block timeliness.
    Late,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BuilderConfig {
    pub count: usize,
    /// Fraction of builders using each non-honest strategy.
    pub withhold_fraction: f64,
    pub late_fraction: f64,
    /// When honest builders reveal, ms into the slot.
    pub reveal_ms: u64,
    /// When late builders reveal, ms into the slot.
    pub late_reveal_ms: u64,
    /// Median block value to a builder, in gwei, and its log-space spread.
    pub value_median_gwei: f64,
    pub value_sigma: f64,
    /// Fraction of its value a builder bids.
    pub bid_fraction: f64,
    /// Probability that a proposer builds its own payload instead of taking bids.
    pub self_build_probability: f64,
}

impl Default for BuilderConfig {
    fn default() -> Self {
        Self {
            count: 8,
            withhold_fraction: 0.0,
            late_fraction: 0.0,
            reveal_ms: 4_000,
            late_reveal_ms: 7_000,
            value_median_gwei: 40_000_000.0,
            value_sigma: 0.8,
            bid_fraction: 0.9,
            self_build_probability: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Median one-way propagation delay and its log-space spread.
    pub latency_median_ms: f64,
    pub latency_sigma: f64,
    /// Extra delay for the (large) execution payload and for blob data.
    pub payload_extra_ms: f64,
    pub blob_extra_ms: f64,
    /// Fraction of validators behind a slow link, and the extra delay they see.
    pub partition_fraction: f64,
    pub partition_delay_ms: f64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            latency_median_ms: 250.0,
            latency_sigma: 0.6,
            payload_extra_ms: 150.0,
            blob_extra_ms: 400.0,
            partition_fraction: 0.0,
            partition_delay_ms: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProposerConfig {
    /// Fraction of proposers that publish late, and how late.
    pub late_fraction: f64,
    pub late_publish_ms: u64,
    /// Fraction of proposers that miss their slot entirely.
    pub offline_fraction: f64,
}

impl Default for ProposerConfig {
    fn default() -> Self {
        Self { late_fraction: 0.0, late_publish_ms: 3_500, offline_fraction: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtcAttack {
    /// Colluding members always vote the payload absent and data unavailable.
    VoteAbsent,
    /// Colluding members always vote the payload present and data available.
    VotePresent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PtcConfig {
    pub colluding_fraction: f64,
    pub attack: PtcAttack,
}

impl Default for PtcConfig {
    fn default() -> Self {
        Self { colluding_fraction: 0.0, attack: PtcAttack::VoteAbsent }
    }
}

/// A complete simulation scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub name: String,
    pub seed: u64,
    pub slots: u64,
    /// Attesters per slot; each slot's committee is fresh and carries one
    /// slot's worth of stake.
    pub committee_size: usize,
    pub builders: BuilderConfig,
    pub network: NetworkConfig,
    pub proposers: ProposerConfig,
    pub ptc: PtcConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "baseline".into(),
            seed: 1,
            slots: 1_000,
            committee_size: 2_048,
            builders: BuilderConfig::default(),
            network: NetworkConfig::default(),
            proposers: ProposerConfig::default(),
            ptc: PtcConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let fraction = |name: &str, v: f64| {
            if (0.0..=1.0).contains(&v) {
                Ok(())
            } else {
                Err(ConfigError(format!("{name} must be in [0, 1], got {v}")))
            }
        };
        fraction("builders.withhold_fraction", self.builders.withhold_fraction)?;
        fraction("builders.late_fraction", self.builders.late_fraction)?;
        fraction("builders.bid_fraction", self.builders.bid_fraction)?;
        fraction("builders.self_build_probability", self.builders.self_build_probability)?;
        fraction("network.partition_fraction", self.network.partition_fraction)?;
        fraction("proposers.late_fraction", self.proposers.late_fraction)?;
        fraction("proposers.offline_fraction", self.proposers.offline_fraction)?;
        fraction("ptc.colluding_fraction", self.ptc.colluding_fraction)?;
        if self.builders.withhold_fraction + self.builders.late_fraction > 1.0 {
            return Err(ConfigError("builders.withhold_fraction + late_fraction exceeds 1".into()));
        }
        if self.proposers.late_fraction + self.proposers.offline_fraction > 1.0 {
            return Err(ConfigError("proposers.late_fraction + offline_fraction exceeds 1".into()));
        }
        if self.committee_size < PTC_SIZE {
            return Err(ConfigError(format!("committee_size must be at least PTC_SIZE ({PTC_SIZE})")));
        }
        if self.slots == 0 {
            return Err(ConfigError("slots must be positive".into()));
        }
        if self.builders.count == 0 && self.builders.self_build_probability < 1.0 {
            return Err(ConfigError("builders.count is 0: set self_build_probability to 1".into()));
        }
        if self.network.latency_median_ms <= 0.0 || self.network.latency_sigma < 0.0 {
            return Err(ConfigError("network latency must be positive".into()));
        }
        if self.builders.reveal_ms >= SLOT_MS || self.builders.late_reveal_ms >= SLOT_MS {
            return Err(ConfigError("reveal times must fall inside the slot".into()));
        }
        Ok(())
    }

    pub fn from_json(s: &str) -> Result<Self, ConfigError> {
        let c: Config = serde_json::from_str(s).map_err(|e| ConfigError(e.to_string()))?;
        c.validate()?;
        Ok(c)
    }
}

/// Built-in scenarios.
pub fn presets() -> Vec<Config> {
    let base = Config::default();
    let with = |name: &str, f: &dyn Fn(&mut Config)| {
        let mut c = base.clone();
        c.name = name.into();
        f(&mut c);
        c
    };
    vec![
        base.clone(),
        with("high-latency", &|c| {
            c.network.latency_median_ms = 900.0;
            c.network.latency_sigma = 0.8;
        }),
        with("withholding-builders", &|c| c.builders.withhold_fraction = 0.25),
        with("late-builders", &|c| c.builders.late_fraction = 0.25),
        with("late-proposers", &|c| c.proposers.late_fraction = 0.2),
        with("offline-proposers", &|c| c.proposers.offline_fraction = 0.1),
        with("ptc-attack-absent", &|c| {
            c.ptc.colluding_fraction = 0.55;
            c.ptc.attack = PtcAttack::VoteAbsent;
        }),
        with("ptc-attack-present", &|c| {
            c.ptc.colluding_fraction = 0.55;
            c.ptc.attack = PtcAttack::VotePresent;
            c.builders.withhold_fraction = 0.25;
        }),
        with("partition", &|c| {
            c.network.partition_fraction = 0.3;
            c.network.partition_delay_ms = 3_000.0;
        }),
        with("slow-blobs", &|c| c.network.blob_extra_ms = 6_000.0),
    ]
}

pub fn preset(name: &str) -> Option<Config> {
    presets().into_iter().find(|c| c.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_timing() {
        assert_eq!(due_ms(ATTESTATION_DUE_BPS), 3_000);
        assert_eq!(due_ms(PAYLOAD_DUE_BPS), 6_000);
        assert_eq!(due_ms(PAYLOAD_ATTESTATION_DUE_BPS), 9_000);
        assert_eq!(PAYLOAD_TIMELY_THRESHOLD, 256);
    }

    #[test]
    fn presets_are_valid_and_unique() {
        let all = presets();
        for c in &all {
            c.validate().unwrap_or_else(|e| panic!("{}: {e}", c.name));
        }
        let mut names: Vec<_> = all.iter().map(|c| c.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len());
        assert!(preset("baseline").is_some());
    }

    #[test]
    fn json_round_trip_and_validation() {
        let c = preset("partition").unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(Config::from_json(&json).unwrap(), c);
        assert!(Config::from_json(r#"{"slots": 10, "unknown": 1}"#).is_err());
        assert!(Config::from_json(r#"{"builders": {"withhold_fraction": 1.5}}"#).is_err());
        assert!(Config::from_json(r#"{"committee_size": 100}"#).is_err());
        assert_eq!(Config::from_json(r#"{"slots": 5}"#).unwrap().slots, 5);
    }
}
