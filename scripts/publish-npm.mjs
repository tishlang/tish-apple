#!/usr/bin/env node
/**
 * Publish @tishlang/tish-macos + @tishlang/tish-ios to npm.
 *
 * Published names follow the npm ecosystem pattern of the other tish repos
 * (`@tishlang/tish`, `@tishlang/lattish`): consumers add the scoped package and
 * `import { macos } from "@tishlang/tish-macos"`. The published tarball IS the
 * native host package the compiler resolves (`node_modules/@tishlang/tish-macos`):
 * package.json `tish` block + Cargo.toml + src/. Cargo.toml path deps
 * (monorepo-only) are rewritten to crates.io versions so `tish build` works
 * from a plain `npm install`.
 *
 * The unscoped `tish:macos` / `tish:ios` import spec is the LOCAL DEBUG path
 * only — the compiler walk-up resolves it to `tish-apple/crates/tish-<x>` by
 * the crate package.json name, which therefore stays unscoped in-repo and is
 * rewritten to the scoped name at stage time.
 *
 * Usage:
 *   node scripts/publish-npm.mjs --version 0.2.0 [options]
 *
 * Options:
 *   --version <v>   Version to publish (required; or env VERSION / TAG=v0.2.0)
 *   --core <req>    crates.io tishlang_* train        (default: 2.39, env TISHLANG_CORE_VERSION)
 *   --broker <req>  crates.io tishlang_broker version req (default: 0.1,  env TISH_BROKER_VERSION)
 *   --dry-run       Stage + `npm pack` instead of publishing
 *   --only <name>   Publish a single package (tish-macos or @tishlang/tish-macos, etc.)
 *
 * Staged output lands in dist-npm/<crate>/ so you can inspect exactly what ships.
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const PACKAGES = [
  { dir: "crates/tish-macos", localName: "tish-macos", npmName: "@tishlang/tish-macos" },
  { dir: "crates/tish-ios", localName: "tish-ios", npmName: "@tishlang/tish-ios" },
];

// ---- args ------------------------------------------------------------------

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

const tagEnv = process.env.TAG?.replace(/^v/, "");
const version = opt("--version") ?? process.env.VERSION ?? tagEnv;
// tishlang_* tracks the core tish train (3.x). tishlang_broker is published from
// tishlang/tish-desktop, which versions on its OWN line (1.x) — not the core train.
const core = opt("--core") ?? process.env.TISHLANG_CORE_VERSION ?? "3.0";
const broker = opt("--broker") ?? process.env.TISH_BROKER_VERSION ?? "1.0";
const dryRun = flag("--dry-run");
const only = opt("--only");

if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error("Usage: node scripts/publish-npm.mjs --version <x.y.z> [--core <req>] [--broker <req>] [--dry-run] [--only <pkg>]");
  process.exit(1);
}

// ---- Cargo.toml rewrite ------------------------------------------------------

/**
 * Rewrite one dependency line from `{ path = "…" , ...rest }` to
 * `{ version = "<v>" , ...rest }`, keeping every non-path key (features,
 * default-features, …) exactly as written.
 */
function rewritePathDep(text, depName, newVersion) {
  const re = new RegExp(`^(${depName.replace(/-/g, "\\-")})\\s*=\\s*\\{([^}]*)\\}`, "m");
  const match = text.match(re);
  if (!match) return text;
  const inner = match[2];
  if (!/\bpath\s*=/.test(inner)) return text;
  const rest = inner
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s && !s.startsWith("path"))
    .join(", ");
  const table = rest ? `{ version = "${newVersion}", ${rest} }` : `{ version = "${newVersion}" }`;
  return text.replace(re, `$1 = ${table}`);
}

