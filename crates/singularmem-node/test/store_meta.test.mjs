import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath, seedStore } from './helpers.mjs';

test('Store.formatVersion returns a non-empty version string', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const version = await store.formatVersion();
  // The current on-disk format version is the plain integer "1".
  // Allow for either a plain integer or a semver-shaped string.
  assert.ok(
    typeof version === 'string' && version.length > 0,
    `expected a non-empty string, got ${JSON.stringify(version)}`,
  );
  assert.match(version, /^\d+(\.\d+)*$/, `unexpected version format: ${version}`);
});

test('Store.export returns JSONL with items present', async () => {
  const { path } = freshStorePath();
  seedStore(path, [{ content: 'one' }, { content: 'two' }]);
  const store = await Store.open(path);
  const dump = await store.export();
  const lines = dump.trim().split('\n');
  // Expect at least one meta line + 2 items = 3 lines minimum
  assert.ok(lines.length >= 3, `expected meta + 2 items, got ${lines.length} lines`);

  // First line should be the meta header JSON object.
  const meta = JSON.parse(lines[0]);
  assert.ok(typeof meta === 'object' && meta !== null, 'first line should be a JSON object');
  assert.equal(meta._kind, 'meta', 'meta header should have _kind === "meta"');
  assert.ok(typeof meta._singularmem_format === 'string', 'meta header should have _singularmem_format');

  // Remaining lines should be items.
  const items = lines.slice(1).map((l) => JSON.parse(l));
  assert.equal(items.length, 2);
  assert.ok(items.some((i) => i.content === 'one'));
  assert.ok(items.some((i) => i.content === 'two'));
});

test('Store.export on empty store returns just the meta header', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const dump = await store.export();
  const lines = dump.trim().split('\n');
  assert.equal(lines.length, 1, 'empty store should produce only the meta header');
  const meta = JSON.parse(lines[0]);
  assert.ok(typeof meta === 'object' && meta !== null);
  assert.equal(meta._kind, 'meta');
});
