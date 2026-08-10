use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use epbs_sim::{Config, ConfigError, preset, presets, run, summarize};
use serde_json::Value;

/// Discrete-event simulator for enshrined proposer-builder separation (EIP-7732).
#[derive(Parser, Debug)]
#[command(name = "epbs-sim", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List the built-in scenarios.
    Scenarios,
    /// Print a scenario's full configuration as JSON (a starting point for --config).
    Config {
        #[command(flatten)]
        source: Source,
    },
    /// Run one scenario and print a summary.
    Run {
        #[command(flatten)]
        source: Source,
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
        /// Also write one CSV row per slot to this file.
        #[arg(long)]
        slots_csv: Option<PathBuf>,
    },
    /// Vary one parameter (a dotted path such as builders.withhold_fraction)
    /// across values and seeds; prints one CSV row per run.
    Sweep {
        #[command(flatten)]
        source: Source,
        #[arg(long)]
        param: String,
        /// Comma-separated values.
        #[arg(long, value_delimiter = ',', required = true)]
        values: Vec<String>,
        /// Number of seeds per value (seed, seed + 1, ...).
        #[arg(long, default_value_t = 1)]
        seeds: u64,
    },
}

#[derive(Args, Debug)]
struct Source {
    /// Built-in scenario name (see `scenarios`).
    #[arg(long, default_value = "baseline", conflicts_with = "config")]
    scenario: String,
    /// JSON configuration file.
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    slots: Option<u64>,
    #[arg(long)]
    seed: Option<u64>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Text,
    Json,
}

impl Source {
    fn load(&self) -> Result<Config, String> {
        let mut cfg = match &self.config {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
                Config::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => preset(&self.scenario).ok_or_else(|| {
                let names: Vec<_> = presets().into_iter().map(|c| c.name).collect();
                format!("unknown scenario `{}` (available: {})", self.scenario, names.join(", "))
            })?,
        };
        if let Some(slots) = self.slots {
            cfg.slots = slots;
        }
        if let Some(seed) = self.seed {
            cfg.seed = seed;
        }
        cfg.validate().map_err(|e| e.to_string())?;
        Ok(cfg)
    }
}

/// Returns `cfg` with the field at dotted `path` set to `raw` (parsed as JSON,
/// falling back to a string).
fn with_param(cfg: &Config, path: &str, raw: &str) -> Result<Config, ConfigError> {
    let mut value = serde_json::to_value(cfg).expect("config serializes");
    let mut slot = &mut value;
    for key in path.split('.') {
        slot = slot
            .get_mut(key)
            .ok_or_else(|| ConfigError(format!("unknown parameter `{path}` (no field `{key}`)")))?;
    }
    *slot = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()));
    let out: Config =
        serde_json::from_value(value).map_err(|e| ConfigError(format!("{path} = {raw}: {e}")))?;
    out.validate()?;
    Ok(out)
}

fn main() -> ExitCode {
    match run_cli(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("epbs-sim: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_cli(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Scenarios => {
            for c in presets() {
                println!("{:<22} {}", c.name, describe(&c));
            }
            Ok(())
        }
        Command::Config { source } => {
            let cfg = source.load()?;
            println!("{}", serde_json::to_string_pretty(&cfg).expect("serializable"));
            Ok(())
        }
        Command::Run { source, format, slots_csv } => {
            let cfg = source.load()?;
            let records = run(&cfg);
            let summary = summarize(&records);
            match format {
                Format::Text => print!("{}", summary.render(&cfg.name)),
                Format::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({"config": cfg, "summary": summary}))
                        .expect("serializable")
                ),
            }
            if let Some(path) = slots_csv {
                let mut f = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                writeln!(f, "slot,proposal,builder,strategy,bid_gwei,built_on_full_parent,revealed,timely_attesters,same_slot_votes,ptc_present,ptc_absent,ptc_available,outcome,paid")
                    .map_err(|e| e.to_string())?;
                for r in &records {
                    writeln!(
                        f,
                        "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                        r.slot,
                        label(&r.proposal),
                        r.builder.map_or(String::new(), |b| b.to_string()),
                        r.strategy.map_or(String::new(), |s| label(&s)),
                        r.bid_gwei,
                        r.built_on_full_parent,
                        r.revealed,
                        r.timely_attesters,
                        r.same_slot_votes,
                        r.ptc_present,
                        r.ptc_absent,
                        r.ptc_available,
                        label(&r.outcome),
                        r.paid
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }
        Command::Sweep { source, param, values, seeds } => {
            let base = source.load()?;
            let mut jobs = Vec::new();
            for raw in &values {
                let cfg = with_param(&base, &param, raw).map_err(|e| e.to_string())?;
                for k in 0..seeds.max(1) {
                    let mut c = cfg.clone();
                    c.seed = base.seed + k;
                    jobs.push((raw.clone(), c));
                }
            }
            let results: Vec<_> = std::thread::scope(|scope| {
                let handles: Vec<_> =
                    jobs.iter().map(|(_, c)| scope.spawn(move || summarize(&run(c)))).collect();
                handles.into_iter().map(|h| h.join().expect("simulation thread")).collect()
            });
            println!(
                "param,value,seed,slots,full_rate,full,empty,orphaned,missed,paid_without_payload,honest_reveal_lost,unpaid_canonical,mean_ptc_present"
            );
            for ((raw, c), s) in jobs.iter().zip(results) {
                println!(
                    "{param},{raw},{},{},{:.4},{},{},{},{},{},{},{},{:.1}",
                    c.seed,
                    s.slots,
                    s.full_rate,
                    s.full,
                    s.empty,
                    s.orphaned,
                    s.missed,
                    s.paid_without_payload,
                    s.honest_reveal_lost,
                    s.unpaid_canonical,
                    s.mean_ptc_present
                );
            }
            Ok(())
        }
    }
}

fn describe(c: &Config) -> String {
    let d = Config::default();
    let mut parts = Vec::new();
    let a = serde_json::to_value(c).expect("serializable");
    let b = serde_json::to_value(&d).expect("serializable");
    fn diff(prefix: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, v) in x {
                    let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    diff(&p, v, &y[k], out);
                }
            }
            _ if a != b && prefix != "name" => out.push(format!("{prefix}={a}")),
            _ => {}
        }
    }
    diff("", &a, &b, &mut parts);
    if parts.is_empty() { "defaults".into() } else { parts.join(" ") }
}

/// The serde name of a unit enum variant, e.g. `on_time`.
fn label<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}
