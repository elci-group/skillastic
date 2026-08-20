//! Full-text search over registered skills.

use crate::error::Result;
use crate::registry::Registry;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub name: String,
    pub bucket: String,
    pub invocation: String,
    pub matched_in: Vec<String>,
}

pub fn search(registry: &Registry, query: &str) -> Result<Vec<SearchHit>> {
    let q = query.to_lowercase();
    let skills = registry.list_skills()?;
    let mut hits = Vec::new();

    for skill in skills {
        let mut matched_in = Vec::new();
        if skill.name.to_lowercase().contains(&q) {
            matched_in.push("name".into());
        }
        if skill.bucket.to_lowercase().contains(&q) {
            matched_in.push("bucket".into());
        }
        if skill.invocation.to_string().to_lowercase().contains(&q) {
            matched_in.push("invocation".into());
        }
        if let Ok(body) = registry.skill_body(&skill) {
            if body.to_lowercase().contains(&q) {
                matched_in.push("body".into());
            }
        }
        if skill.requires.iter().any(|r| r.to_lowercase().contains(&q)) {
            matched_in.push("requires".into());
        }

        if !matched_in.is_empty() {
            hits.push(SearchHit {
                name: skill.name,
                bucket: skill.bucket,
                invocation: skill.invocation.to_string(),
                matched_in,
            });
        }
    }

    Ok(hits)
}

pub fn hits_to_table(hits: &[SearchHit]) -> String {
    let mut out = String::from("SKILL           BUCKET        INVOCATION    MATCHED\n");
    out.push_str("--------------- ------------- ------------- ----------------\n");
    for hit in hits {
        out.push_str(&format!(
            "{:<15} {:<13} {:<13} {}\n",
            hit.name,
            hit.bucket,
            hit.invocation,
            hit.matched_in.join(", ")
        ));
    }
    out
}
