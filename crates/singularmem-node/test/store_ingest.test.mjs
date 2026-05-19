// Tests that don't require pre-primed sidecars — pure SQLite write paths.
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

test('Store.ingest with minimal NewItem returns the persisted Item', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({ content: 'hello world' });
  assert.equal(item.content, 'hello world');
  assert.match(item.id, /^[0-9A-HJKMNP-TV-Z]{26}$/, 'id should be a ULID');
  assert.ok(item.createdAt instanceof Date, 'createdAt should be a Date');
  assert.deepEqual(item.tags, []);
});

test('Store.ingest with all fields populated round-trips correctly', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const item = await store.ingest({
    content: 'detailed',
    tags: ['a', 'b'],
    source: 'test-source',
    metadata: { key: 'value' },
  });
  assert.deepEqual(item.tags, ['a', 'b']);
  assert.equal(item.source, 'test-source');
  assert.deepEqual(item.metadata, { key: 'value' });
});

test('Store.ingest with empty content rejects with Validation', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.ingest({ content: '' }),
    (err) => {
      assert.equal(err.code, 'Validation');
      return true;
    },
  );
});

test('Store.ingest with malformed supersedes rejects with InvalidId', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.ingest({ content: 'x', supersedes: 'not-a-ulid' }),
    (err) => {
      assert.equal(err.code, 'InvalidId');
      return true;
    },
  );
});

test('Store.ingest with non-existent supersedes rejects with SupersedesNotFound', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.ingest({
      content: 'x',
      supersedes: '01HZZZZZZZZZZZZZZZZZZZZZZZ',
    }),
    (err) => {
      assert.equal(err.code, 'SupersedesNotFound');
      return true;
    },
  );
});

test('Store.ingest on read-only store rejects with ReadOnly', async () => {
  const { path } = freshStorePath();
  await Store.open(path);  // create the file
  const ro = await Store.open(path, { readOnly: true });
  await assert.rejects(
    () => ro.ingest({ content: 'x' }),
    (err) => {
      assert.equal(err.code, 'ReadOnly');
      return true;
    },
  );
});

test('Store.ingest supersession chain (v1 → v2) is visible via Store.revisions', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const v1 = await store.ingest({ content: 'v1' });
  const v2 = await store.ingest({ content: 'v2', supersedes: v1.id });
  const chain = await store.revisions(v2.id);
  assert.equal(chain.length, 2);
  assert.equal(chain[0].content, 'v1');
  assert.equal(chain[1].content, 'v2');
});
