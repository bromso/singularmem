process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store, adapters } from '../index.js';
import { freshStorePath, seedStoreWithIndexes } from './helpers.mjs';

async function seededCtx() {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [
    { content: 'cats are great pets' },
    { content: 'dogs need walks' },
  ]);
  const store = await Store.open(path);
  // Exact-match query so MockEmbedder produces a hit.
  const ctx = await store.retrieve('cats are great pets');
  // store.retrieve() lifts createdAt to a JS Date; adapter.format() expects
  // a raw millisecond number (f64). Convert back before passing to the napi
  // binding.
  return {
    query: ctx.query,
    blocks: ctx.blocks.map((b) => ({
      ...b,
      createdAt: b.createdAt instanceof Date ? b.createdAt.getTime() : b.createdAt,
    })),
  };
}

test('adapters.plain.format produces a non-empty Markdown string', async () => {
  const ctx = await seededCtx();
  const s = adapters.plain.format(ctx);
  assert.ok(typeof s === 'string');
  assert.ok(s.length > 0);
});

test('adapters.claude.format contains <documents> wrapper', async () => {
  const ctx = await seededCtx();
  const s = adapters.claude.format(ctx);
  assert.ok(s.includes('<documents>'));
  assert.ok(s.includes('<document index='));
});

test('adapters.openai.format contains [1] bracket citation', async () => {
  const ctx = await seededCtx();
  const s = adapters.openai.format(ctx);
  assert.ok(s.includes('[1]'));
});

test('adapters.gemini.format contains "Source 1" header', async () => {
  const ctx = await seededCtx();
  const s = adapters.gemini.format(ctx);
  assert.ok(s.includes('Source 1'));
});

test('adapters.X.name returns the expected string for all four', () => {
  assert.equal(adapters.plain.name, 'plain');
  assert.equal(adapters.claude.name, 'claude');
  assert.equal(adapters.openai.name, 'openai');
  assert.equal(adapters.gemini.name, 'gemini');
});

test('adapters with empty context produce non-error output', () => {
  const empty = { query: 'no matches', blocks: [] };
  assert.ok(adapters.plain.format(empty).length > 0);
  assert.ok(adapters.claude.format(empty).length > 0);
  assert.ok(adapters.openai.format(empty).length > 0);
  assert.ok(adapters.gemini.format(empty).length > 0);
});
