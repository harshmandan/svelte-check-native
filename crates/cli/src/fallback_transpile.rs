//! Compiling a component the way svelte-check's fallback preprocessor
//! hands it to the compiler.
//!
//! With no Svelte config, the language server preprocesses each
//! component with `svelte.preprocess` and a script preprocessor that
//! runs `ts.transpileModule` (ES-next module and target, source map,
//! `verbatimModuleSyntax`) over every `<script>` whose `lang` attribute
//! is exactly `ts`. The compiler then reads the preprocessed component,
//! and every position it reports is mapped back through the source map
//! the preprocessing produced (`TraceMap` + `originalPositionFor`).
//!
//! The lint pass models that transpile on the original text. For
//! components the model cannot follow (see `svn_lint::LintReport`),
//! this module produces the real preprocessed component: tsgo prints
//! each script (one batch process for the whole run), the scripts are
//! spliced in exactly where the preprocessor splices them, and the
//! whole-component source map is rebuilt with the preprocessor's own
//! rules. The component is then linted as the compiler sees it and each
//! diagnostic is mapped back.
//!
//! The source map `svelte.preprocess` builds has two kinds of region:
//!
//! - text it leaves alone gets an identity map with one segment per
//!   token of each line, where a token is a run of word characters,
//!   one other non-space character, or a run of whitespace; a position
//!   inside a token therefore maps to the token's start, and a position
//!   on an empty line maps nowhere;
//! - a transpiled script's content carries TypeScript's own map, moved
//!   to where the content starts.
//!
//! A position that maps nowhere lands at the file's start (the language
//! server clamps the negative line it gets).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One `<script>` tag as `svelte.preprocess` finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptTag {
    /// Byte range of the content between the open and the close tag.
    pub content: std::ops::Range<usize>,
    /// Whether the fallback preprocessor transpiles it (`lang` is `ts`).
    pub transpiled: bool,
}

/// One match of the tag pattern `svelte.preprocess` uses for scripts:
///
/// ```text
/// <!--[^]*?-->|<script((?:\s+[^=>'"/\s]+=(?:"[^"]*"|'[^']*'|[^>\s]+)|\s+[^=>'"/\s]+)*\s*)(?:\/>|>([\S\s]*?)<\/script>)
/// ```
///
/// Comments are matched so a tag inside one does not count.
enum TagMatch {
    Comment,
    Script {
        /// The attribute text (whitespace included).
        attrs: std::ops::Range<usize>,
        /// The content; `None` for a self-closing tag.
        content: Option<std::ops::Range<usize>>,
    },
}

fn tag_matches(source: &str) -> Vec<TagMatch> {
    let bytes = source.as_bytes();
    let mut matches = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &source[at..];
        if rest.starts_with("<!--")
            && let Some(end) = rest[4..].find("-->")
        {
            matches.push(TagMatch::Comment);
            at += 4 + end + 3;
            continue;
        }
        if rest.starts_with("<script")
            && let Some((attrs, content, end)) = match_script_tag(source, at + "<script".len())
        {
            matches.push(TagMatch::Script { attrs, content });
            at = end;
            continue;
        }
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
    matches
}

/// The script tags `svelte.preprocess` hands the script preprocessor
/// with content, in source order.
pub(crate) fn script_tags(source: &str) -> Vec<ScriptTag> {
    tag_matches(source)
        .into_iter()
        .filter_map(|m| match m {
            TagMatch::Script {
                attrs,
                content: Some(content),
            } => Some(ScriptTag {
                transpiled: lang_is_ts(&source[attrs]),
                content,
            }),
            _ => None,
        })
        .collect()
}

/// How far the fallback preprocessor's asynchronous steps take a
/// component before it reaches the compiler, relative to the others:
/// 0 when the tag pattern matches nothing, 1 when every match settles
/// without calling the script preprocessor (a comment, a tag with
/// neither attribute text nor content), 2 when some tag is handed to
/// the preprocessor (whatever its `lang`). Each level awaits a few more
/// promise resolutions, and components start their diagnostics
/// together, so a lower level reaches the compiler first.
pub(crate) fn preprocess_steps(source: &str) -> u8 {
    tag_matches(source)
        .iter()
        .map(|m| match m {
            TagMatch::Comment => 1,
            TagMatch::Script { attrs, content } => {
                if attrs.is_empty() && content.as_ref().is_none_or(|c| c.is_empty()) {
                    1
                } else {
                    2
                }
            }
        })
        .max()
        .unwrap_or(0)
}

