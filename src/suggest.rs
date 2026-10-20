//! Inference-powered skill refinement suggestions.
//!
//! For each target skill, the body, metadata, usage stats, and review
//! findings are bundled into a JSON prompt and sent to one of two backends,
//! both following existing project precedent:
//!
//! - `llm_command` from `.skillastic/config.json` — an external command
//!   receiving the prompt payload on stdin, like `migrate::LlmAdapter`.
//! - Groq — `curl` subprocess with `GROQ_API_KEY`, like `audit`'s
//!   `--infer` path. Prompts use `temperature: 0`.
//!
//! Suggestions are advisory: they are stored in `.skillastic/suggestions.json`
//! and never applied to skill bodies automatically.

use crate::error::Result;
use crate::model::{
    Config, ReviewReport, Skill, SkillSuggestions, SuggestionItem, SuggestionStore,
};
use crate::registry::Registry;
use chrono::Utc;
use semver::Version;
use std::io::Write;
use std::process::{Command, Stdio};

/// Cap on skills sent to inference per run, bounding cost.
pub const SUGGEST_MAX_SKILLS: usize = 5;
const DEFAULT_MODEL: &str = "llama-3.1-8b-instant";

/// Inference backend for one suggest run.
pub enum Backend {
    /// No backend available (no `llm_command`, no `GROQ_API_KEY`).
    None,
    /// External command from config; payload on stdin, JSON on stdout.
    LlmCommand(String),
    /// Groq chat completions via curl.
    Groq,
}

/// Pick a backend: an explicit `llm_command` wins (local, user-configured),
/// then Groq when an API key is present.
pub fn resolve_backend(config: &Config) -> Backend {
    if let Some(command) = &config.llm_command {
        Backend::LlmCommand(command.clone())
    } else if std::env::var_os("GROQ_API_KEY").is_some() {
        Backend::Groq
    } else {
        Backend::None
    }
}

pub struct SuggestOutcome {
    pub store: SuggestionStore,
    /// How many skills were actually sent to a backend.
    pub attempted: usize,
}

/// Produce and persist suggestions for `targets` (already capped by the
/// caller to `SUGGEST_MAX_SKILLS`). Per-skill failures are stored on the
/// entry and never abort the batch.
pub fn run(
    registry: &Registry,
    app_version: &Version,
    targets: &[Skill],
    review: &ReviewReport,
    model: Option<&str>,
    backend: &Backend,
) -> Result<SuggestOutcome> {
    let config = registry.config()?;
    let model = model
        .or(config.suggest_model.as_deref())
        .unwrap_or(DEFAULT_MODEL)
        .to_string();

    let mut store = registry.suggestions()?;
    let mut attempted = 0;
    for skill in targets.iter().take(SUGGEST_MAX_SKILLS) {
        let entry = match backend {
            Backend::None => skipped(
                skill,
                &model,
                "no llm_command configured and GROQ_API_KEY not set",
            ),
            Backend::LlmCommand(command) => {
                attempted += 1;
                match run_llm_command(registry, skill, review, app_version, command, &model) {
                    Ok(entry) => entry,
                    Err(e) => failed(skill, "llm_command", &model, "failed_command", e),
                }
            }
            Backend::Groq => {
                attempted += 1;
                match run_groq(registry, skill, review, app_version, &model) {
                    Ok(entry) => entry,
                    Err(e) => failed(skill, "groq", &model, "failed_api", e),
                }
            }
        };
        store.upsert(entry);
    }
    registry.save_suggestions(&store)?;
    Ok(SuggestOutcome { store, attempted })
}

fn skipped(skill: &Skill, model: &str, reason: &str) -> SkillSuggestions {
    SkillSuggestions {
        skill: skill.name.clone(),
        generated_at: Utc::now(),
        backend: "none".into(),
        model: model.into(),
        status: "skipped_no_backend".into(),
        summary: String::new(),
        items: Vec::new(),
        error: Some(reason.into()),
    }
}

fn failed(
    skill: &Skill,
    backend: &str,
    model: &str,
    status: &str,
    error: impl std::fmt::Display,
) -> SkillSuggestions {
    SkillSuggestions {
        skill: skill.name.clone(),
        generated_at: Utc::now(),
        backend: backend.into(),
        model: model.into(),
        status: status.into(),
        summary: String::new(),
        items: Vec::new(),
        error: Some(error.to_string()),
    }
}

