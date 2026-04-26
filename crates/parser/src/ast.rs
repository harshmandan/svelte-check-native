//! Svelte template AST.
//!
//! The template tree lives inside [`Fragment`], which is a list of [`Node`]s.
//! Every node carries a [`Range`] into the source. Interpolated JS/TS
//! expressions carry byte ranges rather than parsed ASTs — the oxc parse
//! happens lazily in `analyze`, so the template parser itself stays fast
//! and doesn't need an arena lifetime.
//!
//! ### Design notes
//!
//! - Ranges refer to the original source (not the template-only slice) so
//!   that diagnostics with template positions can be reported against the
//!   full `.svelte` file.
//! - Tag name stored as `SmolStr` (inlined up to 23 bytes — tag names are
//!   short).
//! - We represent "expression inside a mustache" as an `Range`; to get an
//!   oxc AST, call `parse_script_body` on the substring.

use smol_str::SmolStr;
use svn_core::Range;

/// A sequence of template nodes forming a fragment (top-level template,
/// an element's children, a control-flow block's body, etc.).
#[derive(Debug, Clone, Default)]
pub struct Fragment {
    /// The nodes in this fragment, in source order. Whitespace-only text
    /// nodes are preserved; emitters decide whether to keep or drop them.
    pub nodes: Vec<Node>,
    /// Range covering the entire fragment in the source.
    pub range: Range,
}

/// A single template node.
///
/// Variants are boxed where they're structurally big (Element) or wrap a
/// fragment (IfBlock etc., once those land) to keep `Node` enum small.
/// For now we keep variants inline — boxing can be added when measurements
/// justify it.
#[derive(Debug, Clone)]
pub enum Node {
    /// Plain text content (may contain interpolations — those become
    /// [`Node::Interpolation`] siblings).
    Text(Text),
    /// `{expression}` — a mustache interpolation tag.
    Interpolation(Interpolation),
    /// `<!-- ... -->` HTML comment.
    Comment(Comment),
    /// `<element>...</element>` — a DOM element.
    Element(Element),
    /// `<Component>...</Component>` — Svelte component invocation.
    ///
    /// Distinguished from `Element` by the tag name starting with an
    /// uppercase letter or containing a `.` (namespace access).
    Component(Component),
    /// `<svelte:foo>` — Svelte special element.
    SvelteElement(SvelteElement),
    /// `{#if cond}...{:else if c}...{:else}...{/if}`
    IfBlock(IfBlock),
    /// `{#each expr as item, index (key)}...{:else}...{/each}`
    EachBlock(EachBlock),
    /// `{#await promise}...{:then v}...{:catch e}...{/await}`
    AwaitBlock(AwaitBlock),
    /// `{#key expr}...{/key}`
    KeyBlock(KeyBlock),
    /// `{#snippet name(params)}...{/snippet}`
    SnippetBlock(SnippetBlock),
}

impl Node {
    /// Range in the source covered by this node.
    pub fn range(&self) -> Range {
        match self {
            Self::Text(t) => t.range,
            Self::Interpolation(i) => i.range,
            Self::Comment(c) => c.range,
            Self::Element(e) => e.range,
            Self::Component(c) => c.range,
            Self::SvelteElement(e) => e.range,
            Self::IfBlock(b) => b.range,
            Self::EachBlock(b) => b.range,
            Self::AwaitBlock(b) => b.range,
            Self::KeyBlock(b) => b.range,
            Self::SnippetBlock(b) => b.range,
        }
    }
}

/// Plain text content. Includes whitespace.
#[derive(Debug, Clone)]
pub struct Text {
    pub content: String,
    pub range: Range,
}

