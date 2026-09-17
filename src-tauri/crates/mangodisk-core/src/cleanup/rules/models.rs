use std::path::PathBuf;

use crate::cleanup::CleanupCategory;

/// Schema version for declarative rule sources.
///
/// Increment this value when rule semantics change incompatibly so baselines
/// and future persistent indexes cannot silently mix catalog generations.
pub(crate) const RULE_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformConstraint {
    #[cfg(target_os = "macos")]
    Macos,
    #[cfg(windows)]
    Windows,
    #[cfg(target_os = "linux")]
    Linux,
}

impl PlatformConstraint {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Self::Macos => "macos",
            #[cfg(windows)]
            Self::Windows => "windows",
            #[cfg(target_os = "linux")]
            Self::Linux => "linux",
        }
    }
}

/// Full lifecycle retained for catalog validation and future signed rule packs.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleLifecycle {
    Candidate,
    Verified,
    Stable,
    Deprecated,
    Disabled,
}

impl RuleLifecycle {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Verified => "verified",
            Self::Stable => "stable",
            Self::Deprecated => "deprecated",
            Self::Disabled => "disabled",
        }
    }
}

/// High-impact rules require a dedicated confirmation flow before registration.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRiskLevel {
    Safe,
    Recoverable,
    HighImpact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MatcherSpec {
    /// Matches an entire root and is restricted to explicit cache boundaries.
    All,
    FileOnly,
    NameEquals(Vec<String>),
    NameGlob(Vec<String>),
    ExtensionIn(Vec<String>),
    PathSegmentIn(Vec<String>),
    OlderThanDays(u64),
    NewerThanDays(u64),
    LargerThanBytes(u64),
    SmallerThanBytes(u64),
    MaxDepth(usize),
    AllOf(Vec<MatcherSpec>),
    AnyOf(Vec<MatcherSpec>),
    Not(Box<MatcherSpec>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionSpec {
    DeleteMatchingContents {
        requires_app_close: bool,
    },
    /// Deletes the declared cache root as one staged directory tree.
    ///
    /// Declarative validation restricts this strategy to explicitly
    /// rebuildable, non-default roots with an unfiltered matcher. Runtime
    /// ownership validation can still downgrade it to per-entry deletion.
    DeleteWholeRoot {
        requires_app_close: bool,
    },
}

impl ExecutionSpec {
    pub(crate) fn requires_app_close(self) -> bool {
        match self {
            Self::DeleteMatchingContents { requires_app_close }
            | Self::DeleteWholeRoot { requires_app_close } => requires_app_close,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RootSpec {
    /// Templates are resolved through controlled platform variables only.
    pub resolved_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ApplicabilityProbe {
    AnyRootExists,
    PathExists(PathBuf),
    ApplicationInstalled(Vec<String>),
    ExecutableAvailable(Vec<String>),
    ApplicationVersion {
        identifier: String,
        minimum: Option<String>,
        maximum_exclusive: Option<String>,
    },
    SystemVersion {
        minimum: Option<String>,
        maximum_exclusive: Option<String>,
    },
    FileSystemIn(Vec<String>),
    CapabilityAvailable(Vec<String>),
    ProcessRunning(Vec<String>),
    AnyOf(Vec<ApplicabilityProbe>),
    AllOf(Vec<ApplicabilityProbe>),
    Not(Box<ApplicabilityProbe>),
}

#[derive(Debug, Clone)]
pub(crate) struct VerificationMetadata {
    pub lifecycle: RuleLifecycle,
    pub evidence: String,
    pub verified_at: String,
    pub verified_platform: PlatformConstraint,
}

/// Validated source model compiled into executable filesystem rules.
#[derive(Debug, Clone)]
pub(crate) struct RuleSpec {
    pub id: String,
    pub schema_version: u32,
    pub rule_version: u32,
    pub platform: PlatformConstraint,
    pub category: CleanupCategory,
    pub risk: RuleRiskLevel,
    pub default_selected: bool,
    pub recommended_selected: bool,
    pub applicability: Vec<ApplicabilityProbe>,
    pub roots: Vec<RootSpec>,
    pub matcher: MatcherSpec,
    pub execution: ExecutionSpec,
    pub required_stopped_processes: Vec<String>,
    pub verification: VerificationMetadata,
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledRule {
    pub id: String,
    pub schema_version: u32,
    pub rule_version: u32,
    pub platform: PlatformConstraint,
    pub category: CleanupCategory,
    pub risk: RuleRiskLevel,
    pub default_selected: bool,
    pub recommended_selected: bool,
    pub applicability: Vec<ApplicabilityProbe>,
    pub roots: Vec<PathBuf>,
    pub matcher: MatcherSpec,
    pub execution: ExecutionSpec,
    pub remove_empty_directories: bool,
    pub required_stopped_processes: Vec<String>,
    pub verification: VerificationMetadata,
}

impl CompiledRule {
    pub(crate) fn requires_app_close(&self) -> bool {
        self.execution.requires_app_close()
    }

    pub(crate) fn deletes_whole_root(&self) -> bool {
        matches!(self.execution, ExecutionSpec::DeleteWholeRoot { .. })
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        id: &str,
        root: PathBuf,
        category: CleanupCategory,
        matcher: MatcherSpec,
    ) -> Self {
        let platform = {
            #[cfg(target_os = "macos")]
            {
                PlatformConstraint::Macos
            }
            #[cfg(windows)]
            {
                PlatformConstraint::Windows
            }
            #[cfg(target_os = "linux")]
            {
                PlatformConstraint::Linux
            }
        };
        Self {
            id: id.to_string(),
            schema_version: RULE_SCHEMA_VERSION,
            rule_version: 1,
            platform,
            category,
            risk: RuleRiskLevel::Safe,
            default_selected: true,
            recommended_selected: true,
            applicability: vec![ApplicabilityProbe::AnyRootExists],
            roots: vec![root],
            matcher,
            execution: ExecutionSpec::DeleteMatchingContents {
                requires_app_close: false,
            },
            remove_empty_directories: false,
            required_stopped_processes: Vec::new(),
            verification: VerificationMetadata {
                lifecycle: RuleLifecycle::Verified,
                evidence: "fixture".to_string(),
                verified_at: "2026-07-17".to_string(),
                verified_platform: platform,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn whole_root_fixture(id: &str, root: PathBuf, category: CleanupCategory) -> Self {
        let mut fixture = Self::fixture(id, root, category, MatcherSpec::All);
        fixture.execution = ExecutionSpec::DeleteWholeRoot {
            requires_app_close: false,
        };
        fixture
    }
}
