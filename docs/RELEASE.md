# How to Release tish-apple

Mirrors the [tish](https://github.com/tishlang/tish/blob/main/docs/RELEASE.md) / [tish-desktop](https://github.com/tishlang/tish-desktop/blob/main/docs/RELEASE.md) release flow: conventional commits → push `main` → GitHub **prerelease** → promote to a full release → crates.io + npm publish.

| Surface | Package |
|---------|---------|
| npm | `@tishlang/tish-macos`, `@tishlang/tish-ios` (scoped — npm ecosystem pattern, like `@tishlang/tish` / `@tishlang/lattish`) |
| crates.io | `tish-apple-common`, `tish-macos`, `tish-ios` |

Published apps depend on the scoped npm packages and `import { macos } from "@tishlang/tish-macos"`. The unscoped `tish:macos` / `tish:ios` import spec is **local debug only** — the compiler walk-up resolves it to `tish-apple/crates/tish-<x>` inside the monorepo, so it never exercises the published package.

---

## Before You Start: One-Time Setup

### 1. GitHub Secrets (Settings → Secrets and variables → Actions)

| Secret | Purpose |
|--------|---------|
| `CARGO_REGISTRY_TOKEN` | crates.io API token |

npm uses **OIDC trusted publishing** (no `NPM_TOKEN`).

### 2. npm Trusted Publishers

For each package, on npmjs.com → package → Settings → **Trusted Publisher** → GitHub Actions:

| Field | Value |
|-------|-------|
| Organization or user | `tishlang` |
| Repository | `tish-apple` |
| Workflow filename | `npm-release.yml` |
| Environment | *(blank)* |

Packages:

- `@tishlang/tish-macos`
- `@tishlang/tish-ios`

Create the packages on npm (empty publish or “Create package”) before the first OIDC publish if npm requires them to exist.

### 3. Upstream crates

`tish-macos` / `tish-ios` depend on **`tish_broker`** (from [tish-desktop](https://github.com/tishlang/tish-desktop)) and the **`tishlang_*`** train (from [tish](https://github.com/tishlang/tish)). Publish those first, or pass matching versions when re-dispatching the crates/npm workflows.

---

## Every Release

### Step 1: Commit with a release-triggering message

You need at least one commit that triggers a version bump. Use conventional commits:

```
feat: add something new        → minor (0.1.0 → 0.2.0)
fix: fix a bug                 → patch (0.1.0 → 0.1.1)
perf: make it faster           → patch
feat!: breaking change         → major (0.1.0 → 1.0.0)
```

`docs:` and `chore:` do **not** trigger a release.

### Step 2: Push to `main`

```bash
git push origin main
```

### Step 3: Let CI run

- Open **Actions** in the tish-apple repo
- Wait for **Release (prerelease)** to finish
- A GitHub **prerelease** (e.g. `v0.1.0`) should appear under Releases

If it fails:

- **“No incremental release would be triggered”** → Add a `feat:`, `fix:`, or `perf:` commit and push again

### Step 4: Promote the prerelease to a full release

1. Go to **Releases**
2. Find the **latest prerelease**
3. Click **Edit**
4. **Uncheck** “Set as a pre-release”
5. Click **Update release**

This runs the NPM and Crates.io release workflows. They run automatically; no further action needed.

### Manual re-run

- **Actions → Crates.io release → Run workflow** with the tag (skips versions already on crates.io). Optional `tishlang_core_version` / `tish_broker_version` if the ecosystem train moved.
- **Actions → NPM release → Run workflow** with the tag (skips packages already at that version).

---

## Verify

```bash
npm view @tishlang/tish-macos version
npm view @tishlang/tish-ios version
# https://crates.io/crates/tish-macos
```

Standalone smoke (no monorepo):

```bash
mkdir /tmp/apple-smoke && cd /tmp/apple-smoke
npm init -y
npm install @tishlang/tish @tishlang/tish-macos
# add a tiny main.tish that imports from "@tishlang/tish-macos", then:
# npx tish build --target native --native-backend rust src/main.tish -o dist/app
```

Or just run any `examples/*` directory — they consume the published packages.

---

## Notes

- **Monorepo vs registry:** local builds keep path deps on `tishlang_*` and `tish_broker`. Release workflows rewrite to crates.io versions.
- **npm tarballs** include `Cargo.toml` + `src/` so the compiler can path-depend on `node_modules/@tishlang/tish-macos` outside this checkout.
- **Names:** the crate `package.json` names stay unscoped (`tish-macos`, `tish-ios`) so the local-debug `tish:macos` walk-up keeps resolving; `scripts/publish-npm.mjs` rewrites them to the `@tishlang/` scope at stage time.
- Root package `tish-apple` is `private: true` — never published.