/// `{expression}` — a single-expression mustache interpolation, OR one
/// of Svelte's `{@…}` directive tags (`@const`, `@html`, `@render`,
/// `@debug`, `@attach`). The [`kind`] discriminator lets consumers
/// branch on shape without scanning the source bytes.
///
/// `expression_range` is the byte range of the "expression payload"
/// between `{` and `}`:
///
/// - For [`InterpolationKind::Expression`]: the whole body.
/// - For [`InterpolationKind::AtConst`] / [`InterpolationKind::AtTag`]:
///   the body after the directive keyword + whitespace, so `{@const
///   foo = 1}` has `expression_range` covering `foo = 1` (not
///   `@const foo = 1`).
#[derive(Debug, Clone)]
pub struct Interpolation {
    /// What kind of `{…}` this is — plain expression or one of
    /// Svelte's directive tags. See [`InterpolationKind`].
    pub kind: InterpolationKind,
    /// Range of the expression payload inside the braces (after the
    /// directive keyword + whitespace for `@*` tags).
    pub expression_range: Range,
    /// Range of the full `{…}` including braces.
    pub range: Range,
}

/// What flavour of `{…}` tag an [`Interpolation`] is. Set by the
/// template parser at parse time so downstream passes (emit, analyze,
/// lint) don't have to re-peek at the source bytes to classify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpolationKind {
    /// `{EXPR}` — value interpolation.
    Expression,
    /// `{@const NAME = EXPR}` — template-scope declaration. Emit
    /// produces a real `const <pattern> = <expr>;` at the current
    /// template-check block so TS pins the inferred type.
    AtConst,
    /// `{@html EXPR}` — raw-HTML interpolation. Emit produces a bare
    /// `(EXPR);` expression statement so tsgo type-checks the
    /// expression against the surrounding scope (catches TS2304 on
    /// typos, TS2339 on missing-property reads, etc.). Mirrors
    /// upstream svelte2tsx's `RawMustacheTag.ts`.
    AtHtml,
    /// `{@render EXPR}` — snippet-render call (Svelte 5+). Emit
    /// produces `__svn_ensure_snippet(EXPR);` so tsgo type-checks
    /// EXPR's call arguments against the snippet's declared
    /// `Snippet<[…]>` parameter shape, plus validates that EXPR
    /// resolves to a snippet at all. Mirrors upstream's `RenderTag.ts`.
    AtRender,
    /// `{@debug a, b, …}` — debug interpolation. Emit produces one
    /// bare `(name);` per comma-separated identifier so tsgo
    /// type-checks each name in scope (catches TS2304 on typos).
    /// Mirrors upstream's `DebugTag.ts`.
    AtDebug,
    /// Any other `{@*}` directive (`@attach`, or an unknown future
    /// tag). `@attach` actually routes through the Spread shape on
    /// elements rather than this enum, so this catch-all is mostly
    /// reserved for forward-compat. Emit treats as side-effect-only.
    AtTag,
}

/// `<!-- ... -->`
#[derive(Debug, Clone)]
pub struct Comment {
    /// The comment body, excluding `<!--` and `-->`.
    pub data: String,
    pub range: Range,
}

/// A normal DOM element. Tag name is lowercase or mixed (`div`, `my-element`).
#[derive(Debug, Clone)]
pub struct Element {
    pub name: SmolStr,
    pub attributes: Vec<Attribute>,
    pub children: Fragment,
    /// True for self-closing (`<br />`) or void elements (`<br>`, `<img>`).
    pub self_closing: bool,
    /// Range of the full element, including opening and closing tags.
    pub range: Range,
}

/// A Svelte component invocation. Tag name starts uppercase or contains `.`.
#[derive(Debug, Clone)]
pub struct Component {
    pub name: SmolStr,
    pub attributes: Vec<Attribute>,
    pub children: Fragment,
    pub self_closing: bool,
    pub range: Range,
}

/// A `<svelte:foo>` special element.
#[derive(Debug, Clone)]
pub struct SvelteElement {
    pub kind: SvelteElementKind,
    pub attributes: Vec<Attribute>,
    pub children: Fragment,
    pub self_closing: bool,
    pub range: Range,
}

