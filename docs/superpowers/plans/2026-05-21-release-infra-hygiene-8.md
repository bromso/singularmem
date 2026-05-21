# Release-infra hygiene (8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pay down the technical debt accumulated landing v0.15.0 — bump all GH Actions off the deprecated Node 20 runtime, regenerate `release.yml` via `dist init` against cargo-dist 0.31.0 templates, drop the `allow-dirty = ["ci"]` escape-hatch.

**Architecture:** Single PR titled `chore(ci): sub-project 8 — release infra hygiene`. Surface is `.github/workflows/*.yml`, `Cargo.toml`, `dist-workspace.toml`. RC dry-run via `v0.16.0-rc1` tag against the PR branch validates the full release DAG before merge. Real release fires on `v0.16.0` tag.

**Tech Stack:** GitHub Actions, `cargo-dist` v0.31.0, Homebrew tap (`bromso/homebrew-tap`), no code changes.

**Spec:** `docs/superpowers/specs/2026-05-21-release-infra-hygiene-8-design.md`

---

## File Structure

**Modify:**
- `.github/workflows/ci.yml` — bump action versions
- `.github/workflows/dco.yml` — bump action versions
- `.github/workflows/npm-publish.yml` — bump action versions
- `.github/workflows/release.yml` — regenerated wholesale by `dist init`
- `Cargo.toml` — possibly touched by `dist init` ([profile.dist] shape)
- `dist-workspace.toml` — drop `allow-dirty = ["ci"]`; reconcile any other `dist init` changes
- `crates/singularmem-node/package.json` — version bump (post-merge follow-on commit)

**No new files. No deletions.**

**Out-of-PR operational pre-req:**
- Manual npm publish of `singularmem@0.15.0` per `crates/singularmem-node/RELEASING.md`. TOTP-gated; user must execute. Confirms v0.15.0 is fully released before the sub-project 8 PR stacks v0.16.0 changes on top.

---

## Task 1: Manual npm publish of `singularmem@0.15.0` (USER, operational)

**Files:** none in this repo. Affects npm registry only.

This is the pre-req from the spec's Section "Recommended approach" step 2. Not part of the PR diff. **Cannot be done by a coding agent** — requires the maintainer's TOTP from an authenticator app.

- [ ] **Step 1: Resolve the latest npm-publish.yml run ID**

```bash
cd /Users/jonasbroms/Sites/singularmem/crates/singularmem-node
RUN_ID=$(gh run list --workflow=npm-publish.yml --limit=1 --json databaseId --jq '.[0].databaseId')
echo "Using run: $RUN_ID"
```

If the latest run is not from the `v0.15.0` tag push, find the right one with:

```bash
gh run list --workflow=npm-publish.yml --branch=v0.15.0 --json databaseId,conclusion,headBranch --limit=10
```

Pick the most recent run on the `v0.15.0` ref. The publish job will have failed (known 404 issue from sub-project 6); the build job(s) should have succeeded.

- [ ] **Step 2: Download the 4 platform artifacts**

```bash
mkdir -p artifacts
gh run download $RUN_ID --pattern 'binding-*' --dir artifacts/
```

Expected: 4 directories like `artifacts/binding-linux-x64-gnu/`, each containing one `.node` file.

- [ ] **Step 3: Stage artifacts + bump per-platform versions**

```bash
npm run artifacts
npx napi prepublish -t npm --skip-gh-release || true
```

The `napi prepublish` step ends in an EOTP failure — expected and ignored. It still writes the version bumps to disk.

- [ ] **Step 4: Publish each platform sub-package**

```bash
cd npm/linux-x64-gnu  && npm publish --access public --otp=<TOTP-FROM-APP> && cd ../..
cd npm/darwin-x64     && npm publish --access public --otp=<TOTP-FROM-APP> && cd ../..
cd npm/darwin-arm64   && npm publish --access public --otp=<TOTP-FROM-APP> && cd ../..
cd npm/win32-x64-msvc && npm publish --access public --otp=<TOTP-FROM-APP> && cd ../..
```

Fresh OTP per command from the authenticator.

- [ ] **Step 5: Publish main package (disable prepublishOnly hook first)**

