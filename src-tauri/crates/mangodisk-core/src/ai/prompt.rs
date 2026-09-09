use super::context::{AiContext, AiPlatform, AiSubject};
use crate::privacy::{PrivacyCapabilityState, PrivacyDataKind};
use crate::system_maintenance::SystemMaintenanceStatus;
use crate::system_settings::{SystemSettingStatus, SystemSettingTargetState};

// The build script validates these same embedded assets. Parse them only once;
// request handling uses typed fields and never reads files or Markdown headings.
static PROMPTS: std::sync::LazyLock<super::prompt_schema::PromptCatalog> =
    std::sync::LazyLock::new(|| super::prompt_schema::load().expect("build-validated AI prompts"));

/// Module guidance describes native semantics; it never grants model output
/// authority to select items, change capabilities or execute an operation.
pub(super) fn system_prompt(language: &str, context: Option<&AiContext>) -> String {
    let mut prompt = PROMPTS.system.shared.replace("{{language}}", language);
    let sections = [
        module_guidance(context.map(|context| &context.subject)),
        context.map(scope_guidance).unwrap_or_default(),
        context.map(state_guidance).unwrap_or_default(),
    ];
    // Templates are bundled at compile time. Only the validated language tag is
    // substituted; item metadata stays in the separate, untrusted user message.
    for section in sections {
        if !section.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(section);
        }
    }
    prompt
}

/// Keep domain facts separate from shared language, presentation and safety instructions.
fn module_guidance(subject: Option<&AiSubject>) -> &'static str {
    match subject {
        None => &PROMPTS.system.connection_test,
        Some(AiSubject::Cleanup { .. }) => &PROMPTS.cleanup.general,
        Some(AiSubject::Privacy { .. }) => &PROMPTS.privacy.general,
        Some(AiSubject::Startup { .. }) => &PROMPTS.startup.general,
        Some(AiSubject::SystemOptimization { .. }) => &PROMPTS.system_optimization.general,
        Some(AiSubject::SystemMaintenance { .. }) => &PROMPTS.system_maintenance.general,
    }
}

/// State-specific facts take precedence over general advice. In particular,
/// unreadable privacy data must not be described as empty from its zero count.
fn state_guidance(context: &AiContext) -> &'static str {
    match &context.subject {
        AiSubject::Cleanup { .. } => &PROMPTS.cleanup.scan_results,
        AiSubject::Privacy {
            capability: PrivacyCapabilityState::PermissionRequired,
            ..
        } => &PROMPTS.privacy.permission_required,
        AiSubject::Privacy {
            capability:
                PrivacyCapabilityState::Unsupported
                | PrivacyCapabilityState::SchemaUnsupported
                | PrivacyCapabilityState::Unavailable,
            ..
        } => &PROMPTS.privacy.unavailable,
        AiSubject::Privacy {
            capability:
                PrivacyCapabilityState::BrowserRunning | PrivacyCapabilityState::ApplicationRunning,
            item_count: 0,
            ..
        } => &PROMPTS.privacy.application_running,
        AiSubject::Privacy {
            capability: PrivacyCapabilityState::Empty,
            item_count: 0,
            ..
        } => &PROMPTS.privacy.empty,
        AiSubject::SystemOptimization {
            status: SystemSettingStatus::Optimized,
            pending_target: None,
            ..
        } => &PROMPTS.system_optimization.active,
        AiSubject::SystemOptimization {
            status: SystemSettingStatus::Recommended,
            pending_target: None,
            ..
        } => &PROMPTS.system_optimization.inactive,
        AiSubject::SystemOptimization {
            pending_target: Some(SystemSettingTargetState::Default),
            ..
        } => &PROMPTS.system_optimization.restore_defaults,
        AiSubject::SystemOptimization {
            pending_target: Some(SystemSettingTargetState::Optimized),
            ..
        } => &PROMPTS.system_optimization.apply_optimization,
        _ => "",
    }
}

