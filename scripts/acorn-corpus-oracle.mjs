// Writes crates/acorn/tests/corpus/javascript.json and typescript.json:
// every input of acorn's own test suite (and, for TypeScript, of
// acorn-typescript's) with what the parser does with it, read the four
// ways the Svelte compiler reads text.
//
//   git clone --depth 1 --branch <version> https://github.com/acornjs/acorn bench/_acorn
//   git clone --depth 1 https://github.com/sveltejs/acorn-typescript bench/_acorn_typescript
//   node scripts/acorn-corpus-oracle.mjs bench/_acorn <dir with node_modules> [bench/_acorn_typescript]
//
// The second argument is where the `acorn` and `@sveltejs/acorn-typescript`
// packages the Svelte compiler uses are installed (any project with
// svelte in node_modules). The acorn checkout has to be at that
// version. Only inputs are taken from the checkouts; the parsers that
// answer are the installed ones.

import { createRequire } from 'node:module';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const [checkout, installed, ts_checkout] = process.argv.slice(2).map((p) => resolve(p));
if (!checkout || !installed) {
	console.error('usage: acorn-corpus-oracle.mjs <acorn checkout> <dir with node_modules/acorn>');
	process.exit(2);
}

const require = createRequire(join(installed, 'x.js'));
const acorn = require('acorn');
const Module = require('node:module');

// acorn's tests register themselves with its driver; collect the code
// of each instead. They load acorn by a relative path that only exists
// after a build, so hand them the installed one.
const inputs = new Set();
const real_load = Module._load;
Module._load = function (request, ...rest) {
	return request.startsWith('.') && /(^|\/)acorn(-loose)?$/.test(request)
		? acorn
		: real_load.call(this, request, ...rest);
};
const test_dir = join(checkout, 'test');
const driver = require(join(test_dir, 'driver.js'));
const collect_into = (set, code) => {
	// A lone surrogate has no UTF-8 form, so it cannot be a file's text.
	if (typeof code === 'string' && code.isWellFormed() && code.length < 20000) set.add(code);
};
const collect = (code) => collect_into(inputs, code);
driver.test = collect;
driver.testFail = collect;
driver.testAssert = collect;
for (const file of readdirSync(test_dir).filter((f) => /^tests.*\.js$/.test(f)).sort()) {
	require(join(test_dir, file));
}

const options = { sourceType: 'module', ecmaVersion: 16, locations: true };
const byte = (text, index) => Buffer.byteLength(text.slice(0, index));

/** What `read` does with `text`: its end, or acorn's error. */
function run(text, read) {
	try {
		const end = read();
		return { end: end == null ? null : byte(text, end) };
	} catch (error) {
		if (!(error instanceof SyntaxError) || error.pos == null) {
			return { crash: String(error.message).slice(0, 80) };
		}
		return { pos: byte(text, error.pos), msg: error.message.replace(/ \(\d+:\d+\)$/, '') };
	}
}

/** The same, for a read one character into `text`. */
function shifted(result) {
	if (result.pos != null) return { ...result, pos: result.pos - 1 };
	if (result.end != null) return { end: result.end - 1 };
	return result;
}

/**
 * What `Parser` does with each of `codes`.
 * @param {typeof acorn.Parser} Parser
 * @param {Iterable<string>} codes
 */
function answers(Parser, codes) {
	// The compiler empties the list of undeclared exports after every
	// statement of a component's script.
	class Script extends Parser {
		parseStatement(...args) {
			const statement = super.parseStatement(...args);
			this.undefinedExports = {};
			return statement;
		}
	}
	const corpus = [];
	for (const code of codes) {
		// A component's script, a tag's expression or statement is never
		// at offset zero of what the parser is given: one space stands
		// for the rest.
		const padded = ' ' + code;
		corpus.push({
			code,
			script: shifted(run(padded, () => void Script.parse(padded, options))),
			module: run(code, () => void Parser.parse(code, options)),
			exprAt1: shifted(
				run(padded, () => Parser.parseExpressionAt(padded, 1, { ...options, preserveParens: true }).end)
			),
			stmtAt1: shifted(
				run(padded, () => {
					const parser = new Parser(options, padded, 1);
					parser.nextToken();
					return parser.parseStatement(null, true, Object.create(null)).end;
				})
			)
		});
	}
	return corpus;
}

const corpus_dir = join(fileURLToPath(new URL('..', import.meta.url)), 'crates/acorn/tests/corpus');

const javascript = answers(acorn.Parser, inputs);
writeFileSync(join(corpus_dir, 'javascript.json'), JSON.stringify(javascript));
console.log(`${javascript.length} inputs, acorn ${acorn.version} -> javascript.json`);

if (ts_checkout) {
	const entry = require.resolve('@sveltejs/acorn-typescript');
	const { tsPlugin } = await import(pathToFileURL(entry).href);
	const version = JSON.parse(readFileSync(join(dirname(entry), 'package.json'), 'utf8')).version;
	// TypeScript reads JavaScript too, its own way.
	const ts_inputs = new Set(inputs);
	const walk = (dir) => {
		for (const entry of readdirSync(dir, { withFileTypes: true })) {
			const path = join(dir, entry.name);
			if (entry.isDirectory()) walk(path);
			else if (/\.(ts|mts|cts)$/.test(entry.name) && !/\.(test|d)\.ts$/.test(entry.name)) {
				collect_into(ts_inputs, readFileSync(path, 'utf8'));
			}
		}
	};
	walk(join(ts_checkout, 'test'));
	const typescript = answers(acorn.Parser.extend(tsPlugin()), ts_inputs);
	writeFileSync(join(corpus_dir, 'typescript.json'), JSON.stringify(typescript));
	console.log(`${typescript.length} inputs, acorn-typescript ${version} -> typescript.json`);
}