/// An attribute on an element, component, or svelte:* element.
#[derive(Debug, Clone)]
pub enum Attribute {
    /// `name="value"`, `name=value`, or `name` (boolean).
    ///
    /// The value may contain text-with-interpolations: `class="a {b} c"`.
    Plain(PlainAttr),
    /// `name={expression}` — expression attribute.
    Expression(ExpressionAttr),
    /// `{name}` — shorthand expansion of `name={name}`.
    Shorthand(ShorthandAttr),
    /// `{...expr}` — object spread.
    Spread(SpreadAttr),
    /// `bind:foo`, `on:click`, `use:action`, `class:x`, `style:y`,
    /// `transition:name`, `in:name`, `out:name`, `animate:name`, `let:name`.
    Directive(Directive),
}

impl Attribute {
    pub fn range(&self) -> Range {
        match self {
            Self::Plain(a) => a.range,
            Self::Expression(a) => a.range,
            Self::Shorthand(a) => a.range,
            Self::Spread(a) => a.range,
            Self::Directive(a) => a.range,
        }
    }
}

/// `name`, `name=value`, `name="value"`, `name='value'`.
#[derive(Debug, Clone)]
pub struct PlainAttr {
    pub name: SmolStr,
    /// `None` for a boolean attribute (no `=`).
    pub value: Option<AttrValue>,
    pub range: Range,
}

/// Value of a plain attribute. A sequence of literal text and `{expr}`
/// interpolations. For unquoted values this is always a single Text part.
#[derive(Debug, Clone)]
pub struct AttrValue {
    pub parts: Vec<AttrValuePart>,
    pub range: Range,
    pub quoted: bool,
}

#[derive(Debug, Clone)]
pub enum AttrValuePart {
    /// Literal text chunk.
    Text { content: String, range: Range },
    /// `{expression}` interpolation.
    Expression {
        expression_range: Range,
        range: Range,
    },
}

/// `name={expression}`.
#[derive(Debug, Clone)]
pub struct ExpressionAttr {
    pub name: SmolStr,
    pub expression_range: Range,
    pub range: Range,
}

/// `{name}` — shorthand expansion of `name={name}`.
#[derive(Debug, Clone)]
pub struct ShorthandAttr {
    pub name: SmolStr,
    pub range: Range,
}

/// `{...expr}` or `{@attach expr}`. The `is_attach` flag distinguishes
/// the two: `{...expr}` spreads the object's properties onto the
/// element, while `{@attach EXPR}` provides an `Attachment<T>` callback
/// (Svelte 5.29+, `svelte/attachments`). Emit handles them differently
/// — a real spread emits `...EXPR,` inside the element's prop literal,
/// while an attach tag emits `[Symbol("@attach")]: EXPR,` so the
/// arrow's parameter picks up the element's contextual type via the
/// `[key: symbol]: Attachment<T>` index signature on `HTMLAttributes`.
#[derive(Debug, Clone)]
pub struct SpreadAttr {
    pub expression_range: Range,
    pub range: Range,
    /// True for `{@attach EXPR}`, false for `{...EXPR}`.
    pub is_attach: bool,
}

/// `<prefix>:<name>[|<modifier>]*[={value}]`.
///
/// Examples:
/// - `bind:value`
/// - `bind:value={expr}`
/// - `bind:value={getter, setter}` (Svelte 5 getter/setter pair)
/// - `on:click|once|preventDefault={handler}`
/// - `transition:fade|local={{ duration: 200 }}`
#[derive(Debug, Clone)]
pub struct Directive {
    pub kind: DirectiveKind,
    /// Name after the `:`. E.g. `value` for `bind:value`, `click` for `on:click`.
    pub name: SmolStr,
    /// `|modifier` entries. E.g. `["once", "preventDefault"]` for
    /// `on:click|once|preventDefault`.
    pub modifiers: Vec<SmolStr>,
    pub value: Option<DirectiveValue>,
    pub range: Range,
}

/// Which directive prefix this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    Bind,
    On,
    Use,
    Class,
    Style,
    Transition,
    In,
    Out,
    Animate,
    Let,
}

