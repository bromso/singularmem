import { spawnSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/**
 * Create a fresh empty tempdir + a path to a (not-yet-existing) SQLite file
 * inside it. Caller is responsible for actually populating the file via
 * the root CLI or via the napi binding itself.
 */
export function freshStorePath() {
  const dir = mkdtempSync(join(tmpdir(), 'sm-node-test-'));
  return { dir, path: join(dir, 'store.db') };
}

/**
 * Seed a store at `path` by spawning the root `singularmem` CLI binary.
 * Each item is `{ content: string, tags?: string[], source?: string }`.
 * Uses `cargo run -q -p singularmem -- ingest ...` so it works without
 * needing a pre-built binary on disk.
 *
 * Verified CLI flags (from `cargo run -p singularmem -- ingest --help`):
 *   --store <PATH>       path to the SQLite store file
 *   --content <CONTENT>  item content as a literal string
 *   --tag <TAGS>         tag (repeatable)
 *   --source <SOURCE>    free-form provenance label
 */
export function seedStore(path, items) {
  for (const item of items) {
    const args = [
      'run', '-q', '-p', 'singularmem', '--',
      'ingest',
      '--store', path,
      '--content', item.content,
    ];
    for (const tag of item.tags || []) {
      args.push('--tag', tag);
    }
    if (item.source) args.push('--source', item.source);
    const result = spawnSync('cargo', args, { stdio: 'pipe', encoding: 'utf8' });
    if (result.error) {
      throw new Error(`failed to spawn cargo: ${result.error.message}`);
    }
    if (result.status !== 0) {
      throw new Error(
        `cargo ingest failed (exit ${result.status}):\nstdout: ${result.stdout}\nstderr: ${result.stderr}`,
      );
    }
  }
}
