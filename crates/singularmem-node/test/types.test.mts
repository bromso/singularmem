// Compile-time only — verifies the public types match the 5a + 5b spec.
// Run with: npx tsc --noEmit test/types.test.mts

import {
  Store,
  adapters,
  type Item,
  type StoreOptions,
  type ListOptions,
  type SearchOptions,
  type SearchHit,
  type SearchResults,
  type RetrieveOptions,
  type MemoryBlock,
  type RetrievedContext,
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

void [
  _check, _openCheck,
  opts, opts2, listOpts, listOpts2,
  searchOpts, searchOptsEmpty, retrieveOpts, retrieveOptsEmpty,
  _formattedPlain, _formattedClaude, _formattedOpenAi, _formattedGemini,
  _adapterPlainName, _adapterClaudeName, _adapterOpenAiName, _adapterGeminiName,
];
