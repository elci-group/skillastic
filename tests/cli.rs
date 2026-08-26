use serde_json::Value;
use std::fs;
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
        "init",
        "setup",
        "add",
        "remove",
        "list",
        "show",
        "status",
        "capture",
        "audit",
        "migrate",
        "verify",
        "history",
        "promoted",
        "lint",
        "doctor",
        "domain-model",
        "adr",
        "docs",
        "search",
        "enroll",
        "monitor",
    ] {
        assert!(stdout.contains(command), "help omitted {command}");
    }
}

/// `enroll --dry-run` must never touch the real machine (no systemd units,
/// no binary installs, no registry writes) — it only reports what it found.
#[test]
fn enroll_dry_run_discovers_without_installing_anything() {
    let dir = TempDir::new().unwrap();
    let with_workspace = dir.path().join("has-skillastic");
    fs::create_dir_all(with_workspace.join(".git")).unwrap();
    fs::create_dir_all(with_workspace.join(".skillastic")).unwrap();
    let without_workspace = dir.path().join("plain-git");
    fs::create_dir_all(without_workspace.join(".git")).unwrap();

    let report = success_json(
        &dir,
        &[
            "enroll",
            "--dry-run",
            "--root",
            dir.path().to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["scope"], "user");
    assert!(report["binary_path"].is_null());
    assert!(report["unit_path"].is_null());

    let enrolled = report["enrolled"].as_array().unwrap();
    assert!(
        enrolled
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("has-skillastic"))
    );

    let skipped = report["skipped_no_workspace"].as_array().unwrap();
    assert!(
        skipped
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("plain-git"))
    );
    assert!(
        !enrolled
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("plain-git"))
    );

    // Dry run: no side effects anywhere, including on the discovered repo.
    assert!(!without_workspace.join(".skillastic").exists());
}

#[test]
fn enroll_dry_run_with_init_missing_lists_but_does_not_initialize() {
    let dir = TempDir::new().unwrap();
    let plain = dir.path().join("plain-git");
    fs::create_dir_all(plain.join(".git")).unwrap();

    let report = success_json(
        &dir,
        &[
            "enroll",
            "--dry-run",
            "--init-missing",
            "--root",
            dir.path().to_str().unwrap(),
            "--json",
        ],
    );
    let initialized = report["initialized"].as_array().unwrap();
    assert!(
        initialized
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("plain-git"))
    );
    assert!(
        report["skipped_no_workspace"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Still a dry run: nothing actually got initialized on disk.
    assert!(!plain.join(".skillastic").exists());
}

#[test]
fn enroll_dry_run_system_scope_works_without_root() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("repo/.git")).unwrap();

    let report = success_json(
        &dir,
        &[
            "enroll",
            "--dry-run",
            "--system",
            "--root",
            dir.path().to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(report["scope"], "system");
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
    // ask-skillastic is seeded on init, plus the skill we added.
    assert_eq!(listed.as_array().unwrap().len(), 2);
    let frontend = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "frontend")
        .expect("frontend skill should be listed");
    assert_eq!(frontend["status"], "active");

    let status = success_json(&dir, &["status", "--app-version", "1.2.3", "--json"]);
    assert_eq!(status["app_version"], "1.2.3");
    let frontend_resolution = status["resolutions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["skill"] == "frontend")
        .expect("frontend should have a resolution");
    assert_eq!(frontend_resolution["decision"], "load");

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
    // ask-skillastic is seeded on init; the broken skill must not appear.
    assert!(
        !listed
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "broken"),
        "broken skill should not be registered"
    );
}

#[test]
fn setup_creates_agent_docs() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let output = skillastic(&dir, &["setup", "--non-interactive"]);
    assert!(output.status.success());

    assert!(
        dir.path()
            .join(".skillastic/agents/issue-tracker.md")
            .is_file()
    );
    assert!(
        dir.path()
            .join(".skillastic/agents/triage-labels.md")
            .is_file()
    );
    assert!(dir.path().join(".skillastic/agents/domain.md").is_file());
}

#[test]
fn doctor_reports_workspace_health() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    // Before setup, doctor reports missing agent docs.
    let output = skillastic(&dir, &["doctor"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Workspace issues"));
    assert!(stdout.contains("missing setup file"));

    // After setup, the workspace is healthy.
    skillastic(&dir, &["setup", "--non-interactive"]);
    let output = skillastic(&dir, &["doctor"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Workspace is healthy"));
}

#[test]
fn add_with_template_seeds_workflow_skill() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let added = success_json(
        &dir,
        &[
            "add",
            "tdd",
            "--version",
            "1.0.0",
            "--compatible",
            ">=1.0.0",
            "--template",
            "tdd",
            "--invocation",
            "model",
            "--bucket",
            "engineering",
            "--app-version",
            "1.0.0",
            "--json",
        ],
    );
    assert_eq!(added["name"], "tdd");
    assert_eq!(added["bucket"], "engineering");
    assert_eq!(added["invocation"], "model_invoked");

    let listed = success_json(&dir, &["list", "--json"]);
    let tdd = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "tdd")
        .expect("tdd skill should be listed");
    assert_eq!(tdd["bucket"], "engineering");
}

#[test]
fn domain_model_and_adr_create_files() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let output = skillastic(&dir, &["domain-model"]);
    assert!(output.status.success());
    assert!(dir.path().join("CONTEXT.md").is_file());

    let output = skillastic(&dir, &["adr", "Use immutable snapshots"]);
    assert!(output.status.success());
    let adr_path = dir
        .path()
        .join(".skillastic/adr/0001-use-immutable-snapshots.md");
    assert!(adr_path.is_file());
}

#[test]
fn docs_generate_and_search_work() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let output = skillastic(&dir, &["docs", "generate", "ask-skillastic"]);
    assert!(output.status.success());
    assert!(
        dir.path()
            .join(".skillastic/docs/ask-skillastic.md")
            .is_file()
    );

    let output = skillastic(&dir, &["search", "command"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("ask-skillastic"));
}

#[test]
fn promoted_command_validates_seeded_skill() {
    let dir = TempDir::new().unwrap();
    success_json(&dir, &["init", "--app-version", "1.0.0", "--json"]);

    let promoted = success_json(&dir, &["promoted", "--json"]);
    assert!(
        promoted["skills"]
            .as_array()
            .unwrap()
            .contains(&"ask-skillastic".into())
    );
    assert!(promoted["missing"].as_array().unwrap().is_empty());
}