/// JSON payload describing one skill for inference; shared by both backends.
fn prompt_payload(
    registry: &Registry,
    skill: &Skill,
    review: &ReviewReport,
    app_version: &Version,
) -> serde_json::Value {
    let body = registry.skill_body(skill).unwrap_or_default();
    let skill_review = review.skills.iter().find(|r| r.skill == skill.name);
    let findings: &[crate::model::Finding] =
        skill_review.map(|r| r.findings.as_slice()).unwrap_or(&[]);
    serde_json::json!({
        "skill": {
            "name": skill.name,
            "skill_version": skill.skill_version.to_string(),
            "compatible_apps": skill.compatible_apps,
            "verified_app_version": skill.verified_app_version.to_string(),
            "confidence": skill.confidence,
            "status": skill.status.to_string(),
        },
        "body": body,
        "usage": skill_review.map(|r| &r.stats),
        "findings": findings,
        "current_app_version": app_version.to_string(),
    })
}

const PROMPT_INSTRUCTIONS: &str = "You refine AI-agent skill instruction documents. \
Suggest concrete, advisory refinements based on the provided body, usage stats, and review findings. \
Return a JSON object of the form {\"summary\": string, \"items\": [{\"title\": string, \"rationale\": string, \
\"suggested_edit\": string}]}. Keep suggestions grounded in the provided material; do not invent version \
numbers; do not rewrite the body, propose edits instead.";

/// Parse a model response into suggestions. Accepts the `{summary, items}`
/// object, a bare items array, or fenced JSON, mirroring `audit`'s leniency.
fn parse_suggestions(skill: &Skill, backend: &str, model: &str, text: &str) -> SkillSuggestions {
    let mut clean = text.trim();
    if let Some(rest) = clean.strip_prefix("```json") {
        clean = rest;
    } else if let Some(rest) = clean.strip_prefix("```") {
        clean = rest;
    }
    let clean = clean
        .trim()
        .strip_suffix("```")
        .unwrap_or(clean.trim())
        .trim();
    let value: serde_json::Value = match serde_json::from_str(clean) {
        Ok(v) => v,
        Err(e) => {
            return invalid_output(skill, backend, model, &e.to_string());
        }
    };
    let items_value = value.get("items").cloned().unwrap_or_else(|| value.clone());
    let items: Vec<SuggestionItem> = match serde_json::from_value(items_value) {
        Ok(items) => items,
        Err(e) => {
            return invalid_output(skill, backend, model, &e.to_string());
        }
    };
    let summary = value
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    SkillSuggestions {
        skill: skill.name.clone(),
        generated_at: Utc::now(),
        backend: backend.into(),
        model: model.into(),
        status: "complete".into(),
        summary,
        items,
        error: None,
    }
}

fn invalid_output(skill: &Skill, backend: &str, model: &str, error: &str) -> SkillSuggestions {
    SkillSuggestions {
        skill: skill.name.clone(),
        generated_at: Utc::now(),
        backend: backend.into(),
        model: model.into(),
        status: "invalid_model_output".into(),
        summary: String::new(),
        items: Vec::new(),
        error: Some(error.into()),
    }
}