function rewriteCargoToml(file) {
  let text = fs.readFileSync(file, "utf8");
  text = text.replace(/^version\s*=\s*.*$/m, `version = "${version}"`);

  for (const dep of ["tishlang_core", "tishlang_ui", "tishlang_runtime"]) {
    text = rewritePathDep(text, dep, core);
  }
  text = rewritePathDep(text, "tishlang_broker", broker);
  // Crate name is underscored (tishlang_apple_common); its directory stays hyphenated.
  text = rewritePathDep(text, "tishlang_apple_common", version);

  // Hard guard: a path dep left in the tarball makes the package unusable for
  // consumers (the path doesn't exist outside the monorepo). Fail, don't publish.
  const leftover = text
    .split("\n")
    .filter((l) => /^\s*[\w-]+\s*=.*\bpath\s*=/.test(l) && !l.trim().startsWith("#"));
  if (leftover.length > 0) {
    console.error(`ERROR: unrewritten path dependencies in ${file}:`);
    for (const l of leftover) console.error("  " + l.trim());
    process.exit(1);
  }

  fs.writeFileSync(file, text);
}

// ---- staging -----------------------------------------------------------------

function stage(pkg) {
  const src = path.join(ROOT, pkg.dir);
  const out = path.join(ROOT, "dist-npm", pkg.localName);
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });

  fs.copyFileSync(path.join(src, "package.json"), path.join(out, "package.json"));
  fs.copyFileSync(path.join(src, "Cargo.toml"), path.join(out, "Cargo.toml"));
  fs.cpSync(path.join(src, "src"), path.join(out, "src"), { recursive: true });
  for (const extra of ["README.md", "lsp-pragmas.d.tish"]) {
    if (fs.existsSync(path.join(src, extra))) {
      fs.copyFileSync(path.join(src, extra), path.join(out, extra));
    }
  }
  fs.copyFileSync(path.join(ROOT, "LICENSE"), path.join(out, "LICENSE"));

  rewriteCargoToml(path.join(out, "Cargo.toml"));

  const pkgJsonPath = path.join(out, "package.json");
  const p = JSON.parse(fs.readFileSync(pkgJsonPath, "utf8"));
  if (p.name !== pkg.localName) {
    console.error(`ERROR: ${pkg.dir}/package.json name is "${p.name}", expected "${pkg.localName}" (the compiler resolves the local-debug tish:${pkg.localName.replace(/^tish-/, "")} spec by that exact name).`);
    process.exit(1);
  }
  if (!p.tish?.module) {
    console.error(`ERROR: ${pkg.dir}/package.json is missing "tish": { "module": true } — published package would not resolve as a native module.`);
    process.exit(1);
  }
  delete p.private;
  delete p.scripts;
  // Publish under the npm scope; consumers import "@tishlang/tish-macos" and the
  // compiler matches node_modules/@tishlang/tish-macos by this exact name.
  p.name = pkg.npmName;
  p.version = version;
  p.license = "PIF";
  p.repository = p.repository ?? { type: "git", url: "https://github.com/tishlang/tish-apple.git" };
  p.publishConfig = { access: "public", ...(p.publishConfig ?? {}) };
  p.files = ["Cargo.toml", "src/", "LICENSE", "README.md", "lsp-pragmas.d.tish"];
  fs.writeFileSync(pkgJsonPath, JSON.stringify(p, null, 2) + "\n");

  return out;
}

// ---- publish -----------------------------------------------------------------

function alreadyPublished(name) {
  try {
    execFileSync("npm", ["view", `${name}@${version}`, "version"], { stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
}

let failed = false;
for (const pkg of PACKAGES) {
  if (only && pkg.localName !== only && pkg.npmName !== only) continue;
  console.log(`\n=== ${pkg.npmName}@${version} (core ${core}, broker ${broker}) ===`);

  if (!dryRun && alreadyPublished(pkg.npmName)) {
    console.log(`${pkg.npmName}@${version} is already on npm — skipping.`);
    continue;
  }

  const out = stage(pkg);
  console.log(`staged: ${path.relative(ROOT, out)}`);

  try {
    if (dryRun) {
      execFileSync("npm", ["pack"], { cwd: out, stdio: "inherit" });
      console.log(`dry-run OK — tarball in ${path.relative(ROOT, out)}`);
    } else {
      execFileSync("npm", ["publish", "--access", "public"], { cwd: out, stdio: "inherit" });
      console.log(`published ${pkg.npmName}@${version}`);
    }
  } catch (err) {
    console.error(`ERROR: npm ${dryRun ? "pack" : "publish"} failed for ${pkg.npmName}: ${err.message}`);
    failed = true;
  }
}

process.exit(failed ? 1 : 0);
