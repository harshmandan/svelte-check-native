// Mirrors, byte-for-byte, what upstream svelte2tsx's
// `upsertKitServerHooksFile` / `upsertKitClientHooksFile` /
// `upsertKitUniversalHooksFile` / `upsertKitParamsFile` splice onto an
// untyped hooks or param-matcher export.
//
// The annotation pair per export is:
//   param  — `: Parameters<import('<kit>').<Type>>[0]` at the end of the
//            lone parameter
//   return — ` : ReturnType<import('<kit>').<Type>> ` at the `=>` token
//            (arrow) or the body's `{` (function declaration)
//
// `<kit>` is `@sveltejs/kit/hooks` from SvelteKit 3 on, `@sveltejs/kit`
// before it. A param matcher is the one case with a concrete type pair
// rather than the Parameters/ReturnType projection: `param: string` and
// a `boolean` return.
//
// Everything here must type-check clean.

export const handle = ({ event, resolve }: Parameters<import('@sveltejs/kit/hooks').Handle>[0]) : ReturnType<import('@sveltejs/kit/hooks').Handle> => {
	return resolve(event);
};

export function handleFetch({ request, fetch }: Parameters<import('@sveltejs/kit/hooks').HandleFetch>[0]) : ReturnType<import('@sveltejs/kit/hooks').HandleFetch> {
	return fetch(request);
}

export const handleError = ({ event }: Parameters<import('@sveltejs/kit/hooks').HandleServerError>[0]) : ReturnType<import('@sveltejs/kit/hooks').HandleServerError> => {
	return { message: event.url.pathname };
};

export const reroute = ({ url }: Parameters<import('@sveltejs/kit/hooks').Reroute>[0]) : ReturnType<import('@sveltejs/kit/hooks').Reroute> => url.pathname;

export function match(param: string) : boolean {
	return param === 'apple';
}
