use super::{HelperKind, LogicError, StartRequest};

pub const HELPER_PATH: &str = r"C:\Program Files\one-mk\one-mk-helper.exe";
pub const MAX_COMMAND_UNITS: usize = 32_766;

/// Lexical canonical allowlist, not filesystem/reparse-point verification.
pub fn validate_helper_path(path: &str) -> Result<(), LogicError> {
    if path.eq_ignore_ascii_case(HELPER_PATH) { Ok(()) } else { Err(LogicError::Path) }
}

/// Windows CRT/CreateProcess argument encoding, not cmd.exe shell escaping.
pub fn command_line(arguments: &[&str]) -> Result<String, LogicError> {
    let mut output = String::new();
    let mut units = 0;
    for (index, argument) in arguments.iter().enumerate() {
        if argument.contains('\0') { return Err(LogicError::Invalid); }
        if index > 0 { append(&mut output, &mut units, ' ')?; }
        let quoted = argument.is_empty() || argument.chars().any(|character| character.is_whitespace() || character == '"');
        if !quoted {
            for character in argument.chars() { append(&mut output, &mut units, character)?; }
            continue;
        }
        append(&mut output, &mut units, '"')?;
        let mut slashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                slashes += 1;
                if slashes > MAX_COMMAND_UNITS { return Err(LogicError::Oversized); }
                continue;
            }
            let count = if character == '"' { slashes * 2 + 1 } else { slashes };
            for _ in 0..count { append(&mut output, &mut units, '\\')?; }
            slashes = 0;
            append(&mut output, &mut units, character)?;
        }
        for _ in 0..slashes * 2 { append(&mut output, &mut units, '\\')?; }
        append(&mut output, &mut units, '"')?;
    }
    if arguments.is_empty() { return Err(LogicError::Invalid); }
    Ok(output)
}

fn append(output: &mut String, units: &mut usize, character: char) -> Result<(), LogicError> {
    *units += character.len_utf16();
    if *units > MAX_COMMAND_UNITS { return Err(LogicError::Oversized); }
    output.push(character);
    Ok(())
}

/// No arbitrary executable, flags, environment, credential or shell input.
pub fn helper_command_line(path: &str, request: StartRequest) -> Result<String, LogicError> {
    validate_helper_path(path)?;
    if request.kind == HelperKind::None || request.generation == 0 { return Err(LogicError::Invalid); }
    let session = request.session.to_string();
    let generation = request.generation.to_string();
    let kind = match request.kind { HelperKind::UserDesktop => "user", HelperKind::SecureDesktop => "secure", HelperKind::None => return Err(LogicError::Invalid) };
    command_line(&[HELPER_PATH, "--ipc-helper", "--session", &session, "--kind", kind, "--generation", &generation])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_windows_quoting_cases() {
        for (argument, expected) in [
            ("", r#""""#), ("simple", "simple"), ("two words", r#""two words""#),
            ("tab\there", "\"tab\there\""), (r#"a"b"#, r#""a\"b""#),
            (r#"a\"b"#, r#""a\\\"b""#), (r#"a\\"b"#, r#""a\\\\\"b""#),
            (r"C:\plain\", r"C:\plain\"), (r"C:\two words\", r#""C:\two words\\""#),
            (r"C:\two words\\", r#""C:\two words\\\\""#),
            ("العربية 🐧", "\"العربية 🐧\""), ("🦀", "🦀"),
        ] {
            assert_eq!(command_line(&[argument]).unwrap(), expected, "{argument:?}");
        }
        assert_eq!(command_line(&["one", "", "two words", "three"]).unwrap(), "one \"\" \"two words\" three");
    }

    #[test]
    fn command_limits_are_utf16_and_nul_is_rejected() {
        assert_eq!(command_line(&[]), Err(LogicError::Invalid));
        assert_eq!(command_line(&["x\0y"]), Err(LogicError::Invalid));
        assert!(command_line(&[&"x".repeat(MAX_COMMAND_UNITS)]).is_ok());
        assert_eq!(command_line(&[&"x".repeat(MAX_COMMAND_UNITS + 1)]), Err(LogicError::Oversized));
        assert_eq!(command_line(&[&"🦀".repeat(MAX_COMMAND_UNITS / 2 + 1)]), Err(LogicError::Oversized));
        assert_eq!(command_line(&[&format!(" {}", "\\".repeat(MAX_COMMAND_UNITS))]), Err(LogicError::Oversized));
    }

    #[test]
    fn fixed_path_rejects_hostile_aliases_and_prefix_confusion() {
        assert_eq!(validate_helper_path(HELPER_PATH), Ok(()));
        assert_eq!(validate_helper_path(r"c:\PROGRAM FILES\ONE-MK\ONE-MK-HELPER.EXE"), Ok(()));
        for path in ["", "one-mk-helper.exe", r"\Program Files\one-mk\one-mk-helper.exe",
            r"C:Program Files\one-mk\one-mk-helper.exe", r"D:\Program Files\one-mk\one-mk-helper.exe",
            r"C:\Program Files\one-mk\..\one-mk\one-mk-helper.exe", r"C:\Program Files\one-mk\.\one-mk-helper.exe",
            r"\\server\Program Files\one-mk\one-mk-helper.exe", r"\\?\C:\Program Files\one-mk\one-mk-helper.exe",
            r"\\.\C:\Program Files\one-mk\one-mk-helper.exe", r"C:\PROGRA~1\one-mk\one-mk-helper.exe",
            r"C:\Program Files\ONE-MK~1\one-mk-helper.exe", r"C:\Program Files\one-mk\one-mk-helper.exe:payload",
            r"C:\Program Files\one-mk\one-mk-helper.exe::$DATA", r"C:\Program Files\one-mk-tools\one-mk-helper.exe",
            r"C:\Program Files\one-mk\sub\one-mk-helper.exe", r"C:/Program Files/one-mk/one-mk-helper.exe",
            "C:\\Program Files\\one-mk\\one-mk-helper.exe ", "C:\\Program Files\\one-mk\\one-mk-helper.exe.",
            "C:\\Program Files\\one-mk\\one-mk-helper.exe\0", r#""C:\Program Files\one-mk\one-mk-helper.exe""#,
            r"C:\Program Files\\one-mk\one-mk-helper.exe", "C:\\Program Files\\one-mk\\one-mk-helper.exе"] {
            assert_eq!(validate_helper_path(path), Err(LogicError::Path), "{path:?}");
        }
    }

    #[test]
    fn helper_command_has_only_fixed_fields_and_no_secret() {
        let request = StartRequest { session: 10, generation: 42, kind: HelperKind::SecureDesktop };
        assert_eq!(helper_command_line(HELPER_PATH, request).unwrap(),
            "\"C:\\Program Files\\one-mk\\one-mk-helper.exe\" --ipc-helper --session 10 --kind secure --generation 42");
        assert!(helper_command_line("evil.exe", request).is_err());
        assert!(helper_command_line(HELPER_PATH, StartRequest { kind: HelperKind::None, ..request }).is_err());
        assert!(helper_command_line(HELPER_PATH, StartRequest { generation: 0, ..request }).is_err());
    }
}
