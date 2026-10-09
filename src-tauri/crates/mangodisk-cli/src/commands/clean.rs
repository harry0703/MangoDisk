use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};

use mangodisk_core::{
    diagnostic_path, CleanupGroup, CleanupRequest, CleanupScanResult, CleanupScanService,
    CleanupService, OperationCancellationToken, ScanItemStatus, ScanRuleResult,
};
use serde_json::json;

use crate::{
    arguments::{CleanArgs, CleanSelection, OutputFormat},
    commands::{CliFailure, CommandContext},
    exit_code::CliExitCode,
    output::CommandOutcome,
    progress::{CliCleanupProgress, CliProgressSink},
};

const GROUP_ORDER: [CleanupGroup; 10] = [
    CleanupGroup::System,
    CleanupGroup::UserCache,
    CleanupGroup::Browser,
    CleanupGroup::Application,
    CleanupGroup::Development,
    CleanupGroup::Project,
    CleanupGroup::Xcode,
    CleanupGroup::ApplicationOptimization,
    CleanupGroup::Ai,
    CleanupGroup::Container,
];

pub fn run(
    arguments: CleanArgs,
    context: &CommandContext<'_>,
) -> Result<CommandOutcome, CliFailure> {
    let scan = {
        let _active = context
            .cancellation
            .activate(OperationCancellationToken::cleanup_scan());
        let progress = CliProgressSink::new(
            context.format,
            context.progress_enabled,
            context.include_full_paths,
        );
        progress.begin_scan();
        CleanupScanService::scan_with_deep_project_discovery(
            arguments.deep_project_discovery,
            progress,
        )?
    };
    let scan_summary = render_scan(
        &scan,
        arguments.details,
        context.include_full_paths,
        context.color_enabled,
    );

    if !arguments.apply {
        return CommandOutcome::success(
            "clean.scan",
            format!("{scan_summary}\n\nNo files changed."),
            scan,
        )
        .map_err(Into::into);
    }

    let selection = arguments.selection.unwrap_or(CleanSelection::Recommended);
    let selected_rules = selected_rules(&scan, selection);
    let selected_rule_ids = selected_rules
        .iter()
        .map(|rule| rule.rule_id.clone())
        .collect::<Vec<_>>();
    let selected_bytes = selected_rules.iter().map(|rule| rule.bytes).sum::<u64>();

    if selected_rule_ids.is_empty() {
        return CommandOutcome::success(
            "clean.apply",
            format!(
                "{scan_summary}\n\nNo {} cleanup items are currently available. No files changed.",
                selection_label(selection)
            ),
            json!({
                "scan": scan,
                "selection": selection_label(selection),
                "selectedRuleIds": selected_rule_ids,
                "selectedBytes": selected_bytes,
                "cleanup": null,
            }),
        )
        .map_err(Into::into);
    }

    if context.format == OutputFormat::Human {
        println!("{scan_summary}");
        eprintln!(
            "\nSelected {} item(s), {} using the {} selection.",
            selected_rule_ids.len(),
            format_bytes(selected_bytes),
            selection_label(selection)
        );
    }

    if !arguments.dry_run
        && !arguments.yes
        && !confirm_cleanup(
            context.format,
            selection,
            selected_rule_ids.len(),
            selected_bytes,
        )?
    {
        let mut outcome = CommandOutcome::success(
            "clean.apply",
            "Cleanup cancelled. No files changed.",
            json!({
                "scan": scan,
                "selection": selection_label(selection),
                "selectedRuleIds": selected_rule_ids,
                "selectedBytes": selected_bytes,
                "cleanup": null,
                "cancelled": true,
            }),
        )
        .map_err(CliFailure::from)?;
        outcome.exit_code = CliExitCode::Cancelled;
        return Ok(outcome);
    }

    let request = CleanupRequest {
        rule_ids: selected_rule_ids.clone(),
        dry_run: arguments.dry_run,
        project_roots: Vec::new(),
        source_selections: Vec::new(),
    };
    let result = {
        let _active = context
            .cancellation
            .activate(OperationCancellationToken::cleanup());
        let mut progress = CliCleanupProgress::new(
            context.format,
            context.progress_enabled,
            context.include_full_paths,
        );
        progress.begin();
        CleanupService::execute_with_progress(request, |snapshot| {
            let rule_label = snapshot.current_rule_id.as_deref().map(humanize_rule_id);
            progress.report(snapshot, rule_label.as_deref());
        })?
    };
    let exit_code = if context.cancellation.was_cancelled() {
        CliExitCode::Cancelled
    } else if result.failed_item_count > 0 {
        CliExitCode::CompletedWithWarnings
    } else {
        CliExitCode::Success
    };
    let action = if result.dry_run {
        "Dry run completed"
    } else {
        "Cleanup completed"
    };
    let human = if result.dry_run {
        format!(
            "{action}: {} selected, {} item(s) passed preflight, {} item(s) skipped or failed.",
            format_bytes(selected_bytes),
            result.affected_item_count,
            result.failed_item_count
        )
    } else {
        format!(
            "{action}: {} removed, {} item(s) affected, {} item(s) skipped or failed.",
            format_bytes(result.released_bytes),
            result.affected_item_count,
            result.failed_item_count
        )
    };
    let mut outcome = CommandOutcome::success(
        "clean.apply",
        human,
        json!({
            "scan": scan,
            "selection": selection_label(selection),
            "selectedRuleIds": selected_rule_ids,
            "selectedBytes": selected_bytes,
            "cleanup": result,
        }),
    )
    .map_err(CliFailure::from)?;
    outcome.exit_code = exit_code;
    Ok(outcome)
}

