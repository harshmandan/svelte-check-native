// Minimal Svelte type shims, shipped by svelte-check-native into the
// project cache so that components can be type-checked even when the
// real `svelte` npm package is not installed in node_modules.
//
// Includes ambient declarations for the Svelte 5 runes ($state,
// $derived, $effect, $props, $bindable, $inspect, $host). These are
// macros that the Svelte compiler rewrites at build time — TypeScript
// needs them declared as globals so references to them in `<script>`
// bodies don't fire TS2304 "Cannot find name".

// Runes are declared at top level (script mode) rather than inside
// `declare global` because this file is a `.d.ts` script (no top-level
// imports/exports), so its declarations are already global.

/** `$state<T>(initial?)` declares reactive state. Macro.
 *
 * Matches svelte's real `$state` signature — strict inference from the
 * initial value when no explicit generic is given. Calls like
 * `$state<T>(0)` where T is a generic parameter and 0 isn't assignable
 * to T do fire TS2345; that matches Svelte's own type behavior.
 */
declare function $state<T>(initial: T): T;
declare function $state<T>(): T | undefined;
declare namespace $state {
    function raw<T>(initial: T): T;
    function raw<T>(): T | undefined;
    function snapshot<T>(value: T): T;
}

/** `$derived(expression)` re-evaluates whenever its dependencies change. */
declare function $derived<T>(expression: T): T;
declare namespace $derived {
    function by<T>(fn: () => T): T;
}

/** `$effect(fn)` runs a side effect after every dependency change. */
declare function $effect(fn: () => void | (() => void)): void;
declare namespace $effect {
    function pre(fn: () => void | (() => void)): void;
    function root(fn: () => void | (() => void)): () => void;
    function tracking(): boolean;
}

/** `$props<T>()` declares the component's prop bag. */
declare function $props<T = Record<string, any>>(): T;
declare namespace $props {
    function id(): string;
}

/** `$bindable<T>(fallback?)` marks a prop as two-way bindable. */
declare function $bindable<T>(fallback?: T): T;

/** `$inspect(...values)` logs values whenever they change in dev. */
declare function $inspect<T extends any[]>(
    ...values: T
): { with(fn: (type: 'init' | 'update', ...values: T) => void): void };

/** `$host<T>()` returns the host element for a custom-element component. */
declare function $host<T = HTMLElement>(): T;

// Internal helpers emitted by svelte-check-native into generated `.svelte.ts`
// files. Declared here so the generated code type-checks. The `__svn_*`
// prefix marks them as ours; user code shouldn't touch them.

/** Iterable wrapper for `{#each}` blocks. Accepts arrays, ArrayLike (`{ length: N }`), and any other iterable. */
declare function __svn_each_items<T>(value: T): Iterable<__SvnEachItem<T>>;

/** Resolved item type for `__svn_each_items`. The `0 extends 1 & T` guard preserves `any` (avoids the conditional-type-distribution-collapses-to-unknown trap). */
type __SvnEachItem<T> = 0 extends 1 & T
    ? any
    : T extends ArrayLike<infer U>
        ? U
        : T extends Iterable<infer U>
            ? U
            : never;

//
// We declare only what's needed to make type-checking succeed for code
// that imports from the standard `svelte/*` entry points. When the real
// `svelte` package IS installed, its declarations win because they live
// inside node_modules and are loaded first by tsgo's resolver.
//
// This file is regenerated into the cache directory on every check;
// edits here belong in svn-typecheck's source.

declare module 'svelte' {
    export interface ComponentConstructorOptions<
        Props extends Record<string, unknown> = Record<string, unknown>,
    > {
        target: Element | Document | ShadowRoot;
        anchor?: Element;
        props?: Props;
        context?: Map<unknown, unknown>;
        hydrate?: boolean;
        intro?: boolean;
    }

    export interface SvelteComponent<
        Props extends Record<string, unknown> = Record<string, unknown>,
        Events extends Record<string, unknown> = Record<string, unknown>,
        Slots extends Record<string, unknown> = Record<string, unknown>,
    > {
        $set(props: Partial<Props>): void;
        $on<K extends Extract<keyof Events, string>>(
            type: K,
            callback: (e: Events[K]) => void,
        ): () => void;
        $destroy(): void;
        // Phantom fields for type inference.
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    }

