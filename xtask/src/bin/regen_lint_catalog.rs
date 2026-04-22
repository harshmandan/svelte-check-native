//! Regenerate `crates/svn-lint/src/codes.rs` + `messages.rs` from
//! upstream's `messages/compile-warnings/*.md`.
//!
//! Upstream keeps the warning-code list + message templates in
//! `packages/svelte/messages/compile-warnings/*.md` (one `## code`
//! section per warning, `>` blockquote for the message). We read
//! those files, emit a Rust catalog that looks/behaves the same as
//! the generated JS `warnings.js`.
//!
//! Run after bumping the `.svelte-upstream/svelte` pin.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

fn main() -> Result<()> {
    let manifest_dir = env_workspace_root()?;
    let src =
        manifest_dir.join(".svelte-upstream/svelte/packages/svelte/messages/compile-warnings");
    anyhow::ensure!(
        src.is_dir(),
        "upstream messages dir not found at {}. Run `git clone --filter=blob:none --no-checkout \
         https://github.com/sveltejs/svelte.git .svelte-upstream/svelte` and `git -C \
         .svelte-upstream/svelte checkout HEAD -- packages/svelte/messages` first.",
        src.display()
    );

    let out_dir = manifest_dir.join("crates/svn-lint/src");
    anyhow::ensure!(out_dir.is_dir(), "svn-lint crate src not found");

    let catalog = read_catalog(&src)?;
    let codes_rs = render_codes(&catalog);
    let messages_rs = render_messages(&catalog);

    fs::write(out_dir.join("codes.rs"), codes_rs).context("writing codes.rs")?;
    fs::write(out_dir.join("messages.rs"), messages_rs).context("writing messages.rs")?;

    println!("regenerated {} warning codes", catalog.len());
    Ok(())
}

fn env_workspace_root() -> Result<PathBuf> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").context("CARGO_MANIFEST_DIR not set")?;
    // xtask/ is nested one level under the workspace root.
    Ok(PathBuf::from(manifest)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

/// One warning entry.
#[derive(Debug)]
struct Warning {
    /// Message templates — one per blockquote row. In presence-of-overload
    /// order, earliest first. Each is the raw text with `%name%` placeholders.
    templates: Vec<String>,
}

/// Read every `*.md` under `compile-warnings/` and return a sorted
/// code → warning map.
fn read_catalog(dir: &Path) -> Result<BTreeMap<String, Warning>> {
    let mut map = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        parse_md_file(&raw, &mut map)?;
    }
    Ok(map)
}