```bash
npm pkg delete scripts.prepublishOnly
npm publish --access public --otp=<TOTP-FROM-APP>
npm pkg set 'scripts.prepublishOnly=napi prepublish -t npm --skip-gh-release'
```

- [ ] **Step 6: Verify**

```bash
for pkg in singularmem singularmem-linux-x64-gnu singularmem-darwin-x64 singularmem-darwin-arm64 singularmem-win32-x64-msvc; do
  echo "$pkg: $(npm view $pkg version)"
done
```

Expected: all 5 print `0.15.0`.

- [ ] **Step 7: Reset per-platform package.json templates back to 0.0.0 sentinel + commit**

```bash
cd /Users/jonasbroms/Sites/singularmem/crates/singularmem-node
for dir in linux-x64-gnu darwin-x64 darwin-arm64 win32-x64-msvc; do
  sed -i.bak 's/"version": "0.15.0"/"version": "0.0.0"/' npm/$dir/package.json
  rm npm/$dir/package.json.bak
done
cd /Users/jonasbroms/Sites/singularmem
git add crates/singularmem-node/npm/*/package.json
git commit -s -m "chore(node): reset per-platform package.json templates to 0.0.0 sentinel"
git push origin main
```

This commit lands directly on main (does not block the sub-project 8 PR; runs in parallel).

---

## Task 2: Audit and bump GH Actions in non-release workflows

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/dco.yml`
- Modify: `.github/workflows/npm-publish.yml`

`release.yml` is intentionally NOT touched here — Task 4's `dist init` regenerates it wholesale.

- [ ] **Step 1: Create the sub-project branch**

```bash
cd /Users/jonasbroms/Sites/singularmem
git checkout main
git pull --ff-only
git checkout -b release-infra-hygiene-8
```

- [ ] **Step 2: Inventory current action usage in non-release workflows**

```bash
grep -n "uses:" .github/workflows/ci.yml .github/workflows/dco.yml .github/workflows/npm-publish.yml | grep -v "^\s*#"
```

Expected output: a list of `actions/checkout@v4`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`, `actions/setup-node@v4`, `actions/cache@v4`, possibly `actions-rust-lang/setup-rust-toolchain@vX`, `rustsec/audit-check@v1`, `swatinem/rust-cache@v2`, etc.

Note every distinct action @ version. Write the list to a scratch file for later reference.

- [ ] **Step 3: Check upstream for the latest major of each Node-20 action**

For each action found in Step 2, check its latest release on GitHub:

```bash
for repo in actions/checkout actions/upload-artifact actions/download-artifact actions/setup-node actions/cache; do
  echo "=== $repo ==="
  gh api "/repos/$repo/releases?per_page=3" --jq '.[].tag_name'
done
```

Expected: each action's most recent stable release. Verify the major version (`v5`, `v6`, etc.) by checking the release notes for "Node 24" or "node20→node24" in the description:

```bash
gh api /repos/actions/checkout/releases/latest --jq '.body' | head -20
```

**Decision rule:**
- If a `@v5` exists and its release notes mention Node 24 / Node 20 removal, bump to `@v5`.
- If only `@v4` exists, leave it at `@v4` BUT add `env: FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: "true"` at the workflow level (or per-step). Document this in the PR description as a residual workaround.
- For actions outside the `actions/` org (e.g., `swatinem/rust-cache`, `rustsec/audit-check`), check their own release notes; skip the bump if they aren't emitting the Node 20 deprecation warning.

- [ ] **Step 4: Bump action versions in `ci.yml`**

Use `sed -i.bak` or `Edit` for each `uses:` line that needs bumping. Example:

```bash
sed -i.bak 's|uses: actions/checkout@v4|uses: actions/checkout@v5|g' .github/workflows/ci.yml
sed -i.bak 's|uses: actions/upload-artifact@v4|uses: actions/upload-artifact@v5|g' .github/workflows/ci.yml
sed -i.bak 's|uses: actions/download-artifact@v4|uses: actions/download-artifact@v5|g' .github/workflows/ci.yml
sed -i.bak 's|uses: actions/setup-node@v4|uses: actions/setup-node@v5|g' .github/workflows/ci.yml
sed -i.bak 's|uses: actions/cache@v4|uses: actions/cache@v5|g' .github/workflows/ci.yml
rm .github/workflows/ci.yml.bak
```

