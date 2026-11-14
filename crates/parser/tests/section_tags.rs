//! A top-level `<script>` tag the compiler's strict reading rejects
//! still has a shape. svelte-check's language server reads the tag with
//! an HTML parser, which takes `lang = "ts"` for a `lang` attribute: it
//! decides whether the component's errors come from TypeScript and
//! whether an "expected" error is hidden because a `lang` attribute
//! points at another preprocessor. The compiler's error is reported
//! beside that reading.

#![allow(clippy::expect_used)]

use svn_parser::ScriptLang;

#[test]
fn a_rejected_script_tag_keeps_its_lang() {
    for (source, at) in [
        ("<script lang = \"ts\">let a = 1;</script>", 13),
        ("<script lang=\"ts\" lang=\"ts\">let a = 1;</script>", 18),
        ("<script lang=\"ts\" />", 18),
    ] {
        let (doc, errors) = svn_parser::parse_sections(source);
        let codes: Vec<_> = errors
            .iter()
            .map(|e| (e.code_slug(), e.range().start))
            .collect();
        assert_eq!(
            codes.first().map(|c| c.1),
            Some(at),
            "{source:?}: {codes:?}"
        );
        let script = doc.instance_script.as_ref().expect("the script");
        assert_eq!(script.lang, ScriptLang::Ts, "{source:?}");
    }
}
