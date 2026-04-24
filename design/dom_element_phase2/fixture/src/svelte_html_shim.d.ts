// Trimmed svelteHTML namespace — enough for Phase 2 patterns.
// See design/dom_element_emit/fixture/ for the Phase 1 version.

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
        oncopy?: ((event: ClipboardEvent & { currentTarget: T }) => any) | null | undefined;
    }

    interface HTMLAttributes<T extends EventTarget> extends DOMAttributes<T> {
        class?: string | null | undefined;
        id?: string | null | undefined;
        title?: string | null | undefined;
        style?: string | null | undefined;
    }

    interface HTMLButtonAttributes extends HTMLAttributes<HTMLButtonElement> {
        type?: "submit" | "reset" | "button" | null | undefined;
        disabled?: boolean | null | undefined;
    }

    interface HTMLDivAttributes extends HTMLAttributes<HTMLDivElement> {}
    interface HTMLTitleAttributes extends HTMLAttributes<HTMLTitleElement> {}
    interface HTMLBodyAttributes extends HTMLAttributes<HTMLBodyElement> {}
    interface HTMLHeadAttributes extends HTMLAttributes<HTMLHeadElement> {}

    interface IntrinsicElements {
        button: HTMLButtonAttributes;
        div: HTMLDivAttributes;
        title: HTMLTitleAttributes;
        // svelte:* namespaces — keys include the colon verbatim.
        "svelte:body": HTMLBodyAttributes;
        "svelte:head": HTMLHeadAttributes;
        "svelte:window": HTMLAttributes<any>;
        "svelte:document": HTMLAttributes<any>;
        "svelte:options": { [name: string]: any };
        "svelte:fragment": { slot?: string };
        [name: string]: HTMLAttributes<any>;
    }
}

// Style-directive value type-check helper. Mirrors upstream's
// `__sveltets_2_ensureType` — validates the directive value is of
// the expected primitive type(s) (for style:, String or Number).
declare function __svn_ensure_type<T>(
    type: new (...args: any[]) => T,
    value: T | null | undefined,
): void;
declare function __svn_ensure_type<T1, T2>(
    type1: new (...args: any[]) => T1,
    type2: new (...args: any[]) => T2,
    value: T1 | T2 | null | undefined,
): void;