/// Operational scope comes from the implementation, not guesses from broad UI
/// labels. Keep these distinctions close to the AI adapter and regression tests.
fn scope_guidance(context: &AiContext) -> &'static str {
    match &context.subject {
        AiSubject::Cleanup { scan, .. } if scan.rule_id == "special.macos-universal-binaries" => {
            &PROMPTS.cleanup.remove_architecture
        }
        AiSubject::Privacy {
            kind: PrivacyDataKind::NetworkConnectionHistory,
            ..
        } if matches!(context.platform, AiPlatform::Windows) => {
            &PROMPTS.privacy.windows_network_history
        }
        AiSubject::Privacy {
            kind:
                PrivacyDataKind::BrowsingHistory
                | PrivacyDataKind::SearchHistory
                | PrivacyDataKind::DownloadHistory,
            ..
        } => &PROMPTS.privacy.history,
        AiSubject::Privacy {
            kind: PrivacyDataKind::EditorLocalHistory,
            ..
        } => &PROMPTS.privacy.editor_history,
        AiSubject::Privacy {
            kind: PrivacyDataKind::CurrentClipboard,
            ..
        } => &PROMPTS.privacy.current_clipboard,
        AiSubject::Privacy {
            kind:
                PrivacyDataKind::RecentDocuments
                | PrivacyDataKind::RecentItems
                | PrivacyDataKind::RecentPaths,
            ..
        } => &PROMPTS.privacy.recent_items,
        AiSubject::SystemOptimization {
            status: SystemSettingStatus::Unavailable,
            ..
        } => &PROMPTS.system_optimization.unavailable,
        AiSubject::SystemMaintenance {
            status: SystemMaintenanceStatus::Healthy,
            ..
        } => &PROMPTS.system_maintenance.healthy,
        AiSubject::SystemMaintenance {
            status: SystemMaintenanceStatus::Unavailable,
            ..
        } => &PROMPTS.system_maintenance.unavailable,
        AiSubject::SystemMaintenance { task_id, .. }
            if task_id == "windows.maintenance.system-integrity" =>
        {
            &PROMPTS.system_maintenance.system_integrity
        }
        AiSubject::SystemMaintenance { task_id, .. }
            if task_id == "windows.maintenance.update-components" =>
        {
            &PROMPTS.system_maintenance.update_services
        }
        AiSubject::SystemMaintenance { task_id, .. }
            if task_id == "windows.maintenance.print-queue" =>
        {
            &PROMPTS.system_maintenance.print_queue
        }
        AiSubject::SystemMaintenance { task_id, .. }
            if task_id == "windows.maintenance.system-disk" =>
        {
            &PROMPTS.system_maintenance.system_disk
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(index: usize) -> AiContext {
        let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/ai-context-v2.json"
        ))
        .unwrap();
        serde_json::from_value(fixtures[index].clone()).unwrap()
    }

    #[test]
    fn native_conditions_select_every_bundled_section() {
        use std::collections::HashSet;

        let mut selected = HashSet::from([&*PROMPTS.system.shared, module_guidance(None)]);
        let mut visit = |item: &AiContext| {
            for section in [
                module_guidance(Some(&item.subject)),
                scope_guidance(item),
                state_guidance(item),
            ] {
                if !section.is_empty() {
                    selected.insert(section);
                }
            }
        };
        for index in 0..5 {
            visit(&context(index));
        }
        let mut cleanup = context(0);
        if let AiSubject::Cleanup { scan, .. } = &mut cleanup.subject {
            scan.rule_id = "special.macos-universal-binaries".into();
        }
        visit(&cleanup);

        let mut privacy = context(1);
        privacy.platform = AiPlatform::Windows;
        for kind in [
            PrivacyDataKind::NetworkConnectionHistory,
            PrivacyDataKind::BrowsingHistory,
            PrivacyDataKind::EditorLocalHistory,
            PrivacyDataKind::CurrentClipboard,
            PrivacyDataKind::RecentItems,
        ] {
            for capability in [
                PrivacyCapabilityState::PermissionRequired,
                PrivacyCapabilityState::Unavailable,
                PrivacyCapabilityState::ApplicationRunning,
                PrivacyCapabilityState::Empty,
            ] {
                if let AiSubject::Privacy {
                    kind: value,
                    capability: state,
                    item_count,
                    ..
                } = &mut privacy.subject
                {
                    *value = kind;
                    *state = capability;
                    *item_count = 0;
                }
                visit(&privacy);
            }
        }
        let mut optimization = context(3);
        for status in [
            SystemSettingStatus::Recommended,
            SystemSettingStatus::Optimized,
            SystemSettingStatus::Unavailable,
        ] {
            for target in [
                None,
                Some(SystemSettingTargetState::Default),
                Some(SystemSettingTargetState::Optimized),
            ] {
                if let AiSubject::SystemOptimization {
                    status: value,
                    pending_target,
                    ..
                } = &mut optimization.subject
                {
                    *value = status;
                    *pending_target = target;
                }
                visit(&optimization);
            }
        }
        let mut maintenance = context(4);
        for status in [
            SystemMaintenanceStatus::Available,
            SystemMaintenanceStatus::Healthy,
            SystemMaintenanceStatus::Unavailable,
        ] {
            for task in [
                "windows.maintenance.system-integrity",
                "windows.maintenance.update-components",
                "windows.maintenance.print-queue",
                "windows.maintenance.system-disk",
            ] {
                if let AiSubject::SystemMaintenance {
                    status: value,
                    task_id,
                    ..
                } = &mut maintenance.subject
                {
                    *value = status;
                    *task_id = task.into();
                }
                visit(&maintenance);
            }
        }
        // New prompt fields must be reachable through typed native conditions.
        let bundled = PROMPTS.sections().map(|(_, text)| text).collect();
        assert_eq!(selected, bundled);
    }

    #[test]
    fn output_language_is_explicit_independently_of_item_language() {
        for tag in [
            "en-US", "zh-CN", "zh-TW", "ja-JP", "fr-FR", "pt-BR", "zh-Hant",
        ] {
            for index in 0..5 {
                let mut item = context(index);
                item.title = "\u{6d4f}\u{89c8}\u{5668}\u{7f13}\u{5b58}".into();
                let prompt = system_prompt(tag, Some(&item));
                assert!(prompt.contains(&format!("REQUIRED OUTPUT LANGUAGE: {tag}")));
                assert!(!prompt.contains("{{language}}"));
                assert!(prompt.contains("Never infer the response language from the input data"));
            }
        }
    }

    #[test]
    fn every_module_requests_compact_markdown_without_action_authority() {
        for index in 0..5 {
            let prompt = system_prompt("zh-CN", Some(&context(index)));
            assert!(prompt.contains("Use concise Markdown"));
            assert!(prompt.contains("blank line before lists"));
            assert!(prompt.contains("Bold decision-relevant phrases"));
            assert!(prompt.contains("HTML, commands or fenced code blocks"));
            assert!(!prompt.contains("Use plain text"));
            assert!(!prompt.contains("Used by every module"));
            assert!(!prompt.contains("Keep section headings stable"));
            assert!(!prompt.contains("BCP 47 language tag"));
        }
    }

    #[test]
    fn history_scope_does_not_imply_session_or_wifi_removal() {
        let mut item = context(1);
        if let AiSubject::Privacy { kind, .. } = &mut item.subject {
            *kind = PrivacyDataKind::BrowsingHistory;
        }
        assert!(scope_guidance(&item).contains("not cookies"));
        if let AiSubject::Privacy { kind, .. } = &mut item.subject {
            *kind = PrivacyDataKind::NetworkConnectionHistory;
        }
        item.platform = AiPlatform::Windows;
        assert!(scope_guidance(&item).contains("Map Network Drive MRU"));
        item.platform = AiPlatform::Macos;
        assert!(scope_guidance(&item).is_empty());
    }

    #[test]
    fn privacy_unreadable_zero_counts_are_not_empty_results() {
        let mut item = context(1);
        for capability in [
            PrivacyCapabilityState::PermissionRequired,
            PrivacyCapabilityState::Unsupported,
            PrivacyCapabilityState::SchemaUnsupported,
            PrivacyCapabilityState::Unavailable,
            PrivacyCapabilityState::BrowserRunning,
            PrivacyCapabilityState::ApplicationRunning,
        ] {
            if let AiSubject::Privacy {
                capability: state,
                item_count,
                estimated_bytes,
                ..
            } = &mut item.subject
            {
                *state = capability;
                *item_count = 0;
                *estimated_bytes = 0;
            }
            assert!(state_guidance(&item).starts_with("Observed limitation:"));
            assert!(!state_guidance(&item).contains("No cleanup is currently needed"));
            assert!(system_prompt("zh-CN", Some(&item)).ends_with(state_guidance(&item)));
        }
        if let AiSubject::Privacy { capability, .. } = &mut item.subject {
            *capability = PrivacyCapabilityState::Empty;
        }
        assert!(state_guidance(&item)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("selected scope and time range"));
        assert!(state_guidance(&item).contains("No cleanup is currently needed"));
    }

    #[test]
    fn cleanup_does_not_turn_missing_process_details_into_a_machine_diagnosis() {
        let item = context(0);
        let prompt = system_prompt("zh-CN", Some(&item));
        assert!(prompt.ends_with(state_guidance(&item)));
        assert!(state_guidance(&item).contains("NEVER a full process scan"));
        assert!(state_guidance(&item).contains("Omit missing source or diagnostic details"));
    }

    #[test]
    fn optimization_separates_existing_behavior_from_pending_selection() {
        let mut item = context(3);
        if let AiSubject::SystemOptimization {
            status,
            pending_target,
            ..
        } = &mut item.subject
        {
            *status = SystemSettingStatus::Optimized;
            *pending_target = None;
        }
        assert!(state_guidance(&item).contains("ALREADY active"));
        if let AiSubject::SystemOptimization {
            status,
            pending_target,
            ..
        } = &mut item.subject
        {
            *status = SystemSettingStatus::Recommended;
            *pending_target = None;
        }
        assert!(state_guidance(&item).contains("NOT selected"));
        if let AiSubject::SystemOptimization { pending_target, .. } = &mut item.subject {
            *pending_target = Some(SystemSettingTargetState::Default);
        }
        assert!(state_guidance(&item).contains("NOT applied"));
    }

    #[test]
    fn maintenance_state_precedes_task_instructions() {
        let mut item = context(4);
        if let AiSubject::SystemMaintenance {
            task_id, status, ..
        } = &mut item.subject
        {
            *task_id = "windows.maintenance.system-integrity".into();
            *status = SystemMaintenanceStatus::Available;
        }
        assert!(scope_guidance(&item).contains("waits for their results"));
        if let AiSubject::SystemMaintenance { status, .. } = &mut item.subject {
            *status = SystemMaintenanceStatus::Healthy;
        }
        assert!(scope_guidance(&item).contains("no action"));
        if let AiSubject::SystemMaintenance { status, .. } = &mut item.subject {
            *status = SystemMaintenanceStatus::Unavailable;
        }
        assert!(scope_guidance(&item).contains("unavailable now"));
    }
}
