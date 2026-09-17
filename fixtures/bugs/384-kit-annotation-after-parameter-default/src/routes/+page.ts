// The load-event annotation lands after the default value, as in
// upstream's typed copy, which then fails to parse.
export function load({ url } = {}) { return { x: url.nope }; }
