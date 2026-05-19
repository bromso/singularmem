// Tests for ingest's per-call hook wiring: requires pre-primed Tantivy +
// USearch sidecars. The hooks attach for the duration of each ingest call
// only (matches singularmem-mcp's pattern); subsequent search probes
// re-open the sidecars without lock conflict.
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

function primeSidecars(path) {
  const r = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'singularmem', '--', 'reindex', '--with-embeddings', '--store', path],
    {
      stdio: 'pipe',
      encoding: 'utf8',
      env: { ...process.env, SINGULARMEM_TEST_EMBEDDER: 'mock' },
    },
  );
  if (r.status !== 0) throw new Error(`reindex failed: ${r.stderr}`);
}

test('Store.ingest into a fresh store leaves no sidecars (search throws NoIndexes)', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await store.ingest({ content: 'no indexes yet' });
  await assert.rejects(
    () => store.search('anything'),
    (err) => err.code === 'NoIndexes',
  );
});

test('Store.ingest after pre-priming sidecars makes the item searchable', async () => {
  const { path } = freshStorePath();
  primeSidecars(path);
  const store = await Store.open(path);
  // Exact-match query so MockEmbedder produces a hit
  await store.ingest({ content: 'cats are great pets' });
  const results = await store.search('cats are great pets');
  assert.ok(
    results.hits.length >= 1,
    'newly ingested item should be searchable via the per-call hook wiring',
  );
});

test('Store.ingest on read-only store rejects with ReadOnly even when sidecars exist', async () => {
  const { path } = freshStorePath();
  primeSidecars(path);
  const ro = await Store.open(path, { readOnly: true });
  await assert.rejects(
    () => ro.ingest({ content: 'should reject' }),
    (err) => err.code === 'ReadOnly',
  );
});
