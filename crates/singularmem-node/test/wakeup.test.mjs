// `Store.wakeup` (sub-project 16, Task 5).
// Spec: docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md § "Node binding".
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

// `mkdtempSync`'s random suffix can include uppercase letters, but scope
// paths are lowercased on write (`singularmem_core::scope::validate`), so
// a fixed lowercase basename avoids exercising that unrelated behaviour.
function freshProject() {
  const root = mkdtempSync(join(tmpdir(), 'sm-node-wakeup-'));
  const project = join(root, 'proj-a');
  mkdirSync(project);
  return project;
}

test('wakeup returns the project scopes header and recent items', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const store = await Store.open(path);
  await store.ingest({ content: 'first decision', scope: 'claude-code/proj-a' });
  await store.ingest({ content: 'second decision', scope: 'claude-code/proj-a' });
  await store.ingest({ content: 'elsewhere', scope: 'claude-code/other' });
  const w = await store.wakeup({ project });
  assert.equal(w.total, 2);
  assert.equal(w.shown, 2);
  assert.deepEqual(w.scopes, ['claude-code/proj-a', 'codex/proj-a', 'cursor/proj-a']);
  assert.ok(w.text.startsWith('# Singularmem wake-up — claude-code/proj-a'), w.text);
  assert.ok(w.text.includes('second decision'));
  assert.ok(!w.text.includes('elsewhere'));
});

test('includeFiles adds the files/<basename> scope', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const store = await Store.open(path);
  await store.ingest({ content: 'readme chunk', scope: 'files/proj-a' });
  const withoutFiles = await store.wakeup({ project });
  assert.ok(!withoutFiles.scopes.includes('files/proj-a'));
  assert.ok(!withoutFiles.text.includes('readme chunk'));
  const withFiles = await store.wakeup({ project, includeFiles: true });
  assert.ok(withFiles.scopes.includes('files/proj-a'));
  assert.ok(withFiles.text.includes('readme chunk'));
});

test('adapter selects the requested formatter', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const store = await Store.open(path);
  await store.ingest({ content: 'decision', scope: 'claude-code/proj-a' });
  const w = await store.wakeup({ project, adapter: 'claude' });
  assert.ok(w.text.includes('<documents>'), w.text);
});

test('maxBytes drops oldest blocks and keeps the header', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const store = await Store.open(path);
  await store.ingest({ content: 'decision', scope: 'claude-code/proj-a' });
  const w = await store.wakeup({ project, maxBytes: 256 });
  assert.ok(w.text.startsWith('# Singularmem wake-up'));
  assert.ok(w.text.length <= 256, w.text.length);
});

test('shown is the post-limit count, not the post-maxBytes count', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const store = await Store.open(path);
  for (let i = 0; i < 10; i += 1) {
    await store.ingest({ content: `decision number ${i}`, scope: 'claude-code/proj-a' });
  }
  const w = await store.wakeup({ project, maxBytes: 600 });
  // `shown` reports items considered after `limit`, unaffected by the
  // `maxBytes` budget; the header inside `text` reports how many blocks
  // actually survived that budget, which can be fewer.
  assert.equal(w.shown, 10);
  const match = w.text.match(/showing last (\d+)/);
  assert.ok(match, w.text);
  const survived = Number(match[1]);
  assert.ok(survived < 10, `expected maxBytes to drop some blocks: ${survived}`);
});

test('project defaults to process.cwd()', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  const w = await store.wakeup();
  const base = process.cwd().split('/').pop().toLowerCase();
  assert.deepEqual(w.scopes, [
    `claude-code/${base}`,
    `codex/${base}`,
    `cursor/${base}`,
  ]);
});

test('wakeup rejects an unknown adapter and a missing project', async () => {
  const { path } = freshStorePath();
  const store = await Store.open(path);
  await assert.rejects(store.wakeup({ project: process.cwd(), adapter: 'gpt' }), (e) => e.code === 'Validation');
  await assert.rejects(store.wakeup({ project: '/definitely/not/here' }), (e) => e.code === 'Validation' && /project/.test(e.message));
});

test('a read-only store still serves wakeup', async () => {
  const { path } = freshStorePath();
  const project = freshProject();
  const rw = await Store.open(path);
  await rw.ingest({ content: 'decision', scope: 'claude-code/proj-a' });
  const ro = await Store.open(path, { readOnly: true });
  const w = await ro.wakeup({ project });
  assert.equal(w.shown, 1);
});
