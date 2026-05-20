# Releasing `singularmem`

This document covers the release process for the npm package. Day-to-day development never touches this — it's a reference for maintainers cutting a new version.

## ⚠️ Known issue: automated CI publish blocked (as of v0.14.0)

The `.github/workflows/npm-publish.yml` workflow successfully builds all 4 platform binaries on every PR + tag push, but the final `npm publish` steps consistently fail with HTTP 404 from the registry, even with a correctly-scoped `NPM_TOKEN` (Automation token, "Bypass 2FA" enabled, confirmed via in-CI `npm whoami` returning the right username).

Local publishes from the maintainer's machine work with a `--otp=<code>` flag, so the npm account itself is publish-capable. The mismatch between CI and local is unresolved.

**Workaround:** until the CI publish issue is fixed, releases ship via the **manual publish** procedure documented below. The CI build matrix is still useful — it proves the cross-platform compile works on every PR and produces ready-to-publish artifacts.

## One-time setup (already done as of v0.14.0)

1. **npm user `jonasbroms` owns:**
   - The unscoped `singularmem` package name
   - The unscoped `singularmem-<triple>` per-platform names (4 total)
   - All published with the `latest` dist-tag

2. **GitHub repo secrets:**
   - `NPM_TOKEN`: a Classic Automation token under the `jonasbroms` npm account, with "Bypass 2FA" enabled. (Note: as of v0.14.0 the token works for `npm whoami` from CI but the actual publish PUT fails with 404. Cause unknown; see "Known issue" above.)

3. **GitHub Actions workflow:**
   - `.github/workflows/npm-publish.yml` triggers on `v*.*.*` tag pushes. The build matrix runs reliably; the publish job is currently broken.

## Manual release flow (current — used for v0.14.0 and onward until CI is fixed)

1. **Merge PR** for the sub-project
2. **Bump versions** in `Cargo.toml` (workspace.package.version) and `crates/singularmem-node/package.json`. Also bump every `optionalDependencies` entry's pinned version to match.
3. **Commit + push** the version bump on `main`
4. **Tag** the commit:
   ```bash
   git tag -a v0.X.0 -m "v0.X.0 — <one-line summary>"
   git push origin v0.X.0
   ```
5. **Wait** ~10 minutes for the CI `build` matrix to complete on the tag push. Note the workflow run ID:
   ```bash
   RUN_ID=$(gh run list --workflow=npm-publish.yml --limit=1 --json databaseId --jq '.[0].databaseId')
   echo $RUN_ID
   ```
6. **Download the 4 platform .node artifacts:**
   ```bash
   cd crates/singularmem-node
   mkdir -p artifacts
   gh run download $RUN_ID --pattern 'binding-*' --dir artifacts/
   ```
7. **Move artifacts into each sub-package directory + bump per-platform versions:**
   ```bash
   npm run artifacts        # napi artifacts: copies .node files into npm/<triple>/
   # The prepublishOnly hook runs napi prepublish which bumps the per-platform versions
   # AND tries to npm publish. The publish fails with EOTP (no OTP in CLI), but the
   # version bumps are persisted on disk.
   npx napi prepublish -t npm --skip-gh-release || true   # ignore the EOTP failure
   ```
8. **Publish each sub-package + the main package manually with TOTP:**
   ```bash
   # 4 platform sub-packages — grab a fresh OTP from your authenticator for each
   cd npm/linux-x64-gnu       && npm publish --access public --otp=<TOTP> && cd ../..
   cd npm/darwin-x64          && npm publish --access public --otp=<TOTP> && cd ../..
   cd npm/darwin-arm64        && npm publish --access public --otp=<TOTP> && cd ../..
   cd npm/win32-x64-msvc      && npm publish --access public --otp=<TOTP> && cd ../..

   # Main package (from crates/singularmem-node/)
   # NOTE: the prepublishOnly hook will re-run napi prepublish which will fail with
   # "cannot publish over the previously published versions" because the sub-packages
   # are already on npm. Temporarily disable the hook before this command:
   npm pkg delete scripts.prepublishOnly
   npm publish --access public --otp=<TOTP>
   # Restore the hook for the committed state:
   npm pkg set 'scripts.prepublishOnly=napi prepublish -t npm --skip-gh-release'
   ```
9. **Verify all 5 packages:**
   ```bash
   for pkg in singularmem singularmem-linux-x64-gnu singularmem-darwin-x64 singularmem-darwin-arm64 singularmem-win32-x64-msvc; do
     echo "$pkg: $(npm view $pkg version)"
   done
   # All 5 should print the new version
   ```
10. **Reset the per-platform package.json templates back to `0.0.0` sentinel:**
    ```bash
    for dir in linux-x64-gnu darwin-x64 darwin-arm64 win32-x64-msvc; do
      sed -i.bak 's/"version": "X.Y.Z"/"version": "0.0.0"/' npm/$dir/package.json
      rm npm/$dir/package.json.bak
    done
    git add npm/*/package.json
    git commit -s -m "chore(node): reset per-platform package.json templates to 0.0.0 sentinel"
    git push origin main
    ```

## Future automated release flow (once CI publish is fixed)

If/when the CI publish issue is resolved (sub-project 6b or similar), the flow becomes:

1. Merge PR
2. Bump versions
3. Tag + push
4. Wait ~10 min — workflow runs automatically and publishes
5. Verify with `npm view singularmem version`

(Same as steps 1-5 of the manual flow but with publish automation.)

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
