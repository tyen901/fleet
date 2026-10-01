// Cargo resolves optional Git dependencies even when their feature is disabled.
// Build a source-identical, dependency-pruned workspace so UI development does
// not need access to Flux. The normal workspace remains the Flux-enabled path.
import { spawn } from 'node:child_process';
import { cp, mkdir, readFile, writeFile, readdir, rm, symlink } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const projected = path.join(root, 'target', 'no-flux-workspace');
const args = process.argv.slice(2);
if (!args.length || args.some((arg) => arg === '--all-features' || arg.includes('flux'))) {
  throw new Error(
    'Usage: node scripts/cargo-no-flux.mjs <cargo command> [arguments without the flux feature]',
  );
}
const omitted = new Set(['crates/flux', 'crates/inventory']);
const rootManifest = await readFile(path.join(root, 'Cargo.toml'), 'utf8');
const members = [...rootManifest.match(/members\s*=\s*\[([\s\S]*?)\]/)[1].matchAll(/"([^"]+)"/g)]
  .map((match) => match[1])
  .filter((member) => !omitted.has(member));
const prune = (manifest) =>
  manifest
    .replace(/^\s*"crates\/(?:flux|inventory)",\s*\n/gm, '')
    .replace(/^(?:flux|fleet-flux|fleet-inventory)\s*=.*\n/gm, '')
    .replace(/^default = \["flux"\]$/gm, 'default = []')
    .replace(/^(\[features\]\n)/m, '$1flux = []\n');
await mkdir(projected, { recursive: true });
await writeFile(path.join(projected, 'Cargo.toml'), prune(rootManifest));
let inputs = rootManifest;
for (const member of members) {
  const source = path.join(root, member);
  const destination = path.join(projected, member);
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  for (const entry of await readdir(source, { withFileTypes: true })) {
    if (entry.name === 'Cargo.toml' || entry.name === 'target') continue;
    const from = path.join(source, entry.name);
    const to = path.join(destination, entry.name);
    // Directory junctions work on Windows without symlink privileges. Source
    // and assets have one owner; only manifests/build-script files are copied.
    if (entry.isDirectory())
      await symlink(from, to, process.platform === 'win32' ? 'junction' : 'dir');
    else await cp(from, to);
  }
  const manifest = await readFile(path.join(source, 'Cargo.toml'), 'utf8');
  inputs += manifest;
  await writeFile(path.join(destination, 'Cargo.toml'), prune(manifest));
}
const rootLock = await readFile(path.join(root, 'Cargo.lock'), 'utf8');
const fingerprint = createHash('sha256').update(inputs).update(rootLock).digest('hex');
const stamp = path.join(projected, 'inputs.sha256');
const initialized = (await readFile(stamp, 'utf8').catch(() => '')) === fingerprint;
const lock = path.join(projected, 'Cargo.lock');
if (!initialized) await writeFile(lock, rootLock);
function cargo(arguments_, quiet = false) {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', arguments_, {
      cwd: projected,
      env: { ...process.env, CARGO_TARGET_DIR: path.join(root, 'target', 'no-flux') },
      stdio: quiet ? ['inherit', 'ignore', 'inherit'] : 'inherit',
    });
    child.once('error', reject);
    child.once('exit', (code) =>
      code === 0 ? resolve() : reject(new Error(`Cargo exited with ${code}`)),
    );
  });
}
if (!initialized) {
  await cargo(['metadata', '--format-version', '1', '--quiet'], true);
  const packages = (text) =>
    text
      .split('[[package]]')
      .slice(1)
      .map((block) =>
        ['name', 'version', 'source']
          .map((key) => block.match(new RegExp(`^${key} = "([^"\\n]+)"`, 'm'))?.[1] ?? '')
          .join('|'),
      );
  const pinned = new Set(packages(rootLock));
  const changed = packages(await readFile(lock, 'utf8')).filter((pkg) => !pinned.has(pkg));
  if (changed.length)
    throw new Error(`No-Flux resolution changed pinned dependencies: ${changed.join(', ')}`);
  await writeFile(stamp, fingerprint);
}
await cargo(args);
