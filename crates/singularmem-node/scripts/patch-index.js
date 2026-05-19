#!/usr/bin/env node
/**
 * Post-build patch for index.js.
 *
 * napi-rs generates `index.js` and re-exports the raw native binding.
 * This script replaces the final export block with a thin JS wrapper that:
 *
 * 1. Promotes `createdAt` from a millisecond number to a JS `Date` on items
 *    returned by `Store.get`, `Store.list`, `Store.revisions`, and hit items
 *    returned by `Store.search`.
 * 2. Wraps `Store.open` to return instances of the JS `Store` class rather
 *    than the raw native object, so the JS wrapper methods are available.
 * 3. Forwards `Store.formatVersion` and `Store.export` directly to the native
 *    binding (no item lifting required).
 * 4. Implements `Store.search` which lifts `createdAt` on each hit's item.
 *
 * Must be run after `napi build` (see the `postbuild` npm lifecycle hook).
 */

const fs = require('fs')
const path = require('path')

const indexPath = path.join(__dirname, '..', 'index.js')
let src = fs.readFileSync(indexPath, 'utf8')

const MARKER = 'const { Store, version } = nativeBinding\n\nmodule.exports.Store = Store\nmodule.exports.version = version'
const REPLACEMENT = `const { Store: _NativeStore, version } = nativeBinding

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

  formatVersion() {
    return this._native.formatVersion()
  }

  export() {
    return this._native.export()
  }
}

module.exports.Store = Store
module.exports.version = version`

if (!src.includes(MARKER)) {
  console.error('patch-index.js: could not find marker in index.js — patch skipped')
  process.exit(1)
}

src = src.replace(MARKER, REPLACEMENT)
fs.writeFileSync(indexPath, src, 'utf8')
console.log('patch-index.js: index.js patched successfully')
