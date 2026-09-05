// Knowledge-graph surface on `Store` (sub-project 16, Task 4).
// Spec: docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md § "Node binding".
process.env.SINGULARMEM_TEST_EMBEDDER = 'mock';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Store } from '../index.js';
import { freshStorePath } from './helpers.mjs';

async function open() {
  const { path } = freshStorePath();
  return Store.open(path);
}

test('addFact returns the fact and queryEntity finds it', async () => {
  const store = await open();
  const fact = await store.addFact({
    subject: 'Singularmem',
    predicate: 'uses',
    object: 'Tantivy',
    validFrom: '2026-05-16',
  });
  assert.equal(fact.subject.name, 'Singularmem');
  assert.equal(fact.predicate, 'uses');
  assert.equal(fact.object.entity && fact.object.entity.name, 'Tantivy');
  // napi omits `None` fields from the object entirely, so absent optionals
  // read as `undefined` — the same convention as `Item.supersedes`.
  assert.equal(fact.object.value, undefined);
  assert.equal(fact.validFrom, '2026-05-16T00:00:00Z');
  assert.equal(fact.validTo, undefined);
  assert.equal(fact.confidence, 1);
  const facts = await store.queryEntity('Singularmem');
  assert.equal(facts.length, 1);
  assert.equal(facts[0].id, fact.id);
});

test('supersedeFact closes the old fact; asOf sees the right one', async () => {
  const store = await open();
  await store.addFact({
    subject: 'Singularmem',
    predicate: 'uses',
    object: 'Tantivy',
    validFrom: '2026-05-16',
  });
  const r = await store.supersedeFact('Singularmem', 'uses', 'Tantivy', 'Meilisearch', {
    at: '2026-09-01',
  });
  assert.ok(r.closed);
  assert.equal(r.closed.validTo, '2026-09-01T00:00:00Z');
  assert.equal(r.opened.object.entity.name, 'Meilisearch');
  const before = await store.queryEntity('Singularmem', { asOf: '2026-06-01' });
  assert.deepEqual(
    before.map((f) => f.object.entity.name),
    ['Tantivy'],
  );
  const after = await store.queryEntity('Singularmem', { asOf: '2026-10-01' });
  assert.deepEqual(
    after.map((f) => f.object.entity.name),
    ['Meilisearch'],
  );
  const history = await store.factHistory(r.closed.id);
  assert.equal(history.length, 2);
});

test('invalidateFact, timeline order, stats, entities, value objects', async () => {
  const store = await open();
  await store.addFact({
    subject: 'Singularmem',
    predicate: 'owned_by',
    object: 'Jonas',
    objectIsValue: true,
  });
  await store.addFact({
    subject: 'Singularmem',
    predicate: 'uses',
    object: 'Tantivy',
    validFrom: '2026-05-16',
  });
  const closed = await store.invalidateFact('Singularmem', 'uses', 'Tantivy', {
    at: '2026-09-01',
  });
  assert.equal(closed.validTo, '2026-09-01T00:00:00Z');
  assert.equal((await store.queryPredicate('uses')).length, 0);
  // Core order: `valid_from IS NOT NULL, valid_from, recorded_at, id` — the
  // NULL-`validFrom` `owned_by` head (open) sorts before the closed `uses` head.
  const tl = await store.timeline('Singularmem');
  assert.deepEqual(
    tl.map((e) => e.current),
    [true, false],
  );
  assert.equal(tl[0].fact.predicate, 'owned_by');
  const stats = await store.graphStats();
  assert.deepEqual({ ...stats }, { entities: 2, openFacts: 1, closedFacts: 1, predicates: 2 });
  const ents = await store.entities();
  assert.deepEqual(
    ents.map((e) => e.name),
    ['Singularmem', 'Tantivy'],
  );
  // Head revisions only: the open `owned_by` fact and the closing `uses` one.
  assert.equal(ents[0].factCount, 2);
  const owned = await store.queryPredicate('owned_by');
  assert.equal(owned[0].object.value, 'Jonas');
  assert.equal(owned[0].object.entity, undefined);
});

test('direction, scope and kind filters narrow the results', async () => {
  const store = await open();
  await store.addFact({
    subject: 'Singularmem',
    predicate: 'uses',
    object: 'Tantivy',
    subjectKind: 'project',
    objectKind: 'library',
    scope: 'team/backend',
  });
  assert.equal((await store.queryEntity('Tantivy', { direction: 'incoming' })).length, 1);
  assert.equal((await store.queryEntity('Tantivy', { direction: 'outgoing' })).length, 0);
  assert.equal((await store.queryEntity('Singularmem', { scope: 'team' })).length, 1);
  assert.equal(
    (await store.queryEntity('Singularmem', { scope: 'team', scopeExact: true })).length,
    0,
  );
  const libs = await store.entities({ kind: 'library' });
  assert.deepEqual(
    libs.map((e) => e.name),
    ['Tantivy'],
  );
  assert.equal(libs[0].kind, 'library');
  const scoped = await store.graphStats({ scope: 'team/backend' });
  assert.equal(scoped.openFacts, 1);
});

test('coded errors: FactNotFound and bad timestamps', async () => {
  const store = await open();
  await assert.rejects(
    store.invalidateFact('Nobody', 'uses', 'Nothing'),
    (e) => e.code === 'FactNotFound',
  );
  await assert.rejects(
    store.addFact({ subject: 'a', predicate: 'p', object: 'b', validFrom: 'not-a-date' }),
    (e) => e.code === 'Validation' && /validFrom/.test(e.message),
  );
  await assert.rejects(
    store.queryEntity('a', { direction: 'sideways' }),
    (e) => e.code === 'Validation' && /direction/.test(e.message),
  );
  await assert.rejects(
    store.queryEntity('a', { asOf: 'nope' }),
    (e) => e.code === 'Validation' && /asOf/.test(e.message),
  );
  await assert.rejects(store.factHistory('not-a-ulid'), (e) => e.code === 'InvalidId');
  await assert.rejects(
    store.factHistory('01HXAAAAAAAAAAAAAAAAAAAAA0'),
    (e) => e.code === 'FactIdNotFound',
  );
});

test('read-only stores reject graph writes with ReadOnly', async () => {
  const { path } = freshStorePath();
  const rw = await Store.open(path);
  await rw.addFact({ subject: 'Singularmem', predicate: 'uses', object: 'Tantivy' });
  const ro = await Store.open(path, { readOnly: true });
  assert.equal((await ro.queryEntity('Singularmem')).length, 1);
  await assert.rejects(
    ro.addFact({ subject: 'a', predicate: 'p', object: 'b' }),
    (e) => e.code === 'ReadOnly',
  );
});
