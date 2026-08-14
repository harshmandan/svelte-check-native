// Same shapes as clean.ts with the bodies deliberately wrong. Each
// export fires exactly one diagnostic, proving the injected annotations
// are load-bearing rather than decorative — an injection that resolved
// to `any` would leave this file clean.

// TS2322: a Reroute must return a pathname string, not a number.
export const reroute = ({ url }: Parameters<import('@sveltejs/kit/hooks').Reroute>[0]) : ReturnType<import('@sveltejs/kit/hooks').Reroute> => 42;

// TS2322: a param matcher returns boolean.
export function match(param: string) : boolean {
	return param;
}

// TS2339: the argument of a HandleServerError has no `nope`.
export const handleError = ({ event }: Parameters<import('@sveltejs/kit/hooks').HandleServerError>[0]) : ReturnType<import('@sveltejs/kit/hooks').HandleServerError> => {
	return { message: event.nope };
};

// TS1064, and it is NOT a mistake in this fixture — it is what upstream
// produces for the single most common way to write this hook:
//
//   export const handle = async ({ event, resolve }) => resolve(event);
//
// `ReturnType<Handle>` is `MaybePromise<Response>`, not `Promise<T>`, so
// annotating an `async` function with it is an error by construction.
// Upstream injects the annotation anyway and reports the diagnostic, so
// parity means we reproduce it rather than "fixing" it.
export const handle = async ({ event, resolve }: Parameters<import('@sveltejs/kit/hooks').Handle>[0]) : ReturnType<import('@sveltejs/kit/hooks').Handle> => {
	return resolve(event);
};