    export class SvelteComponentTyped<
        Props extends Record<string, unknown> = Record<string, unknown>,
        Events extends Record<string, unknown> = Record<string, unknown>,
        Slots extends Record<string, unknown> = Record<string, unknown>,
    > implements SvelteComponent<Props, Events, Slots>
    {
        constructor(options: ComponentConstructorOptions<Props>);
        $set(props: Partial<Props>): void;
        $on<K extends Extract<keyof Events, string>>(
            type: K,
            callback: (e: Events[K]) => void,
        ): () => void;
        $destroy(): void;
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    }

    // Svelte 5 `Component` type — function form.
    export type Component<
        Props extends Record<string, any> = Record<string, any>,
        Exports extends Record<string, any> = Record<string, any>,
        Bindings extends keyof Props | '' = string,
    > = (
        ...args: any[]
    ) => {
        props: Props;
        exports: Exports;
        bindings: Bindings;
    };

    export type Snippet<Parameters extends any[] = []> = {
        (...args: Parameters): any;
    };

    export function onMount(fn: () => void | (() => void)): void;
    export function onDestroy(fn: () => void): void;
    export function beforeUpdate(fn: () => void): void;
    export function afterUpdate(fn: () => void): void;
    export function tick(): Promise<void>;
    export function untrack<T>(fn: () => T): T;
    export function mount<Props extends Record<string, any>>(
        component: Component<Props>,
        options: { target: Element; props?: Props },
    ): { exports: any };
    export function unmount(component: any): Promise<void>;
    export function hydrate<Props extends Record<string, any>>(
        component: Component<Props>,
        options: { target: Element; props?: Props },
    ): { exports: any };
    export function getContext<T = unknown>(key: any): T;
    export function setContext<T>(key: any, value: T): T;
    export function hasContext(key: any): boolean;
    export function getAllContexts<T extends Map<any, any> = Map<any, any>>(): T;
    export function createEventDispatcher<
        Events extends Record<string, unknown> = Record<string, unknown>,
    >(): <K extends Extract<keyof Events, string>>(type: K, detail?: Events[K]) => boolean;
}

declare module 'svelte/store' {
    export interface Subscriber<T> {
        (value: T): void;
    }
    export interface Unsubscriber {
        (): void;
    }
    export interface Updater<T> {
        (value: T): T;
    }
    export interface Readable<T> {
        subscribe(run: Subscriber<T>, invalidate?: () => void): Unsubscriber;
    }
    export interface Writable<T> extends Readable<T> {
        set(this: void, value: T): void;
        update(this: void, updater: Updater<T>): void;
    }

    export type StartStopNotifier<T> = (
        set: (value: T) => void,
        update: (fn: Updater<T>) => void,
    ) => void | (() => void);

    export function readable<T>(value?: T, start?: StartStopNotifier<T>): Readable<T>;
    export function writable<T>(value?: T, start?: StartStopNotifier<T>): Writable<T>;
    export function derived<S extends Readable<unknown> | Array<Readable<unknown>>, T>(
        stores: S,
        fn: (values: any, set?: (value: T) => void) => T | void,
        initial_value?: T,
    ): Readable<T>;
    export function get<T>(store: Readable<T>): T;
    export function readonly<T>(store: Writable<T>): Readable<T>;
}

declare module 'svelte/transition' {
    export interface TransitionConfig {
        delay?: number;
        duration?: number;
        easing?: (t: number) => number;
        css?: (t: number, u: number) => string;
        tick?: (t: number, u: number) => void;
    }
    export type TransitionFn<P = any> = (
        node: Element,
        params?: P,
        options?: { direction?: 'in' | 'out' | 'both' },
    ) => TransitionConfig | (() => TransitionConfig);

