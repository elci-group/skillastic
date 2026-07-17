use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillasticError {
    #[error("workspace not initialized (missing {0}); run `skillastic init` first")]
    NotInitialized(String),

    #[error("skill not found: {0}")]
    SkillNotFound(String),

    #[error("skill already exists: {0}")]
    SkillExists(String),

    #[error("invalid semver: {0}")]
    Semver(#[from] semver::Error),

    #[error("invalid version requirement '{0}': {1}")]
    VersionReq(String, String),

    #[error("git error: {0}")]
    Git(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("llm adapter error: {0}")]
    Llm(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SkillasticError>;
