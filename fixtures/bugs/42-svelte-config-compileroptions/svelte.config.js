// svelte.config.js with `experimental.async: true`. Without our bridge
// honoring this flag the inner `await` in the template would fail with
// `experimental_async` from svelte/compiler.
export default {
  compilerOptions: {
    experimental: {
      async: true,
    },
  },
}
