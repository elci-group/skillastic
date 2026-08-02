//! Version Resolver: deterministic skill/app compatibility decisions.
//!
//! Decision matrix (from the Skillastic spec):
//!
//! | Situation                                             | Decision      |
//! |-------------------------------------------------------|---------------|
//! | App patch update (in range)                           | Load          |
//! | App minor update (in range)                           | Validate      |
//! | App major update / out of `compatible_apps`           | Migrate       |
//! | Dep change beyond what the bump explains, or downgrade| DeepAnalysis  |
//! | No valid compatibility range on the skill             | Incompatible  |

use crate::archaeology::Archaeology;
use crate::error::{Result, SkillasticError};
use crate::model::{Decision, Resolution, Skill, VersionDelta};
use semver::{Version, VersionReq};

/// Knowledge of dependency movement between the two app versions.
/// `None` = unknown (no git history available).
pub type DepsChanged = Option<bool>;

/// Resolves skills against an app version, computing the dependency-change
/// signal per skill from git archaeology when history is available.
pub struct Resolver<'a> {
    arch: Option<&'a Archaeology>,
    to_ref: Option<String>,
}

impl<'a> Resolver<'a> {
    /// `arch` may be None (project not under git); the resolver then works
    /// purely from semver, with the dependency signal unknown.
    pub fn new(arch: Option<&'a Archaeology>, to_app: &Version) -> Self {
        let to_ref = arch.and_then(|a| {
            a.git()
                .resolve_ref(&to_app.to_string())
                .or_else(|| a.git().resolve_ref("HEAD"))
        });
        Self { arch, to_ref }
    }

    /// Did dependencies move between this skill's verified app version and
    /// the target? None when there is no usable git history.
    pub fn deps_changed_for(&self, from_app: &Version) -> DepsChanged {
        let arch = self.arch?;
        let from_ref = arch.git().resolve_ref(&from_app.to_string())?;
        Some(arch.deps_changed(&from_ref, self.to_ref.as_deref()?))
    }

    pub fn resolve(&self, skill: &Skill, to_app: &Version) -> Result<Resolution> {
        resolve(
            skill,
            to_app,
            self.deps_changed_for(&skill.verified_app_version),
        )
    }

    pub fn resolve_all(&self, skills: &[Skill], to_app: &Version) -> Result<Vec<Resolution>> {
        skills.iter().map(|s| self.resolve(s, to_app)).collect()
    }
}

pub fn resolve(skill: &Skill, to_app: &Version, deps_changed: DepsChanged) -> Result<Resolution> {
    let from_app = skill.verified_app_version.clone();
    let delta = VersionDelta::between(&from_app, to_app);

    let mut saw_valid_req = false;
    let mut in_range = false;
    for raw in &skill.compatible_apps {
        if let Ok(req) = parse_req(raw) {
            saw_valid_req = true;
            if req.matches(to_app) {
                in_range = true;
            }
        }
    }

    let (decision, reason) = if !skill.compatible_apps.is_empty() && !saw_valid_req {
        (
            Decision::Incompatible,
            "no parseable semver range in compatible_apps".to_string(),
        )
    } else if delta == VersionDelta::Downgrade {
        (
            Decision::DeepAnalysis,
            format!("app version moved backwards ({from_app} -> {to_app})"),
        )
    } else if !in_range && saw_valid_req {
        (
            Decision::Migrate,
            format!(
                "app {to_app} is outside compatible range {:?}",
                skill.compatible_apps
            ),
        )
    } else {
        match delta {
            VersionDelta::Same => (Decision::Load, format!("skill verified for app {to_app}")),
            VersionDelta::Patch => match deps_changed {
                Some(true) => (
                    Decision::DeepAnalysis,
                    format!(
                        "dependencies changed on a patch bump ({from_app} -> {to_app}); \
                         deeper analysis required"
                    ),
                ),
                Some(false) => (
                    Decision::Load,
                    format!("patch bump {from_app} -> {to_app}, dependencies unchanged"),
                ),
                None => (
                    Decision::Load,
                    format!("patch bump {from_app} -> {to_app} (dependency state unknown)"),
                ),
            },
            VersionDelta::Minor => (
                Decision::Validate,
                format!("minor bump {from_app} -> {to_app}; skill loads but needs validation"),
            ),
            VersionDelta::Major => (
                Decision::Migrate,
                format!("major bump {from_app} -> {to_app} within compatible range"),
            ),
            VersionDelta::Downgrade => unreachable!("handled above"),
        }
    };

    Ok(Resolution {
        skill: skill.name.clone(),
        from_app,
        to_app: to_app.clone(),
        decision,
        reason,
    })
}

pub fn resolve_all(
    skills: &[Skill],
    to_app: &Version,
    deps_changed: DepsChanged,
) -> Result<Vec<Resolution>> {
    skills
        .iter()
        .map(|s| resolve(s, to_app, deps_changed))
        .collect()
}

/// Parse a semver requirement. Accepts the spec's space-separated form
/// (`">=2.1.0 <3.0.0"`) in addition to the standard comma form.
pub fn parse_req(raw: &str) -> Result<VersionReq> {
    let trimmed = raw.trim();
    let normalized = if trimmed.contains(',') || !trimmed.contains(' ') {
        trimmed.to_string()
    } else {
        trimmed.split_whitespace().collect::<Vec<_>>().join(", ")
    };
    VersionReq::parse(&normalized)
        .map_err(|e| SkillasticError::VersionReq(raw.to_string(), e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn skill(range: &str, verified: &str) -> Skill {
        Skill::new("s", v("1.0.0"), vec![range.into()], v(verified))
    }

    fn decide(skill: &Skill, to: &str, deps: DepsChanged) -> Decision {
        resolve(skill, &v(to), deps).unwrap().decision
    }

    #[test]
    fn spec_matrix() {
        let s = skill(">=2.1.0, <3.0.0", "2.4.1");
        // same version -> load
        assert_eq!(decide(&s, "2.4.1", None), Decision::Load);
        // patch update -> load
        assert_eq!(decide(&s, "2.4.2", Some(false)), Decision::Load);
        // minor update -> validate
        assert_eq!(decide(&s, "2.5.0", None), Decision::Validate);
        // major update, out of range -> migrate
        assert_eq!(decide(&s, "3.0.0", None), Decision::Migrate);
        // unknown dep change on patch -> deep analysis
        assert_eq!(decide(&s, "2.4.2", Some(true)), Decision::DeepAnalysis);
        // downgrade -> deep analysis
        assert_eq!(decide(&s, "2.3.0", None), Decision::DeepAnalysis);
    }

    #[test]
    fn major_bump_inside_a_spanning_range_migrates() {
        let s = skill(">=2.0.0, <4.0.0", "2.9.0");
        assert_eq!(decide(&s, "3.0.0", None), Decision::Migrate);
    }

    #[test]
    fn garbage_range_is_incompatible() {
        let s = skill("not-a-version", "2.4.1");
        assert_eq!(decide(&s, "2.4.1", None), Decision::Incompatible);
    }

    #[test]
    fn space_separated_range_parses() {
        let req = parse_req(">=2.1.0 <3.0.0").unwrap();
        assert!(req.matches(&v("2.4.1")));
        assert!(!req.matches(&v("3.0.0")));
        // standard forms still work
        assert!(parse_req("^2.1").unwrap().matches(&v("2.9.9")));
        assert!(parse_req(">=1.0.0, <2.0.0").unwrap().matches(&v("1.5.0")));
    }
}
