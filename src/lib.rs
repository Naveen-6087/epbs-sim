#![forbid(unsafe_code)]

pub mod config;
pub mod forkchoice;
pub mod metrics;
pub mod rng;
pub mod sim;

pub use config::{Config, ConfigError, preset, presets};
pub use metrics::{Summary, summarize};
pub use sim::{SlotOutcome, SlotRecord, run};
