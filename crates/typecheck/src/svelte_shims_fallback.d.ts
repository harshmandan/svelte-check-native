// Fallback Svelte module shims — only written into the cache when no
// real `svelte` package is reachable from the workspace's
// node_modules chain. When real svelte IS installed these shims would
// shadow the richer real types and surface false-positive TS2305
// errors ("module has no exported member named 'X'") on user code
// that imports names we didn't enumerate.

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
    // ClassValue mirrors clsx-style accepted shapes — string, array, or
    // object map of class-name → boolean. Real Svelte 5.10+ exports this.
    export type ClassValue = any;
    // Event handler aliases — Svelte 5 re-exports these from
    // svelte/elements as ergonomic shorthands.
    export type EventHandler<E extends Event = Event, T extends EventTarget = Element> =
        (event: E & { currentTarget: EventTarget & T }) => any;
    export type ClipboardEventHandler<T extends EventTarget = Element> = EventHandler<ClipboardEvent, T>;
    export type CompositionEventHandler<T extends EventTarget = Element> = EventHandler<CompositionEvent, T>;
    export type DragEventHandler<T extends EventTarget = Element> = EventHandler<DragEvent, T>;
    export type FocusEventHandler<T extends EventTarget = Element> = EventHandler<FocusEvent, T>;
    export type FormEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type ChangeEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type KeyboardEventHandler<T extends EventTarget = Element> = EventHandler<KeyboardEvent, T>;
    export type MouseEventHandler<T extends EventTarget = Element> = EventHandler<MouseEvent, T>;
    export type TouchEventHandler<T extends EventTarget = Element> = EventHandler<TouchEvent, T>;
    export type PointerEventHandler<T extends EventTarget = Element> = EventHandler<PointerEvent, T>;
    export type UIEventHandler<T extends EventTarget = Element> = EventHandler<UIEvent, T>;
    export type WheelEventHandler<T extends EventTarget = Element> = EventHandler<WheelEvent, T>;
    export type AnimationEventHandler<T extends EventTarget = Element> = EventHandler<AnimationEvent, T>;
    export type TransitionEventHandler<T extends EventTarget = Element> = EventHandler<TransitionEvent, T>;
}

declare module 'svelte/compiler' {
    export const VERSION: string;
    export function compile(source: string, options?: any): any;
    export function parse(source: string, options?: any): any;
    export function preprocess(source: string, transformers: any, options?: any): Promise<{ code: string; map: any }>;
    export function walk(ast: any, walker: any): any;
}
