//! Docs page generation for skills.

use crate::error::{Result, SkillasticError};
use crate::registry::Registry;
use crate::templates;
use std::fs;
use std::path::PathBuf;

pub fn generate(registry: &Registry, name: &str) -> Result<PathBuf> {
    let _skill = registry.load_skill(name)?;
    let docs_dir = registry.root().join("docs");
    fs::create_dir_all(&docs_dir)?;
    let path = docs_dir.join(format!("{name}.md"));

    let mut contents = format!(
        "# {}\n\n",
        name.split('-').map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }).collect::<Vec<_>>().join(" ")
    );
    contents.push_str(templates::DOCS_PAGE_TEMPLATE);

    fs::write(&path, contents)?;
    Ok(path)
}

pub fn render(registry: &Registry, name: &str) -> Result<String> {
    let path = registry.root().join("docs").join(format!("{name}.md"));
    if !path.is_file() {
        return Err(SkillasticError::Other(format!(
            "no docs page for '{name}'; run `skillastic docs generate {name}`"
        )));
    }
    Ok(fs::read_to_string(path)?)
}
