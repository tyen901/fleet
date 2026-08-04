// Enforces the UI Design Rules in AGENTS.md against the stylesheets.
// Run with `npm run lint:design`.
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const cssRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  'apps',
  'fleet',
  'assets',
  'css',
);

// tokens.css defines the scales, so it is the one file allowed raw values.
const TOKENS_FILE = 'tokens.css';
// The section label and the brand wordmark are the only uppercase text.
const UPPERCASE_FILES = new Set(['typography.css', 'onboarding.css']);

const SPACING_PROPS = new Set([
  'gap',
  'row-gap',
  'column-gap',
  'padding',
  'padding-top',
  'padding-bottom',
  'padding-left',
  'padding-right',
  'margin',
  'margin-top',
  'margin-bottom',
  'margin-left',
  'margin-right',
]);

const RULES = [
  {
    prop: 'font-size',
    ok: (value) => /^var\(--text-(title|body|label|caption)\)$/.test(value.trim()),
    message: 'font-size must be one of the four --text-* roles',
  },
  {
    prop: 'font-weight',
    ok: (value) => /^var\(--weight-(regular|medium)\)$/.test(value.trim()),
    message: 'font-weight must be --weight-regular or --weight-medium',
  },
  {
    prop: 'letter-spacing',
    ok: (value) => value.trim() === 'var(--tracking-label)',
    message: 'letter-spacing is only ever --tracking-label, on the section label',
  },
];

async function cssFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...(await cssFiles(full)));
    else if (entry.name.endsWith('.css')) files.push(full);
  }
  return files;
}

const problems = [];

for (const file of await cssFiles(cssRoot)) {
  const name = path.basename(file);
  if (name === TOKENS_FILE) continue;
  const relative = path.relative(cssRoot, file).split(path.sep).join('/');
  const lines = (await readFile(file, 'utf8')).split('\n');

  lines.forEach((line, index) => {
    const at = `${relative}:${index + 1}`;
    const match = /^\s*([a-z-]+):\s*([^;]+);/.exec(line);
    if (!match) return;
    const [, prop, value] = match;

    for (const rule of RULES) {
      if (prop === rule.prop && !rule.ok(value)) {
        problems.push(`${at}  ${rule.message} (found "${value.trim()}")`);
      }
    }

    if (prop === 'text-transform' && value.trim() === 'uppercase' && !UPPERCASE_FILES.has(name)) {
      problems.push(`${at}  uppercase belongs to the section label in typography.css`);
    }

    if (SPACING_PROPS.has(prop) && /\d+px/.test(value)) {
      problems.push(`${at}  spacing must use the --space-* scale (found "${value.trim()}")`);
    }

    if (prop === 'padding') {
      const parts = value.trim().match(/(?:[^\s(]|\([^)]*\))+/g) ?? [];
      const distinct = new Set(parts);
      if (parts.length > 1 && distinct.size > 1 && !distinct.has('0')) {
        problems.push(`${at}  padding must be even on all sides (found "${value.trim()}")`);
      }
    }
  });
}

if (problems.length > 0) {
  process.stderr.write(`Design rule violations (${problems.length}):\n`);
  for (const problem of problems) process.stderr.write(`  ${problem}\n`);
  process.exit(1);
}

process.stdout.write('Design rules hold.\n');
