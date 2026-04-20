//! Parser for tsgo's pretty-printed output.
//!
//! tsgo (and tsc) print diagnostics in this shape (after stripping ANSI):
//!
//! ```text
//! src/Index.ts:5:8 - error TS2322: Type 'string' is not assignable to type 'number'.
//!
//!   5 const x: number = "hi";
//!            ~
//! ```
//!
//! We extract: file, line (1-based), column (1-based), severity, code,
//! message. The `~~~~` underline on a subsequent line gives us the span
//! length so we can report end positions; we look up to 4 lines ahead.

use std::path::PathBuf;

/// One diagnostic recovered from tsgo's stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDiagnostic {
    /// Filename as printed by tsgo. May be a generated `.svelte.ts` path
    /// inside the cache; the orchestrator maps it back to the source.
    pub file: PathBuf,
    /// 1-based line number as printed.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
    pub severity: Severity,
    /// TypeScript error code, numeric (e.g. 2322).
    pub code: u32,
    pub message: String,
    /// Span length recovered from a `~~~~` underline on a following line,
    /// if present. None when no underline could be parsed.
    pub span_length: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Parse tsgo's full stdout into a list of diagnostics. ANSI escape
/// sequences are stripped before parsing.
pub fn parse(stdout: &str) -> Vec<RawDiagnostic> {
    let plain = strip_ansi(stdout);
    let lines: Vec<&str> = plain.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(d) = parse_header(lines[i]) {
            // Look up to 4 subsequent lines for an underline `~~~~`.
            let mut span = None;
            let upper = (i + 5).min(lines.len());
            for ahead in lines.iter().take(upper).skip(i + 1) {
                if let Some(len) = parse_underline(ahead) {
                    span = Some(len);
                    break;
                }
            }
            let mut diagnostic = d;
            diagnostic.span_length = span;
            out.push(diagnostic);
        }
        i += 1;
    }
    out
}

/// Parse a single header line. Returns `None` if the line doesn't match.
///
/// Format: `<file>:<line>:<col> - <severity> TS<code>: <message>`
fn parse_header(line: &str) -> Option<RawDiagnostic> {
    // Find ` - error TS` or ` - warning TS` somewhere in the line.
    let (sep_idx, severity) = if let Some(idx) = line.find(" - error TS") {
        (idx, Severity::Error)
    } else if let Some(idx) = line.find(" - warning TS") {
        (idx, Severity::Warning)
    } else {
        return None;
    };

    // Left of separator: `<file>:<line>:<col>`.
    let location = &line[..sep_idx];
    let (file_str, line_no, col_no) = split_location(location)?;

    // Right of separator: `error TS<code>: <message>` (or warning).
    let after = &line[sep_idx + 3..]; // strip " - "
    let after = after.strip_prefix(match severity {
        Severity::Error => "error TS",
        Severity::Warning => "warning TS",
    })?;
    let colon_idx = after.find(": ")?;
    let code: u32 = after[..colon_idx].parse().ok()?;
    let message = after[colon_idx + 2..].trim().to_string();

    Some(RawDiagnostic {
        file: PathBuf::from(file_str),
        line: line_no,
        column: col_no,
        severity,
        code,
        message,
        span_length: None,
    })
}

/// Split `path/to/file.ts:LINE:COL` into its parts.
///
/// Path may contain colons (e.g. on Windows `C:`), so we anchor on the
/// trailing two `:NUMBER:NUMBER` pieces.
fn split_location(location: &str) -> Option<(&str, u32, u32)> {
    let last_colon = location.rfind(':')?;
    let col: u32 = location[last_colon + 1..].parse().ok()?;
    let middle = &location[..last_colon];
    let prev_colon = middle.rfind(':')?;
    let line: u32 = middle[prev_colon + 1..].parse().ok()?;
    let file = &middle[..prev_colon];
    Some((file, line, col))
}