Adjust the target version (`@v5`) per Step 3's findings.

- [ ] **Step 5: Verify ci.yml syntax is still valid**

```bash
gh workflow view ci.yml --repo bromso/singularmem 2>&1 || true
yamllint .github/workflows/ci.yml 2>/dev/null || python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo "valid YAML"
```

Expected: "valid YAML". If `yamllint` reports indentation issues, the `sed` didn't break the file but yamllint is picky — only YAML-parse errors block.

- [ ] **Step 6: Repeat Steps 4-5 for `dco.yml` and `npm-publish.yml`**

```bash
for wf in dco.yml npm-publish.yml; do
  for action in checkout upload-artifact download-artifact setup-node cache; do
    sed -i.bak "s|uses: actions/$action@v4|uses: actions/$action@v5|g" .github/workflows/$wf
  done
  rm .github/workflows/$wf.bak
  python3 -c "import yaml; yaml.safe_load(open('.github/workflows/$wf'))" && echo "$wf valid"
done
```

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/dco.yml .github/workflows/npm-publish.yml
git commit -s -m "chore(ci): bump GH Actions to versions running Node 24

Node 20 actions hit a hard deprecation cutoff on 2026-06-02. This bump
moves actions/checkout, actions/upload-artifact, actions/download-artifact,
actions/setup-node, actions/cache to @v5 across ci.yml, dco.yml, and
npm-publish.yml. release.yml is regenerated separately by dist init in
a later commit of this PR.

Sub-project 8."
```

---

## Task 3: Install cargo-dist 0.31.0 locally

**Files:** none. Local-only step.

- [ ] **Step 1: Check current installed version (if any)**

```bash
which dist && dist --version 2>&1 || echo "dist not installed"
```

If already at 0.31.0, skip to Task 4.

- [ ] **Step 2: Install cargo-dist 0.31.0**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.31.0/cargo-dist-installer.sh | sh
```

Expected: installs to `~/.cargo/bin/dist`. Verify:

```bash
dist --version
# Expected: dist 0.31.0
```

If install fails (URL changed, network issue), check https://github.com/axodotdev/cargo-dist/releases/tag/v0.31.0 for the correct installer URL and adapt.

---

## Task 4: Run `dist init`, capture and reconcile diffs

**Files:**
- Modify (possibly): `Cargo.toml`
- Modify: `dist-workspace.toml`
- Modify: `.github/workflows/release.yml`

`dist init` is opinionated. The spec's "Constraints" table enumerates what must survive regeneration.

- [ ] **Step 1: Snapshot current state**

```bash
cd /Users/jonasbroms/Sites/singularmem
git diff --stat HEAD
# Expected: no changes (Task 2 already committed)
cp Cargo.toml /tmp/Cargo.toml.pre-dist-init
cp dist-workspace.toml /tmp/dist-workspace.toml.pre-dist-init
cp .github/workflows/release.yml /tmp/release.yml.pre-dist-init
```

- [ ] **Step 2: Run `dist init` interactively**

```bash
dist init
```

Answer prompts to match current settings:

| Prompt | Answer |
|---|---|
| Cargo-dist version | `0.31.0` (matches installed) |
| CI provider | `github` |
| Installers | Toggle ON: `shell`, `homebrew`. Toggle OFF: `powershell`, `msi`, `npm`, anything else |
| Targets | Toggle ON: `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Toggle OFF: `x86_64-pc-windows-msvc`, all `aarch64-*-linux-*`, everything else |
| Generate GitHub release | yes |
| Publish to crates.io | no |
| Tap repo for Homebrew | `bromso/homebrew-tap` |
| Tap formula name | accept default |
| Tap PAT secret name | `HOMEBREW_TAP_TOKEN` |
| Allow dirty | empty (drop the existing `ci` entry) |
| PR run mode | `upload` |
| Install path | `CARGO_HOME` |
| `[profile.dist]` | accept whatever dist suggests (probably re-adds `lto = "thin"`); we tolerate this per spec |

When prompted "OK to apply these changes?", confirm.

If `dist init` asks about `publish-jobs`, ensure `homebrew` is included.

- [ ] **Step 3: Inspect the diff**

```bash
git diff dist-workspace.toml
git diff Cargo.toml
git diff .github/workflows/release.yml | head -100
```

For each file, note:
- What `dist init` ADDED that wasn't there before
- What `dist init` REMOVED that was there before
- What `dist init` CHANGED

Open a scratch buffer / note file. Categorize each diff into:
- **Accept** — improvement or canonical shape, keep as-is
- **Revert** — undoes a session-locked constraint, must be reverted

The spec's "Constraints" table is the truth-of-record for "what must be preserved".

- [ ] **Step 4: Apply explicit reverts for constraint violations**

For each constraint that `dist init` undid, manually re-apply the edit. Examples (the actual list depends on `dist init`'s output):

```bash
# Example: if dist init re-added Windows MSVC to targets
# Edit dist-workspace.toml; restore 3-platform list:
#   targets = ["x86_64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin"]
```

Use `Edit` to make targeted reverts. After each revert:

```bash
git diff dist-workspace.toml | head -20
```

Confirm only the constraint-restoring line changed.

- [ ] **Step 5: Drop `allow-dirty = ["ci"]` from `dist-workspace.toml`**

If `dist init` already removed it (likely, since it was removed from the prompt answers in Step 2), skip. Otherwise:

```bash
sed -i.bak '/^allow-dirty/d' dist-workspace.toml
rm dist-workspace.toml.bak
# Also remove the explanatory comment block above it if still present
```

Verify:

```bash
grep -q '^allow-dirty' dist-workspace.toml && echo "STILL PRESENT — fix manually" || echo "absent"
```

Expected: `absent`.

- [ ] **Step 6: Verify the cargo-dist installer URL in release.yml matches 0.31.0**

```bash
grep "cargo-dist/releases" .github/workflows/release.yml
```

Expected: URL contains `v0.31.0/cargo-dist-installer.sh`. If `dist init` re-emitted v0.27.0 (unlikely but possible), fix manually:

```bash
sed -i.bak 's|cargo-dist/releases/download/v0\.27\.0|cargo-dist/releases/download/v0.31.0|g' .github/workflows/release.yml
rm .github/workflows/release.yml.bak
```

- [ ] **Step 7: Validate dist config + workflow shape**

```bash
dist plan
```

Expected: outputs a JSON plan describing the 3-platform matrix, both binaries (`singularmem` + `singularmem-mcp`), 2 shell installer scripts, Homebrew formulas. No errors. No "out of date contents" warnings (the consistency check should now pass without allow-dirty).

If `dist plan` reports "out of date contents", `dist init` left something dirty. Re-run `dist init` and accept all changes, or manually fix per the diff.

- [ ] **Step 8: Commit the regenerated state**

```bash
git add Cargo.toml dist-workspace.toml .github/workflows/release.yml
git commit -s -m "chore(ci): regenerate release.yml via dist init against cargo-dist 0.31.0