    export const fade: TransitionFn<{ delay?: number; duration?: number; easing?: (t: number) => number }>;
    export const blur: TransitionFn<any>;
    export const fly: TransitionFn<any>;
    export const slide: TransitionFn<any>;
    export const scale: TransitionFn<any>;
    export const draw: TransitionFn<any>;
    export const crossfade: (params?: any) => [TransitionFn<any>, TransitionFn<any>];
}

declare module 'svelte/animate' {
    export interface AnimationConfig {
        delay?: number;
        duration?: number;
        easing?: (t: number) => number;
        css?: (t: number, u: number) => string;
        tick?: (t: number, u: number) => void;
    }
    export const flip: (
        node: Element,
        from: { from: DOMRect; to: DOMRect },
        params?: { delay?: number; duration?: number | ((len: number) => number); easing?: (t: number) => number },
    ) => AnimationConfig;
}

declare module 'svelte/easing' {
    export type EasingFunction = (t: number) => number;
    export const linear: EasingFunction;
    export const backIn: EasingFunction;
    export const backOut: EasingFunction;
    export const backInOut: EasingFunction;
    export const bounceIn: EasingFunction;
    export const bounceOut: EasingFunction;
    export const bounceInOut: EasingFunction;
    export const circIn: EasingFunction;
    export const circOut: EasingFunction;
    export const circInOut: EasingFunction;
    export const cubicIn: EasingFunction;
    export const cubicOut: EasingFunction;
    export const cubicInOut: EasingFunction;
    export const elasticIn: EasingFunction;
    export const elasticOut: EasingFunction;
    export const elasticInOut: EasingFunction;
    export const expoIn: EasingFunction;
    export const expoOut: EasingFunction;
    export const expoInOut: EasingFunction;
    export const quadIn: EasingFunction;
    export const quadOut: EasingFunction;
    export const quadInOut: EasingFunction;
    export const quartIn: EasingFunction;
    export const quartOut: EasingFunction;
    export const quartInOut: EasingFunction;
    export const quintIn: EasingFunction;
    export const quintOut: EasingFunction;
    export const quintInOut: EasingFunction;
    export const sineIn: EasingFunction;
    export const sineOut: EasingFunction;
    export const sineInOut: EasingFunction;
}

declare module 'svelte/motion' {
    import type { Readable } from 'svelte/store';
    export interface Spring<T> extends Readable<T> {
        set(value: T, opts?: { hard?: boolean; soft?: boolean | number }): Promise<void>;
        update(fn: (value: T, target: T) => T, opts?: { hard?: boolean; soft?: boolean | number }): Promise<void>;
        stiffness: number;
        damping: number;
        precision: number;
    }
    export interface Tweened<T> extends Readable<T> {
        set(value: T, opts?: { delay?: number; duration?: number; easing?: (t: number) => number }): Promise<void>;
        update(fn: (value: T, target: T) => T, opts?: any): Promise<void>;
    }
    export function spring<T>(value?: T, opts?: any): Spring<T>;
    export function tweened<T>(value?: T, opts?: any): Tweened<T>;
}

declare module 'svelte/action' {
    export interface ActionReturn<P = any, A = any> {
        update?: (parameter: P) => void;
        destroy?: () => void;
        $$_attributes?: A;
    }
    export interface Action<E extends Element = Element, P = any, A = any> {
        (node: E, parameter?: P): void | ActionReturn<P, A>;
    }
}

declare module 'svelte/legacy' {
    export function createBubbler(): any;
    export function nonpassive<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function passive<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function once<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function self<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function trusted<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function preventDefault<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function stopPropagation<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function stopImmediatePropagation<T extends Event>(handler: (event: T) => void): (event: T) => void;
}

declare module 'svelte/elements' {
    export type HTMLAttributes<T extends EventTarget = HTMLElement> = any;
    export type SVGAttributes<T extends EventTarget = SVGElement> = any;
    export type DOMAttributes<T extends EventTarget = Element> = any;
}

declare module 'svelte/compiler' {
    export const VERSION: string;
    export function compile(source: string, options?: any): any;
    export function parse(source: string, options?: any): any;
    export function preprocess(source: string, transformers: any, options?: any): Promise<{ code: string; map: any }>;
    export function walk(ast: any, walker: any): any;
}
