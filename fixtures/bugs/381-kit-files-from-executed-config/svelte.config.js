import { files } from './kit-files.js';

// Upstream svelte-check runs this config to read `kit.files`, so a
// value only known at run time still applies.
export default { kit: { files } };