/// Parse a single `.md` file. Format:
///
/// ```text
/// ## code_name
///
/// > first template line
/// > continued line (joined with \n)
/// > ...
///
/// > overload second template (more %vars%)
///
/// Prose that doesn't start with `> ` (discarded here — lives in the
/// generated docs site).
/// ```
fn parse_md_file(raw: &str, out: &mut BTreeMap<String, Warning>) -> Result<()> {
    let raw = raw.replace("\r\n", "\n");
    let lines: Vec<&str> = raw.split('\n').collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(code) = line.strip_prefix("## ") {
            let code = code.trim().to_string();
            i += 1;

            // Collect subsequent paragraphs. A paragraph is a run of
            // non-empty lines; consecutive `> `-prefixed paragraphs
            // become templates, the rest is discarded.
            let mut templates: Vec<String> = Vec::new();
            while i < lines.len() && !lines[i].starts_with("## ") {
                // Skip blank lines separating paragraphs.
                while i < lines.len() && lines[i].trim().is_empty() {
                    i += 1;
                }
                if i >= lines.len() || lines[i].starts_with("## ") {
                    break;
                }
                // Collect one paragraph.
                let start = i;
                while i < lines.len() && !lines[i].trim().is_empty() && !lines[i].starts_with("## ")
                {
                    i += 1;
                }
                let para = &lines[start..i];
                if para.iter().all(|l| l.starts_with(">")) {
                    // Blockquote paragraph → message template.
                    // Strip leading `> ` or `>` on each line, join with \n.
                    let joined = para
                        .iter()
                        .map(|l| {
                            l.strip_prefix("> ")
                                .unwrap_or_else(|| l.trim_start_matches('>'))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    templates.push(joined);
                }
                // Non-blockquote → prose, ignored.
            }

            if templates.is_empty() {
                anyhow::bail!("warning `{}` has no message template", code);
            }
            out.insert(code, Warning { templates });
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Render `codes.rs` — the enum + string-table.
fn render_codes(catalog: &BTreeMap<String, Warning>) -> String {
    let mut out = String::new();
    out.push_str(
        "// GENERATED — do not edit. Run `cargo run -p xtask --bin regen-lint-catalog`.\n",
    );
    out.push_str("//\n");
    out.push_str(
        "// Source: .svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*.md\n\n",
    );
    out.push_str("#![allow(non_camel_case_types)]\n\n");

    // Code enum.
    out.push_str("/// All known compile-warning codes from `svelte/compiler`.\n");
    out.push_str("///\n");
    out.push_str("/// Variant name matches the upstream snake_case code verbatim so the\n");
    out.push_str("/// `as_str` round-trip is trivial and generated-code scope is tiny.\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    out.push_str("pub enum Code {\n");
    for code in catalog.keys() {
        out.push_str("    ");
        out.push_str(code);
        out.push_str(",\n");
    }
    out.push_str("}\n\n");

    // as_str() + try_from_str().
    out.push_str("impl Code {\n");
    out.push_str("    pub const fn as_str(self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for code in catalog.keys() {
        out.push_str(&format!("            Self::{code} => \"{code}\",\n"));
    }
    out.push_str("        }\n");
    out.push_str("    }\n\n");
    out.push_str("    pub fn try_from_str(s: &str) -> Option<Self> {\n");
    out.push_str("        match s {\n");
    for code in catalog.keys() {
        out.push_str(&format!("            \"{code}\" => Some(Self::{code}),\n"));
    }
    out.push_str("            _ => None,\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // CODES array + count.
    out.push_str("/// All known codes, alphabetically sorted.\n");
    out.push_str(&format!(
        "pub const CODES: &[&str; {}] = &[\n",
        catalog.len()
    ));
    for code in catalog.keys() {
        out.push_str(&format!("    \"{code}\",\n"));
    }
    out.push_str("];\n");

    out
}

/// Render `messages.rs` — one function per warning.
///
/// Functions mirror upstream's signature: the node position is already
/// captured elsewhere (in `LintContext::emit`), so these return the
/// formatted *message* string only. Signature is one `&str` per
/// placeholder (in first-occurrence order) and, for overloaded
/// warnings, `Option<&str>` for the overload-toggling var.
fn render_messages(catalog: &BTreeMap<String, Warning>) -> String {
    let mut out = String::new();
    out.push_str(
        "// GENERATED — do not edit. Run `cargo run -p xtask --bin regen-lint-catalog`.\n",
    );
    out.push_str("//\n");
    out.push_str(
        "// Source: .svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*.md\n\n",
    );
    out.push_str("#![allow(non_snake_case, dead_code, clippy::too_many_arguments)]\n\n");
    out.push_str("//! Message-text builders for each warning code.\n\n");

    for (code, warn) in catalog {
        let vars_per_template: Vec<Vec<String>> = warn
            .templates
            .iter()
            .map(|t| collect_placeholders(t))
            .collect();
        let all_vars = union_preserve_order(&vars_per_template);
        render_one_message_fn(
            &mut out,
            code,
            &warn.templates,
            &vars_per_template,
            &all_vars,
        );
    }
    out
}

/// Emit one `pub fn <code>(args) -> String { … }`.
///
/// Variable names that collide with Rust keywords (`type`, `ref`, …) are
/// suffixed with `_` in the Rust signature; the templates are rewritten
/// accordingly so `%type%` → `{type_}`.
fn render_one_message_fn(
    out: &mut String,
    code: &str,
    templates: &[String],
    vars_per_template: &[Vec<String>],
    all_vars: &[String],
) {
    let href = format!("https://svelte.dev/e/{code}");

    // Determine function signature.
    let sig_args: Vec<String> = all_vars
        .iter()
        .map(|v| {
            let rust_name = safe_ident(v);
            // A var that's present in ALL templates is required (&str).
            // A var that's only in later-overload templates is
            // `Option<&str>` — passing `Some(..)` selects the
            // overload, `None` falls back to the earlier template.
            let required = vars_per_template.iter().all(|vs| vs.iter().any(|x| x == v));
            if required {
                format!("{rust_name}: &str")
            } else {
                format!("{rust_name}: Option<&str>")
            }
        })
        .collect();

    out.push_str("/// ");
    out.push_str(&templates[0].replace('\n', " "));
    out.push('\n');
    out.push_str(&format!("pub fn {code}("));
    out.push_str(&sig_args.join(", "));
    out.push_str(") -> String {\n");

    if templates.len() == 1 {
        out.push_str("    ");
        render_format_call(out, &templates[0], &vars_per_template[0], &href);
        out.push('\n');
    } else {
        // Overload: generate `if let Some(D) = d { … } else if … else { … }`
        // from rightmost template back to the leftmost.
        for (i, template) in templates.iter().enumerate().rev() {
            if i == 0 {
                // Base case — no `if`, followed by the closing `}`.
                out.push_str("    } else {\n        ");
                render_format_call(out, template, &vars_per_template[i], &href);
                out.push_str("\n    }\n");
            } else {
                let prev_vars = &vars_per_template[i - 1];
                let new_var = vars_per_template[i]
                    .iter()
                    .find(|v| !prev_vars.iter().any(|p| &p == v))
                    .map(String::as_str)
                    .unwrap_or_else(|| {
                        vars_per_template[i]
                            .last()
                            .map(String::as_str)
                            .unwrap_or("")
                    });
                let new_var_rust = safe_ident(new_var);
                if i == templates.len() - 1 {
                    out.push_str("    ");
                } else {
                    out.push_str("    } else ");
                }
                out.push_str(&format!(
                    "if let Some({new_var_rust}) = {new_var_rust} {{\n        "
                ));
                render_format_call(out, template, &vars_per_template[i], &href);
                out.push('\n');
            }
        }
    }
    out.push_str("}\n\n");
}

/// Emit the `format!("<template>\n<href>", args…)` call body.
///
/// Every variable referenced in `vars` is expected to already be in
/// scope at the call site as `&str` under its safe-ident name —
/// either because it's a required function argument (Required &str),
/// or because it's been bound by an enclosing `if let Some(x) = x`.
///
/// Overload-only vars that are `Option<&str>` at function-arg level
/// must only appear in templates that are guarded by `if let Some`
/// on that same var — that's the shape upstream's message overloads
/// preserve, and it's why we can treat every template's var list as
/// pointing at local `&str` bindings without per-var unwrap logic.
fn render_format_call(out: &mut String, template: &str, vars: &[String], href: &str) {
    out.push_str("format!(\"");
    out.push_str(&rust_format_template(template));
    out.push_str("\\n");
    // Backslash-escape URL characters conservatively for `format!`.
    for c in href.chars() {
        match c {
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    for v in vars {
        let rust_name = safe_ident(v);
        // Rust requires named args to match the identifier in the
        // format string — since `rust_format_template` emits
        // `{<safe_ident>}`, the named arg key must be `rust_name` too.
        // That identifier, the value, and the outer binding all match.
        out.push_str(&format!(", {rust_name} = {rust_name}"));
    }
    out.push(')');
}

/// Map an upstream variable name to a safe Rust identifier. Variables
/// whose names collide with Rust keywords get a trailing underscore.
fn safe_ident(name: &str) -> String {
    const RESERVED: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn",
    ];
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

/// Extract `%name%` placeholders from a template, in first-occurrence order.
fn collect_placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // Find closing %.
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'%' {
                end += 1;
            }
            if end < bytes.len() && end > start {
                let name = &template[start..end];
                if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    let s = name.to_string();
                    if !out.contains(&s) {
                        out.push(s);
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Convert a message template (with `%name%` placeholders and literal
/// backticks / quotes) into a Rust format!() template string.
///
/// Iterates on char boundaries so multi-byte UTF-8 (em-dash, curly
/// quotes, etc.) survives the transformation.
fn rust_format_template(template: &str) -> String {
    let mut out = String::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'%' {
                end += 1;
            }
            if end < bytes.len() && end > start {
                let name = &template[start..end];
                if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    out.push('{');
                    out.push_str(&safe_ident(name));
                    out.push('}');
                    i = end + 1;
                    continue;
                }
            }
        }

        // For ASCII, handle escapes. For multi-byte UTF-8, take the
        // whole scalar value verbatim.
        if b.is_ascii() {
            match b {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'{' => out.push_str("{{"),
                b'}' => out.push_str("}}"),
                _ => out.push(b as char),
            }
            i += 1;
        } else if let Some(ch) = template[i..].chars().next() {
            // Copy the full multi-byte scalar at `i..i+len`.
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

/// Union of var lists preserving first-occurrence order.
fn union_preserve_order(per_template: &[Vec<String>]) -> Vec<String> {
    let mut out = Vec::new();
    for vars in per_template {
        for v in vars {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
    }
    out
}
