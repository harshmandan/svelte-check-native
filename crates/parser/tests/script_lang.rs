//! Which language the compiler reads a component's JavaScript in.
//!
//! Each answer is what the compiler's own `regex_lang_attribute`
//! captures for the text.

use svn_parser::tag_read::compiler_reads_typescript;

#[test]
fn a_quoted_attribute_holding_a_close_bracket_is_stepped_over() {
    for tag in [
        r#"<script generics="T extends Record<string, number>" lang="ts">"#,
        r#"<script data-x="a>b" lang="ts">"#,
        r#"<script generics="T extends (x: number) => void" lang="ts">"#,
        "<script data-x='a>b' lang=ts>",
        r#"<script lang="ts" generics="T extends A<B>">"#,
        r#"<script a="x>y" lang="js" b="p>q" lang="ts">"#,
        r#"<script a="x>y" b=c lang=ts>"#,
        r#"<script a="x>y" b="c"d lang=ts>"#,
        "<script a=\"x>y\"\n\tlang=\"ts\">",
        r#"<script a="x>y" lang=ts"x>"#,
        "<script\u{a0}a=\"x>y\"\u{a0}lang=\"ts\">",
        r#"<script a="x>y" b c="d" lang="ts">"#,
        r#"<script a="lang=ts>" >"#,
        r#"<script a="x>" lang=ts b="lang=js>">"#,
        "<script a=\"x>y\" lang=\"\">\n<script lang=\"ts\">",
    ] {
        assert!(compiler_reads_typescript(tag), "{tag}");
    }
}

#[test]
fn a_lang_the_regex_does_not_reach_is_not_read() {
    for tag in [
        // The reading up to the first `>` comes first, and finds this.
        r#"<script data-x="lang=js a>b" lang="ts">"#,
        // `lang=` has to follow an attribute with a value directly.
        r#"<script a="x>y" b lang="ts">"#,
        r#"<script a="x>y"lang="ts">"#,
        r#"<script a="x>y" lang="ts""#,
        r#"<script a="x>y" xlang="ts">"#,
        r#"<script a = "x>y" lang="ts">"#,
        r#"<script a="x>y" b/c="d" lang="ts">"#,
        r#"<script a="x>y" lang="ts>"#,
        r#"<!-- <script a="x>y" lang="ts"> --><script a="x>y" lang="js">"#,
        r#"<script a="x>y" lang="js"></script><script lang="ts">"#,
    ] {
        assert!(!compiler_reads_typescript(tag), "{tag}");
    }
}
