//! A `<style>` or `<script>` element inside the template.
//!
//! The compiler takes the body of an element named exactly `style` or
//! `script` as text, up to the first literal `</style>` / `</script>`
//! (`indexOf`), and then requires that close tag. Any other spelling of
//! the name (`<sTyle>`) is an ordinary element. Each case is what svelte
//! 5.57.3's `parse()` does with the template.

use svn_parser::ast::{Fragment, Node};

fn parse(source: &str) -> (Fragment, Vec<(&'static str, u32)>) {
    let (doc, _) = svn_parser::parse_sections(source);
    let (fragment, errors) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
    let errors = errors
        .iter()
        .map(|e| (e.code_slug(), e.range().start))
        .collect();
    (fragment, errors)
}

/// The children of the element nested in the first node.
fn inner_children(fragment: &Fragment) -> &[Node] {
    let Some(Node::Element(outer)) = fragment.nodes.first() else {
        panic!("first node is not an element: {:?}", fragment.nodes.first());
    };
    let Some(Node::Element(inner)) = outer.children.nodes.first() else {
        panic!("no nested element: {:?}", outer.children.nodes);
    };
    &inner.children.nodes
}

#[test]
fn the_close_tag_is_the_first_literal_one() {
    let (fragment, errors) = parse("<div><style>a{}</STYLE></style></div>");
    assert_eq!(errors, vec![]);
    match inner_children(&fragment) {
        [Node::Text(text)] => assert_eq!((text.range.start, text.range.end), (12, 23)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_close_tag_with_space_before_its_bracket_closes_nothing() {
    for (source, at) in [
        ("<svelte:head><style>a{}</style ></svelte:head>", 46),
        ("<div><style>a{}</div>\n\n", 21),
        ("<div><script>a</script\n></div>", 30),
    ] {
        let (_, errors) = parse(source);
        assert_eq!(errors.first(), Some(&("expected_token", at)), "{source:?}");
    }
}

#[test]
fn another_spelling_of_the_name_is_an_ordinary_element() {
    let (fragment, errors) = parse("<div><sTyle>{a}</sTyle></div>");
    assert_eq!(errors, vec![]);
    assert!(
        matches!(inner_children(&fragment), [Node::Interpolation(_)]),
        "{:?}",
        inner_children(&fragment)
    );
}
