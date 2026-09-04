// Route the napi binding through MockEmbedder so tests don't depend on a
// fastembed model download. Must be set BEFORE `Store` runs any search.
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

/**
 * Pre-create empty Tantivy + USearch sidecars at `path` via the root CLI's
 * `reindex --with-embeddings`, without seeding any items through the CLI.
 * Items are ingested afterwards via the node binding itself (so `scope` is
 * exercised through the binding, not the CLI), and `store.ingest()` picks
 * up the sidecars automatically once they exist on disk.
 */
function createEmptySidecars(path) {
  const r = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'singularmem', '--', 'reindex', '--with-embeddings', '--store', path],
    {
      stdio: 'pipe',
      encoding: 'utf8',
      env: { ...process.env, SINGULARMEM_TEST_EMBEDDER: 'mock' },
    },
  );
  if (r.error) throw new Error(`failed to spawn cargo: ${r.error.message}`);
  if (r.status !== 0) {
    throw new Error(`reindex failed (exit ${r.status}):\nstdout: ${r.stdout}\nstderr: ${r.stderr}`);
  }
}

test('ingest with scope normalizes to lowercase', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({ content: 'hello', scope: 'Team/X' });
  assert.equal(item.scope, 'team/x');
});

test('list({ scope }) matches descendants; scopeExact excludes them', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({ content: 'hello', scope: 'Team/X' });

  const descendants = await store.list({ scope: 'team' });
  assert.equal(descendants.length, 1);
  assert.equal(descendants[0].id, item.id);

  const exact = await store.list({ scope: 'team', scopeExact: true });
  assert.equal(exact.length, 0);

  const exactMatch = await store.list({ scope: 'team/x', scopeExact: true });
  assert.equal(exactMatch.length, 1);
  assert.equal(exactMatch[0].id, item.id);
});

test('search({ scope }) and retrieve({ scope }) return scoped items', async () => {
  const { path } = freshStorePath();
  createEmptySidecars(path);
  const store = await Store.open(path);
  await store.ingest({ content: 'scoped search content', scope: 'team/x' });

  const results = await store.search('scoped search content', { scope: 'team' });
  assert.ok(results.hits.length >= 1, 'expected at least one hit within scope');

  const ctx = await store.retrieve('scoped search content', { scope: 'team' });
  assert.ok(ctx.blocks.length >= 1, 'expected at least one block within scope');
});

test('search({ scope }) excludes items outside the scope', async () => {
  const { path } = freshStorePath();
  createEmptySidecars(path);
  const store = await Store.open(path);
  await store.ingest({ content: 'unscoped search content xyzzy' });

  const results = await store.search('unscoped search content xyzzy', { scope: 'team' });
  assert.equal(results.hits.length, 0, 'unscoped item should not match a scope filter');
});

test('scopes() aggregates counts by path', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await store.ingest({ content: 'one', scope: 'team/x' });
  await store.ingest({ content: 'two', scope: 'team/x' });

  const counts = await store.scopes();
  assert.deepEqual(counts, [{ path: 'team/x', count: 2 }]);
});

test('setScope moves an item to a new scope; scopes() reflects the move', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({ content: 'movable', scope: 'team/x' });

  const updated = await store.setScope(item.id, 'other');
  assert.equal(updated.scope, 'other');
  assert.equal(updated.id, item.id);

  const counts = await store.scopes();
  assert.deepEqual(counts, [{ path: 'other', count: 1 }]);
});

test('setScope(id, null) clears the scope', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({ content: 'clearable', scope: 'team/x' });

  const cleared = await store.setScope(item.id, null);
  assert.equal(cleared.scope, undefined);

  const counts = await store.scopes();
  assert.deepEqual(counts, []);
});

test('list({ scope: "a//b" }) rejects with Validation', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.list({ scope: 'a//b' }),
    (err) => {
      assert.equal(err.code, 'Validation');
      return true;
    },
  );
});
