#!/usr/bin/env node
// diff-emit.mjs — diff upstream svelte2tsx's overlay against ours for
// a given .svelte file, side-by-side. Optional --probe mode appends a
// type-probe to both overlays and runs tsgo to surface the inferred
// types (useful when diagnostics are silent but shouldn't be).
//
// CLAUDE.md's diagnostic method starts with "diff the real upstream
// artifact" on any count divergence. This script automates steps 2-4
// of that protocol so the comparison is one command instead of manual
// copy-paste.
//
// Usage:
//   node scripts/diff-emit.mjs <path/to/File.svelte>
//       Print upstream + our overlay side-by-side via `diff`.
//
//   node scripts/diff-emit.mjs <path/to/File.svelte> --upstream
//       Print ONLY the upstream overlay.
//
//   node scripts/diff-emit.mjs <path/to/File.svelte> --ours
//       Print ONLY our overlay (requires prior run so cache is populated).
//
//   node scripts/diff-emit.mjs <path/to/File.svelte> --probe "IDENT"
//       Append a type-probe after both overlays and run tsgo to reveal
//       the type IDENT resolves to. Expects IDENT to be an import or
//       local name from the script. Useful for diagnosing why a
//       type like UI.Dropdown falls through to the loose overload.
//       Example:
//         node scripts/diff-emit.mjs src/App.svelte --probe "UI.Dropdown"
//       Emits probe:
//         type __Probe = typeof IDENT;
//         declare const __p: __Probe;
//         const __c = __svn_ensure_component(__p);
//         const __x: string = new __c({ target: null, props: {} });
//       The TS2322 on `__x: string = <actual type>` reveals the
//       inferred constructor's props shape.
//
// The workspace + tsconfig are inferred from the file path by walking
// up for the nearest tsconfig.json + node_modules. Override with
// --workspace <path> / --tsconfig <path>.
//
// The upstream svelte2tsx version is picked by searching the
// workspace's node_modules; prefer whichever is highest-version. Pass
// --isTsFile / --isJsFile to override the auto-detection (looks at
// `<script lang="ts">` in the source).

import { readFileSync, existsSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, dirname, join, basename, relative } from 'node:path';
import { execSync, spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';

const args = process.argv.slice(2);
if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
    console.error(
        'Usage: diff-emit.mjs <path/to/File.svelte> [--upstream|--ours|--probe IDENT] [--workspace PATH] [--tsconfig PATH] [--isTsFile|--isJsFile]',
    );
    process.exit(args.length === 0 ? 2 : 0);
}

const filePath = resolve(args[0]);
if (!existsSync(filePath)) {
    console.error(`File not found: ${filePath}`);
    process.exit(2);
}

function flag(name) {
    return args.includes(name);
}
function value(name) {
    const i = args.indexOf(name);
    return i === -1 ? null : args[i + 1];
}

let workspace = value('--workspace');
let tsconfig = value('--tsconfig');
let forceTs = flag('--isTsFile') ? true : flag('--isJsFile') ? false : null;
const mode = flag('--upstream')
    ? 'upstream'
    : flag('--ours')
      ? 'ours'
      : flag('--probe')
        ? 'probe'
        : 'diff';
const probeIdent = mode === 'probe' ? value('--probe') : null;

if (!workspace) {
    // Walk up from filePath for a node_modules dir.
    let dir = dirname(filePath);
    while (dir !== '/' && dir.length > 1) {
        if (existsSync(join(dir, 'node_modules'))) {
            workspace = dir;
            break;
        }
        dir = dirname(dir);
    }
    if (!workspace) {
        console.error('Could not find workspace (parent with node_modules/). Pass --workspace.');
        process.exit(2);
    }
}
if (!tsconfig) {
    tsconfig = join(workspace, 'tsconfig.json');
    if (!existsSync(tsconfig)) {
        console.error(`tsconfig not at ${tsconfig}. Pass --tsconfig.`);
        process.exit(2);
    }
}