fn is_js_space(c: char) -> bool {
    // JavaScript's `\s`: Unicode white space plus the byte-order mark,
    // without NEL.
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn is_js_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The rest of the tag pattern after `<script`, from `from`: the
/// attribute text range, the content range (`None` for a self-closing
/// tag) and the end of the match.
#[allow(clippy::type_complexity)]
fn match_script_tag(
    source: &str,
    from: usize,
) -> Option<(
    std::ops::Range<usize>,
    Option<std::ops::Range<usize>>,
    usize,
)> {
    let run = |at: usize, pred: &dyn Fn(char) -> bool| -> usize {
        source[at..]
            .char_indices()
            .find(|&(_, c)| !pred(c))
            .map_or(source.len(), |(i, _)| at + i)
    };
    let name_char = |c: char| !matches!(c, '=' | '>' | '\'' | '"' | '/') && !is_js_space(c);
    let mut at = from;
    // Where the last attribute's unquoted value ends, when the tag
    // ends right after it: the value may give its last `/` back to
    // close the tag as `/>`.
    let mut unquoted_value_end = None;
    loop {
        let ws_end = run(at, &is_js_space);
        if ws_end == at {
            break;
        }
        let name_end = run(ws_end, &name_char);
        if name_end == ws_end {
            break;
        }
        at = name_end;
        unquoted_value_end = None;
        if source[at..].starts_with('=') {
            let value = at + 1;
            let quoted = |q: char| {
                source[value..]
                    .strip_prefix(q)
                    .and_then(|body| body.find(q))
                    .map(|len| value + 1 + len + 1)
            };
            if let Some(end) = quoted('"').or_else(|| quoted('\'')) {
                at = end;
            } else {
                let end = run(value, &|c| c != '>' && !is_js_space(c));
                if end > value {
                    at = end;
                    unquoted_value_end = Some((value, end));
                }
                // Otherwise `=` is not part of the attribute list and
                // the tag cannot end here: the match fails below.
            }
        }
    }
    let attrs = from..at;
    let close = run(at, &is_js_space);
    if source[close..].starts_with("/>") {
        return Some((attrs, None, close + 2));
    }
    if source[close..].starts_with('>') {
        let content_start = close + 1;
        if let Some(len) = source[content_start..].find("</script>") {
            let content_end = content_start + len;
            return Some((
                attrs,
                Some(content_start..content_end),
                content_end + "</script>".len(),
            ));
        }
        // No closing tag anywhere: an unquoted value ending in `/`
        // right before `>` can still close the tag as `/>`.
        if let Some((value, end)) = unquoted_value_end
            && end == close
            && end - 1 > value
            && source[..end].ends_with('/')
        {
            return Some((from..end - 1, None, end + 1));
        }
    }
    None
}

/// Whether the attribute text gives `lang` the value `ts`, read the way
/// `svelte.preprocess` reads attributes into an object (the last `lang`
/// wins; a valueless or empty-valued attribute is `true`):
/// `([\w-$]+\b)(?:=(?:"([^"]*)"|'([^']*)'|(\S+)))?`, matched repeatedly.
fn lang_is_ts(attrs: &str) -> bool {
    let chars: Vec<(usize, char)> = attrs.char_indices().collect();
    let name_char = |c: char| is_js_word(c) || c == '-' || c == '$';
    let word_at = |i: usize| chars.get(i).is_some_and(|&(_, c)| is_js_word(c));
    let mut lang: Option<bool> = None;
    let mut i = 0;
    while i < chars.len() {
        if !name_char(chars[i].1) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && name_char(chars[end].1) {
            end += 1;
        }
        // `\b` after the name: give characters back until the name ends
        // on a word boundary.
        while end > i && word_at(end - 1) == word_at(end) {
            end -= 1;
        }
        if end == i {
            i += 1;
            continue;
        }
        let byte = |k: usize| chars.get(k).map_or(attrs.len(), |&(b, _)| b);
        let name = &attrs[byte(i)..byte(end)];
        let mut next = end;
        let mut value: Option<&str> = None;
        if chars.get(end).is_some_and(|&(_, c)| c == '=') {
            let v = end + 1;
            let quoted = |q: char| -> Option<usize> {
                if chars.get(v).is_some_and(|&(_, c)| c == q) {
                    (v + 1..chars.len()).find(|&k| chars[k].1 == q)
                } else {
                    None
                }
            };
            if let Some(close) = quoted('"').or_else(|| quoted('\'')) {
                value = Some(&attrs[byte(v + 1)..byte(close)]);
                next = close + 1;
            } else {
                let mut e = v;
                while e < chars.len() && !is_js_space(chars[e].1) {
                    e += 1;
                }
                if e > v {
                    value = Some(&attrs[byte(v)..byte(e)]);
                    next = e;
                }
            }
        }
        if name == "lang" {
            lang = Some(value.is_some_and(|v| v == "ts"));
        }
        i = next;
    }
    lang == Some(true)
}

/// One source-map segment: generated column, then the original line
/// and column it maps from (`None` for a segment that maps nowhere).
/// Columns count UTF-16 code units, as source maps do.
type Segment = (u32, Option<(u32, u32)>);

/// A script printed by TypeScript, with its source map (lines of
/// segments, positions relative to the script's content).
#[derive(Debug, Clone)]
pub(crate) struct PrintedScript {
    pub code: String,
    pub map: Vec<Vec<Segment>>,
}

/// Print every script with tsgo, the way `ts.transpileModule` prints it
/// for the fallback preprocessor. `None` for a script tsgo cannot
/// print.
///
/// The scripts go through tsgo in as few processes as possible. tsgo
/// can crash printing a script TypeScript itself recovers from; a crash
/// ends the whole process, possibly halfway through writing an output,
/// so a batch that crashed is split in two and each half printed again,
/// down to the single script that cannot be printed.
pub(crate) fn print_scripts(workspace: &Path, contents: &[&str]) -> Vec<Option<PrintedScript>> {
    match svn_typecheck::discover(workspace) {
        Ok(tsgo) => print_split(&tsgo, contents),
        Err(_) => vec![None; contents.len()],
    }
}

fn print_split(tsgo: &svn_typecheck::TsgoBinary, contents: &[&str]) -> Vec<Option<PrintedScript>> {
    if let Some(printed) = print_batch(tsgo, contents) {
        return printed;
    }
    if contents.len() <= 1 {
        return vec![None; contents.len()];
    }
    let (left, right) = contents.split_at(contents.len() / 2);
    let (mut left, right) = rayon::join(|| print_split(tsgo, left), || print_split(tsgo, right));
    left.extend(right);
    left
}

/// One tsgo process over `contents`; `None` when it crashed.
fn print_batch(
    tsgo: &svn_typecheck::TsgoBinary,
    contents: &[&str],
) -> Option<Vec<Option<PrintedScript>>> {
    let dir = scratch_dir()?;
    let printed = print_in(&dir, tsgo, contents);
    let _ = std::fs::remove_dir_all(&dir);
    printed
}

fn scratch_dir() -> Option<PathBuf> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "svelte-check-native-transpile-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("in")).ok()?;
    Some(dir)
}

