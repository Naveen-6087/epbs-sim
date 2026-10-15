use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_epbs-sim"))
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn lists_and_prints_scenarios() {
    let out = stdout(&run(&["scenarios"]));
    assert!(out.lines().any(|l| l.starts_with("baseline") && l.ends_with("defaults")));
    assert!(out.contains("withholding-builders   builders.withhold_fraction=0.25"));
    let cfg: serde_json::Value =
        serde_json::from_str(&stdout(&run(&["config", "--scenario", "partition"]))).unwrap();
    assert_eq!(cfg["network"]["partition_fraction"], 0.3);
}

#[test]
fn run_text_json_and_slot_csv() {
    let text = stdout(&run(&["run", "--slots", "30"]));
    assert!(text.starts_with("scenario: baseline\nslots: 30\n"));
    let csv = std::env::temp_dir().join(format!("epbs-slots-{}.csv", std::process::id()));
    let out = run(&["run", "--slots", "30", "--format", "json", "--slots-csv", &csv.to_string_lossy()]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["summary"]["slots"], 30);
    assert_eq!(v["config"]["name"], "baseline");
    let rows = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(rows.lines().count(), 31);
    assert!(rows.lines().nth(1).unwrap().starts_with("1,on_time,"));
    std::fs::remove_file(csv).unwrap();
}

#[test]
fn config_files_and_overrides() {
    let out = run(&[
        "run",
        "--config",
        "scenarios/mixed-adversaries.json",
        "--slots",
        "40",
        "--seed",
        "9",
        "--format",
        "json",
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["config"]["name"], "mixed-adversaries");
    assert_eq!(v["config"]["seed"], 9);
    assert_eq!(v["summary"]["slots"], 40);
}

#[test]
fn sweep_rows_are_ordered_and_reproducible() {
    let args = [
        "sweep",
        "--param",
        "builders.withhold_fraction",
        "--values",
        "0,0.5",
        "--slots",
        "40",
        "--seeds",
        "2",
    ];
    let a = stdout(&run(&args));
    assert_eq!(a, stdout(&run(&args)));
    let rows: Vec<&str> = a.lines().collect();
    assert_eq!(rows.len(), 5);
    assert!(rows[1].starts_with("builders.withhold_fraction,0,1,40,1.0000"));
    assert!(rows[4].starts_with("builders.withhold_fraction,0.5,2,"));
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec!["run", "--scenario", "nope"],
        vec!["run", "--config", "missing.json"],
        vec!["sweep", "--param", "nope.field", "--values", "1"],
        vec!["sweep", "--param", "builders.withhold_fraction", "--values", "2.0"],
        vec!["run", "--slots", "0"],
    ] {
        assert_eq!(run(&args).status.code(), Some(2), "{args:?}");
    }
}
