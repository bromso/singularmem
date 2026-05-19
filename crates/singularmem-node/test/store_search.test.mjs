// Route the napi binding through MockEmbedder so tests don't depend on a
// fastembed model download. Must be set BEFORE `Store` runs the search.
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { Store } from '../index.js';
import { freshStorePath, seedStore, seedStoreWithIndexes } from './helpers.mjs';

test('Store.search returns hits with full Item content', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [
    { content: 'cats are great pets' },
    { content: 'dogs need walks' },
  ]);
  const store = await Store.open(path);
  const results = await store.search('cats');
  assert.ok(results.hits.length >= 1, 'expected at least one hit');
  const hit = results.hits[0];
  assert.ok(hit.item.content.length > 0, 'top hit should have content');
  assert.ok(hit.item.createdAt instanceof Date, 'createdAt should be a Date');
  assert.ok(typeof hit.score === 'number', 'score should be a number');
  assert.ok(['rrf', 'bm25', 'cosine'].includes(hit.kind), 'kind should be a valid ScoreKind');
});

test('Store.search with mode "lexical" works', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'lexical match test' }]);
  const store = await Store.open(path);
  const results = await store.search('lexical', { mode: 'lexical' });
  assert.ok(results.hits.length >= 1, 'expected at least one hit');
  assert.equal(results.hits[0].kind, 'bm25');
  assert.equal(
    results.hits[0].semanticRank,
    undefined,
    'single-ranker hits omit other side',
  );
});

test('Store.search with mode "semantic" works', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'semantic match' }]);
  const store = await Store.open(path);
  // MockEmbedder produces hash-based vectors, so an exact-match query gives a
  // cosine 1.0 hit (the only reliable way to get a deterministic semantic
  // hit without the real fastembed model).
  const results = await store.search('semantic match', { mode: 'semantic' });
  assert.ok(results.hits.length >= 1, 'expected at least one hit');
  assert.equal(results.hits[0].kind, 'cosine');
  assert.equal(results.hits[0].lexicalRank, undefined);
});

test('Store.search with mode "hybrid" returns RRF-fused results with both ranks', async () => {
  const { path } = freshStorePath();
  seedStoreWithIndexes(path, [{ content: 'hybrid match' }]);
  const store = await Store.open(path);
  // Same exact-match trick so MockEmbedder produces a cosine hit alongside
  // BM25's lexical hit, exercising both ranker arms of the RRF fusion.
  const results = await store.search('hybrid match', { mode: 'hybrid' });
  assert.ok(results.hits.length >= 1, 'expected at least one hit');
  assert.equal(results.hits[0].kind, 'rrf');
  assert.ok(
    typeof results.hits[0].lexicalRank === 'number',
    'hybrid mode populates lexical_rank',
  );
  assert.ok(
    typeof results.hits[0].semanticRank === 'number',
    'hybrid mode populates semantic_rank',
  );
});

test('Store.search mode "auto" throws NoIndexes when no sidecars exist', async () => {
  const { path } = freshStorePath();
  // Use --no-index so the CLI does NOT create the .tantivy sidecar on ingest.
  // By default `singularmem ingest` always wires up the Tantivy hook and
  // creates the sidecar directory, so we must opt out explicitly here.
  const r = spawnSync(
    'cargo',
    [
      'run', '-q', '-p', 'singularmem', '--',
      'ingest', '--no-index', '--store', path, '--content', 'no indexes here',
    ],
    {
      stdio: 'pipe',
      encoding: 'utf8',
      env: { ...process.env, SINGULARMEM_TEST_EMBEDDER: 'mock' },
    },
  );
  if (r.error) throw new Error(`failed to spawn cargo: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`ingest failed (exit ${r.status}): ${r.stderr}`);

  const store = await Store.open(path);
  await assert.rejects(
    () => store.search('anything'),
    (err) => {
      assert.equal(err.code, 'NoIndexes');
      return true;
    },
  );
});

test('Store.search mode "hybrid" throws HybridMissingIndex when one sidecar is absent', async () => {
  const { path } = freshStorePath();
  // Pre-prime ONLY tantivy by using reindex WITHOUT --with-embeddings
  const r = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'singularmem', '--', 'reindex', '--store', path],
    {
      stdio: 'pipe',
      encoding: 'utf8',
      env: { ...process.env, SINGULARMEM_TEST_EMBEDDER: 'mock' },
    },
  );
  if (r.status !== 0) throw new Error(`reindex failed: ${r.stderr}`);
  seedStore(path, [{ content: 'tantivy only' }]);

  const store = await Store.open(path);
  await assert.rejects(
    () => store.search('anything', { mode: 'hybrid' }),
    (err) => {
      assert.equal(err.code, 'HybridMissingIndex');
      return true;
    },
  );
});
