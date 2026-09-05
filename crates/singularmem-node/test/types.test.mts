// Compile-time only — verifies the public types match the 5a + 5b spec.
// Run with: npx tsc --noEmit test/types.test.mts

import {
  Store,
  adapters,
  type Item,
  type NewItem,
  type StoreOptions,
  type ListOptions,
  type SearchOptions,
  type SearchHit,
  type SearchResults,
  type RetrieveOptions,
  type MemoryBlock,
  type RetrievedContext,
  type EntityRef,
  type Fact,
  type FactObject,
  type NewFact,
  type GraphQueryOptions,
  type FactChangeOptions,
  type GraphScopeOptions,
  type EntityListOptions,
  type TimelineEntry,
  type GraphStats,
  type EntitySummary,
  type SupersedeResult,
} from '../index.js';

// 5a — StoreOptions, ListOptions, Item, Store methods (unchanged from 5a)
const opts: StoreOptions = { readOnly: true };
const opts2: StoreOptions = {};
const listOpts: ListOptions = { tags: ['a'], limit: 10 };
const listOpts2: ListOptions = {};

declare const item: Item;
const _id: string = item.id;
const _content: string = item.content;
const _created: Date = item.createdAt;
const _supers: string | undefined = item.supersedes;
const _tags: string[] = item.tags;
const _source: string | undefined = item.source;
// metadata is `any` per the 5a accepted deviation; cast for type-safety where needed.
const _meta: Record<string, unknown> = item.metadata as Record<string, unknown>;

// 5b — SearchOptions / SearchHit / SearchResults
const searchOpts: SearchOptions = { mode: 'hybrid', limit: 5, fetchMultiplier: 3, rrfK: 60 };
const searchOptsEmpty: SearchOptions = {};

declare const hit: SearchHit;
const _hitItem: Item = hit.item;
const _hitScore: number = hit.score;
// `kind` is `string` in the generated .d.ts (napi maps Rust `String` to `string`).
// The spec describes it as the union 'rrf' | 'bm25' | 'cosine'; consumers narrow on read.
const _hitKind: string = hit.kind;
const _hitLexicalRank: number | undefined = hit.lexicalRank;
const _hitSemanticRank: number | undefined = hit.semanticRank;

declare const sr: SearchResults;
const _srQuery: string = sr.query;
const _srHits: SearchHit[] = sr.hits;

// 5b — RetrieveOptions / MemoryBlock / RetrievedContext
const retrieveOpts: RetrieveOptions = { mode: 'auto', minScore: 0.5 };
const retrieveOptsEmpty: RetrieveOptions = {};

declare const block: MemoryBlock;
const _blockId: string = block.id;
const _blockContent: string = block.content;
const _blockScore: number = block.score;
const _blockKind: string = block.kind;
const _blockSource: string | undefined = block.source;
const _blockTags: string[] = block.tags;
const _blockCreated: Date = block.createdAt;

declare const ctx: RetrievedContext;
const _ctxQuery: string = ctx.query;
const _ctxBlocks: MemoryBlock[] = ctx.blocks;

// 5b — adapters namespace
const _adapterPlainName: string = adapters.plain.name;
const _adapterClaudeName: string = adapters.claude.name;
const _adapterOpenAiName: string = adapters.openai.name;
const _adapterGeminiName: string = adapters.gemini.name;
const _formattedPlain: string = adapters.plain.format(ctx);
const _formattedClaude: string = adapters.claude.format(ctx);
const _formattedOpenAi: string = adapters.openai.format(ctx);
const _formattedGemini: string = adapters.gemini.format(ctx);

// Store methods — both 5a and 5b
async function _check(s: Store): Promise<void> {
  // 5a
  const _x: Item = await s.get('01H...');
  const _xs: Item[] = await s.list();
  const _xs2: Item[] = await s.list({ tags: ['x'] });
  const _xs3: Item[] = await s.list({ limit: 5 });
  const _rev: Item[] = await s.revisions('01H...');
  const _v: string = await s.formatVersion();
  const _dump: string = await s.export();
  // 5b
  const _sr: SearchResults = await s.search('q');
  const _sr2: SearchResults = await s.search('q', { mode: 'hybrid' });
  const _rc: RetrievedContext = await s.retrieve('q');
  const _rc2: RetrievedContext = await s.retrieve('q', { minScore: 0.5 });
}

async function _openCheck(): Promise<Store> {
  return Store.open('/tmp/foo.db', { readOnly: false });
}

// 5c — NewItem
const minimal: NewItem = { content: 'hello' };
const full: NewItem = {
  content: 'all fields',
  supersedes: '01H...',
  tags: ['x', 'y'],
  source: 'test',
  metadata: { k: 'v' },
};

async function _ingestCheck(s: Store): Promise<void> {
  const _x: Item = await s.ingest({ content: 'hi' });
  const _y: Item = await s.ingest(full);
}


