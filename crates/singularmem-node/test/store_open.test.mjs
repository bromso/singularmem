import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

test('Store.open returns a Store instance for a fresh path', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  assert.ok(store, 'expected a Store instance');
});

test('Store.open with readOnly: true succeeds on an existing store', async () => {
  const { path } = freshStorePath();
  await Store.open(path);                            // create
  const ro = await Store.open(path, { readOnly: true }); // re-open RO
  assert.ok(ro, 'expected a read-only Store instance');
});

test('Store.open with empty path throws InvalidStorePath', async () => {
  await assert.rejects(
    () => Store.open(''),
    (err) => {
      assert.equal(err.code, 'InvalidStorePath');
      return true;
    },
  );
});
