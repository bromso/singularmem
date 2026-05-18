import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath, seedStore } from './helpers.mjs';

test('Store.get returns the item for a known ID', async () => {
  const { path } = freshStorePath();
  seedStore(path, [{ content: 'hello world', tags: ['greet'] }]);
  const store = await Store.open(path);

  // Use the CLI export verb to discover the seeded item's ID.
  const dumpResult = await import('node:child_process').then((cp) =>
    cp.spawnSync('cargo', ['run', '-q', '-p', 'singularmem', '--', 'export', '--store', path], {
      stdio: 'pipe', encoding: 'utf8',
    }),
  );
  if (dumpResult.status !== 0) {
    throw new Error(`export failed: ${dumpResult.stderr}`);
  }
  const lines = dumpResult.stdout.trim().split('\n');
  // First line is meta header; second is the item.
  const item = JSON.parse(lines[1]);
  const id = item.id;

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
