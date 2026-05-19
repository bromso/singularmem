// MockEmbedder env var must be set before the napi binding runs.
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath, seedStoreWithIndexes } from './helpers.mjs';

test('Store.retrieve returns RetrievedContext with query and blocks', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [
    { content: 'cats are great pets' },
    { content: 'dogs need walks' },
  ]);
  const store = await Store.open(path);
  // Exact match needed for MockEmbedder's hash-based vectors
  const ctx = await store.retrieve('cats are great pets');
  assert.equal(ctx.query, 'cats are great pets');
  assert.ok(ctx.blocks.length >= 1);
});

test('Store.retrieve blocks have createdAt as Date', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'date test' }]);
  const store = await Store.open(path);
  const ctx = await store.retrieve('date test');
  assert.ok(ctx.blocks[0].createdAt instanceof Date);
});

test('Store.retrieve minScore filter drops low-scoring blocks', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [
    { content: 'minScore filter test' },
  ]);
  const store = await Store.open(path);
  const ctxAll = await store.retrieve('minScore filter test');
  const ctxFiltered = await store.retrieve('minScore filter test', { minScore: 999 });
  assert.ok(ctxAll.blocks.length >= 1);
  assert.equal(ctxFiltered.blocks.length, 0, 'high minScore should filter out everything');
});

test('Store.retrieve with empty query throws EmptyQuery', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'a' }]);
  const store = await Store.open(path);
  await assert.rejects(
    () => store.retrieve(''),
    (err) => {
      assert.equal(err.code, 'EmptyQuery');
      return true;
    },
  );
});

test('Store.retrieve with mode "lexical" works', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'lexical retrieve' }]);
  const store = await Store.open(path);
  const ctx = await store.retrieve('lexical', { mode: 'lexical' });
  assert.equal(ctx.blocks[0].kind, 'bm25');
});