fn print_in(
    dir: &Path,
    tsgo: &svn_typecheck::TsgoBinary,
    contents: &[&str],
) -> Option<Vec<Option<PrintedScript>>> {
    let mut files = Vec::with_capacity(contents.len());
    for (i, content) in contents.iter().enumerate() {
        let name = format!("in/{i}.ts");
        std::fs::write(dir.join(&name), content).ok()?;
        files.push(serde_json::Value::String(name));
    }
    // `transpileModule`'s settings: no type information, nothing
    // resolved, each file on its own. Every file is a module so that
    // scripts without imports or exports do not share one global scope
    // (a namespace or enum merged across them prints differently).
    let tsconfig = serde_json::json!({
        "compilerOptions": {
            "target": "esnext",
            "module": "esnext",
            "moduleDetection": "force",
            "sourceMap": true,
            "verbatimModuleSyntax": true,
            "isolatedModules": true,
            "noCheck": true,
            "noResolve": true,
            "noLib": true,
            "types": [],
            "noEmitOnError": false,
            "newLine": "lf",
            "rootDir": "in",
            "outDir": "out"
        },
        "files": files,
    });
    std::fs::write(dir.join("tsconfig.json"), tsconfig.to_string()).ok()?;
    let mut cmd = if tsgo.needs_node {
        let mut c = Command::new("node");
        c.arg(&tsgo.path);
        c
    } else {
        Command::new(&tsgo.path)
    };
    // Syntax errors are expected (the printed recovery is the point), so
    // the exit status does not tell a crash apart; the Go runtime's
    // report on stderr does.
    let output = cmd
        .arg("--project")
        .arg(dir.join("tsconfig.json"))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code().is_none()
        || stderr.contains("panic:")
        || stderr.contains("fatal error:")
    {
        return None;
    }
    Some(
        (0..contents.len())
            .map(|i| {
                let code = std::fs::read_to_string(dir.join(format!("out/{i}.js"))).ok()?;
                let map = std::fs::read_to_string(dir.join(format!("out/{i}.js.map"))).ok()?;
                let map: serde_json::Value = serde_json::from_str(&map).ok()?;
                let mappings = decode_mappings(map.get("mappings")?.as_str()?)?;
                Some(as_transpile_module_prints(code, mappings))
            })
            .collect(),
    )
}

