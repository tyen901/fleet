import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const args = process.argv.slice(2);
const localFlux = args.includes('--local-flux');
const cargoArgs = args.filter((arg) => arg !== '--local-flux');
const lockfile = resolve(root, 'Cargo.lock');
const gitLock = localFlux ? readFileSync(lockfile) : null;

if (localFlux) {
  // Cargo patches change source identities, so the local run needs its own lock.
  for (let i = cargoArgs.length - 1; i >= 0; i--) {
    if (cargoArgs[i] === '--locked') cargoArgs.splice(i, 1);
  }
  const path = resolve(root, '../flux/crates/flux').replaceAll('\\', '/');
  cargoArgs.unshift('--config', `patch."https://github.com/tyen901/flux.git".flux.path=${JSON.stringify(path)}`);
}

let result;
try {
  result = spawnSync('cargo', cargoArgs, { cwd: root, stdio: 'inherit' });
} finally {
  if (localFlux) writeFileSync(lockfile, gitLock);
}
if (result.error) throw result.error;
process.exit(result.status ?? 1);
