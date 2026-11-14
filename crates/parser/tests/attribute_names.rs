//! Where an attribute's name ends.
//!
//! The compiler's `read_tag_name` ends an attribute name at white space,
//! `/`, `>`, a quote or `=`, and at nothing else: not at `<`, and a `[`
//! does not keep a `>` or `/` inside the name. Each case lists the
//! attribute spans svelte 5.57.3's `parse()` gives the first node.

use svn_parser::ast::Node;

fn attribute_spans(source: &str) -> Vec<(u32, u32)> {
    let (doc, _) = svn_parser::parse_sections(source);
    let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
    let attributes = match fragment.nodes.first() {
        Some(Node::Element(element)) => &element.attributes,
        Some(Node::Component(component)) => &component.attributes,
        other => panic!("{source:?}: first node is {other:?}"),
    };
    attributes
        .iter()
        .map(|attribute| (attribute.range().start, attribute.range().end))
        .collect()
}

#[test]
fn attribute_names_end_where_the_compiler_ends_them() {
    let cases: &[(&str, &[(u32, u32)])] = &[
        ("<div class:[&>li]:x={c}></div>", &[(5, 13)]),
        ("<div a<b></div>", &[(5, 8)]),
        ("<div class:a<b={c}></div>", &[(5, 18)]),
        ("<div a=1<b></div>", &[(5, 8), (8, 10)]),
        ("<Comp <x></Comp>", &[(6, 8)]),
        ("<div class:bg-[var(--p)]={c}></div>", &[(5, 28)]),
        ("<div a\u{c}b></div>", &[(5, 6), (7, 8)]),
    ];
    let mut failures = Vec::new();
    for (source, want) in cases {
        let got = attribute_spans(source);
        if got != *want {
            failures.push(format!("{source:?}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
