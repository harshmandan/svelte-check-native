// Writes crates/acorn/tests/corpus/javascript.json: every input of
// acorn's own test suite with what acorn does with it, read the four
// ways the Svelte compiler reads text.
//
//   git clone --depth 1 --branch <version> https://github.com/acornjs/acorn bench/_acorn
//   node scripts/acorn-corpus-oracle.mjs bench/_acorn <dir with node_modules/acorn>
//
// The second argument is where the `acorn` package the Svelte compiler
// uses is installed (any project with svelte in node_modules). Its
// version has to be the one the checkout is at.

import { createRequire } from 'node:module';
import { readdirSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const [checkout, installed] = process.argv.slice(2).map((p) => resolve(p));
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
const collect = (code) => {
	// A lone surrogate has no UTF-8 form, so it cannot be a file's text.
	if (typeof code === 'string' && code.isWellFormed() && code.length < 20000) inputs.add(code);
};
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

// The compiler empties the list of undeclared exports after every
// statement of a component's script.
class Script extends acorn.Parser {
	parseStatement(...args) {
		const statement = super.parseStatement(...args);
		this.undefinedExports = {};
		return statement;
	}
}

const corpus = [];
for (const code of inputs) {
	// A component's script, a tag's expression or statement is never at
	// offset zero of what acorn is given: one space stands for the rest.
	const padded = ' ' + code;
	corpus.push({
		code,
		script: shifted(run(padded, () => void Script.parse(padded, options))),
		module: run(code, () => void acorn.parse(code, options)),
		exprAt1: shifted(
			run(padded, () => acorn.parseExpressionAt(padded, 1, { ...options, preserveParens: true }).end)
		),
		stmtAt1: shifted(
			run(padded, () => {
				const parser = new acorn.Parser(options, padded, 1);
				parser.nextToken();
				return parser.parseStatement(null, true, Object.create(null)).end;
			})
		)
	});
}

const out = join(
	fileURLToPath(new URL('..', import.meta.url)),
	'crates/acorn/tests/corpus/javascript.json'
);
writeFileSync(out, JSON.stringify(corpus));
console.log(`${corpus.length} inputs, acorn ${acorn.version} -> ${out}`);