/// tsgo output in `ts.transpileModule`'s form: a file forced to be a
/// module gets an `export {};` line nothing maps to when it has no
/// import or export of its own, which `transpileModule` does not add.
/// The trailing `sourceMappingURL` comment stays: the preprocessor
/// removes it (see [`strip_source_mapping_url`]).
fn as_transpile_module_prints(code: String, mut map: Vec<Vec<Segment>>) -> PrintedScript {
    const MARKER: &str = "export {};\n";
    let body_end = code.rfind("//# sourceMappingURL=").unwrap_or(code.len());
    if code[..body_end].ends_with(MARKER) {
        let start = body_end - MARKER.len();
        let line = code[..start].matches('\n').count();
        if map.get(line).is_none_or(Vec::is_empty) {
            if line < map.len() {
                map.remove(line);
            }
            return PrintedScript {
                code: format!("{}{}", &code[..start], &code[body_end..]),
                map,
            };
        }
    }
    PrintedScript { code, map }
}

/// The preprocessor removes the first `//# sourceMappingURL=…` comment
/// from a script's output (`parse_attached_sourcemap`).
fn strip_source_mapping_url(code: &str) -> String {
    let mut from = 0;
    while let Some(rel) = code[from..].find("//") {
        let start = from + rel;
        let mut rest = &code[start + 2..];
        if let Some(r) = rest.strip_prefix(['#', '@']) {
            rest = r.trim_start_matches(is_js_space);
            if let Some(r) = rest.strip_prefix("sourceMappingURL") {
                rest = r.trim_start_matches(is_js_space);
                if let Some(r) = rest.strip_prefix('=') {
                    rest = r.trim_start_matches(is_js_space);
                    let url_len = rest.find(is_js_space).unwrap_or(rest.len());
                    let end = code.len() - rest.len() + url_len;
                    return format!("{}{}", &code[..start], &code[end..]);
                }
            }
        }
        from = start + 2;
    }
    code.to_string()
}

