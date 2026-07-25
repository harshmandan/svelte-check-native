// Realistic stand-in for `svelte-kit sync`'s generated
// `.svelte-kit/types/src/routes/<route>/$types.d.ts` (SvelteKit 2.16+
// shape). Unlike fixtures 55/56/249 — which hardcode `PageData` — this
// file keeps the real generated indirection: `PageData` is INFERRED from
// the sibling `proxy+page.server.ts` module through
// `typeof import('./proxy+page.server.js').load`, and `PageProps` is the
// explicit-annotation surface (`let { data, form }: PageProps = $props()`)
// SvelteKit scaffolds today. The `@sveltejs/kit` helper types the real
// file imports via `import type * as Kit from '@sveltejs/kit'` are inlined
// in the `Kit` namespace at the bottom (bug fixtures install no
// node_modules); unreferenced generated exports (Snapshot, SubmitFunction,
// RequestEvent, matchers) are trimmed.

type Expand<T> = T extends infer O ? { [K in keyof O]: O[K] } : never;
type RouteParams = {};
type RouteId = '/';
type MaybeWithVoid<T> = {} extends T ? T | void : T;
export type RequiredKeys<T> = {
    [K in keyof T]-?: {} extends { [P in K]: T[K] } ? never : K;
}[keyof T];
type OutputDataShape<T> = MaybeWithVoid<
    Omit<App.PageData, RequiredKeys<T>> &
        Partial<Pick<App.PageData, keyof T & keyof App.PageData>> &
        Record<string, any>
>;
type EnsureDefined<T> = T extends null | undefined ? {} : T;
type OptionalUnion<
    U extends Record<string, any>,
    A extends keyof U = U extends U ? keyof U : never,
> = U extends unknown ? { [P in Exclude<A, keyof U>]?: never } & U : never;
type PageServerParentData = EnsureDefined<LayoutServerData>;
type PageParentData = EnsureDefined<LayoutData>;
type LayoutParentData = EnsureDefined<{}>;

export type PageServerLoad<
    OutputData extends
        OutputDataShape<PageServerParentData> = OutputDataShape<PageServerParentData>,
> = Kit.ServerLoad<RouteParams, PageServerParentData, OutputData, RouteId>;
export type PageServerLoadEvent = Parameters<PageServerLoad>[0];
type ActionsExport = typeof import('./proxy+page.server.js').actions;
export type ActionData = Expand<Kit.AwaitedActions<ActionsExport>> | null;
export type PageServerData = Expand<
    OptionalUnion<
        EnsureDefined<
            Kit.LoadProperties<Awaited<ReturnType<typeof import('./proxy+page.server.js').load>>>
        >
    >
>;
export type PageData = Expand<
    Omit<PageParentData, keyof PageServerData> & EnsureDefined<PageServerData>
>;
export type Actions<
    OutputData extends Record<string, any> | void = Record<string, any> | void,
> = Kit.Actions<RouteParams, OutputData, RouteId>;
export type PageProps = { params: RouteParams; data: PageData; form: ActionData };
export type LayoutServerData = null;
export type LayoutData = Expand<LayoutParentData>;

/**
 * Minimal inline copies of the `@sveltejs/kit` helper types referenced
 * above. Shapes match kit 2.x `types/index.d.ts` where inference depends
 * on them; load/action event parameters are collapsed to `any` (they play
 * no part in the PageData/ActionData chain under test).
 */
declare namespace Kit {
    type MaybePromise<T> = T | Promise<T>;
    interface ActionFailure<T = undefined> {
        status: number;
        data: T;
    }
    type UnpackValidationError<T> =
        T extends ActionFailure<infer X> ? X : T extends void ? undefined : T;
    type OptionalUnion<
        U extends Record<string, any>,
        A extends keyof U = U extends U ? keyof U : never,
    > = U extends unknown ? { [P in Exclude<A, keyof U>]?: never } & U : never;
    export type ServerLoad<
        Params extends Partial<Record<string, string>> = Partial<Record<string, string>>,
        ParentData extends Record<string, any> = Record<string, any>,
        OutputData extends Record<string, any> | void = Record<string, any> | void,
        RouteId extends string | null = string | null,
    > = (event: any) => MaybePromise<OutputData>;
    export type Action<
        Params extends Partial<Record<string, string>> = Partial<Record<string, string>>,
        OutputData extends Record<string, any> | void = Record<string, any> | void,
        RouteId extends string | null = string | null,
    > = (event: any) => MaybePromise<OutputData>;
    export type Actions<
        Params extends Partial<Record<string, string>> = Partial<Record<string, string>>,
        OutputData extends Record<string, any> | void = Record<string, any> | void,
        RouteId extends string | null = string | null,
    > = Record<string, Action<Params, OutputData, RouteId>>;
    export type AwaitedActions<T extends Record<string, (...args: any) => any>> = OptionalUnion<
        { [Key in keyof T]: UnpackValidationError<Awaited<ReturnType<T[Key]>>> }[keyof T]
    >;
    export type LoadProperties<input extends Record<string, any> | void> = input extends void
        ? undefined
        : input extends Record<string, any>
          ? input
          : unknown;
}
