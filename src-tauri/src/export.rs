//! Streaming-export formatting helpers (Feature C3).
//!
//! Pure functions only: MySQL row values (as `serde_json::Value`, produced by
//! `MySQLDriver::decode_cell`) → one CSV / JSONL text line. The row streaming
//! loop lives in [`crate::drivers::mysql`]; these helpers keep the escaping
//! rules isolated and unit-testable without a database.

use crate::models::ExportFormat;
use serde_json::Value;
use std::collections::HashMap;

/// RFC 4180 escaping: quote a field only when it contains a comma, quote,
/// CR or LF; embedded quotes double up.
pub fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Render one JSON value as its CSV cell text.
///
/// NULL → empty field; Bool → `1`/`0` (matches MySQL display and `value_to_bind`);
/// everything else keeps its JSON text form (numbers verbatim, strings unquoted,
/// nested structures as compact JSON which `csv_escape` will then quote).
fn csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        Value::String(s) => csv_escape(s),
        Value::Number(_) | Value::Array(_) | Value::Object(_) => csv_escape(&v.to_string()),
    }
}

/// One CSV data line (no trailing newline) in `columns` order.
pub fn csv_line(columns: &[String], row: &HashMap<String, Value>) -> String {
    let mut cells = Vec::with_capacity(columns.len());
    for c in columns {
        cells.push(csv_cell(row.get(c).unwrap_or(&Value::Null)));
    }
    cells.join(",")
}

/// CSV header line: column names escaped like any other field.
pub fn csv_header(columns: &[String]) -> String {
    let mut cells = Vec::with_capacity(columns.len());
    for c in columns {
        cells.push(csv_escape(c));
    }
    cells.join(",")
}

/// One JSONL line (no trailing newline): a flat object keyed by column name,
/// preserving query column order rather than alphabetical key order.
pub fn json_line(columns: &[String], row: &HashMap<String, Value>) -> String {
    let mut parts = Vec::with_capacity(columns.len());
    for c in columns {
        let key = Value::String(c.clone()).to_string(); // JSON-escaped key
        let val = row.get(c).unwrap_or(&Value::Null).to_string();
        parts.push(format!("{key}:{val}"));
    }
    format!("{{{}}}", parts.join(","))
}

/// Format a whole line (header or data row) plus the trailing newline.
pub fn format_line(
    format: ExportFormat,
    columns: &[String],
    row: Option<&HashMap<String, Value>>,
) -> String {
    let line = match (format, row) {
        (ExportFormat::Csv, None) => csv_header(columns),
        (ExportFormat::Csv, Some(r)) => csv_line(columns, r),
        (_, Some(r)) => json_line(columns, r),
        // JSONL has no header concept — callers never ask for one.
        (_, None) => String::new(),
    };
    format!("{line}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    // ===== csv_escape =====

    #[test]
    fn csv_escape_plain_text_untouched() {
        assert_eq!(csv_escape("abc123"), "abc123");
        assert_eq!(csv_escape(""), "");
    }

    #[test]
    fn csv_escape_comma_quotes_field() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
    }

    #[test]
    fn csv_escape_embedded_quote_doubles() {
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn csv_escape_newline_quotes_field() {
        assert_eq!(csv_escape("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(csv_escape("a\rb"), "\"a\rb\"");
    }

    // ===== csv_line =====

    #[test]
    fn csv_line_null_renders_empty() {
        let cols = vec!["id".to_string(), "note".to_string()];
        let r = row(&[("id", json!(7)), ("note", Value::Null)]);
        assert_eq!(csv_line(&cols, &r), "7,");
    }

    #[test]
    fn csv_line_bool_renders_1_0() {
        let cols = vec!["flag".to_string()];
        assert_eq!(csv_line(&cols, &row(&[("flag", json!(true))])), "1");
        assert_eq!(csv_line(&cols, &row(&[("flag", json!(false))])), "0");
    }

    #[test]
    fn csv_line_embedded_comma_and_quote_and_newline() {
        let cols = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let r = row(&[
            ("a", json!("x,y")),
            ("b", json!("he said \"ok\"")),
            ("c", json!("two\nlines")),
        ]);
        assert_eq!(csv_line(&cols, &r), "\"x,y\",\"he said \"\"ok\"\"\",\"two\nlines\"");
    }

    #[test]
    fn csv_line_missing_column_is_null() {
        let cols = vec!["present".to_string(), "absent".to_string()];
        let r = row(&[("present", json!("v"))]);
        assert_eq!(csv_line(&cols, &r), "v,");
    }

    #[test]
    fn csv_header_escapes_names() {
        let cols = vec!["id".to_string(), "weird,name".to_string()];
        assert_eq!(csv_header(&cols), "id,\"weird,name\"");
    }

    // ===== json_line =====

    #[test]
    fn json_line_preserves_column_order_and_types() {
        let cols = vec!["b".to_string(), "a".to_string()];
        let r = row(&[("a", json!(1)), ("b", json!("x"))]);
        assert_eq!(json_line(&cols, &r), r#"{"b":"x","a":1}"#);
    }

    #[test]
    fn json_line_null_and_missing_fields() {
        let cols = vec!["a".to_string(), "gone".to_string()];
        let r = row(&[("a", Value::Null)]);
        assert_eq!(json_line(&cols, &r), r#"{"a":null,"gone":null}"#);
    }

    #[test]
    fn json_line_escapes_strings_and_keys() {
        let cols = vec!["k\"ey".to_string()];
        let r = row(&[("k\"ey", json!("va\"lue\n"))]);
        assert_eq!(json_line(&cols, &r), "{\"k\\\"ey\":\"va\\\"lue\\n\"}");
    }

    // ===== format_line =====

    #[test]
    fn format_line_appends_newline() {
        let cols = vec!["id".to_string()];
        let r = row(&[("id", json!(1))]);
        assert_eq!(format_line(ExportFormat::Csv, &cols, Some(&r)), "1\n");
        assert_eq!(format_line(ExportFormat::Csv, &cols, None), "id\n");
        assert_eq!(format_line(ExportFormat::JsonLines, &cols, Some(&r)), "{\"id\":1}\n");
    }
}