impl DirectiveKind {
    /// Parse the prefix (`bind`, `on`, etc.).
    pub fn parse(prefix: &str) -> Option<Self> {
        match prefix {
            "bind" => Some(Self::Bind),
            "on" => Some(Self::On),
            "use" => Some(Self::Use),
            "class" => Some(Self::Class),
            "style" => Some(Self::Style),
            "transition" => Some(Self::Transition),
            "in" => Some(Self::In),
            "out" => Some(Self::Out),
            "animate" => Some(Self::Animate),
            "let" => Some(Self::Let),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::On => "on",
            Self::Use => "use",
            Self::Class => "class",
            Self::Style => "style",
            Self::Transition => "transition",
            Self::In => "in",
            Self::Out => "out",
            Self::Animate => "animate",
            Self::Let => "let",
        }
    }
}

/// The RHS of a directive after `=`.
#[derive(Debug, Clone)]
pub enum DirectiveValue {
    /// `bind:foo={expr}`, `on:click={handler}`, etc.
    Expression {
        expression_range: Range,
        range: Range,
    },
    /// `bind:foo={getter, setter}` — Svelte 5 getter/setter pair.
    BindPair {
        getter_range: Range,
        setter_range: Range,
        range: Range,
    },
    /// `style:left="100px"` or `transition:fade="literal"` — quoted string
    /// value (possibly with interpolations).
    Quoted(AttrValue),
}

/// Which of the `<svelte:*>` special elements this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvelteElementKind {
    /// `<svelte:self>` — recursive self-reference
    SelfRef,
    /// `<svelte:component this={Cmp}>` — dynamic component
    Component,
    /// `<svelte:element this="div">` — dynamic HTML element
    Element,
    /// `<svelte:window>` — window-level events
    Window,
    /// `<svelte:document>` — document-level events
    Document,
    /// `<svelte:body>` — body-level events
    Body,
    /// `<svelte:head>` — renders to `<head>`
    Head,
    /// `<svelte:options>` — compiler directives
    Options,
    /// `<svelte:fragment>` — wraps a named slot/snippet without a DOM element
    Fragment,
    /// `<svelte:boundary>` — error-boundary (Svelte 5.3+)
    Boundary,
}

impl SvelteElementKind {
    /// Parse the part after `svelte:` — e.g. `parse("element")` →
    /// `Some(Element)`.
    pub fn parse(suffix: &str) -> Option<Self> {
        match suffix {
            "self" => Some(Self::SelfRef),
            "component" => Some(Self::Component),
            "element" => Some(Self::Element),
            "window" => Some(Self::Window),
            "document" => Some(Self::Document),
            "body" => Some(Self::Body),
            "head" => Some(Self::Head),
            "options" => Some(Self::Options),
            "fragment" => Some(Self::Fragment),
            "boundary" => Some(Self::Boundary),
            _ => None,
        }
    }

    /// Canonical spelling (what follows `svelte:`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfRef => "self",
            Self::Component => "component",
            Self::Element => "element",
            Self::Window => "window",
            Self::Document => "document",
            Self::Body => "body",
            Self::Head => "head",
            Self::Options => "options",
            Self::Fragment => "fragment",
            Self::Boundary => "boundary",
        }
    }
}

// ===== Control-flow blocks ===============================================

/// `{#if cond}...{:else if c}...{:else}...{/if}`
#[derive(Debug, Clone)]
pub struct IfBlock {
    /// Byte range of the primary condition expression (between `{#if ` and `}`).
    pub condition_range: Range,
    pub consequent: Fragment,
    /// Zero or more `{:else if c}` arms, in source order.
    pub elseif_arms: Vec<ElseIfArm>,
    /// `{:else}` body, if present.
    pub alternate: Option<Fragment>,
    pub range: Range,
}

#[derive(Debug, Clone)]
pub struct ElseIfArm {
    pub condition_range: Range,
    pub body: Fragment,
}

