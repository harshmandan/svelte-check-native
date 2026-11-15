// What `svelte-check --tsgo` does to turn a component into TypeScript
// (`language-tools/packages/svelte-check/src/incremental.ts`), shared by
// the scripts that compare our emit with upstream's so they all produce
// the same upstream side.

import path from 'node:path';

/** svelte-check's `isTsSvelte`: any `<script>` tag with `lang` `ts` or `typescript`. */
export function isTsSvelte(text) {
    const scriptTag = /<script\b((?:\s+[^=>'"\/\s]+(?:=(?:"[^"]*"|'[^']*'|[^>\s]+))?)*)\s*>/gi;
    const langAttr = /\blang\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/i;
    let m;
    while ((m = scriptTag.exec(text)) !== null) {
        const lang = langAttr.exec(m[1] ?? '');
        if (!lang) continue;
        const v = (lang[1] ?? lang[2] ?? lang[3] ?? '').toLowerCase();
        if (v === 'ts' || v === 'typescript') return true;
    }
    return false;
}

/**
 * Where svelte-check writes the generated file for `sourcePath` (relative
 * to `workspace`): `.svelte-check/svelte/<dir>/++<name>.svelte.<ts|js>`.
 */
export function generatedPath(workspace, sourcePath, isTsFile) {
    const rel = path.relative(workspace, sourcePath);
    return path.join(
        workspace,
        '.svelte-check/svelte',
        path.dirname(rel),
        `++${path.basename(rel).replace(/\.svelte$/, `.svelte.${isTsFile ? 'ts' : 'js'}`)}`
    );
}

/**
 * The options svelte-check passes `svelte2tsx` for one component, given
 * the project's `svelte/compiler` (`parse`, `VERSION`).
 */
export function svelte2tsxOptions({ parse, version, workspace, sourcePath, isTsFile }) {
    return {
        parse,
        version,
        filename: sourcePath,
        isTsFile,
        mode: 'ts',
        emitOnTemplateError: false,
        emitJsDoc: true,
        rewriteExternalImports: {
            workspacePath: workspace,
            generatedPath: generatedPath(workspace, sourcePath, isTsFile),
        },
    };
}
