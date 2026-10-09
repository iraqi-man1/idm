#!/usr/bin/env node
// Prepares everything a release build bundles:
//   * FFmpeg, ffprobe and yt-dlp, downloaded from the pinned URLs in
//     resources/sidecars.lock.json and verified with SHA-256 (a mismatch
//     stops the build: nothing unverified is ever bundled),
//   * the native messaging host (velox-nmh), built in release mode,
//   * the browser extensions.
// Tools land in apps/desktop/src-tauri/binaries/<name>-<target>[.exe], the
// layout Tauri's `externalBin` expects (see tauri.release.conf.json).
//
// Usage:
//   node scripts/prepare-release.mjs [--target <triple>] [--skip-host] [--skip-extensions]
//   node scripts/prepare-release.mjs update-lock   # refresh hashes from upstream checksum lists
//
// Needs curl, tar and (on Linux/macOS, for zip archives) unzip.

import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lockPath = path.join(root, "resources", "sidecars.lock.json");
const binDir = path.join(root, "apps", "desktop", "src-tauri", "binaries");
const cacheDir = path.join(root, "target", "sidecars-cache");
const TOOLS = ["ffmpeg", "ffprobe", "yt-dlp"];

function fail(msg) {
  console.error(`error: ${msg}`);
  process.exit(1);
}

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { stdio: "inherit", shell: process.platform === "win32" && cmd === "pnpm", ...opts });
  if (r.status !== 0) fail(`${cmd} ${args.join(" ")} failed (${r.status ?? r.error})`);
}

function hostTriple() {
  return /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))[1];
}

function sha256(file) {
  const h = createHash("sha256");
  const fd = fs.openSync(file, "r");
  const buf = Buffer.alloc(1 << 20);
  let n;
  while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) h.update(buf.subarray(0, n));
  fs.closeSync(fd);
  return h.digest("hex");
}

/** Download `url` (curl honours HTTPS_PROXY) into the cache, verified. */
function fetchVerified(entry, tool) {
  fs.mkdirSync(cacheDir, { recursive: true });
  const cached = path.join(cacheDir, `${entry.sha256}-${entry.asset}`);
  if (fs.existsSync(cached) && sha256(cached) === entry.sha256) return cached;
  const tmp = `${cached}.part`;
  console.log(`downloading ${entry.url}`);
  run("curl", ["-fsSL", "--retry", "3", "--proto", "=https", "--tlsv1.2", "-o", tmp, entry.url]);
  const got = sha256(tmp);
  if (got !== entry.sha256) {
    fs.rmSync(tmp, { force: true });
    fail(
      `${tool}: SHA-256 mismatch for ${entry.asset}\n  expected ${entry.sha256}\n  got      ${got}\n` +
        "The upstream file changed. Run `node scripts/prepare-release.mjs update-lock`, review the diff and commit it.",
    );
  }
  fs.renameSync(tmp, cached);
  return cached;
}

function extract(archive, kind, member, dest) {
  const dir = fs.mkdtempSync(path.join(cacheDir, "x-"));
  try {
    if (kind === "zip" && process.platform !== "win32") run("unzip", ["-o", "-q", archive, member, "-d", dir]);
    else run("tar", [kind === "tar.xz" ? "-xJf" : "-xf", archive, "-C", dir, member]);
    fs.copyFileSync(path.join(dir, member), dest);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

function prepareSidecars(lock, target) {
  const entries = lock.targets[target];
  if (!entries) fail(`no sidecars are pinned for ${target} (resources/sidecars.lock.json)`);
  const exe = target.includes("windows") ? ".exe" : "";
  fs.mkdirSync(binDir, { recursive: true });
  for (const tool of TOOLS) {
    const entry = entries[tool];
    if (!entry || !entry.sha256) {
      fail(`no verified source for ${tool} on ${target}; add its url and sha256 to resources/sidecars.lock.json first`);
    }
    const file = fetchVerified(entry, tool);
    const dest = path.join(binDir, `${tool}-${target}${exe}`);
    if (entry.archive) extract(file, entry.archive, entry.member, dest);
    else fs.copyFileSync(file, dest);
    if (entry.license_member) {
      // The tool's own license text ships next to the notices (LGPL).
      const licenses = path.join(binDir, "licenses");
      fs.mkdirSync(licenses, { recursive: true });
      extract(file, entry.archive, entry.license_member, path.join(licenses, `${tool}-LICENSE.txt`));
    }
    fs.chmodSync(dest, 0o755);
    console.log(`${tool}: ${lock.tools[tool].version} (${entry.sha256.slice(0, 12)}…) -> ${path.relative(root, dest)}`);
  }
}

function buildHost(target) {
  run("cargo", ["build", "--release", "--locked", "-p", "velox-nm", "--bin", "velox-nmh", "--target", target], { cwd: root });
  const exe = target.includes("windows") ? ".exe" : "";
  const built = path.join(root, "target", target, "release", `velox-nmh${exe}`);
  const dest = path.join(binDir, `velox-nmh-${target}${exe}`);
  fs.mkdirSync(binDir, { recursive: true });
  fs.copyFileSync(built, dest);
  fs.chmodSync(dest, 0o755);
  console.log(`velox-nmh -> ${path.relative(root, dest)}`);
}

function updateLock(lock) {
  let changed = 0;
  for (const tool of TOOLS) {
    const list = execFileSync("curl", ["-fsSL", "--proto", "=https", lock.tools[tool].checksums], { encoding: "utf8" });
    const sums = new Map(
      list
        .split("\n")
        .map((l) => l.trim().split(/\s+/))
        .filter((p) => p.length === 2)
        .map(([h, n]) => [n.replace(/^\*/, ""), h.toLowerCase()]),
    );
    for (const [target, entries] of Object.entries(lock.targets)) {
      const e = entries[tool];
      if (!e?.asset) continue;
      const h = sums.get(e.asset);
      if (!h) fail(`${tool}: ${e.asset} is not listed in ${lock.tools[tool].checksums}`);
      if (h !== e.sha256) {
        console.log(`${target} ${tool}: ${e.sha256 ?? "none"} -> ${h}`);
        e.sha256 = h;
        changed++;
      }
    }
  }
  fs.writeFileSync(lockPath, `${JSON.stringify(lock, null, 2)}\n`);
  console.log(changed ? `${changed} entries updated; review and commit the lock file.` : "lock file is up to date");
}

const args = process.argv.slice(2);
const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
if (args[0] === "update-lock") {
  updateLock(lock);
} else {
  const ti = args.indexOf("--target");
  const target = ti >= 0 ? args[ti + 1] : hostTriple();
  prepareSidecars(lock, target);
  if (!args.includes("--skip-host")) buildHost(target);
  if (!args.includes("--skip-extensions")) run("pnpm", ["--filter", "extensions", "build"], { cwd: root });
  console.log(`ready: build with \`pnpm --filter desktop tauri build --target ${target} --config src-tauri/tauri.release.conf.json\``);
}
