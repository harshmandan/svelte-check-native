//! A script syntax error svelte-check keeps to itself still ends the
//! compile.
//!
//! When the compiler's error message says something was "expected", the
//! component names a language (`lang="ts"`) and the project configures
//! no preprocessor, the language server takes the syntax for another
//! preprocessor's and shows nothing. The compiler threw all the same:
//! no later error and no warning comes out of that component.
//!
//! Each case is what svelte-check 4.7.6 with svelte 5.57.2 reports for
//! the component in a project without a Svelte config: nothing.

use std::path::Path;
use svn_lint::CompatFeatures;

const CASES: &[&str] = &[
    // "Unexpected token" at the second `function`; without it the
    // compiler would go on to refuse the second `f`.
    "<script lang=\"ts\">if (x)\n    function f() { return 23; }\nelse\n    function f() { return 42; }\n</script>",
    // "Unexpected token" at `using`, before the second `x`.
    "<script lang=\"ts\">{\n    using x = resource1, x = resource2;\n}\n</script>",
    // "Unexpected keyword 'default'", before the default export is
    // refused.
    "<script lang=\"ts\">export { default };\n</script>",
];

#[test]
fn nothing_is_reported_after_a_hidden_script_error() {
    let mut failures = Vec::new();
    for &src in CASES {
        let warnings = svn_lint::lint_file_with_options(
            src,
            Path::new("T.svelte"),
            svn_lint::LintOptions::default(),
            CompatFeatures::MODERN,
        );
        if !warnings.is_empty() {
            let got: Vec<_> = warnings.iter().map(|w| w.code.as_str()).collect();
            failures.push(format!("{src:?}\n  got: {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