Drops the allow-dirty = [\"ci\"] escape-hatch and the 5 hand-applied
patches accumulated during v0.15.0's release saga. release.yml is now
canonical dist-0.31.0 output.

Residual edits explicitly preserved (re-applied after dist init):
- 3-platform targets (no x86_64-pc-windows-msvc) — ort-sys CRT mismatch
- (any other reverts applied in Step 4 — list here)

Sub-project 8."
```

The commit message's residual-edits list IS the spec's acceptance criterion 3 audit trail.

---

## Task 5: Push branch and open PR

**Files:** none in this repo. GitHub PR creation.

- [ ] **Step 1: Push the branch**

```bash
git push -u origin release-infra-hygiene-8
```

- [ ] **Step 2: Create the PR**

```bash
gh pr create --title "chore(ci): sub-project 8 — release infra hygiene" --body "$(cat <<'EOF'
## Summary
- Bumps GH Actions in ci.yml, dco.yml, npm-publish.yml off the deprecated Node 20 runtime (hard cutoff 2026-06-02)
- Regenerates .github/workflows/release.yml via `dist init` against cargo-dist 0.31.0 templates
- Drops the `allow-dirty = ["ci"]` escape-hatch from dist-workspace.toml that was added during v0.15.0's release saga

## Residual edits after dist init regeneration

The following manual edits were re-applied after dist init to preserve session-locked decisions:

- `dist-workspace.toml` `targets`: drops `x86_64-pc-windows-msvc` (ort-sys CRT mismatch on Windows; tracked as future sub-project)
- (List any other reverts performed in Task 4 Step 4 here)

## Test plan
- [ ] PR CI: all jobs green on ci.yml, dco.yml, npm-publish.yml (build matrix, publish job skipped)
- [ ] PR CI: release.yml runs in upload mode, all 3 platforms build successfully
- [ ] No "Node.js 20 actions are deprecated" annotation on any workflow run
- [ ] RC dry-run: tag v0.16.0-rc1 against this branch fires full release.yml DAG (host + publish-homebrew + announce all succeed)
- [ ] Post-merge: v0.16.0 tag fires full pipeline, GH Release published with 20 assets, Homebrew tap updated to 0.16.0

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected output: a PR URL. Note it for later reference.

---

## Task 6: PR CI validation

**Files:** none. Wait for CI to run.

- [ ] **Step 1: Wait for all PR checks to complete**

```bash
PR_NUM=$(gh pr view --json number --jq .number)
gh pr checks $PR_NUM --watch
```

Expected: all checks pass. The npm-publish.yml workflow's publish job is still expected to fail (known 404; out of scope). The build matrix portion of npm-publish.yml should pass.

- [ ] **Step 2: Verify no Node 20 deprecation warnings**

```bash
RUN_IDS=$(gh pr checks $PR_NUM --json name,link --jq '.[].link | split("/")[-1]')
for run_id in $RUN_IDS; do
  echo "=== Run $run_id ==="
  gh api /repos/bromso/singularmem/actions/runs/$run_id/jobs --jq '.jobs[].annotations[]? | select(.message | contains("Node.js 20"))' 2>/dev/null
done
```

Expected: no output (no Node 20 deprecation annotations on any job in any workflow).

If any job still emits the warning, identify which action wasn't bumped:

```bash
gh run view <run_id> --log | grep -B 2 "Node.js 20 actions are deprecated"
```

Bump that action and amend the appropriate commit.

- [ ] **Step 3: Verify release.yml build matrix succeeded**

```bash
RELEASE_RUN=$(gh run list --workflow=release.yml --branch=release-infra-hygiene-8 --limit=1 --json databaseId --jq '.[0].databaseId')
gh run view $RELEASE_RUN --json jobs --jq '.jobs[] | {name, status, conclusion}'
```

Expected: `plan`, `build-local-artifacts × 3`, `build-global-artifacts` all `completed/success`. `host`, `publish-homebrew-formula`, `announce` are SKIPPED on PR runs (tag-gated) — that's correct.

---

## Task 7: RC dry-run via `v0.16.0-rc1` tag

**Files:** none in this repo. Tag + GH Release operations.

The RC dry-run exercises the host + publish-homebrew + announce jobs that the PR run skips. Per the spec's Open Question #3, we let the homebrew formula PR fire and manually revert if the rc is rejected.

- [ ] **Step 1: Tag the PR branch HEAD as `v0.16.0-rc1`**

```bash
git checkout release-infra-hygiene-8
git pull --ff-only
git tag -a v0.16.0-rc1 -m "v0.16.0-rc1 — release-infra-hygiene RC dry-run"
git push origin v0.16.0-rc1
```

- [ ] **Step 2: Watch the release.yml run**

```bash
sleep 20
RC_RUN=$(gh run list --workflow=release.yml --branch=v0.16.0-rc1 --limit=1 --json databaseId --jq '.[0].databaseId')
echo "RC run: $RC_RUN"
gh run watch $RC_RUN --exit-status
```

Expected: all jobs succeed, including `host`, `publish-homebrew-formula`, `announce`.

- [ ] **Step 3: Verify RC release artifacts**

```bash
gh release view v0.16.0-rc1 --json assets --jq '.assets[].name'
```

Expected: 20 assets matching v0.15.0's shape:
- 6 archives (3 platforms × 2 binaries): `singularmem-{aarch64,x86_64}-apple-darwin.tar.xz`, `singularmem-x86_64-unknown-linux-gnu.tar.xz`, plus matching `singularmem-mcp-*.tar.xz`
- 6 sha256 checksums
- 2 installer scripts: `singularmem-installer.sh`, `singularmem-mcp-installer.sh`
- 2 Homebrew formulas: `singularmem.rb`, `singularmem-mcp.rb`
- `source.tar.gz` + `source.tar.gz.sha256`
- `dist-manifest.json` + `sha256.sum`

- [ ] **Step 4: Verify Homebrew formula PR was raised**

```bash
gh pr list --repo bromso/homebrew-tap --state open --json title,number
```

Expected: a PR titled something like "singularmem 0.16.0-rc1" and "singularmem-mcp 0.16.0-rc1" (or one PR with both updates).

**Do NOT merge the formula PR.** The RC is for validation only. Close it (next step) once the RC is verified.

- [ ] **Step 5: Cleanup — delete RC release, tag, and formula PR**

```bash
# Delete the GH Release + tag
gh release delete v0.16.0-rc1 --repo bromso/singularmem --yes --cleanup-tag

# Verify tag is gone locally + remotely
git fetch --prune --prune-tags origin
git tag -l v0.16.0-rc1
# Expected: empty output

# Close the homebrew-tap formula PR(s)
HBREW_PRS=$(gh pr list --repo bromso/homebrew-tap --state open --json number --jq '.[].number')
for pr in $HBREW_PRS; do
  gh pr close $pr --repo bromso/homebrew-tap --comment "RC validation complete; closing without merge. v0.16.0 final release will land separately."
done
```

If `gh release delete` doesn't clean up the tag (older gh versions), do it manually:

```bash
git push --delete origin v0.16.0-rc1
git tag -d v0.16.0-rc1
```

---

## Task 8: Merge the PR

**Files:** none in this repo. GitHub merge operation.

- [ ] **Step 1: Final review**

```bash
gh pr view $PR_NUM --json reviews,checks
```

Confirm all checks green and any required reviewers approved.

- [ ] **Step 2: Merge via squash or merge commit (project convention from prior sub-projects: merge commit)**

```bash
gh pr merge $PR_NUM --merge --delete-branch
```

If the project uses squash, swap to `--squash`. Past sub-projects (6, 7) used `--merge` for the PR's commit message visibility.

- [ ] **Step 3: Sync local main**

```bash
git checkout main
git pull --ff-only
git log --oneline -3
```

Expected: top commit is the merge commit for sub-project 8.

---

## Task 9: Version bump 0.15.0 → 0.16.0

**Files:**
- Modify: `Cargo.toml` (workspace.package.version)
- Modify: `crates/singularmem-node/package.json` (version + optionalDependencies entries)
- Modify: `Cargo.lock` (auto-updated by `cargo build`)

- [ ] **Step 1: Edit `Cargo.toml`**

```bash
sed -i.bak 's/^version = "0.15.0"/version = "0.16.0"/' Cargo.toml
rm Cargo.toml.bak
grep '^version' Cargo.toml
# Expected: version = "0.16.0"
```

- [ ] **Step 2: Edit `crates/singularmem-node/package.json`**

```bash
node -e '
const p = require("./crates/singularmem-node/package.json");
p.version = "0.16.0";
for (const k of Object.keys(p.optionalDependencies || {})) {
  p.optionalDependencies[k] = "0.16.0";
}
require("fs").writeFileSync(
  "./crates/singularmem-node/package.json",
  JSON.stringify(p, null, 2) + "\n"
);
'
grep -E '"version"|"singularmem-' crates/singularmem-node/package.json
```

Expected: main `"version": "0.16.0"`, all 4 `optionalDependencies` entries pinned to `"0.16.0"`.

- [ ] **Step 3: Update Cargo.lock**

```bash
cargo build --release 2>&1 | tail -5
```

Expected: builds clean. `Cargo.lock` will show diffs only for the workspace's own crates' version entries.

- [ ] **Step 4: Sanity check**

```bash
git diff --stat
# Expected: Cargo.toml, Cargo.lock, crates/singularmem-node/package.json all modified
```

- [ ] **Step 5: Commit + push**

```bash
git add Cargo.toml Cargo.lock crates/singularmem-node/package.json
git commit -s -m "chore: bump workspace version 0.15.0 → 0.16.0"
git push origin main
```

---

## Task 10: Tag v0.16.0 + verify acceptance criteria

**Files:** none in this repo. Tag + verification operations.

- [ ] **Step 1: Tag**

```bash
git tag -a v0.16.0 -m "v0.16.0 — sub-project 8 (release infra hygiene)"
git push origin v0.16.0
```

- [ ] **Step 2: Watch the release.yml run**

```bash
sleep 20
RELEASE_RUN=$(gh run list --workflow=release.yml --branch=v0.16.0 --limit=1 --json databaseId --jq '.[0].databaseId')
echo "Release run: $RELEASE_RUN"
gh run watch $RELEASE_RUN --exit-status
```

Expected: all 7 jobs succeed: `plan`, `build-local-artifacts × 3`, `build-global-artifacts`, `host`, `publish-homebrew-formula`, `announce`.

- [ ] **Step 3: Verify acceptance criterion 1 — no Node 20 warnings**

```bash
gh api /repos/bromso/singularmem/actions/runs/$RELEASE_RUN/jobs --jq '.jobs[].annotations[]? | select(.message | contains("Node.js 20"))'
```

Expected: empty output.

Repeat for the CI workflow run on the version-bump commit:

```bash
CI_RUN=$(gh run list --workflow=ci.yml --branch=main --limit=1 --json databaseId --jq '.[0].databaseId')
gh api /repos/bromso/singularmem/actions/runs/$CI_RUN/jobs --jq '.jobs[].annotations[]? | select(.message | contains("Node.js 20"))'
```

Expected: empty.

- [ ] **Step 4: Verify acceptance criterion 2 — `allow-dirty` is gone**

```bash
grep -q '^allow-dirty' dist-workspace.toml && echo "STILL PRESENT" || echo "absent"
```

Expected: `absent`.

- [ ] **Step 5: Verify acceptance criterion 3 — release.yml matches `dist init` output**

```bash
cp .github/workflows/release.yml /tmp/release.yml.committed
dist init
git diff .github/workflows/release.yml
```

Expected: no diff, OR only diffs matching the residual-edits list documented in the PR description (Task 5 Step 2). If diffs match the documented residuals, pass. Reset:

```bash
git checkout .github/workflows/release.yml Cargo.toml dist-workspace.toml
```

- [ ] **Step 6: Verify acceptance criteria 5 & 6 — release pipeline succeeded with all 20 assets**

```bash
gh release view v0.16.0 --json assets --jq '.assets | length'
# Expected: 20

gh release view v0.16.0 --json assets --jq '.assets[].name' | sort
# Expected (matches v0.15.0 shape):
# dist-manifest.json
# sha256.sum
# singularmem-aarch64-apple-darwin.tar.xz
# singularmem-aarch64-apple-darwin.tar.xz.sha256
# singularmem-installer.sh
# singularmem-mcp-aarch64-apple-darwin.tar.xz
# singularmem-mcp-aarch64-apple-darwin.tar.xz.sha256
# singularmem-mcp-installer.sh
# singularmem-mcp-x86_64-apple-darwin.tar.xz
# singularmem-mcp-x86_64-apple-darwin.tar.xz.sha256
# singularmem-mcp-x86_64-unknown-linux-gnu.tar.xz
# singularmem-mcp-x86_64-unknown-linux-gnu.tar.xz.sha256
# singularmem-mcp.rb
# singularmem-x86_64-apple-darwin.tar.xz
# singularmem-x86_64-apple-darwin.tar.xz.sha256
# singularmem-x86_64-unknown-linux-gnu.tar.xz
# singularmem-x86_64-unknown-linux-gnu.tar.xz.sha256
# singularmem.rb
# source.tar.gz
# source.tar.gz.sha256
```

- [ ] **Step 7: Verify acceptance criterion 7 — Homebrew tap updated**

```bash
gh api repos/bromso/homebrew-tap/contents/Formula/singularmem.rb --jq '.content' | base64 -d | grep 'version "0.16.0"'
gh api repos/bromso/homebrew-tap/contents/Formula/singularmem-mcp.rb --jq '.content' | base64 -d | grep 'version "0.16.0"'
```

Expected: each command prints exactly one line containing `version "0.16.0"`.

- [ ] **Step 8: Verify acceptance criterion 8 — `brew install` works on a clean macOS machine**

This must be run on a clean macOS test environment (or the maintainer's machine with `bromso/tap` not previously installed):

```bash
brew untap bromso/tap 2>/dev/null || true
brew install bromso/tap/singularmem
singularmem --version
# Expected: 0.16.0
brew install bromso/tap/singularmem-mcp
singularmem-mcp --version
# Expected: 0.16.0
```

If `singularmem-mcp` is shipped under the same formula (`singularmem.rb` brings both binaries), `brew install bromso/tap/singularmem` alone may suffice. Check the v0.15.0 formula content for precedent.

- [ ] **Step 9: Verify acceptance criterion 9 — curl-installer works on a clean Linux machine**

This must be run on a clean Linux x86_64 test environment (or via Docker / a fresh user account):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bromso/singularmem/releases/download/v0.16.0/singularmem-installer.sh | sh
~/.cargo/bin/singularmem --version
# Expected: 0.16.0
```

Repeat for `singularmem-mcp-installer.sh` if it exists.

- [ ] **Step 10: Update project memory**

```bash
# Update memory file with sub-project 8 completion + new release-infra state
# (Done by the agent running this plan; recorded in
# /Users/jonasbroms/.claude/projects/-Users-jonasbroms-Sites-singularmem/memory/)
```

The agent should:
- Update `project_release_infra_state.md` to reflect:
  - cargo-dist 0.31.0 in use, no allow-dirty, no manual edits
  - npm 0.15.0 + 0.16.0 both shipped
  - Homebrew tap at 0.16.0
  - Windows still broken, Linux arm64 still missing
- Update `project_singularmem_overview.md` to add sub-project 8 to the completed-sub-projects list with date + tag + key facts
- Refresh sub-project 9 candidates (Windows MSVC fix moves up; npm 6b still pending; etc.)

---

## Risks & rollback

Per the spec's "Error handling" section:

- **`v0.16.0` release pipeline fails at host or publish-homebrew.** Delete the v0.16.0 tag + GH Release; iterate on a fix commit; re-tag. Same procedure as v0.15.0's saga.
- **Bumped action @v5 breaks a workflow's behavior unexpectedly.** Revert the specific action's bump in a follow-on commit; set `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` workflow-level env as the workaround. No re-tag needed (CI runs trigger on next merge).
- **`dist init` produces a regression we didn't catch in RC.** Force-revert the regenerated release.yml to v0.15.0's shape via `git revert` on the dist-init commit; re-introduce allow-dirty if needed; re-tag.

Rollback authority: the maintainer. Coding agents should NOT force-push or re-tag without explicit user instruction.

---

## Acceptance criteria summary (mirrors spec Section "Acceptance criteria")

- [ ] 1. No Node 20 deprecation warnings on any workflow run
- [ ] 2. `dist-workspace.toml` has no `allow-dirty` key
- [ ] 3. `release.yml` matches `dist init` output (modulo documented residuals)
- [ ] 4. PR CI all green
- [ ] 5. `v0.16.0` release pipeline all 7 jobs succeed
- [ ] 6. GH Release `v0.16.0` exists with 20 assets
- [ ] 7. Homebrew tap updated to 0.16.0
- [ ] 8. `brew install bromso/tap/singularmem` produces 0.16.0
- [ ] 9. Curl-installer produces 0.16.0

When all 9 are checked, sub-project 8 is done.