/// `{#each expr as item, index (key)}...{:else}...{/each}`.
///
/// `as_clause` is optional (Svelte allows `{#each items}` without binding).
/// The exact pattern shape is stored as a byte range; `analyze` can parse it
/// with oxc when needed.
#[derive(Debug, Clone)]
pub struct EachBlock {
    /// Byte range of the iterable expression.
    pub expression_range: Range,
    /// `{#each items}` (no `as`) → None.
    pub as_clause: Option<EachAsClause>,
    pub body: Fragment,
    /// `{:else}` empty-list body, if present.
    pub alternate: Option<Fragment>,
    pub range: Range,
}

#[derive(Debug, Clone)]
pub struct EachAsClause {
    /// Byte range of the destructuring pattern (may be an identifier,
    /// array-destructuring, or object-destructuring).
    pub context_range: Range,
    /// `index` binding range (the `i` in `as item, i`).
    pub index_range: Option<Range>,
    /// `(key)` expression range, if a key is provided.
    pub key_range: Option<Range>,
}

/// `{#await promise}...{:then v}...{:catch e}...{/await}` and its short forms.
#[derive(Debug, Clone)]
pub struct AwaitBlock {
    pub expression_range: Range,
    /// `{#await p}` branch (shown while pending).
    pub pending: Option<Fragment>,
    /// `{:then value}` branch. May appear as `{#await p then v}` short form.
    pub then_branch: Option<ThenBranch>,
    /// `{:catch err}` branch. May appear as `{#await p catch e}` short form.
    pub catch_branch: Option<CatchBranch>,
    pub range: Range,
}

#[derive(Debug, Clone)]
pub struct ThenBranch {
    /// `{:then value}` binding pattern range. `None` for `{:then}` without
    /// a binding.
    pub context_range: Option<Range>,
    pub body: Fragment,
}

#[derive(Debug, Clone)]
pub struct CatchBranch {
    pub context_range: Option<Range>,
    pub body: Fragment,
}

/// `{#key expr}...{/key}`
#[derive(Debug, Clone)]
pub struct KeyBlock {
    pub expression_range: Range,
    pub body: Fragment,
    pub range: Range,
}

/// `{#snippet name(params)}...{/snippet}`
#[derive(Debug, Clone)]
pub struct SnippetBlock {
    pub name: SmolStr,
    /// Byte range of the parameter list (inside parens). Empty range if
    /// `{#snippet name()}` had no params.
    pub parameters_range: Range,
    pub body: Fragment,
    pub range: Range,
}

// ===== Static helpers ====================================================

/// HTML "void" elements that have no closing tag.
///
/// Per the WHATWG spec. Used to decide whether parsing an opening tag should
/// eagerly close without looking for `</tag>`.
pub fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Does a tag name refer to a Svelte component (uppercase or dotted)?
pub fn is_component_tag(name: &str) -> bool {
    if name.contains('.') {
        return true;
    }
    name.chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svelte_element_kind_round_trip() {
        for kind in [
            SvelteElementKind::SelfRef,
            SvelteElementKind::Component,
            SvelteElementKind::Element,
            SvelteElementKind::Window,
            SvelteElementKind::Document,
            SvelteElementKind::Body,
            SvelteElementKind::Head,
            SvelteElementKind::Options,
            SvelteElementKind::Fragment,
            SvelteElementKind::Boundary,
        ] {
            assert_eq!(SvelteElementKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SvelteElementKind::parse("nope"), None);
    }

    #[test]
    fn void_element_detection() {
        assert!(is_void_element("br"));
        assert!(is_void_element("img"));
        assert!(is_void_element("input"));
        assert!(!is_void_element("div"));
        assert!(!is_void_element("span"));
    }

    #[test]
    fn component_tag_detection() {
        assert!(is_component_tag("Button"));
        assert!(is_component_tag("MyWidget"));
        assert!(is_component_tag("ui.Button"));
        assert!(!is_component_tag("div"));
        assert!(!is_component_tag("my-widget")); // custom element, not component
        assert!(!is_component_tag(""));
    }
}