fn selected_rules(scan: &CleanupScanResult, selection: CleanSelection) -> Vec<&ScanRuleResult> {
    scan.rules
        .iter()
        .filter(|rule| {
            rule.selectable
                && rule.bytes > 0
                && match selection {
                    CleanSelection::Recommended => rule.recommended_selected,
                    CleanSelection::All => true,
                }
        })
        .collect()
}

fn confirm_cleanup(
    format: OutputFormat,
    selection: CleanSelection,
    selected_count: usize,
    selected_bytes: u64,
) -> Result<bool, CliFailure> {
    if format != OutputFormat::Human || !io::stdin().is_terminal() {
        return Err(CliFailure::confirmation_required(
            "non-interactive cleanup requires --yes; use --dry-run to verify without changes",
        ));
    }

    let warning = match selection {
        CleanSelection::Recommended => "Apply the recommended cleanup",
        CleanSelection::All => "Apply all selectable items, including items that require review",
    };
    eprint!(
        "{warning} ({selected_count} item(s), {})? [y/N] ",
        format_bytes(selected_bytes)
    );
    io::stderr().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn render_scan(
    scan: &CleanupScanResult,
    details: bool,
    include_full_paths: bool,
    color_enabled: bool,
) -> String {
    let summary = render_scan_summary(scan, color_enabled);
    let limited_rule_count = scan
        .rules
        .iter()
        .filter(|rule| rule.status == ScanItemStatus::Limited)
        .count() as u64;
    let mut lines = vec![
        style("Cleanup scan", "1;36", color_enabled),
        summary.clone(),
    ];
    let mut rendered_group = false;

    for group in GROUP_ORDER {
        let mut rules = scan
            .rules
            .iter()
            .filter(|rule| rule.group == group && rule.bytes > 0)
            .collect::<Vec<_>>();
        if rules.is_empty() {
            continue;
        }
        rendered_group = true;
        rules.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        let group_bytes = rules.iter().map(|rule| rule.bytes).sum::<u64>();
        lines.push(String::new());
        lines.push(style(
            &format!(
                "{} ({}, {} item(s))",
                group_label(group),
                format_bytes(group_bytes),
                rules.len()
            ),
            "1;34",
            color_enabled,
        ));
        for rule in rules {
            lines.push(render_rule_line(rule));
            if details {
                lines.push(format!(
                    "    {} file(s), {} source(s){}",
                    rule.file_count,
                    rule.source_count,
                    if rule.sources_truncated {
                        " (summary truncated)"
                    } else {
                        ""
                    }
                ));
                for source in &rule.sources {
                    let path = if include_full_paths {
                        source.path.clone()
                    } else {
                        diagnostic_path(Path::new(&source.path))
                    };
                    lines.push(format!("    - {}  {}", format_bytes(source.bytes), path));
                }
            }
        }
    }

    if rendered_group || limited_rule_count > 0 {
        lines.push(String::new());
        if limited_rule_count > 0 {
            lines.push(render_scan_warning(limited_rule_count, color_enabled));
        }
        lines.push(summary);
    }

    lines.join("\n")
}

fn render_scan_warning(limited_item_count: u64, color_enabled: bool) -> String {
    let warning_label = if limited_item_count == 1 {
        "1 optional cleanup item could not be inspected".to_string()
    } else {
        format!("{limited_item_count} optional cleanup items could not be inspected")
    };
    style(
        &format!("{warning_label}. Set MANGODISK_LOG=warn for diagnostic details."),
        "33",
        color_enabled,
    )
}

fn render_scan_summary(scan: &CleanupScanResult, color_enabled: bool) -> String {
    let recommended_bytes = scan
        .rules
        .iter()
        .filter(|rule| rule.recommended_selected && rule.selectable && rule.bytes > 0)
        .map(|rule| rule.bytes)
        .sum::<u64>();

    format!(
        "{} candidate(s), {} estimated cleanable data, {} recommended, completed in {}.",
        scan.rules
            .iter()
            .filter(|rule| rule.selectable && rule.bytes > 0)
            .count(),
        style(&format_bytes(scan.reclaimable_bytes), "1;32", color_enabled),
        style(&format_bytes(recommended_bytes), "1;32", color_enabled),
        format_duration(scan.elapsed_ms)
    )
}

fn render_rule_line(rule: &ScanRuleResult) -> String {
    let label = humanize_rule_id(&rule.rule_id);
    let bytes = format_bytes(rule.bytes);
    let marker = if rule.recommended_selected && rule.selectable {
        Some("recommended")
    } else if !rule.selectable {
        Some("unavailable")
    } else {
        None
    };

    match marker {
        Some(marker) => format!("  {label:<44} {bytes:>10}  {marker}"),
        None => format!("  {label:<44} {bytes:>10}"),
    }
}

const fn selection_label(selection: CleanSelection) -> &'static str {
    match selection {
        CleanSelection::Recommended => "recommended",
        CleanSelection::All => "all",
    }
}

