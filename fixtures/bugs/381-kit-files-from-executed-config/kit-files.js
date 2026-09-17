// A computed matcher directory, and a hooks path spelled with `./`,
// which never matches an absolute file path, so the hooks file is
// left untyped.
export const files = {
    params: ['src', 'matchers'].join('/'),
    hooks: { server: './src/hooks.server' }
};
