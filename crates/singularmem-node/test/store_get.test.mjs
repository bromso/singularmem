import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath, seedStore } from './helpers.mjs';

test('Store.get returns the item for a known ID', async () => {
  const { path } = freshStorePath();
  seedStore(path, [{ content: 'hello world', tags: ['greet'] }]);
  const store = await Store.open(path);

  const items = await store.list();
  assert.equal(items.length, 1);
  const id = items[0].id;

  const fetched = await store.get(id);
  assert.equal(fetched.content, 'hello world');
  assert.deepEqual(fetched.tags, ['greet']);
  assert.ok(fetched.createdAt instanceof Date, 'createdAt should be a Date');
});

test('Store.get throws NotFound for a missing ULID', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.get('01HZZZZZZZZZZZZZZZZZZZZZZZ'),
    (err) => {
      assert.equal(err.code, 'NotFound');
      return true;
    },
  );
});

test('Store.get throws InvalidId for malformed input', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.get('not-a-ulid'),
    (err) => {
      assert.equal(err.code, 'InvalidId');
      return true;
    },
  );
});