// 16 — knowledge graph (sub-project 16, Task 4).
// napi renders `Option<T>` as an optional property, so absent values are
// `undefined` rather than `null` (same convention as `Item.supersedes`).
declare const fact: Fact;
const _factId: string = fact.id;
const _factSubject: EntityRef = fact.subject;
const _factSubjectId: string = fact.subject.id;
const _factPredicate: string = fact.predicate;
const _factObject: FactObject = fact.object;
const _factObjectEntity: EntityRef | undefined = fact.object.entity;
const _factObjectValue: string | undefined = fact.object.value;
const _factValidFrom: string | undefined = fact.validFrom;
const _factValidTo: string | undefined = fact.validTo;
const _factConfidence: number = fact.confidence;
const _factSourceItemId: string | undefined = fact.sourceItemId;
const _factScope: string | undefined = fact.scope;
const _factSupersedes: string | undefined = fact.supersedes;
const _factRecordedAt: string = fact.recordedAt;

const newFactMinimal: NewFact = { subject: 'A', predicate: 'uses', object: 'B' };
const newFactFull: NewFact = {
  subject: 'A',
  predicate: 'uses',
  object: 'literal',
  objectIsValue: true,
  subjectKind: 'project',
  objectKind: 'library',
  validFrom: '2026-05-16',
  validTo: '2026-09-01T00:00:00Z',
  confidence: 0.5,
  sourceItemId: '01H...',
  scope: 'team/backend',
};

const graphQueryOpts: GraphQueryOptions = {
  direction: 'incoming',
  asOf: '2026-06-01',
  recordedAt: '2026-06-01',
  scope: 'team',
  scopeExact: true,
};
const graphQueryOptsEmpty: GraphQueryOptions = {};
const factChangeOpts: FactChangeOptions = { objectIsValue: true, at: '2026-09-01', scope: 'team' };
const factChangeOptsEmpty: FactChangeOptions = {};
const graphScopeOpts: GraphScopeOptions = { scope: 'team', scopeExact: false };
const entityListOpts: EntityListOptions = { kind: 'library', scope: 'team', scopeExact: false };

declare const entry: TimelineEntry;
const _entryFact: Fact = entry.fact;
const _entryCurrent: boolean = entry.current;

declare const stats: GraphStats;
const _statsEntities: number = stats.entities;
const _statsOpen: number = stats.openFacts;
const _statsClosed: number = stats.closedFacts;
const _statsPredicates: number = stats.predicates;

declare const summary: EntitySummary;
const _summaryId: string = summary.id;
const _summaryName: string = summary.name;
const _summaryKind: string | undefined = summary.kind;
const _summaryFactCount: number = summary.factCount;

declare const superseded: SupersedeResult;
const _supersededClosed: Fact | undefined = superseded.closed;
const _supersededOpened: Fact = superseded.opened;

async function _graphCheck(s: Store): Promise<void> {
  const _added: Fact = await s.addFact(newFactMinimal);
  const _added2: Fact = await s.addFact(newFactFull);
  const _byEntity: Fact[] = await s.queryEntity('A');
  const _byEntity2: Fact[] = await s.queryEntity('A', graphQueryOpts);
  const _byPredicate: Fact[] = await s.queryPredicate('uses');
  const _byPredicate2: Fact[] = await s.queryPredicate('uses', graphQueryOptsEmpty);
  const _closed: Fact = await s.invalidateFact('A', 'uses', 'B');
  const _closed2: Fact = await s.invalidateFact('A', 'uses', 'B', factChangeOpts);
  const _sup: SupersedeResult = await s.supersedeFact('A', 'uses', 'B', 'C');
  const _sup2: SupersedeResult = await s.supersedeFact('A', 'uses', 'B', 'C', factChangeOptsEmpty);
  const _tl: TimelineEntry[] = await s.timeline();
  const _tl2: TimelineEntry[] = await s.timeline('A', graphScopeOpts);
  const _st: GraphStats = await s.graphStats();
  const _st2: GraphStats = await s.graphStats(graphScopeOpts);
  const _ents: EntitySummary[] = await s.entities();
  const _ents2: EntitySummary[] = await s.entities(entityListOpts);
  const _hist: Fact[] = await s.factHistory('01H...');
}

void [
  _check, _openCheck, _ingestCheck, _graphCheck,
  opts, opts2, listOpts, listOpts2,
  searchOpts, searchOptsEmpty, retrieveOpts, retrieveOptsEmpty,
  _formattedPlain, _formattedClaude, _formattedOpenAi, _formattedGemini,
  _adapterPlainName, _adapterClaudeName, _adapterOpenAiName, _adapterGeminiName,
  minimal, full,
  newFactMinimal, newFactFull, graphQueryOpts, graphQueryOptsEmpty,
  factChangeOpts, factChangeOptsEmpty, graphScopeOpts, entityListOpts,
];