/// Decode a source map's `mappings` (base64 VLQ, one source).
fn decode_mappings(mappings: &str) -> Option<Vec<Vec<Segment>>> {
    let mut lines = vec![Vec::new()];
    let (mut src_line, mut src_col) = (0i64, 0i64);
    for line in mappings.split(';') {
        let segments = lines.last_mut()?;
        let mut gen_col = 0i64;
        for segment in line.split(',').filter(|s| !s.is_empty()) {
            let fields = decode_vlq(segment)?;
            gen_col += *fields.first()?;
            if fields.len() >= 4 {
                src_line += fields[2];
                src_col += fields[3];
                segments.push((
                    u32::try_from(gen_col).ok()?,
                    Some((u32::try_from(src_line).ok()?, u32::try_from(src_col).ok()?)),
                ));
            } else {
                segments.push((u32::try_from(gen_col).ok()?, None));
            }
        }
        lines.push(Vec::new());
    }
    lines.pop();
    Some(lines)
}

fn decode_vlq(segment: &str) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    let (mut value, mut shift) = (0i64, 0u32);
    for b in segment.bytes() {
        let digit = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as i64;
        value += (digit & 31) << shift;
        if digit & 32 != 0 {
            shift += 5;
            continue;
        }
        let negative = value & 1 != 0;
        value >>= 1;
        out.push(if negative { -value } else { value });
        value = 0;
        shift = 0;
    }
    Some(out)
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Text with its source map, combined the way the preprocessor's
/// `MappedCode` concatenates pieces.
struct MappedCode {
    string: String,
    lines: Vec<Vec<Segment>>,
}

impl MappedCode {
    fn empty() -> Self {
        Self {
            string: String::new(),
            lines: Vec::new(),
        }
    }

    /// Text left as it was, starting at original `(line, column)`: one
    /// segment per token of each line.
    fn from_source(text: &str, (line, column): (u32, u32)) -> Self {
        if text.is_empty() {
            return Self::empty();
        }
        let mut lines = Vec::new();
        for (i, text_line) in text.split('\n').enumerate() {
            let shift = if i == 0 { column } else { 0 };
            let mut segments = Vec::new();
            let mut col = 0u32;
            let mut chars = text_line.chars().peekable();
            while let Some(c) = chars.next() {
                segments.push((col, Some((line + i as u32, col + shift))));
                col += c.len_utf16() as u32;
                let same_token: &dyn Fn(char) -> bool = if is_js_word(c) {
                    &is_js_word
                } else if is_js_space(c) {
                    &is_js_space
                } else {
                    &|_| false
                };
                while let Some(&next) = chars.peek()
                    && same_token(next)
                {
                    col += next.len_utf16() as u32;
                    chars.next();
                }
            }
            lines.push(segments);
        }
        Self {
            string: text.to_string(),
            lines,
        }
    }

    /// Preprocessor output with its map, padded to one map line per
    /// line of code.
    fn from_processed(code: String, mut lines: Vec<Vec<Segment>>) -> Self {
        let line_count = code.split('\n').count();
        while lines.len() < line_count {
            lines.push(Vec::new());
        }
        Self {
            string: code,
            lines,
        }
    }

    fn concat(&mut self, other: Self) {
        if other.string.is_empty() {
            return;
        }
        if self.string.is_empty() {
            *self = other;
            return;
        }
        let column_offset = utf16_len(&self.string[self.string.rfind('\n').map_or(0, |i| i + 1)..]);
        self.string.push_str(&other.string);
        let mut other_lines = other.lines.into_iter();
        let Some(first) = other_lines.next() else {
            return;
        };
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        if let Some(last) = self.lines.last_mut() {
            last.extend(first.into_iter().map(|(c, o)| (c + column_offset, o)));
        }
        self.lines.extend(other_lines);
    }
}

/// A component as the fallback preprocessor hands it to the compiler.
pub(crate) struct Preprocessed {
    pub text: String,
    /// Per generated line, segments sorted by generated column.
    lines: Vec<Vec<Segment>>,
}

