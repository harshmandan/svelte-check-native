//! Which `<script>` blocks svelte2tsx recognises.
//!
//! svelte2tsx finds a component's script blocks with its own regular
//! expression (`utils/htmlxparser.ts` `scriptRegex`, see
//! [`svn_parser::script_tag_expression_spans`]), not with the Svelte
//! parser: an opening `<script …>` tag, the shortest run of text after
//! it, and a literal `</script>`. Comments are skipped. A block the
//! Svelte parser accepts but the expression misses — `</script\n>`,
//! say, with no literal close tag after it — is not a script to
//! svelte2tsx: it processes no script, and the block's text stays in
//! the template output verbatim. One the expression runs on past, to a
//! later script's literal close tag, merges with everything up to there
//! into one script.

use svn_core::Range;

/// The source spans (opening `<` through the closing `>`) of the script
/// blocks svelte2tsx's expression matches, in source order.
pub(crate) fn recognised_script_spans(source: &str) -> Vec<(usize, usize)> {
    svn_parser::script_tag_expression_spans(source)
}

/// The document as svelte2tsx sees its scripts when its expression runs
/// one script on past the parser's close tag: the Svelte parser closes
/// `<script …>…</script >`, but the expression's shortest match runs to
/// the next literal `</script>`, so everything up to there — any later
/// script block included — is the body of one script with the first
/// tag's attributes. `None` when no script extends that way.
///
/// Also returns where each close tag the merged body swallowed starts
/// (absolute offsets): text svelte2tsx reads as script code.
pub(crate) fn merge_extended_scripts<'src>(
    doc: &svn_parser::Document<'src>,
    spans: &[(usize, usize)],
) -> Option<(svn_parser::Document<'src>, Vec<u32>)> {
    let sections = [doc.module_script.as_ref(), doc.instance_script.as_ref()];
    let (first, end) = sections.iter().flatten().find_map(|s| {
        spans
            .iter()
            .find(|&&(start, end)| {
                start == s.open_tag_range.start as usize && end > s.close_tag_range.end as usize
            })
            .map(|&(_, end)| (*s, end))
    })?;
    let close_start = end - "</script>".len();
    let content_start = first.open_tag_range.end as usize;
    let content = doc.source.get(content_start..close_start)?;
    let merged = svn_parser::ScriptSection {
        open_tag_range: first.open_tag_range,
        content_range: Range::new(content_start as u32, close_start as u32),
        close_tag_range: Range::new(close_start as u32, end as u32),
        content,
        lang: first.lang,
        context: first.context,
        generics: first.generics.clone(),
        attrs: first.attrs.clone(),
    };
    let span = (first.open_tag_range.start as usize, end);
    let swallowed_close_tags = sections
        .iter()
        .flatten()
        .map(|s| s.close_tag_range.start)
        .filter(|&at| (at as usize) > span.0 && (at as usize) < close_start)
        .collect();
    let outside = |s: &Option<svn_parser::ScriptSection<'src>>| {
        s.clone()
            .filter(|s| (s.open_tag_range.start as usize) >= span.1)
    };
    let (module_script, instance_script) = match merged.context {
        svn_parser::ScriptContext::Module => (Some(merged), outside(&doc.instance_script)),
        svn_parser::ScriptContext::Instance => (outside(&doc.module_script), Some(merged)),
    };
    let merged_doc = svn_parser::Document {
        source: doc.source,
        module_script,
        instance_script,
        style: doc.style.clone(),
        template: svn_parser::Template {
            text_runs: doc
                .template
                .text_runs
                .iter()
                .copied()
                .filter(|r| (r.end as usize) <= span.0 || (r.start as usize) >= span.1)
                .collect(),
        },
    };
    Some((merged_doc, swallowed_close_tags))
}