/// If the line contains an underline (whitespace + `~~~~~~`), return the
/// number of `~` chars. Otherwise None.
fn parse_underline(line: &str) -> Option<u32> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('~') {
        let count = trimmed.chars().take_while(|&c| c == '~').count();
        Some(count as u32)
    } else {
        None
    }
}

/// Strip ANSI escape sequences (ESC [ ... letter). Conservative: handles
/// CSI sequences which is what tsc/tsgo use for color.
///
/// Byte-indexing is safe: ESC (0x1b), `[` (0x5b), and CSI terminators
/// (0x40..=0x7e) are all ASCII, and in valid UTF-8 an ASCII byte can
/// only appear at a char boundary. Non-ESC runs are copied as string
/// slices so multibyte chars in filenames/messages survive intact.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut run_start = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            out.push_str(&input[run_start..i]);
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip terminator
            }
            run_start = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&input[run_start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_error_diagnostic() {
        let stdout =
            "src/Index.ts:5:8 - error TS2322: Type 'string' is not assignable to type 'number'.";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.file, PathBuf::from("src/Index.ts"));
        assert_eq!(d.line, 5);
        assert_eq!(d.column, 8);
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code, 2322);
        assert!(d.message.contains("not assignable"));
        assert_eq!(d.span_length, None);
    }

    #[test]
    fn recovers_span_from_underline() {
        let stdout = "\
src/foo.ts:1:5 - error TS2322: bad
\x20\x20
\x20\x201 const xy = 1;
\x20\x20  ~~~~~~
";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].span_length, Some(6));
    }

    #[test]
    fn parses_warning_severity() {
        let stdout =
            "src/foo.ts:3:1 - warning TS6133: 'x' is declared but its value is never read.";
        let diags = parse(stdout);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(diags[0].code, 6133);
    }

    #[test]
    fn parses_multiple_diagnostics_separated_by_blank_lines() {
        let stdout = "\
src/a.ts:1:1 - error TS1000: a
src/b.ts:2:2 - error TS2000: b
src/c.ts:3:3 - warning TS3000: c
";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 3);
        assert_eq!(diags[0].code, 1000);
        assert_eq!(diags[1].code, 2000);
        assert_eq!(diags[2].code, 3000);
    }

    #[test]
    fn ignores_non_diagnostic_lines() {
        let stdout = "\
Found 0 errors. Watching for file changes.

random other tsgo banter
src/x.ts:1:1 - error TS1: actual
also ignored
";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, 1);
    }

    #[test]
    fn split_location_handles_colon_in_windows_path() {
        let (file, line, col) = split_location("C:\\src\\foo.ts:5:8").unwrap();
        assert_eq!(file, "C:\\src\\foo.ts");
        assert_eq!(line, 5);
        assert_eq!(col, 8);
    }

    #[test]
    fn strips_ansi_color_codes() {
        let stripped = strip_ansi("\x1b[31merror\x1b[0m here");
        assert_eq!(stripped, "error here");
    }

    #[test]
    fn parses_diagnostic_inside_colored_output() {
        let stdout = "\x1b[31msrc/x.ts\x1b[0m:1:1 - error TS1: bad";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, PathBuf::from("src/x.ts"));
    }

    #[test]
    fn strip_ansi_preserves_multibyte_chars() {
        // Unicode in both the ANSI-wrapped and plain regions.
        let stripped = strip_ansi("\x1b[31mτ\x1b[0m — naïve résumé 日本語");
        assert_eq!(stripped, "τ — naïve résumé 日本語");
    }

    #[test]
    fn parses_diagnostic_with_unicode_path_and_message() {
        let stdout = "src/日本語/Café.ts:1:1 - error TS1: naïve résumé τ";
        let diags = parse(stdout);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, PathBuf::from("src/日本語/Café.ts"));
        assert_eq!(diags[0].message, "naïve résumé τ");
    }
}