/// Splice the printed scripts (one per transpiled tag, in order) into
/// `source`.
pub(crate) fn preprocess(
    source: &str,
    tags: &[ScriptTag],
    printed: &[PrintedScript],
) -> Preprocessed {
    let locate = Locator::new(source);
    let mut out = MappedCode::empty();
    let mut last = 0;
    let mut printed = printed.iter();
    for tag in tags.iter().filter(|t| t.transpiled) {
        let Some(script) = printed.next() else {
            break;
        };
        out.concat(MappedCode::from_source(
            &source[last..tag.content.start],
            locate.at(last),
        ));
        let (base_line, base_col) = locate.at(tag.content.start);
        let lines = script
            .map
            .iter()
            .map(|segments| {
                segments
                    .iter()
                    .map(|&(gen_col, orig)| {
                        let orig = orig.map(|(line, col)| {
                            let col = if line == 0 { col + base_col } else { col };
                            (line + base_line, col)
                        });
                        (gen_col, orig)
                    })
                    .collect()
            })
            .collect();
        out.concat(MappedCode::from_processed(
            strip_source_mapping_url(&script.code),
            lines,
        ));
        last = tag.content.end;
    }
    out.concat(MappedCode::from_source(&source[last..], locate.at(last)));
    let mut lines = out.lines;
    for line in &mut lines {
        line.sort_by_key(|&(col, _)| col);
    }
    Preprocessed {
        text: out.string,
        lines,
    }
}

impl Preprocessed {
    /// `originalPositionFor` (greatest lower bound) for a 0-based line
    /// and UTF-16 column: the original 0-based line and column, `None`
    /// when nothing maps there.
    pub(crate) fn original_position(&self, line: u32, column: u32) -> Option<(u32, u32)> {
        let segments = self.lines.get(line as usize)?;
        let after = segments.partition_point(|&(c, _)| c <= column);
        let found = after.checked_sub(1)?;
        // Several segments at one column: the first of them.
        let col = segments[found].0;
        let first = segments[..found]
            .iter()
            .rposition(|&(c, _)| c != col)
            .map_or(0, |i| i + 1);
        segments[first].1
    }
}

/// Offsets to 0-based `(line, UTF-16 column)`.
struct Locator<'s> {
    source: &'s str,
    line_starts: Vec<usize>,
}

impl<'s> Locator<'s> {
    fn new(source: &'s str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            source,
            line_starts,
        }
    }

    fn at(&self, offset: usize) -> (u32, u32) {
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let start = self.line_starts[line];
        (line as u32, utf16_len(&self.source[start..offset]))
    }
}

/// The language server's handling of one compiler diagnostic range in
/// the preprocessed component (0-based lines, UTF-16 columns): map both
/// ends back (`mapObjWithRangeToOriginal`), then repair what did not map
/// (`adjustMappings`).
pub(crate) fn map_range(
    pre: &Preprocessed,
    start: (u32, u32),
    end: (u32, u32),
) -> ((u32, u32), (u32, u32)) {
    let lookup = |(line, col): (u32, u32)| -> (i64, i64) {
        match pre.original_position(line, col) {
            Some((l, c)) => (l as i64, c as i64),
            None => (-1, 0),
        }
    };
    let mut s = lookup(start);
    let mut e = lookup(end);
    // A range may map one character short: widen it back when it stays
    // on one line.
    if s.0 == e.0 && start.0 == end.0 && e.1 - s.1 == end.1 as i64 - start.1 as i64 - 1 {
        e.1 += 1;
    }
    for p in [&mut s, &mut e] {
        if p.1 < 0 {
            p.1 = 0;
        }
        if p.0 < 0 {
            *p = (0, 0);
        }
    }
    if s > e {
        s = e;
    }
    ((s.0 as u32, s.1 as u32), (e.0 as u32, e.1 as u32))
}

