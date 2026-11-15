#!/usr/bin/env node
// gen-upstream-emit.mjs — write upstream's emit for every emit-snapshot
// sample (language-tools' svelte2tsx / htmlx2jsx samples and our own
// fixtures/bugs), as `upstream.emit.ts` next to our `expected.emit.ts`.
//
// The reference is what `svelte-check --tsgo` itself generates: svelte2tsx
// (pinned in scripts/upstream-emit/package.json to the language-tools
// submodule's version) called with the options svelte-check's
// `incremental.ts` passes. The expected files shipped with upstream's
// samples are not used: for a non-`.v5` sample without an
// `expected-svelte5` file, `expectedv2` is the Svelte 4 output, which
// differs from what Svelte 5 projects get (implicit `children` props,
// for one).
//
// svelte-check takes `parse` and `VERSION` from one `svelte/compiler`,
// the project's. A sample whose workspace installs Svelte 4 (some
// fixtures carry a `node_modules/svelte` stub) gets the pinned Svelte 4
// compiler, every other the pinned Svelte 5 one; each file's header names
// the version used.
//
// The files of a sample are its input's `.svelte` files, found the way
// our `--emit-ts` finds them (`node_modules` and dot directories left
// out), not the ones our snapshot happens to list, so a component our
// emit drops still gets a reference. A file whose template svelte2tsx
// rejects (svelte-check reports the parse error and type-checks nothing
// of it) is listed in `upstream.error.txt` as `file: message`; the other
// files of the sample are still written to `upstream.emit.ts`.
//
// Run after bumping the submodule, then review the diff:
//   (cd scripts/upstream-emit && npm install)
//   node scripts/gen-upstream-emit.mjs
//   node scripts/diff-upstream-emit.mjs

import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { isTsSvelte, svelte2tsxOptions } from './svelte-check-tsgo.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PINNED = path.join(ROOT, 'scripts/upstream-emit');
const SNAP = path.join(ROOT, 'crates/cli/tests/emit_snapshots');
const SAMPLES = path.join(ROOT, 'language-tools/packages/svelte2tsx/test');

const require = createRequire(path.join(PINNED, 'package.json'));
// The pinned compilers, by the Svelte major a workspace installs.
const compilers = {};
let svelte2tsx, s2tVersion;
try {
    ({ svelte2tsx } = require('svelte2tsx'));
    s2tVersion = require('svelte2tsx/package.json').version;
    for (const [major, name] of [
        [5, 'svelte'],
        [4, 'svelte4'],
    ]) {
        const { parse, VERSION } = require(`${name}/compiler`);
        if (Number(VERSION.split('.')[0]) !== major) {
            throw new Error(`${name} is ${VERSION}, expected Svelte ${major}`);
        }
        compilers[major] = { parse, VERSION };
    }
} catch {
    console.error('missing pinned deps: run `(cd scripts/upstream-emit && npm install)`');
    process.exit(2);
}
const submoduleVersion = JSON.parse(
    fs.readFileSync(path.join(ROOT, 'language-tools/packages/svelte2tsx/package.json'), 'utf8')
).version;
if (s2tVersion !== submoduleVersion) {
    console.error(
        `svelte2tsx ${s2tVersion} is pinned but the submodule is at ${submoduleVersion}; ` +
            'update scripts/upstream-emit/package.json and reinstall'
    );
    process.exit(2);
}

/**
 * The `svelte/compiler` svelte-check would load for a workspace: the
 * pinned compiler of the major the nearest installed `svelte` package
 * has (some fixtures pin a Svelte 4 stub), else the pinned Svelte 5.
 */
function compilerFor(workspace) {
    for (let dir = workspace; dir.startsWith(ROOT); dir = path.dirname(dir)) {
        const pkg = path.join(dir, 'node_modules', 'svelte', 'package.json');
        if (fs.existsSync(pkg)) {
            const version = JSON.parse(fs.readFileSync(pkg, 'utf8')).version;
            const compiler = compilers[Number(version.split('.')[0])];
            if (!compiler) {
                console.error(`${pkg}: no pinned compiler for Svelte ${version}`);
                process.exit(2);
            }
            return compiler;
        }
        if (dir === ROOT) break;
    }
    return compilers[5];
}

/**
 * A sample's `.svelte` files, relative and sorted, found as our
 * `--emit-ts` finds them: every directory but `node_modules` and the
 * ones whose name starts with a dot.
 */
function sampleFiles(dir, base = dir, out = []) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === 'node_modules' || entry.name.startsWith('.')) continue;
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) sampleFiles(full, base, out);
        else if (entry.name.endsWith('.svelte')) out.push(path.relative(base, full).split(path.sep).join('/'));
    }
    return out.sort();
}

let written = 0;
let rejected = 0;
// Where each corpus keeps its inputs: upstream's samples, and our own
// bug fixtures.
const inputDir = {
    svelte2tsx: (name) => path.join(SAMPLES, 'svelte2tsx', 'samples', name),
    htmlx2jsx: (name) => path.join(SAMPLES, 'htmlx2jsx', 'samples', name),
    bugs: (name) => path.join(ROOT, 'fixtures', 'bugs', name),
};
for (const corpus of Object.keys(inputDir)) {
    const snapDir = path.join(SNAP, corpus);
    for (const name of fs.readdirSync(snapDir).sort()) {
        const ours = path.join(snapDir, name, 'expected.emit.ts');
        if (!fs.existsSync(ours)) continue;
        const workspace = inputDir[corpus](name);
        if (!fs.existsSync(workspace)) continue;
        const parts = [];
        const failures = [];
        const { parse, VERSION } = compilerFor(workspace);
        for (const rel of sampleFiles(workspace)) {
            const sourcePath = path.join(workspace, rel);
            const text = fs.readFileSync(sourcePath, 'utf8');
            const isTsFile = isTsSvelte(text);
            try {
                const { code } = svelte2tsx(
                    text,
                    svelte2tsxOptions({ parse, version: VERSION, workspace, sourcePath, isTsFile })
                );
                parts.push(`// === ${rel} ===\n${code}`);
            } catch (e) {
                failures.push(`${rel}: ${String(e.message ?? e).split('\n')[0]}`);
            }
        }
        const outDir = path.join(snapDir, name);
        const emitFile = path.join(outDir, 'upstream.emit.ts');
        const errorFile = path.join(outDir, 'upstream.error.txt');
        const header =
            `// generated by scripts/gen-upstream-emit.mjs: svelte2tsx ${s2tVersion}, ` +
            `svelte ${VERSION}, svelte-check --tsgo options\n`;
        if (failures.length) {
            fs.writeFileSync(errorFile, failures.join('\n') + '\n');
            rejected += failures.length;
        } else {
            fs.rmSync(errorFile, { force: true });
        }
        if (parts.length) {
            fs.writeFileSync(emitFile, header + parts.join('\n'));
            written++;
        } else {
            fs.rmSync(emitFile, { force: true });
        }
    }
}
console.log(`${written} upstream emits written, ${rejected} files rejected by svelte2tsx`);