const fn group_label(group: CleanupGroup) -> &'static str {
    match group {
        CleanupGroup::Custom => "Custom cleanup",
        CleanupGroup::System => "System",
        CleanupGroup::UserCache => "User caches",
        CleanupGroup::Browser => "Browser data",
        CleanupGroup::Application => "Application caches",
        CleanupGroup::Development => "Developer tools",
        CleanupGroup::Project => "Project build artifacts",
        CleanupGroup::Xcode => "Xcode data",
        CleanupGroup::ApplicationOptimization => "Application optimization",
        CleanupGroup::Ai => "AI models and caches",
        CleanupGroup::Container => "Container data",
    }
}

fn humanize_rule_id(rule_id: &str) -> String {
    let value = rule_id.rsplit('.').next().unwrap_or(rule_id);
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| match part.to_ascii_lowercase().as_str() {
            "ai" => "AI".to_string(),
            "ios" => "iOS".to_string(),
            "macos" => "macOS".to_string(),
            "xcode" => "Xcode".to_string(),
            "npm" => "npm".to_string(),
            "pnpm" => "pnpm".to_string(),
            "pip" => "pip".to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn format_duration(elapsed_ms: u64) -> String {
    if elapsed_ms < 1_000 {
        format!("{elapsed_ms} ms")
    } else {
        format!("{:.1} s", elapsed_ms as f64 / 1_000.0)
    }
}

fn style(value: &str, ansi_code: &str, enabled: bool) -> String {
    if enabled {
        format!("\u{1b}[{ansi_code}m{value}\u{1b}[0m")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mangodisk_core::{CleanupCategory, DiskInfo, RiskLevel, ScanItemStatus};

    fn scan_rule(rule_id: &str, recommended_selected: bool) -> ScanRuleResult {
        ScanRuleResult {
            rule_id: rule_id.to_string(),
            category: CleanupCategory::Development,
            group: CleanupGroup::Development,
            risk: RiskLevel::Safe,
            default_selected: recommended_selected,
            recommended_selected,
            bytes: 1_048_576,
            file_count: 1,
            available: true,
            selectable: true,
            status: ScanItemStatus::Found,
            running_processes: Vec::new(),
            requires_app_close: false,
            sources: Vec::new(),
            source_count: 1,
            sources_truncated: false,
            scan_elapsed_ms: 1,
        }
    }

    fn scan_result() -> CleanupScanResult {
        let mut limited_rule = scan_rule("container.docker-build-cache", false);
        limited_rule.bytes = 0;
        limited_rule.available = false;
        limited_rule.selectable = false;
        limited_rule.status = ScanItemStatus::Limited;

        CleanupScanResult {
            schema_version: "1.7".to_string(),
            custom_scan_id: None,
            missing_custom_root_count: 0,
            scanned_at_ms: 1,
            disk: DiskInfo {
                name: "fixture".to_string(),
                mount_point: "/fixture".to_string(),
                total_bytes: 4_194_304,
                available_bytes: 2_097_152,
                used_bytes: 2_097_152,
            },
            rules: vec![
                scan_rule("development.npm-cache", true),
                scan_rule("development.rust-toolchains", false),
                limited_rule,
            ],
            application_icons: Vec::new(),
            application_close_identities: Vec::new(),
            warning_count: 1,
            access_limited: false,
            read_failure_count: 0,
            permission_denied_read_failure_count: 0,
            read_failure_details: Vec::new(),
            safe_bytes: 1_048_576,
            reclaimable_bytes: 2_097_152,
            applicability_elapsed_ms: 1,
            applicable_rule_count: 2,
            filtered_rule_count: 0,
            inventory_application_count: 0,
            inventory_process_count: 0,
            elapsed_ms: 1_500,
        }
    }

    #[test]
    fn rule_identifier_is_readable_without_ui_localization() {
        assert_eq!(
            humanize_rule_id("project.rust-build-artifacts"),
            "Rust Build Artifacts"
        );
        assert_eq!(
            humanize_rule_id("ai.xcode-model-cache"),
            "Xcode Model Cache"
        );
    }

    #[test]
    fn byte_format_uses_compact_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1_048_576), "1.00 MB");
        assert_eq!(format_bytes(12 * 1_073_741_824), "12.0 GB");
    }

    #[test]
    fn human_scan_hides_review_markers_and_repeats_the_summary() {
        let output = render_scan(&scan_result(), false, false, false);
        let summary =
            "2 candidate(s), 2.00 MB estimated cleanable data, 1.00 MB recommended, completed in 1.5 s.";

        assert!(output.contains("npm Cache"));
        assert!(output.contains("recommended"));
        assert!(output.contains("Rust Toolchains"));
        assert!(!output.contains("review"));
        assert!(output.contains(
            "1 optional cleanup item could not be inspected. Set MANGODISK_LOG=warn for diagnostic details."
        ));
        assert_eq!(output.matches(summary).count(), 2);
        assert!(output.ends_with(summary));
    }

    #[test]
    fn scan_warning_uses_a_plural_label() {
        assert!(render_scan_warning(2, false)
            .starts_with("2 optional cleanup items could not be inspected."));
    }

    #[test]
    fn recommended_selection_includes_recoverable_application_rules() {
        let mut scan = scan_result();
        let mut application_rule = scan_rule("app.adobe-media-cache", true);
        application_rule.category = CleanupCategory::Application;
        application_rule.group = CleanupGroup::Application;
        application_rule.risk = RiskLevel::Recoverable;
        application_rule.default_selected = false;
        scan.rules.push(application_rule);

        let selected_ids = selected_rules(&scan, CleanSelection::Recommended)
            .into_iter()
            .map(|rule| rule.rule_id.as_str())
            .collect::<Vec<_>>();

        assert!(selected_ids.contains(&"app.adobe-media-cache"));
        assert!(!selected_ids.contains(&"development.rust-toolchains"));
    }
}
