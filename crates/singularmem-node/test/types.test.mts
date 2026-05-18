// Compile-time only — verifies the public types match the 5a spec.
// Run with: npx tsc --noEmit test/types.test.mts

import { Store, type Item, type StoreOptions, type ListOptions } from '../index.js';

// StoreOptions shape
const opts: StoreOptions = { readOnly: true };
const opts2: StoreOptions = {};

// ListOptions shape
const listOpts: ListOptions = { tags: ['a'], limit: 10 };
const listOpts2: ListOptions = {};

// Item shape — every field must be present with the documented type.
declare const item: Item;
const _id: string = item.id;
const _content: string = item.content;
const _created: Date = item.createdAt;
const _supers: string | undefined = item.supersedes;
const _tags: string[] = item.tags;
const _source: string | undefined = item.source;
const _meta: Record<string, unknown> = item.metadata as Record<string, unknown>;

// Store shape
async function _check(s: Store): Promise<void> {
  const _x: Item = await s.get('01H...');
  const _xs: Item[] = await s.list();
  const _xs2: Item[] = await s.list({ tags: ['x'] });
  const _xs3: Item[] = await s.list({ limit: 5 });
  const _rev: Item[] = await s.revisions('01H...');
  const _v: string = await s.formatVersion();
  const _dump: string = await s.export();
}

// Static factory shape
async function _openCheck(): Promise<Store> {
  return Store.open('/tmp/foo.db', { readOnly: false });
}

void [_check, _openCheck, opts, opts2, listOpts, listOpts2];
