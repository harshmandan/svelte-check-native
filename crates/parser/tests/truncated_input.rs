//! Templates that stop in the middle of a tag. The parser reports them
//! and carries on; it never slices inside a character or builds a range
//! that ends before it starts (which a debug build refuses).

fn parse(source: &str) {
    let (doc, _) = svn_parser::parse_sections(source);
    let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
    fn check(fragment: &svn_parser::ast::Fragment) {
        assert!(fragment.range.start <= fragment.range.end);
    }
    check(&fragment);
}

#[test]
fn an_unclosed_attribute_expression_starting_with_a_wide_character() {
    parse("<p a={日本");
    parse("<p a={日本\n<span>é</span>");
    parse("<p {日本");
    parse("<p a=\"{日");
}

#[test]
fn an_each_tag_the_file_ends_in() {
    parse("{#each items");
    parse("<script>let items = [];</script>{#each items");
    parse("{#each items as x, i (y");
    parse("{#each");
    parse("{#each 日本");
}

#[test]
fn other_tags_the_file_ends_in() {
    for source in [
        "{#if a",
        "{#await p",
        "{#await p then 日",
        "{#key 日",
        "{#snippet s(日",
        "{@html 日",
        "{@const 日 = ",
        "{@render 日(",
        "{日",
        "{let 日 = ",
        "<C {...日",
        "<div use:日={日",
    ] {
        parse(source);
    }
}
