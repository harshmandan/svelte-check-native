// Minimal `svelteHTML` namespace — models the subset of upstream's
// `svelte-jsx-v4.d.ts` + `svelte/elements.d.ts` the DOM-element-emit
// patterns validate against.
//
// Matches upstream's per-element attribute shape so that when we emit
//
//   svelteHTML.createElement("button", { type: someString, onclick: h })
//
// tsgo fires the same diagnostics as against the real typings:
//   - wrong attribute type (button's `type`) → TS2322
//   - event handler with destructure on `target` (EventTarget has no
//     `.value`) → TS2339
//   - unknown attribute (HTMLAttributes<T> rejection) → TS2353
//
// Trimmed hard on purpose. Production overlay uses the real
// `svelte-jsx-v4.d.ts` + real `svelte/elements.d.ts` shipped with the
// user's Svelte install; this fixture locks the SHAPE we need to emit,
// not the attribute catalog.

declare namespace svelteHTML {
    function createElement<
        Elements extends IntrinsicElements,
        Key extends keyof Elements
    >(
        element: Key | undefined | null,
        attrs: string extends Key
            ? HTMLAttributes<any>
            : Elements[Key]
    ): Key extends keyof HTMLElementTagNameMap
        ? HTMLElementTagNameMap[Key]
        : any;

    interface DOMAttributes<T extends EventTarget> {
        onclick?: ((event: MouseEvent & { currentTarget: T }) => any) | null | undefined;
        oninput?: ((event: Event & { currentTarget: T }) => any) | null | undefined;
        onchange?: ((event: Event & { currentTarget: T }) => any) | null | undefined;
    }

    interface HTMLAttributes<T extends EventTarget> extends DOMAttributes<T> {
        class?: string | null | undefined;
        id?: string | null | undefined;
        title?: string | null | undefined;
        style?: string | null | undefined;
    }

    interface HTMLButtonAttributes extends HTMLAttributes<HTMLButtonElement> {
        disabled?: boolean | null | undefined;
        name?: string | null | undefined;
        type?: "submit" | "reset" | "button" | null | undefined;
        value?: string | number | null | undefined;
    }

    interface HTMLInputAttributes extends HTMLAttributes<HTMLInputElement> {
        disabled?: boolean | null | undefined;
        name?: string | null | undefined;
        type?: string | null | undefined;
        value?: string | number | null | undefined;
    }

    interface HTMLDivAttributes extends HTMLAttributes<HTMLDivElement> {}
    interface HTMLParagraphAttributes extends HTMLAttributes<HTMLParagraphElement> {}
    interface HTMLSpanAttributes extends HTMLAttributes<HTMLSpanElement> {}
    interface HTMLAnchorAttributes extends HTMLAttributes<HTMLAnchorElement> {
        href?: string | null | undefined;
        target?: string | null | undefined;
    }

    interface IntrinsicElements {
        button: HTMLButtonAttributes;
        input: HTMLInputAttributes;
        div: HTMLDivAttributes;
        p: HTMLParagraphAttributes;
        span: HTMLSpanAttributes;
        a: HTMLAnchorAttributes;
        // index signature: unknown tags fall through to loose HTMLAttributes
        [name: string]: HTMLAttributes<any>;
    }
}
