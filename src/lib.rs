//! Skillastic — adaptive skill runtime for AI agents.
//!
//! Replaces static prompt/tool manifests with a version-aware,
//! continuously maintained capability layer. Five engines:
//! registry, version resolver, commit archaeology, context capture,
//! and skill migration.

pub mod appver;
pub mod archaeology;
pub mod audit;
pub mod capture;
pub mod daemon;
pub mod delta;
pub mod error;
pub mod git;
pub mod migrate;
pub mod model;
pub mod registry;
pub mod resolver;

pub use error::{Result, SkillasticError};
pub use model::SkillInvocation;
