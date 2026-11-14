//! An unquoted directive value, read as the compiler's
//! `read_attribute_value` reads every attribute's: a sequence of text
//! and `{…}` chunks up to white space, a quote, `=`, `<`, `>`, a
//! backtick or `/>`. Each case lists what svelte 5.57.3's `parse()`
//! gives the directive: its span and its value's chunks.

use svn_parser::ast::{AttrValuePart, Attribute, DirectiveValue, Node};

/// (span, chunks): a chunk is `("text" | "expression", start, end)`, an
/// expression's span being its `{…}`.
type Read = ((u32, u32), Vec<(&'static str, u32, u32)>);

fn directive(source: &str) -> Vec<Read> {
    let (doc, _) = svn_parser::parse_sections(source);
    let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
    let Some(Node::Element(element)) = fragment.nodes.first() else {
        panic!("{source:?}: no element");
    };
    element
        .attributes
        .iter()
        .map(|attribute| {
            let range = attribute.range();
            let chunks = match attribute {
                Attribute::Directive(d) => match &d.value {
                    Some(DirectiveValue::Quoted(value)) => value
                        .parts
                        .iter()
                        .map(|part| match part {
                            AttrValuePart::Text { range } => ("text", range.start, range.end),
                            AttrValuePart::Expression { range, .. } => {
                                ("expression", range.start, range.end)
                            }
                        })
                        .collect(),
                    Some(DirectiveValue::Expression { range, .. }) => {
                        vec![("expression", range.start, range.end)]
                    }
                    _ => Vec::new(),
                },
                _ => vec![("attribute", range.start, range.end)],
            };
            ((range.start, range.end), chunks)
        })
        .collect()
}

#[test]
fn unquoted_directive_values_are_text_and_expression_chunks() {
    for (source, want) in [
        (
            "<div style:color=string{mixed}></div>",
            vec![((5, 30), vec![("text", 17, 23), ("expression", 23, 30)])],
        ),
        (
            "<div style:color={a}b></div>",
            vec![((5, 21), vec![("expression", 17, 20), ("text", 20, 21)])],
        ),
        (
            "<div style:color=a/b></div>",
            vec![((5, 20), vec![("text", 17, 20)])],
        ),
        (
            "<div style:color={a}></div>",
            vec![((5, 20), vec![("expression", 17, 20)])],
        ),
    ] {
        assert_eq!(directive(source), want, "{source:?}");
    }
}
