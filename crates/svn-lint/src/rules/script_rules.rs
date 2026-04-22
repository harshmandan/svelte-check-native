//! Rules that fire on `<script>` blocks.
//!
//! In upstream these fire during phase 1-parse (`read/script.js`) and
//! during phase 2-analyze (`2-analyze/index.js`), but we can run them
//! off the already-structured document after `parse_sections`.

use svn_parser::document::{Document, ScriptContext, ScriptSection};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Legal attributes on a `<script>` tag. Mirrors upstream
/// `read/script.js::ALLOWED_ATTRIBUTES`.
const SCRIPT_ALLOWED_ATTRIBUTES: &[&str] = &["context", "generics", "lang", "module"];

pub fn visit_document(doc: &Document<'_>, ctx: &mut LintContext<'_>) {
    if let Some(script) = &doc.instance_script {
        visit_script_section(script, ctx);
    }
    if let Some(script) = &doc.module_script {
        visit_script_section(script, ctx);
        // script_context_deprecated: module script declared with the
        // legacy `context="module"` attribute in runes mode.
        if ctx.runes {
            for attr in &script.attrs {
                if attr.name == "context"
                    && script.context == ScriptContext::Module
                    && attr.value.as_deref() == Some("module")
                {
                    let msg = messages::script_context_deprecated();
                    ctx.emit(Code::script_context_deprecated, msg, attr.range);
                }
            }
        }
    }
}

fn visit_script_section(script: &ScriptSection<'_>, ctx: &mut LintContext<'_>) {
    // script_unknown_attribute: any attr not in the allow-list.
    for attr in &script.attrs {
        if !SCRIPT_ALLOWED_ATTRIBUTES.contains(&attr.name.as_str()) {
            let msg = messages::script_unknown_attribute();
            ctx.emit(Code::script_unknown_attribute, msg, attr.range);
        }
    }
}
