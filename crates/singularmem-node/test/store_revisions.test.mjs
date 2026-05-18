import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { Store } from '../index.js';
import { freshStorePath, seedStore } from './helpers.mjs';

function ingestSupersedes(path, content, supersedesId) {
  const result = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'singularmem', '--', 'ingest', '--store', path,
     '--content', content, '--supersedes', supersedesId],
    { stdio: 'pipe', encoding: 'utf8' },
  );
  if (result.error) {
    throw new Error(`failed to spawn cargo: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`cargo ingest --supersedes failed (exit ${result.status}):\nstdout: ${result.stdout}\nstderr: ${result.stderr}`);
  }
}

test('Store.revisions returns the full chain in order', async () => {
  const { path } = freshStorePath();
  seedStore(path, [{ content: 'v1' }]);
  const store = await Store.open(path);
  const [v1] = await store.list();
  ingestSupersedes(path, 'v2', v1.id);

  // Re-open to see the new item (store is cached in memory).
  const store2 = await Store.open(path);
  const list2 = await store2.list();
  const v2 = list2.find((i) => i.content === 'v2');
  assert.ok(v2, 'v2 should be present after supersedes ingest');

  const chain = await store2.revisions(v2.id);
  assert.equal(chain.length, 2);
  assert.equal(chain[0].content, 'v1');
  assert.equal(chain[1].content, 'v2');
});

test('Store.revisions throws NotFound for unknown ID', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.revisions('01HZZZZZZZZZZZZZZZZZZZZZZZ'),
    (err) => {
      assert.equal(err.code, 'NotFound');
      return true;
    },
  );
});

test('Store.revisions throws InvalidId for malformed input', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(
    () => store.revisions('not-a-ulid'),
    (err) => {
      assert.equal(err.code, 'InvalidId');
      return true;
    },
  );
});
