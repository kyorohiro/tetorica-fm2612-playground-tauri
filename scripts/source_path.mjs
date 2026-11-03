/** Resolve parent sources without changing historical snapshot/report keys. */
import {statSync} from 'node:fs';
import {join, relative, resolve, sep} from 'node:path';

const movedRoots = {web: 'runtime', node: 'node', cli: 'cli', native: 'native', wasm: 'wasm'};

export function sourcePath(root, ...parts) {
  const original = resolve(root, ...parts);
  const segments = relative(resolve(root), original).split(sep);
  const moved = Object.hasOwn(movedRoots, segments[0]) && movedRoots[segments[0]];
  if (moved) {
    let modern;
    try {
      modern = statSync(join(root, 'tetorica_src')).isDirectory();
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    if (modern) return join(root, 'tetorica_src', moved, ...segments.slice(1));
  }
  return original;
}
