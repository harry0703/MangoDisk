//! Bundled prompt schema shared by build validation and runtime loading.
//! Stable fields select instructions; Markdown headings and comments are only prose.
use std::ops::Deref;

use serde::Deserialize;

/// Trim TOML framing whitespace once without changing Markdown inside the text.
/// Invalid bundled text must fail the build, not a user's first explanation request.
#[derive(Deserialize)]
#[serde(try_from = "String")]
pub(super) struct PromptText(String);

impl TryFrom<String> for PromptText {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if value.is_empty() {
            return Err("prompt text must not be empty");
        }
        Ok(Self(value.to_owned()))
    }
}

impl Deref for PromptText {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SystemPrompts {
    pub(super) shared: PromptText,
    pub(super) connection_test: PromptText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CleanupPrompts {
    pub(super) general: PromptText,
    pub(super) scan_results: PromptText,
    pub(super) remove_architecture: PromptText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivacyPrompts {
    pub(super) general: PromptText,
    pub(super) permission_required: PromptText,
    pub(super) unavailable: PromptText,
    pub(super) application_running: PromptText,
    pub(super) empty: PromptText,
    pub(super) windows_network_history: PromptText,
    pub(super) history: PromptText,
    pub(super) editor_history: PromptText,
    pub(super) current_clipboard: PromptText,
    pub(super) recent_items: PromptText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartupPrompts {
    pub(super) general: PromptText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SystemOptimizationPrompts {
    pub(super) general: PromptText,
    pub(super) active: PromptText,
    pub(super) inactive: PromptText,
    pub(super) restore_defaults: PromptText,
    pub(super) apply_optimization: PromptText,
    pub(super) unavailable: PromptText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SystemMaintenancePrompts {
    pub(super) general: PromptText,
    pub(super) healthy: PromptText,
    pub(super) unavailable: PromptText,
    pub(super) system_integrity: PromptText,
    pub(super) update_services: PromptText,
    pub(super) print_queue: PromptText,
    pub(super) system_disk: PromptText,
}

pub(super) struct PromptCatalog {
    pub(super) system: SystemPrompts,
    pub(super) cleanup: CleanupPrompts,
    pub(super) privacy: PrivacyPrompts,
    pub(super) startup: StartupPrompts,
    pub(super) system_optimization: SystemOptimizationPrompts,
    pub(super) system_maintenance: SystemMaintenancePrompts,
}

impl PromptCatalog {
    /// Named sections support precise build errors and selection-coverage tests.
    pub(super) fn sections(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("system.shared", &self.system.shared),
            ("system.connection_test", &self.system.connection_test),
            ("cleanup.general", &self.cleanup.general),
            ("cleanup.scan_results", &self.cleanup.scan_results),
            (
                "cleanup.remove_architecture",
                &self.cleanup.remove_architecture,
            ),
            ("privacy.general", &self.privacy.general),
            (
                "privacy.permission_required",
                &self.privacy.permission_required,
            ),
            ("privacy.unavailable", &self.privacy.unavailable),
            (
                "privacy.application_running",
                &self.privacy.application_running,
            ),
            ("privacy.empty", &self.privacy.empty),
            (
                "privacy.windows_network_history",
                &self.privacy.windows_network_history,
            ),
            ("privacy.history", &self.privacy.history),
            ("privacy.editor_history", &self.privacy.editor_history),
            ("privacy.current_clipboard", &self.privacy.current_clipboard),
            ("privacy.recent_items", &self.privacy.recent_items),
            ("startup.general", &self.startup.general),
            (
                "system-optimization.general",
                &self.system_optimization.general,
            ),
            (
                "system-optimization.active",
                &self.system_optimization.active,
            ),
            (
                "system-optimization.inactive",
                &self.system_optimization.inactive,
            ),
            (
                "system-optimization.restore_defaults",
                &self.system_optimization.restore_defaults,
            ),
            (
                "system-optimization.apply_optimization",
                &self.system_optimization.apply_optimization,
            ),
            (
                "system-optimization.unavailable",
                &self.system_optimization.unavailable,
            ),
            (
                "system-maintenance.general",
                &self.system_maintenance.general,
            ),
            (
                "system-maintenance.healthy",
                &self.system_maintenance.healthy,
            ),
            (
                "system-maintenance.unavailable",
                &self.system_maintenance.unavailable,
            ),
            (
                "system-maintenance.system_integrity",
                &self.system_maintenance.system_integrity,
            ),
            (
                "system-maintenance.update_services",
                &self.system_maintenance.update_services,
            ),
            (
                "system-maintenance.print_queue",
                &self.system_maintenance.print_queue,
            ),
            (
                "system-maintenance.system_disk",
                &self.system_maintenance.system_disk,
            ),
        ]
        .into_iter()
        .map(|(field, text)| (field, &**text))
    }

    fn validate_placeholders(&self) -> Result<(), String> {
        for (field, text) in self.sections() {
            if field == "system.shared" {
                if text.matches("{{language}}").count() != 1 {
                    return Err(format!(
                        "{field}: expected exactly one language placeholder"
                    ));
                }
                let remaining = text.replace("{{language}}", "");
                if remaining.contains("{{") || remaining.contains("}}") {
                    return Err(format!("{field}: unsupported placeholder"));
                }
            } else if text.contains("{{") || text.contains("}}") {
                return Err(format!(
                    "{field}: placeholders are only allowed in system.shared"
                ));
            }
        }
        Ok(())
    }
}

fn parse_document<T: serde::de::DeserializeOwned>(name: &str, source: &str) -> Result<T, String> {
    toml::from_str(source).map_err(|error| format!("{name}.toml: {error}"))
}

/// Called during every affected build and once per process by the prompt adapter.
/// Sources are embedded, so runtime cannot observe a different file from validation.
pub(super) fn load() -> Result<PromptCatalog, String> {
    let catalog = PromptCatalog {
        system: parse_document("system", include_str!("prompts/system.toml"))?,
        cleanup: parse_document("cleanup", include_str!("prompts/cleanup.toml"))?,
        privacy: parse_document("privacy", include_str!("prompts/privacy.toml"))?,
        startup: parse_document("startup", include_str!("prompts/startup.toml"))?,
        system_optimization: parse_document(
            "system-optimization",
            include_str!("prompts/system-optimization.toml"),
        )?,
        system_maintenance: parse_document(
            "system-maintenance",
            include_str!("prompts/system-maintenance.toml"),
        )?,
    };
    catalog.validate_placeholders()?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_markdown_headings_do_not_control_selection() {
        let source = "# Maintainer note\r\ngeneral = '''\r\n## Renamed title\r\nText with C:\\Cache and `code`.\r\n\r\n### More detail\r\nKeep this paragraph.\r\n'''\r\n";
        let document: StartupPrompts = parse_document("startup", source).unwrap();
        assert_eq!(
            &*document.general,
            "## Renamed title\r\nText with C:\\Cache and `code`.\r\n\r\n### More detail\r\nKeep this paragraph."
        );
    }

    #[test]
    fn invalid_document_fields_fail_with_source_diagnostics() {
        for (source, reason) in [
            ("", "missing field"),
            ("genral = 'text'", "unknown field"),
            ("general = 'text'\nextra = 'text'", "unknown field"),
            ("general = 'one'\ngeneral = 'two'", "duplicate key"),
            ("general = '   '", "must not be empty"),
            ("general = 42", "invalid type"),
        ] {
            let error = parse_document::<StartupPrompts>("startup", source)
                .err()
                .expect("invalid prompt must be rejected");
            assert!(error.contains("startup.toml"), "{error}");
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn placeholder_mistakes_are_rejected_before_requests() {
        for replacement in ["", "{{locale}}", "{{language}}{{language}}"] {
            let mut catalog = load().unwrap();
            catalog.system.shared.0 = catalog.system.shared.replace("{{language}}", replacement);
            assert!(catalog.validate_placeholders().is_err());
        }
        let mut catalog = load().unwrap();
        catalog.cleanup.general.0.push_str("{{language}}");
        assert!(catalog.validate_placeholders().is_err());
    }
}
