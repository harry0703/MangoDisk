pub(crate) fn telemetry_os_version() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Some(distribution) = linux_distribution() {
            return distribution;
        }
        // A missing or unusual os-release must not prevent a signed update.
        log::warn!("app_update_os_version_fallback source=os_release outcome=kernel_version");
    }

    tauri_plugin_os::version().to_string()
}

#[cfg(target_os = "linux")]
fn linux_distribution() -> Option<String> {
    let content = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    parse_distribution(&content)
}

#[cfg(any(target_os = "linux", test))]
fn parse_distribution(content: &str) -> Option<String> {
    let field = |key: &str| {
        content
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .map(|value| value.trim().trim_matches(['"', '\'']))
            .filter(|value| !value.is_empty())
    };
    let value = field("PRETTY_NAME=").map(str::to_string).or_else(|| {
        let name = field("NAME=").or_else(|| field("ID="))?;
        let version = field("VERSION=").or_else(|| field("VERSION_ID="));
        Some(match version {
            Some(version) => format!("{name} {version}"),
            None => name.to_string(),
        })
    })?;
    // HTTP header values must stay printable ASCII so telemetry cannot break update checks.
    if !value
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
    {
        return None;
    }
    Some(value.chars().take(128).collect())
}

#[cfg(test)]
mod tests {
    use super::parse_distribution;

    #[test]
    fn reads_distribution_and_version_from_os_release() {
        assert_eq!(
            parse_distribution("ID=ubuntu\nPRETTY_NAME=\"Ubuntu 24.04.5 LTS\"\n"),
            Some("Ubuntu 24.04.5 LTS".to_string())
        );
        assert_eq!(
            parse_distribution("PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n"),
            Some("Debian GNU/Linux 12 (bookworm)".to_string())
        );
    }

    #[test]
    fn falls_back_to_name_and_release_when_pretty_name_is_missing() {
        assert_eq!(
            parse_distribution("NAME=\"AlmaLinux\"\nVERSION_ID=\"9.6\"\n"),
            Some("AlmaLinux 9.6".to_string())
        );
    }

    #[test]
    fn rejects_values_that_would_invalidate_the_update_header() {
        assert_eq!(parse_distribution("PRETTY_NAME=\"bad\tname\""), None);
        assert_eq!(
            parse_distribution("PRETTY_NAME=\"\u{65e5}\u{672c}\u{8a9e} Linux\""),
            None
        );
    }
}
