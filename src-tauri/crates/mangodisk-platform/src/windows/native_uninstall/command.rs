/// Splits a registered command without invoking a shell or searching PATH. Some vendors omit
/// quotes around paths containing spaces. An `.exe`, `.bat`, or `.cmd` suffix followed by
/// whitespace or the end of the string is a boundary; `tools.exe.backup` is not an executable.
#[cfg(test)]
pub(super) fn split_registered_command(command: &str) -> Option<(String, String)> {
    parse_registered_command(command).ok()
}

/// Stable parse stages explain a rejected command without logging any of its private text.
/// The diagnostic is separate from the UI's broader invalid-command reason.
pub(super) fn parse_registered_command(command: &str) -> Result<(String, String), &'static str> {
    let command = command.trim();
    if command.is_empty() {
        return Err("empty_command");
    }
    if command.len() > 32_768 {
        return Err("command_too_long");
    }
    if command.contains(['\0', '\r', '\n']) {
        return Err("control_character");
    }
    if let Some(quoted) = command.strip_prefix('"') {
        let closing_quote = quoted.find('"').ok_or("unclosed_quote")?;
        let executable = &quoted[..closing_quote];
        let tail = &quoted[closing_quote + 1..];
        if executable.is_empty() {
            return Err("empty_executable");
        }
        if tail.starts_with(|c: char| !c.is_whitespace()) {
            return Err("invalid_argument_boundary");
        }
        return Ok((executable.to_string(), tail.trim().to_string()));
    }
    let lowercase = command.to_ascii_lowercase();
    let end = [".exe", ".bat", ".cmd"]
        .into_iter()
        .flat_map(|suffix| lowercase.match_indices(suffix))
        .filter_map(|(index, _)| {
            let end = index + 4;
            (end == command.len() || command[end..].starts_with(char::is_whitespace)).then_some(end)
        })
        .min()
        .ok_or("missing_executable_suffix")?;
    let executable = &command[..end];
    if executable.contains('"') {
        return Err("unexpected_quote");
    }
    Ok((executable.to_string(), command[end..].trim().to_string()))
}

/// Expand only tokens present in the original registration, once. Recursive replacement can
/// hang an inventory scan when an environment value refers to itself. Unresolved tokens remain
/// visible to the caller's validation and are never passed to a command shell.
pub(super) fn expand_environment_variables(
    value: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        output.push_str(&remaining[..start]);
        let token = &remaining[start + 1..];
        let Some(end) = token.find('%') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        match lookup(&token[..end]) {
            Some(replacement) => output.push_str(&replacement),
            None => output.push_str(&remaining[start..start + end + 2]),
        }
        remaining = &token[end + 1..];
    }
    output.push_str(remaining);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_paths_preserve_arguments_and_ignore_suffixes_inside_directories() {
        for extension in ["bat", "CMD"] {
            let executable = format!(r"C:\tools.exe.backup\Vendor Suite\uninstall.{extension}");
            let arguments = r#"/remove "value with spaces & punctuation""#;
            for registered in [
                format!(r#""{executable}" {arguments}"#),
                format!("{executable} {arguments}"),
            ] {
                assert_eq!(
                    split_registered_command(&registered),
                    Some((executable.clone(), arguments.to_string()))
                );
            }
        }
        assert_eq!(split_registered_command(r"C:\uninstall.bat.backup"), None);
        assert_eq!(
            split_registered_command(r#""C:\uninstall.cmd"/remove"#),
            None
        );
    }

    #[test]
    fn environment_expansion_is_bounded_and_preserves_unresolved_tokens() {
        assert_eq!(
            expand_environment_variables(r"%SELF%\app.exe", |_| Some("%SELF%".into())),
            r"%SELF%\app.exe"
        );
        assert_eq!(
            expand_environment_variables(r"%MISSING%\app.exe", |_| None),
            r"%MISSING%\app.exe"
        );
        assert_eq!(
            expand_environment_variables(r"%ROOT%\app.exe", |_| Some(r"C:\Apps".into())),
            r"C:\Apps\app.exe"
        );
    }

    #[test]
    fn executable_suffix_in_directory_does_not_truncate_the_registered_path() {
        assert_eq!(
            split_registered_command(r"C:\tools.exe.backup\Vendor App\Uninstall.EXE /remove"),
            Some((
                r"C:\tools.exe.backup\Vendor App\Uninstall.EXE".into(),
                "/remove".into()
            ))
        );
    }

    #[test]
    fn malformed_boundaries_cannot_turn_into_an_executable_launch() {
        for command in [
            "",
            "\0",
            "uninstall.exe\n/remove",
            r"C:\app.exe.bak",
            r#""C:\app.exe"/remove"#,
            r#""C:\app.exe"#,
        ] {
            assert_eq!(split_registered_command(command), None);
        }
    }

    #[test]
    fn parse_diagnostics_explain_the_failure_without_echoing_command_text() {
        assert_eq!(
            parse_registered_command(r#""C:\private\app.exe"#),
            Err("unclosed_quote")
        );
        assert_eq!(
            parse_registered_command(r#""C:\private\app.exe"/remove"#),
            Err("invalid_argument_boundary")
        );
        assert_eq!(
            parse_registered_command(r"C:\private\app.exe.bak"),
            Err("missing_executable_suffix")
        );
    }

    #[test]
    fn registered_arguments_remain_unchanged_and_are_not_shell_interpreted() {
        assert_eq!(
            split_registered_command(r#""C:\Vendor App\uninstall.exe" /remove "a & b""#),
            Some((
                r"C:\Vendor App\uninstall.exe".into(),
                r#"/remove "a & b""#.into()
            ))
        );
    }
}
