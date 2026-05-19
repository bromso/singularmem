#!/usr/bin/env node
/**
 * Post-build patch for index.js.
 *
 * napi-rs generates `index.js` and re-exports the raw native binding.
 * This script replaces the final export block with a thin JS wrapper that:
 *
 * 1. Promotes `createdAt` from a millisecond number to a JS `Date` on items
 *    returned by `Store.get`, `Store.list`, `Store.revisions`, `Store.ingest`,
 *    and hit items returned by `Store.search`.
 * 2. Wraps `Store.open` to return instances of the JS `Store` class rather
 *    than the raw native object, so the JS wrapper methods are available.
 * 3. Forwards `Store.formatVersion` and `Store.export` directly to the native
 *    binding (no item lifting required).
 * 4. Implements `Store.search` which lifts `createdAt` on each hit's item.
 * 5. Implements `Store.retrieve` which lifts `createdAt` on each block
 *    (flat shape — `b.createdAt`, not `b.item.createdAt`).
 * 6. Implements `Store.ingest` which lifts `createdAt` on the returned Item.
 *
 * Must be run after `napi build` (see the `postbuild` npm lifecycle hook).
 */

const fs = require('fs')
const path = require('path')

const indexPath = path.join(__dirname, '..', 'index.js')
let src = fs.readFileSync(indexPath, 'utf8')

const MARKER = 'const { PlainAdapter, ClaudeAdapter, OpenAiAdapter, GeminiAdapter, Store, version } = nativeBinding\n\nmodule.exports.PlainAdapter = PlainAdapter\nmodule.exports.ClaudeAdapter = ClaudeAdapter\nmodule.exports.OpenAiAdapter = OpenAiAdapter\nmodule.exports.GeminiAdapter = GeminiAdapter\nmodule.exports.Store = Store\nmodule.exports.version = version'
const REPLACEMENT = `const { Store: _NativeStore, version, PlainAdapter, ClaudeAdapter, OpenAiAdapter, GeminiAdapter } = nativeBinding

/**
 * Convert an Item from the native binding into a JS-friendly shape:
 * \`createdAt\` is promoted from a number (ms since epoch) to a \`Date\`.
 */
function liftItem(raw) {
  return Object.assign(Object.create(null), raw, { createdAt: new Date(raw.createdAt) })
}

/** Public Store class — thin wrapper that promotes \`createdAt\` to \`Date\`. */
class Store {
  /** @private */
  constructor(native) {
    this._native = native
  }

  static open(path, options) {
    return _NativeStore.open(path, options).then((native) => new Store(native))
  }

  get(id) {
    return this._native.get(id).then(liftItem)
  }

  list(options) {
    return this._native.list(options).then((items) => items.map(liftItem))
  }

  revisions(id) {
    return this._native.revisions(id).then((items) => items.map(liftItem))
  }

  search(query, options) {
    return this._native.search(query, options).then((res) => ({
      query: res.query,
      hits: res.hits.map((h) => ({ ...h, item: liftItem(h.item) })),
    }))
  }

  retrieve(query, options) {
    return this._native.retrieve(query, options).then((ctx) => ({
      query: ctx.query,
      blocks: ctx.blocks.map((b) => ({ ...b, createdAt: new Date(b.createdAt) })),
    }))
  }

  ingest(item) {
    return this._native.ingest(item).then(liftItem)
  }

  formatVersion() {
    return this._native.formatVersion()
  }

  export() {
    return this._native.export()
  }
}

// Construct the frozen \`adapters\` namespace from the four native classes.
const adapters = Object.freeze({
  plain:  Object.freeze(new PlainAdapter()),
  claude: Object.freeze(new ClaudeAdapter()),
  openai: Object.freeze(new OpenAiAdapter()),
  gemini: Object.freeze(new GeminiAdapter()),
})
module.exports.adapters = adapters
module.exports.Store = Store
module.exports.version = version`

if (!src.includes(MARKER)) {
  console.error('patch-index.js: could not find marker in index.js — patch skipped')
  process.exit(1)
}

src = src.replace(MARKER, REPLACEMENT)
fs.writeFileSync(indexPath, src, 'utf8')
console.log('patch-index.js: index.js patched successfully')

// ── Patch index.d.ts ─────────────────────────────────────────────────────────
//
// napi-rs does not generate a declaration for the `adapters` namespace that
// patch-index.js wires up in index.js. Append it once if it's missing.

const dtsPath = path.join(__dirname, '..', 'index.d.ts')
let dts = fs.readFileSync(dtsPath, 'utf8')

const ADAPTERS_DECL = `
/**
 * The four pre-built prompt adapters, keyed by provider name.
 *
 * Each adapter exposes:
 * - \`name\` — stable lowercase identifier (e.g. \`"claude"\`)
 * - \`format(ctx)\` — synchronous; converts a \`RetrievedContext\` into a
 *   provider-specific prompt string.
 *
 * Supported adapters:
 * - \`adapters.plain\`  — Markdown \`## memory N\` headings
 * - \`adapters.claude\` — Anthropic \`<documents><document index="N">\` XML
 * - \`adapters.openai\` — Bracketed \`[N]\` citations with a leading instruction
 * - \`adapters.gemini\` — Em-dash \`Source N\` headers with grounding directive
 */
export declare const adapters: {
  readonly plain: PlainAdapter
  readonly claude: ClaudeAdapter
  readonly openai: OpenAiAdapter
  readonly gemini: GeminiAdapter
}
`

if (!dts.includes('export declare const adapters')) {
  dts = dts.trimEnd() + '\n' + ADAPTERS_DECL
  fs.writeFileSync(dtsPath, dts, 'utf8')
  console.log('patch-index.js: index.d.ts patched with adapters declaration')
} else {
  console.log('patch-index.js: index.d.ts already has adapters declaration — skipped')
}
