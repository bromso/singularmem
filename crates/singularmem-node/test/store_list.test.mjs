import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath, seedStore } from './helpers.mjs';

test('Store.list returns all items when no options given', async () => {
  const { path } = freshStorePath();
  seedStore(path, [
    { content: 'a' },
    { content: 'b' },
    { content: 'c' },
  ]);
  const store = await Store.open(path);
  const items = await store.list();
  assert.equal(items.length, 3);
});

test('Store.list filters by tags (AND-semantics)', async () => {
  const { path } = freshStorePath();
  seedStore(path, [
    { content: 'one', tags: ['x'] },
    { content: 'two', tags: ['x', 'y'] },
    { content: 'three', tags: ['y'] },
  ]);
  const store = await Store.open(path);
  const items = await store.list({ tags: ['x', 'y'] });
  assert.equal(items.length, 1);
  assert.equal(items[0].content, 'two');
});

test('Store.list respects limit', async () => {
  const { path } = freshStorePath();
  seedStore(path, [
    { content: 'a' },
    { content: 'b' },
    { content: 'c' },
  ]);
  const store = await Store.open(path);
  const items = await store.list({ limit: 2 });
  assert.equal(items.length, 2);
});

test('Store.list returns items ordered oldest to newest', async () => {
  const { path } = freshStorePath();
  seedStore(path, [
    { content: 'first' },
    { content: 'second' },
    { content: 'third' },
  ]);
  const store = await Store.open(path);
  const items = await store.list();
  assert.equal(items[0].content, 'first');
  assert.equal(items[2].content, 'third');
});

test('Store.list items have createdAt as Date instances', async () => {
  const { path } = freshStorePath();
  seedStore(path, [{ content: 'a' }]);
  const store = await Store.open(path);
  const items = await store.list();
  assert.ok(items[0].createdAt instanceof Date, 'createdAt should be a Date');
});
