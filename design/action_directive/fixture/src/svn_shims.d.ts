// Ambient declarations for the action-directive emit-shape validation.
// Mirrors upstream svelte2tsx's __sveltets_2_ensureAction with our
// __svn_ namespace + adds __svn_map_element_tag so the action's first
// parameter gets contextually-typed to the real element type.

type __SvnActionReturnType =
    | {
          update?: (args: any) => void;
          destroy?: () => void;
          $$_attributes?: Record<string, any>;
      }
    | void;

/** Validates the call to `action(element, params)` and surfaces any
 * `$$_attributes` the action advertises (so Svelte's generic-attributes
 * pass can pick them up).
 */
declare function __svn_ensure_action<T extends __SvnActionReturnType>(
    actionCall: T,
): T extends { $$_attributes?: any } ? T['$$_attributes'] : {};

/** Maps an HTML tag name (string literal) back to the real DOM element
 * type. Action directives emit `action(__svn_map_element_tag('form'), params)`
 * so the action sees a real HTMLFormElement — not `unknown`.
 */
declare function __svn_map_element_tag<K extends keyof HTMLElementTagNameMap>(
    tag: K,
): HTMLElementTagNameMap[K];
declare function __svn_map_element_tag<K extends keyof SVGElementTagNameMap>(
    tag: K,
): SVGElementTagNameMap[K];
declare function __svn_map_element_tag(tag: string): HTMLElement;
