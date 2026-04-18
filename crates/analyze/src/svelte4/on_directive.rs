//! `on:event` directive → `onevent` prop-name translation.
//!
//! Svelte 4 spelled event handlers as directives (`on:click={fn}`);
//! Svelte 5 spells them as plain props (`onclick={fn}`). At the type
//! level both resolve to the same handler shape on the component or
//! element; our overlay normalizes the Svelte-4 form to the Svelte-5
//! form at emit time.
//!
//! For component instantiations this matters the most: the emitted
//! `new $$_C({ props: { ... } })` object literal needs `oncustom:
//! handler` so tsgo type-checks the handler against the component's
//! `oncustom: EventHandler<…>` prop. Dropping the directive (our
//! v0.1 behaviour) silently loses that check.
//!
//! For DOM elements the svelte-jsx ambient types already declare
//! `onclick`/`oninput`/etc., so the same naming convention works —
//! no per-element lookup needed.

use svn_parser::{Directive, DirectiveKind};

/// Given an `on:<event>` directive, return the prop-name shape we'd
/// use in the synthesized props object literal. Returns `None` for
/// non-`On` directives (callers should check kind first; this is just
/// a convenience that guards the rename).
///
/// Modifier syntax (`on:click|once|preventDefault`) is stripped — the
/// modifiers are runtime behaviors that don't surface in the type
/// signature. tsgo only cares that the handler fits the event-handler
/// shape.
pub fn prop_name_for(directive: &Directive) -> Option<String> {
    if directive.kind != DirectiveKind::On {
        return None;
    }
    Some(format!("on{}", directive.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use smol_str::SmolStr;
    use svn_core::Range;

    fn make(kind: DirectiveKind, name: &str) -> Directive {
        Directive {
            kind,
            name: SmolStr::from(name),
            modifiers: Vec::new(),
            value: None,
            range: Range { start: 0, end: 0 },
        }
    }

    #[test]
    fn on_click_becomes_onclick() {
        assert_eq!(
            prop_name_for(&make(DirectiveKind::On, "click")).as_deref(),
            Some("onclick"),
        );
    }

    #[test]
    fn on_custom_event_becomes_oncustom_event() {
        // Custom event names just concatenate; tsgo matches the prop
        // name against whatever the component's Props type declares.
        assert_eq!(
            prop_name_for(&make(DirectiveKind::On, "mycustom")).as_deref(),
            Some("onmycustom"),
        );
    }

    #[test]
    fn non_on_directive_returns_none() {
        assert!(prop_name_for(&make(DirectiveKind::Bind, "value")).is_none());
        assert!(prop_name_for(&make(DirectiveKind::Use, "action")).is_none());
        assert!(prop_name_for(&make(DirectiveKind::Class, "active")).is_none());
    }

    #[test]
    fn modifiers_are_stripped_from_prop_name() {
        // `on:click|once|preventDefault` — runtime modifiers don't
        // appear in the type signature. We only care about the event
        // name.
        let mut d = make(DirectiveKind::On, "click");
        d.modifiers = vec![SmolStr::from("once"), SmolStr::from("preventDefault")];
        assert_eq!(prop_name_for(&d).as_deref(), Some("onclick"));
    }
}
