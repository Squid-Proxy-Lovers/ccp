// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

pub(super) fn quoted_path(path: &Path) -> String {
    // Retain Debug's quotes and control, Unicode and non-UTF path escapes.
    // Only undo its doubled backslashes so Windows paths remain copyable.
    format!("{path:?}").replace(r"\\", r"\")
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
            assert_eq!(quoted_path(Path::new(value)), format!("\"{value}\""));
        }
    }

    #[test]
    fn controls_and_quotes_stay_escaped_alongside_windows_separators() {
        assert_eq!(
            quoted_path(Path::new("C:\\profiles\\first\n\r\t\0\x1b\"\u{202e}")),
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

    // Unix paths can contain arbitrary bytes; preserve their diagnostic escapes.
    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_the_existing_byte_escapes() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let path = Path::new(OsStr::from_bytes(b"/tmp/profile-\xff"));
        assert_eq!(quoted_path(path), format!("{path:?}"));
    }
}
