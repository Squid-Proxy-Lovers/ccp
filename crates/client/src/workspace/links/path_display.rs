// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

pub(super) fn quoted_path(path: &Path) -> String {
    quoted_path_for_platform(path, cfg!(windows))
}

fn quoted_path_for_platform(path: &Path, windows: bool) -> String {
    // Unix diagnostics retain literal backslashes and every existing Debug escape.
    let quoted = format!("{path:?}");
    if windows {
        quoted.replace(r"\\", r"\")
    } else {
        quoted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_keep_literal_separators_on_every_platform() {
        for value in [
            r"C:\Users\Alice\profiles\first",
            r"C:\Users\Alice Smith\profiles\café's workspace",
            r"\\server\share\profiles\first",
            r"\\?\C:\Users\Alice\profiles\first",
            r"C:\new\tab\unicode\u{1b}",
            r"C:\trailing\",
        ] {
            assert_eq!(
                quoted_path_for_platform(Path::new(value), true),
                format!("\"{value}\"")
            );
        }
    }

    #[test]
    fn controls_and_quotes_stay_escaped_alongside_windows_separators() {
        assert_eq!(
            quoted_path_for_platform(Path::new("C:\\profiles\\first\n\r\t\0\x1b\"\u{202e}"), true),
            "\"C:\\profiles\\first\\n\\r\\t\\0\\u{1b}\\\"\\u{202e}\""
        );
    }

    #[test]
    fn paths_without_backslashes_keep_the_existing_quoted_debug_output() {
        for value in [
            "",
            "/tmp/profiles/first",
            "/tmp/Alice Smith/café's workspace",
            "/tmp/cafe\u{301}",
            "/tmp/line\nquote\"tab\t",
            "/tmp/control\0\x1b\x7f\u{85}\u{200d}\u{202e}",
        ] {
            let path = Path::new(value);
            assert_eq!(quoted_path(path), format!("{path:?}"));
        }
    }

    #[test]
    fn literal_backslashes_and_escaped_controls_stay_distinct_on_unix() {
        let literal = Path::new(r"profile\nwith\tbackslashes");
        let control = Path::new("profile\nwith\tbackslashes");
        assert_ne!(
            quoted_path_for_platform(literal, false),
            quoted_path_for_platform(control, false)
        );
        assert_eq!(
            quoted_path_for_platform(literal, false),
            format!("{literal:?}")
        );
        assert_eq!(
            quoted_path_for_platform(control, false),
            format!("{control:?}")
        );
        let expected = if cfg!(windows) {
            quoted_path_for_platform(literal, true)
        } else {
            format!("{literal:?}")
        };
        assert_eq!(quoted_path(literal), expected);
    }

    // Unix paths can contain arbitrary bytes; preserve their diagnostic escapes.
    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_the_existing_byte_escapes() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let path = Path::new(OsStr::from_bytes(b"/tmp/profile-\xff"));
        assert_eq!(quoted_path(path), format!("{path:?}"));
    }
}
