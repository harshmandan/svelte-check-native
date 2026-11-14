//! The `}` that closes a tag, where a brace count and acorn's tokenizer
//! disagree about a `/`.
//!
//! The count takes `in` after `ä` for the keyword (it reads names as
//! ASCII), so to it the `/` after starts a regular expression, `/ a /`,
//! and the `}` after that closes the tag. acorn's tokenizer reads the
//! name `äin`, a division, `a`, and a block comment `/* … */` that hides
//! that `}`; svelte 5.57.3 ends each of these tags at the last `}`.

use svn_parser::find_mustache_end;

#[test]
fn a_comment_that_runs_past_the_counted_brace_hides_it() {
    for source in ["{äin / a /* } x */}", "{äin / a /* } x */}<p>{a}</p>"] {
        let want = source.find("*/}").map(|i| i as u32 + 2);
        assert_eq!(find_mustache_end(source, 1), want, "{source:?}");
    }
}
