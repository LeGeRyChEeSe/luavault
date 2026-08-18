//! Structured metadata for log messages that the webview can translate.
//!
//! `tauri-plugin-log` formats one message for its stdout, file and webview
//! targets. Keep the existing French sentence as the readable prefix, then
//! append a non-printable payload for the webview after Unit Separator.

use serde_json::{Map, Value};

/// Delimiter shared with `src/lib/log-filter.ts`. Unit Separator cannot occur
/// in a normal user-visible log sentence or an interpolated filesystem path.
pub const I18N_LOG_SEPARATOR: char = '\u{1f}';

/// Adds an i18n key and JSON arguments without changing the human-readable
/// prefix that stdout and the on-disk log retain.
pub fn i18n_log(fallback: String, key: &str, args: &[(&str, Value)]) -> String {
    let mut values = Map::new();
    for (name, value) in args {
        values.insert((*name).to_owned(), value.clone());
    }

    format!(
        "{fallback}{I18N_LOG_SEPARATOR}{key}{I18N_LOG_SEPARATOR}{}",
        Value::Object(values)
    )
}

#[cfg(test)]
mod tests {
    use super::{i18n_log, I18N_LOG_SEPARATOR};
    use serde_json::json;

    #[test]
    fn keeps_the_readable_prefix_and_encodes_typed_arguments() {
        let message = i18n_log(
            "snapshot: library.zip (2 .lua, 42 octets)".to_owned(),
            "logs.snapshot.created",
            &[("luaCount", json!(2)), ("bytes", json!(42))],
        );

        assert_eq!(
            message,
            format!(
                "snapshot: library.zip (2 .lua, 42 octets){I18N_LOG_SEPARATOR}logs.snapshot.created{I18N_LOG_SEPARATOR}{{\"bytes\":42,\"luaCount\":2}}"
            )
        );
    }
}
