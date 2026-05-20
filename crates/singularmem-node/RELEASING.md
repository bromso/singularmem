# Releasing `singularmem`

This document covers the release process for the npm package. Day-to-day development never touches this — it's a reference for maintainers cutting a new version.

## One-time setup (already done as of v0.14.0)

1. **npm user `bromso` owns:**
   - The unscoped `singularmem` package name
   - The `@bromso` npm scope (auto-created on first scoped publish)

2. **GitHub repo secrets:**
   - `NPM_TOKEN`: an npm automation token (https://www.npmjs.com/settings/~/tokens). Automation tokens bypass 2FA on publish — required for unattended CI publishes.

3. **GitHub Actions workflow:**
   - `.github/workflows/npm-publish.yml` triggers on `v*.*.*` tag pushes and runs the 5-platform build matrix + publish job.

## Release flow (automated)

1. **Merge PR** for the sub-project
2. **Bump versions** in `Cargo.toml` (workspace.package.version) and `crates/singularmem-node/package.json`. Also bump every `optionalDependencies` entry's pinned version to match.
3. **Commit + push** the version bump on `main`
4. **Tag** the commit:
   ```bash
   git tag -a v0.X.0 -m "v0.X.0 — <one-line summary>"
   git push origin v0.X.0
   ```
5. **Wait** ~10 minutes. The npm publish workflow runs:
   - 5-platform build matrix (parallel, ~5-7 min wall-clock)
   - publish job (downloads artifacts, assembles per-platform packages, publishes to npm with provenance)
6. **Verify:**
   ```bash
   npm view singularmem version                              # should match the tag
   npm view singularmem-darwin-arm64 version         # should match
   # (repeat for the other 4 platforms if you want)
   npm audit signatures singularmem                          # should report verified provenance
   ```

## Dry-run before a real release

If you've changed the publish workflow itself or otherwise want to verify without pushing to npm:

1. Push a tag (so `GITHUB_REF` is a tag — required by the version-drift check)
2. Go to **Actions → npm publish → Run workflow** in the GitHub UI
3. Select the tag ref + leave `dry_run: true` (the default)
4. The build matrix runs; the publish job is skipped

Inspect the artifacts in the workflow run UI to verify the .node files look right.

## Recovering from a failed publish

The publish job is NOT atomic. If it fails partway through (e.g., 3 of 5 sub-packages publish but the 4th fails due to a network glitch), the registry ends up in a partial state. Recovery options:

- **Retry**: Re-run the failed publish job from the GitHub Actions UI. `npm publish` is idempotent if you re-publish the same version (it'll error with "cannot republish over the previously published versions"). For the already-published sub-packages, the retry will skip via npm's "already exists" error; the un-published ones complete. Acceptable for transient failures.
- **Un-publish (within 72 hours)**: `npm unpublish singularmem-<triple>@<version>` removes the package. Bump to the next patch version (e.g., 0.14.0 → 0.14.1) and re-tag.
- **Deprecate (after 72 hours)**: `npm deprecate singularmem@<version> "Bad release; use <next version>"` marks the version as deprecated. The package stays installable but consumers see a warning.

In all cases, document the recovery in the next PR's description so future maintainers know.

## Yanking a broken release

If a published version turns out to be broken (e.g., a bug surfaces post-release):

1. **Within 72 hours of publish:** `npm unpublish singularmem@<version>` (+ each sub-package). Bump to patch + re-tag.
2. **After 72 hours:** `npm deprecate singularmem@<version> "<reason>"`. Cut a new patch release immediately.

npm's 72-hour window exists to prevent the npm ecosystem version-confusion attacks of the "left-pad" era. Use deprecation for older releases.

## Pre-release channels

Not used in v0. If/when needed, `npm publish --tag next` publishes to the `next` dist-tag instead of `latest`. Consumers opt in with `npm install singularmem@next`. The workflow doesn't currently support this — to enable, add a `prerelease` boolean to `workflow_dispatch` inputs and gate the `--tag` flag on it.

## Bumping the version manually

The post-merge version-bump commit looks like:

```bash
# After merging PR #N
sed -i.bak 's/version = "0.13.0"/version = "0.14.0"/' Cargo.toml
node -e 'const p = require("./crates/singularmem-node/package.json"); p.version = "0.14.0"; for (const k of Object.keys(p.optionalDependencies || {})) p.optionalDependencies[k] = "0.14.0"; require("fs").writeFileSync("./crates/singularmem-node/package.json", JSON.stringify(p, null, 2) + "\n");'
rm Cargo.toml.bak

cargo build --release  # updates Cargo.lock
git add Cargo.toml Cargo.lock crates/singularmem-node/package.json
git commit -s -m "chore: bump workspace version 0.13.0 → 0.14.0"
git push origin main

git tag -a v0.14.0 -m "v0.14.0 — <summary>"
git push origin v0.14.0
```

(The CI version-drift check verifies the Cargo workspace and package.json stay in sync. The `optionalDependencies` entries must also match — otherwise npm install would fail to find the just-published sub-packages.)
