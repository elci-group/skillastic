use serde_json::Value;
use std::process::{Command, Output};
use tempfile::TempDir;

fn skillastic(dir: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skillastic"))
        .args(args)
        .current_dir(dir.path())
        .output()
        .expect("skillastic binary should run")
}

fn success_json(dir: &TempDir, args: &[&str]) -> Value {
    let output = skillastic(dir, args);
    assert!(
        output.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

#[test]
fn help_exposes_the_core_workflow() {
    let dir = TempDir::new().unwrap();
    let output = skillastic(&dir, &["--help"]);
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in [
        "init", "add", "status", "capture", "audit", "migrate", "verify", "history",
    ] {
        assert!(stdout.contains(command), "help omitted {command}");
    }
}

#[test]
fn json_lifecycle_round_trip() {
    let dir = TempDir::new().unwrap();

    let initialized = success_json(
        &dir,
        &[
            "init",
            "--app-name",
            "demo",
            "--app-version",
            "1.2.3",
            "--json",
        ],
    );
    assert_eq!(initialized["app_name"], "demo");

    let added = success_json(
        &dir,
        &[
            "add",
            "frontend",
            "--version",
            "1.0.0",
            "--compatible",
            ">=1.0.0, <2.0.0",
            "--verify",
            "--app-version",
            "1.2.3",
            "--json",
        ],
    );
    assert_eq!(added["name"], "frontend");
    assert_eq!(added["status"], "active");
    assert_eq!(added["verified_app_version"], "1.2.3");

    let listed = success_json(&dir, &["list", "--json"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["name"], "frontend");

    let status = success_json(&dir, &["status", "--app-version", "1.2.3", "--json"]);
    assert_eq!(status["app_version"], "1.2.3");
    assert_eq!(status["resolutions"][0]["decision"], "load");

    let history = success_json(&dir, &["history", "frontend", "--json"]);
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["name"], "frontend");

    let events = success_json(&dir, &["events", "--json"]);
    assert_eq!(events, Value::Array(Vec::new()));
}

#[test]
fn uninitialized_workspace_returns_an_actionable_error() {
    let dir = TempDir::new().unwrap();
    let output = skillastic(&dir, &["list"]);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("workspace not initialized"));
    assert!(stderr.contains("skillastic init"));
}

#[test]
fn invalid_compatibility_range_is_rejected_without_registering_a_skill() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let output = skillastic(
        &dir,
        &[
            "add",
            "broken",
            "--version",
            "1.0.0",
            "--compatible",
            "definitely-not-semver",
            "--app-version",
            "1.0.0",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid version requirement")
    );

    let listed = success_json(&dir, &["list", "--json"]);
    assert!(listed.as_array().unwrap().is_empty());
}