/// Whether the language server drops an `export_let_unused` warning:
/// TypeScript prints `export enum A` and `export namespace A` as
/// `export var A`, which the compiler reports, so a warning naming a
/// binding the original text exports as an enum or namespace is
/// silenced (`isNoFalsePositive`).
pub(crate) fn is_transpile_false_positive(code: &str, message: &str, original: &str) -> bool {
    if code != "export_let_unused" {
        return false;
    }
    let (Some(first), Some(last)) = (message.find('\''), message.rfind('\'')) else {
        return false;
    };
    // The name goes into a regular expression unescaped
    // (`\bexport\s+?(enum|namespace)\s+?NAME\b`), where a `$` anchors
    // the end of the text and never matches.
    let name = message.get(first + 1..last).unwrap_or("");
    if name.is_empty() || name.contains('$') {
        return false;
    }
    let boundary = |before: Option<char>, after: Option<char>| {
        before.is_some_and(is_js_word) != after.is_some_and(is_js_word)
    };
    let spaces = |s: &str| s.len() - s.trim_start_matches(is_js_space).len();
    original.match_indices("export").any(|(at, kw)| {
        if !boundary(original[..at].chars().next_back(), Some('e')) {
            return false;
        }
        let rest = &original[at + kw.len()..];
        let gap = spaces(rest);
        if gap == 0 {
            return false;
        }
        ["enum", "namespace"].iter().any(|keyword| {
            let Some(after) = rest[gap..].strip_prefix(keyword) else {
                return false;
            };
            let gap = spaces(after);
            gap > 0
                && after[gap..]
                    .strip_prefix(name)
                    .is_some_and(|tail| boundary(name.chars().next_back(), tail.chars().next()))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(source: &str) -> Vec<(&str, bool)> {
        script_tags(source)
            .into_iter()
            .map(|t| (&source[t.content], t.transpiled))
            .collect()
    }

    #[test]
    fn finds_script_tags_like_the_preprocessor() {
        assert_eq!(
            tags("<script lang=\"ts\">a</script><script>b</script>"),
            vec![("a", true), ("b", false)]
        );
        assert_eq!(tags("<!-- <script lang=\"ts\">a</script> -->"), vec![]);
        assert_eq!(tags("<script lang=ts>a</script>"), vec![("a", true)]);
        assert_eq!(
            tags("<script lang='ts' lang=\"js\">a</script>"),
            vec![("a", false)]
        );
        assert_eq!(tags("<script src=x/>"), vec![]);
        assert_eq!(
            tags("<script\n  lang=\"ts\"\n>a</script>"),
            vec![("a", true)]
        );
        assert_eq!(tags("<scripts>a</script>"), vec![]);
        assert_eq!(tags("<script lang=\"ts\">a</script >"), vec![]);
    }

    #[test]
    fn preprocess_steps_follow_what_the_preprocessor_awaits() {
        assert_eq!(preprocess_steps("<p>{a}</p>"), 0);
        assert_eq!(preprocess_steps("<!-- c --><p>{a}</p>"), 1);
        assert_eq!(preprocess_steps("<script></script><p>{a}</p>"), 1);
        assert_eq!(preprocess_steps("<script>let a = 1;</script>"), 2);
        assert_eq!(preprocess_steps("<script lang=\"ts\"></script>"), 2);
    }

    #[test]
    fn identity_regions_map_to_token_starts() {
        let source = "<p>{a}</p>\n\nabc def";
        let pre = preprocess(source, &[], &[]);
        assert_eq!(pre.original_position(0, 1), Some((0, 1)));
        assert_eq!(pre.original_position(2, 2), Some((2, 0)));
        assert_eq!(pre.original_position(2, 5), Some((2, 4)));
        assert_eq!(pre.original_position(1, 0), None);
    }

    #[test]
    fn decodes_vlq_mappings() {
        let lines = decode_mappings(";AACA,GAAG").expect("valid");
        assert_eq!(
            lines,
            vec![vec![], vec![(0, Some((1, 0))), (3, Some((1, 3)))]]
        );
    }

    #[test]
    fn strips_the_first_source_mapping_url() {
        assert_eq!(
            strip_source_mapping_url("a;\n//# sourceMappingURL=0.js.map"),
            "a;\n"
        );
    }

    #[test]
    fn enum_exports_silence_unused_export_warnings() {
        let message = "Component has unused export property 'E'. If it is for external reference only, please consider using `export const E`";
        assert!(is_transpile_false_positive(
            "export_let_unused",
            message,
            "export enum E { A }"
        ));
        assert!(!is_transpile_false_positive(
            "export_let_unused",
            message,
            "export enum EE { A }"
        ));
    }
}
