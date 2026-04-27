// Acts as a "workspace inner file." Imports from an external sibling
// pkg using `../../external_pkg/...`. To make this work in our overlay,
// the relative path needs to be rewritten when emitted into the cache
// directory (which sits BELOW the workspace).

import { hello } from '../../external_pkg/lib.ts';

export const greeting: string = hello('world');
