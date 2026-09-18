//! Which of Svelte's component ambients — `$$props`, `$$restProps`,
//! `$$slots` — a component refers to.
//!
//! Upstream sets `uses$$props` / `uses$$restProps` / `uses$$slots` from
//! identifier nodes while walking the instance script and the template
//! (`processInstanceScriptContent.ts`, `htmlxtojsx_v2/index.ts`) — any
//! identifier with the name, a declaration or property name included,
//! and never from the module script. The flags decide whether the
//! component accepts arbitrary props and whether the ambients get a
//! declaration in the render body. A text match would also count a
//! comment that mentions `$$restProps`.

use oxc_ast::ast::{
    BindingIdentifier, IdentifierName, IdentifierReference, LabelIdentifier, Program,
};
use oxc_ast_visit::Visit;

#[derive(Debug, Default, Clone, Copy)]
pub struct AmbientRefs {
    pub props: bool,
    pub rest_props: bool,
    pub slots: bool,
}

impl AmbientRefs {
    pub fn any(self) -> bool {
        self.props || self.rest_props || self.slots
    }
}

/// Collect the ambient identifiers of the instance script and every
/// template expression.
pub fn find_ambient_refs(
    fragment: &svn_parser::Fragment,
    source: &str,
    parsed_instance: Option<&Program<'_>>,
) -> AmbientRefs {
    // Cheap pre-filter: every ambient starts with `$$`.
    if !source.contains("$$") {
        return AmbientRefs::default();
    }
    let mut probe = AmbientProbe::default();
    if let Some(program) = parsed_instance {
        probe.visit_program(program);
    }
    let alloc = oxc_allocator::Allocator::default();
    for expr in crate::template_expression_ranges(fragment) {
        let Some(text) = source.get(expr.range.start as usize..expr.range.end as usize) else {
            continue;
        };
        if !text.contains("$$") {
            continue;
        }
        let wrapped = if expr.is_declaration {
            format!("let {text}\n;")
        } else {
            format!("({text}\n);")
        };
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        probe.visit_program(&parsed.program);
    }
    probe.refs
}

#[derive(Default)]
struct AmbientProbe {
    refs: AmbientRefs,
}

impl AmbientProbe {
    fn note(&mut self, name: &str) {
        match name {
            "$$props" => self.refs.props = true,
            "$$restProps" => self.refs.rest_props = true,
            "$$slots" => self.refs.slots = true,
            _ => {}
        }
    }
}

impl<'a> Visit<'a> for AmbientProbe {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.note(&it.name);
    }
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.note(&it.name);
    }
    fn visit_identifier_name(&mut self, it: &IdentifierName<'a>) {
        self.note(&it.name);
    }
    fn visit_label_identifier(&mut self, it: &LabelIdentifier<'a>) {
        self.note(&it.name);
    }
}

/// Names the probe recognises, for callers that want to pre-filter.
pub const AMBIENT_NAMES: [&str; 3] = ["$$props", "$$restProps", "$$slots"];

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(src: &str) -> AmbientRefs {
        let (doc, _) = svn_parser::parse_sections(src);
        let (fragment, _) = svn_parser::parse_all_template_runs(src, &doc.template.text_runs);
        let alloc = oxc_allocator::Allocator::default();
        let instance = doc
            .instance_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        find_ambient_refs(&fragment, src, instance.as_ref().map(|p| &p.program))
    }

    #[test]
    fn identifier_references_count() {
        assert!(refs("<script>const r = $$restProps;</script>").rest_props);
        assert!(refs("<div {...$$props} />").props);
        assert!(refs("{#if $$slots.header}x{/if}").slots);
    }

    #[test]
    fn declarations_and_property_names_count_but_the_module_script_does_not() {
        assert!(refs("<script>let $$props = {};</script>").props);
        assert!(refs("<script>const s = o.$$slots;</script>").slots);
        assert!(!refs("<script module>export const p = $$props;</script>").any());
    }

    #[test]
    fn mentions_in_comments_and_strings_do_not() {
        let r = refs("<script>// forward $$restProps later\nconst s = '$$props';</script>");
        assert!(!r.any());
    }
}