if (forceTs === null) {
    const src = readFileSync(filePath, 'utf8');
    forceTs = /<script[^>]*\blang\s*=\s*["']ts["'][^>]*>/.test(src);
}

// Find svelte2tsx. Prefer workspace's node_modules, highest version.
function findSvelte2tsx(ws) {
    const candidates = [];
    function walk(dir, depth = 0) {
        if (depth > 6) return;
        try {
            const ents = readFileSync(join(dir, '.'), 'utf8');
            void ents;
        } catch {}
        try {
            const { readdirSync, statSync } = require('node:fs');
            for (const ent of readdirSync(dir)) {
                const p = join(dir, ent);
                const st = statSync(p, { throwIfNoEntry: false });
                if (!st) continue;
                if (st.isDirectory()) {
                    if (
                        ent === 'svelte2tsx' &&
                        existsSync(join(p, 'index.mjs'))
                    ) {
                        candidates.push(join(p, 'index.mjs'));
                    } else if (depth < 6) {
                        walk(p, depth + 1);
                    }
                }
            }
        } catch {}
    }
    // Faster approach: spawn find
    const out = spawnSync(
        'find',
        [
            join(ws, 'node_modules'),
            '-name',
            'index.mjs',
            '-path',
            '*svelte2tsx*',
            '-not',
            '-path',
            '*/dist/*',
        ],
        { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
    );
    const lines = out.stdout.split('\n').filter(Boolean);
    return lines[0] || null;
}

let s2tsxPath = findSvelte2tsx(workspace);
if (!s2tsxPath) {
    // Fallback: walk up looking for a sibling bench dir with svelte2tsx.
    // Useful for benches that ship as pure npm workspaces with no
    // svelte2tsx locally, when run from the repo root where a sibling
    // bench pulls it in.
    let dir = dirname(workspace);
    while (dir !== '/' && dir.length > 1 && !s2tsxPath) {
        if (existsSync(join(dir, 'bench'))) {
            const r = spawnSync(
                'find',
                [
                    join(dir, 'bench'),
                    '-name',
                    'index.mjs',
                    '-path',
                    '*svelte2tsx*',
                    '-not',
                    '-path',
                    '*/dist/*',
                ],
                { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
            );
            const lines = (r.stdout || '').split('\n').filter(Boolean);
            if (lines.length) {
                s2tsxPath = lines[0];
                console.error(`Using ${s2tsxPath} (workspace has no local svelte2tsx).`);
            }
        }
        dir = dirname(dir);
    }
}
if (!s2tsxPath) {
    console.error(`svelte2tsx not found in ${workspace}/node_modules or any sibling bench.`);
    process.exit(2);
}

async function dumpUpstream() {
    const mod = await import(s2tsxPath);
    const src = readFileSync(filePath, 'utf8');
    const result = mod.svelte2tsx(src, {
        filename: filePath,
        isTsFile: forceTs,
        mode: 'ts',
    });
    return result.code;
}

function findOurs() {
    // Our overlay lives in the workspace's cache.
    const cacheDir = join(workspace, 'node_modules', '.cache', 'svelte-check-native');
    if (!existsSync(cacheDir)) return null;
    // Path inside cache mirrors workspace-relative path, with .svn.(ts|js).
    const relFromWs = relative(workspace, filePath);
    const base = join(cacheDir, 'svelte', relFromWs);
    const candidates = [`${base}.svn.ts`, `${base}.svn.js`];
    for (const c of candidates) if (existsSync(c)) return c;
    return null;
}

async function ensureOurOverlay() {
    // Regenerate the cache if the overlay isn't present yet.
    let p = findOurs();
    if (p) return p;
    console.error('Our overlay not cached. Running svelte-check-native to populate...');
    const bin = join(dirname(process.argv[1]), '..', 'target', 'release', 'svelte-check-native');
    if (!existsSync(bin)) {
        console.error(
            `Binary missing at ${bin}. Run 'cargo build --release' first (from repo root).`,
        );
        process.exit(2);
    }
    execSync(`"${bin}" --workspace "${workspace}" --tsconfig "${tsconfig}" --output machine`, {
        stdio: 'inherit',
        env: { ...process.env },
    });
    p = findOurs();
    if (!p) {
        console.error(`Still no overlay for ${filePath} after re-run.`);
        process.exit(2);
    }
    return p;
}

function runTsgo(cfgPath) {
    const out = spawnSync(
        'find',
        [workspace, '-name', 'tsgo', '-path', '*native-preview-darwin-arm64*', '-type', 'f'],
        { encoding: 'utf8' },
    );
    const tsgo = (out.stdout || '').split('\n').filter(Boolean)[0];
    if (!tsgo) {
        console.error('tsgo not found in workspace.');
        return '';
    }
    const r = spawnSync(tsgo, ['--pretty', 'false', '-p', cfgPath], {
        encoding: 'utf8',
        maxBuffer: 64 * 1024 * 1024,
    });
    return (r.stdout || '') + (r.stderr || '');
}

const upstream = await dumpUpstream();

if (mode === 'upstream') {
    process.stdout.write(upstream);
    process.exit(0);
}

if (mode === 'ours') {
    const p = await ensureOurOverlay();
    process.stdout.write(readFileSync(p, 'utf8'));
    process.exit(0);
}

const ourPath = await ensureOurOverlay();
const ours = readFileSync(ourPath, 'utf8');

if (mode === 'diff') {
    const tmp = mkdtempSync(join(tmpdir(), 'svn-diff-'));
    const uFile = join(tmp, 'upstream.ts');
    const oFile = join(tmp, 'ours.ts');
    writeFileSync(uFile, upstream);
    writeFileSync(oFile, ours);
    const r = spawnSync('diff', ['-u', '--color=always', uFile, oFile], {
        encoding: 'utf8',
        stdio: ['ignore', 'inherit', 'inherit'],
    });
    rmSync(tmp, { recursive: true, force: true });
    process.exit(r.status ?? 0);
}

if (mode === 'probe') {
    console.error(`Probing ${probeIdent} in ${filePath}...`);
    // Append a type-probe to our overlay.
    const probe = `\n// === diff-emit probe: reveal ${probeIdent}'s resolved type ===\ntype __Probe = typeof ${probeIdent};\ndeclare const __p: __Probe;\nconst __c = __svn_ensure_component(__p);\n// TS2322 below reveals the inferred constructor's return.\nconst __x: string = new __c({ target: null, props: {} });\nvoid __x;\n`;
    writeFileSync(ourPath, ours + probe);
    const cachedTsconfig = join(workspace, 'node_modules', '.cache', 'svelte-check-native', 'tsconfig.json');
    // Clear incremental build info so the probe is re-checked.
    const tsBuildInfo = join(workspace, 'node_modules', '.cache', 'svelte-check-native', 'tsbuildinfo.json');
    try {
        rmSync(tsBuildInfo);
    } catch {}
    const out = runTsgo(cachedTsconfig);
    // Restore overlay so subsequent bench runs aren't affected.
    writeFileSync(ourPath, ours);
    const relOverlay = relative(workspace, ourPath);
    // Filter to just the probe's diagnostics.
    const probeDiags = out
        .split('\n')
        .filter((l) => l.includes(relOverlay) && (l.includes('__x') || l.includes('__c')))
        .join('\n');
    if (probeDiags) {
        console.log('=== Probe diagnostics (inferred type leaks out of TS2322 message) ===');
        console.log(probeDiags);
    } else {
        console.log(
            '=== Probe fired no diagnostics — either the ident resolves loose (any), or tsgo accepted the probe as-is ===',
        );
    }
    console.log('');
    console.log(`=== Upstream emit (${filePath}) head ===`);
    console.log(upstream.split('\n').slice(0, 60).join('\n'));
    process.exit(0);
}