/// The type-assertion rewrite svelte2tsx applies to a close tag that a
/// merged module script swallowed, as `(start, end)` offsets into the
/// script body: remove the `<` at `start` and append ` as ` at `end`.
///
/// svelte2tsx rewrites every `<T>expr` in a module script to
/// `expr as T` (`handleTypeAssertion`). TypeScript reads a `</script…>`
/// that opens a statement as exactly that: `<`, a missing type, and a
/// regular expression starting at the `/` that runs to its closing `/`
/// or, unterminated, to the end of the line. The type is empty, so the
/// rewrite drops the `<` and puts ` as ` after the regular expression.
/// A tag that does not open a statement is the right operand of a `<`
/// comparison instead, and is left alone; oxc parsing the body up to
/// the tag cleanly is how a statement start is recognised.
pub(crate) fn close_tag_type_assertion(content: &str, tag: usize) -> Option<(usize, usize)> {
    if content.get(tag..)?.as_bytes().get(..2)? != b"</" {
        return None;
    }
    let alloc = oxc_allocator::Allocator::default();
    let prefix = &content[..tag];
    let parsed = svn_parser::parse_script_body(&alloc, prefix, svn_parser::ScriptLang::Ts);
    if parsed.panicked || !parsed.errors.is_empty() || !prefix_ends_statement(prefix, &parsed) {
        return None;
    }
    Some((tag, svn_parser::typescript_regex_end(content, tag + 1)))
}

/// Whether the text after the prefix's last statement is only
/// whitespace and comments, so the next token opens a statement.
fn prefix_ends_statement(prefix: &str, parsed: &svn_parser::ParsedScript<'_>) -> bool {
    let last_end = parsed
        .program
        .body
        .last()
        .map_or(0, |st| oxc_span::GetSpan::span(st).end as usize);
    let comments_end = parsed
        .program
        .comments
        .iter()
        .map(|c| c.span.end as usize)
        .filter(|&e| e > last_end)
        .max()
        .unwrap_or(last_end);
    // A statement that relies on automatic semicolon insertion would
    // continue into the tag as a comparison; one closed by `;`, or a
    // declaration closed by its `}`, cannot.
    use oxc_ast::ast::Statement as St;
    let ends_cleanly = parsed.program.body.last().is_none_or(|st| {
        prefix[..last_end].trim_end().ends_with(';')
            || matches!(
                st,
                St::FunctionDeclaration(_)
                    | St::ClassDeclaration(_)
                    | St::BlockStatement(_)
                    | St::TSInterfaceDeclaration(_)
                    | St::TSEnumDeclaration(_)
                    | St::TSNamespaceDeclaration(_)
            )
    });
    ends_cleanly && prefix[comments_end.max(last_end)..].trim().is_empty()
}

thread_local! {
    static SWALLOWED_CLOSE_TAGS: std::cell::RefCell<Vec<u32>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `f` with `tags` recorded as the close tags a merged script body
/// swallowed (see [`merge_extended_scripts`]).
pub(crate) fn with_swallowed_close_tags<R>(tags: Vec<u32>, f: impl FnOnce() -> R) -> R {
    let previous = SWALLOWED_CLOSE_TAGS.with(|v| std::mem::replace(&mut *v.borrow_mut(), tags));
    let out = f();
    SWALLOWED_CLOSE_TAGS.with(|v| *v.borrow_mut() = previous);
    out
}

/// The type-assertion rewrites (see [`close_tag_type_assertion`]) to
/// apply to a module script body starting at `base`.
pub(crate) fn module_script_rewrites(content: &str, base: u32) -> Vec<(usize, usize)> {
    SWALLOWED_CLOSE_TAGS.with(|v| {
        v.borrow()
            .iter()
            .filter_map(|&at| at.checked_sub(base))
            .filter_map(|rel| close_tag_type_assertion(content, rel as usize))
            .collect()
    })
}

/// Whether svelte2tsx recognises the script block spanning `open`
/// through `close`.
pub(crate) fn is_recognised(spans: &[(usize, usize)], open: Range, close: Range) -> bool {
    spans.contains(&(open.start as usize, close.end as usize))
}
