//! Domain modeling support: CONTEXT.md and ADRs.

use crate::error::Result;
use crate::registry::Registry;
use crate::templates;
use std::fs;
use std::path::{Path, PathBuf};

/// Seed or update `CONTEXT.md` at the project root.
pub fn ensure_context_md(registry: &Registry) -> Result<PathBuf> {
    let path = registry.project_root().join("CONTEXT.md");
    if !path.exists() {
        fs::write(&path, templates::CONTEXT_MD_TEMPLATE)?;
    }
    Ok(path)
}

/// Create a new ADR in `.skillastic/adr/`.
pub fn create_adr(registry: &Registry, title: &str) -> Result<PathBuf> {
    let slug = slugify(title);
    let adr_dir = registry.root().join("adr");
    fs::create_dir_all(&adr_dir)?;

    let next_number = next_adr_number(&adr_dir)?;
    let filename = format!("{:04}-{}.md", next_number, slug);
    let path = adr_dir.join(&filename);

    let body = templates::ADR_TEMPLATE
        .replace("{number}", &format!("{:04}", next_number))
        .replace("{slug}", &slug)
        .replace("{title}", title);

    fs::write(&path, body)?;

    // Link from CONTEXT.md if it exists.
    let context_path = registry.project_root().join("CONTEXT.md");
    if context_path.is_file() {
        let mut context = fs::read_to_string(&context_path)?;
        if !context.contains("## Decisions") {
            context.push_str("\n\n## Decisions\n\n");
        }
        let link = format!(
            "- [{:04}. {}](.skillastic/adr/{})\n",
            next_number, title, filename
        );
        if !context.contains(&filename) {
            context.push_str(&link);
            fs::write(&context_path, context)?;
        }
    }

    Ok(path)
}

/// Extract candidate domain terms from a skillastic workspace's source files.
/// Very naive: looks for `pub fn`, `pub struct`, `pub enum`, `pub trait` declarations.
pub fn capture_domain_terms(project_root: &Path) -> Vec<String> {
    let src_dir = project_root.join("src");
    if !src_dir.is_dir() {
        return Vec::new();
    }

    let mut terms = Vec::new();
    if let Ok(entries) = fs::read_dir(&src_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if let Ok(contents) = fs::read_to_string(&path) {
                for line in contents.lines() {
                    let line = line.trim();
                    if let Some(stripped) = line
                        .strip_prefix("pub fn ")
                        .or_else(|| line.strip_prefix("pub async fn "))
                    {
                        if let Some(name) = stripped.split(['(', '<']).next() {
                            terms.push(name.trim().to_string());
                        }
                    } else if let Some(stripped) = line
                        .strip_prefix("pub struct ")
                        .or_else(|| line.strip_prefix("pub enum "))
                        .or_else(|| line.strip_prefix("pub trait "))
                    {
                        if let Some(name) = stripped.split(['<', '{', ';']).next() {
                            terms.push(name.trim().to_string());
                        }
                    }
                }
            }
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

/// Parse terms defined in CONTEXT.md. Returns the defined term names.
pub fn context_terms(context_path: &Path) -> Vec<String> {
    if let Ok(contents) = fs::read_to_string(context_path) {
        contents
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with("**") && line.contains("**:") {
                    line.trim_start_matches("**")
                        .split("**:")
                        .next()
                        .map(|s| s.trim().to_string())
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Check whether a skill body contradicts a CONTEXT.md term.
/// Returns pairs of (term, variant found) for cases where the body uses a
/// different string than the canonical term.
pub fn find_domain_contradictions(body: &str, terms: &[String]) -> Vec<(String, String)> {
    let mut contradictions = Vec::new();
    for term in terms {
        // Very simple heuristic: if the body contains a lowercase variant
        // and the canonical term is mixed-case, flag it.
        let lower = term.to_lowercase();
        if term != &lower && body.contains(&lower) && !body.contains(term) {
            contradictions.push((term.clone(), lower));
        }
    }
    contradictions
}

fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .replace("--", "-")
        .trim_matches('-')
        .to_string()
}

fn next_adr_number(adr_dir: &Path) -> Result<u32> {
    let mut max = 0u32;
    if adr_dir.is_dir() {
        for entry in fs::read_dir(adr_dir)? {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if let Some(num_str) = name.split('-').next() {
                if let Ok(num) = num_str.parse::<u32>() {
                    max = max.max(num);
                }
            }
        }
    }
    Ok(max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slugify_produces_clean_ids() {
        assert_eq!(
            slugify("Use immutable snapshots"),
            "use-immutable-snapshots"
        );
        assert_eq!(slugify("Foo--bar!!"), "foo-bar");
    }

    #[test]
    fn capture_domain_terms_from_rust_source() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct Resolver;\npub fn resolve() {}\npub enum Decision { A }\n",
        )
        .unwrap();
        let terms = capture_domain_terms(dir.path());
        assert!(terms.contains(&"Resolver".into()));
        assert!(terms.contains(&"resolve".into()));
        assert!(terms.contains(&"Decision".into()));
    }

    #[test]
    fn context_terms_parse_bold_definitions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CONTEXT.md");
        fs::write(
            &path,
            "## Language\n\n**Application version**:\nThe version string.\n",
        )
        .unwrap();
        let terms = context_terms(&path);
        assert_eq!(terms, vec!["Application version"]);
    }
}