fn run_llm_command(
    registry: &Registry,
    skill: &Skill,
    review: &ReviewReport,
    app_version: &Version,
    command: &str,
    model: &str,
) -> std::result::Result<SkillSuggestions, String> {
    let payload = serde_json::json!({
        "instructions": PROMPT_INSTRUCTIONS,
        "data": prompt_payload(registry, skill, review, app_version),
    });
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start '{command}': {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(payload.to_string().as_bytes())
        .map_err(|e| format!("could not write to '{command}': {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("'{command}' failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "'{command}' exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_suggestions(skill, "llm_command", model, &text))
}

fn run_groq(
    registry: &Registry,
    skill: &Skill,
    review: &ReviewReport,
    app_version: &Version,
    model: &str,
) -> std::result::Result<SkillSuggestions, String> {
    let key = std::env::var("GROQ_API_KEY").map_err(|_| "GROQ_API_KEY not set".to_string())?;
    let payload = prompt_payload(registry, skill, review, app_version);
    let prompt = format!("{PROMPT_INSTRUCTIONS}\n\nSkill data:\n{payload}");
    let body = serde_json::json!({
        "model": model,
        "temperature": 0,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": "You refine AI-agent skill instruction documents."},
            {"role": "user", "content": prompt},
        ],
    });
    let output = Command::new("curl")
        .args([
            "-sS",
            "https://api.groq.com/openai/v1/chat/completions",
            "-H",
            &format!("Authorization: Bearer {key}"),
            "-H",
            "Content-Type: application/json",
            "--data",
            &body.to_string(),
        ])
        .output()
        .map_err(|e| format!("could not start curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid API response: {e}"))?;
    let text = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    Ok(parse_suggestions(skill, "groq", model, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Skill, UsageKind};
    use crate::review;
    use crate::usage;
    use tempfile::TempDir;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn setup() -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path(), Config::default()).unwrap();
        let skill = Skill::new("web", v("1.0.0"), vec!["*".into()], v("1.0.0"));
        registry.add_skill(&skill, "# Web\nDo the thing.").unwrap();
        (dir, registry)
    }

    fn review_for(registry: &Registry) -> ReviewReport {
        review::run(registry, &v("1.0.0")).unwrap()
    }

    #[test]
    fn no_backend_marks_skills_skipped() {
        let (_dir, registry) = setup();
        let skills = registry.list_skills().unwrap();
        let report = review_for(&registry);
        let outcome = run(
            &registry,
            &v("1.0.0"),
            &skills,
            &report,
            None,
            &Backend::None,
        )
        .unwrap();

        assert_eq!(outcome.attempted, 0);
        let entry = &outcome.store.suggestions[0];
        assert_eq!(entry.status, "skipped_no_backend");
        assert!(entry.error.as_ref().unwrap().contains("GROQ_API_KEY"));
    }

    #[test]
    fn llm_command_produces_complete_suggestions() {
        let (_dir, registry) = setup();
        usage::record(&registry, "web", UsageKind::Invoked, None).unwrap();
        let mut config = registry.config().unwrap();
        config.llm_command =
            Some("printf '{\"summary\":\"tighten\",\"items\":[{\"title\":\"Trim\",\"rationale\":\"long\",\"suggested_edit\":\"cut step 2\"}]}'".into());
        registry.save_config(&config).unwrap();

        let targets = vec![registry.load_skill("web").unwrap()];
        let report = review_for(&registry);
        let outcome = run(
            &registry,
            &v("1.0.0"),
            &targets,
            &report,
            None,
            &Backend::LlmCommand(config.llm_command.clone().unwrap()),
        )
        .unwrap();

        assert_eq!(outcome.attempted, 1);
        let entry = outcome
            .store
            .suggestions
            .iter()
            .find(|s| s.skill == "web")
            .unwrap();
        assert_eq!(entry.status, "complete");
        assert_eq!(entry.backend, "llm_command");
        assert_eq!(entry.summary, "tighten");
        assert_eq!(entry.items.len(), 1);
        assert_eq!(entry.items[0].title, "Trim");

        // Persisted for later inspection.
        let stored = registry.suggestions().unwrap();
        assert_eq!(stored.suggestions.len(), 1);
    }

    #[test]
    fn failing_llm_command_is_stored_not_fatal() {
        let (_dir, registry) = setup();
        let skills = registry.list_skills().unwrap();
        let report = review_for(&registry);
        let outcome = run(
            &registry,
            &v("1.0.0"),
            &skills,
            &report,
            None,
            &Backend::LlmCommand("exit 3".into()),
        )
        .unwrap();

        let entry = &outcome.store.suggestions[0];
        assert_eq!(entry.status, "failed_command");
        assert!(entry.error.is_some());
    }

    #[test]
    fn malformed_model_output_is_invalid_model_output() {
        let (_dir, registry) = setup();
        let skills = registry.list_skills().unwrap();
        let report = review_for(&registry);
        let outcome = run(
            &registry,
            &v("1.0.0"),
            &skills,
            &report,
            None,
            &Backend::LlmCommand("echo 'not json'".into()),
        )
        .unwrap();

        let entry = &outcome.store.suggestions[0];
        assert_eq!(entry.status, "invalid_model_output");
    }

    #[test]
    fn store_upserts_by_skill_newest_first() {
        let mut store = SuggestionStore::default();
        for name in ["a", "b", "a"] {
            store.upsert(SkillSuggestions {
                skill: name.into(),
                generated_at: Utc::now(),
                backend: "test".into(),
                model: "test".into(),
                status: "complete".into(),
                summary: String::new(),
                items: Vec::new(),
                error: None,
            });
        }
        assert_eq!(store.suggestions.len(), 2);
        assert_eq!(store.suggestions[0].skill, "a");
    }
}
