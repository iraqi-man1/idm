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
//   node scripts/prepare-release.mjs update-lock [--btbn-tag autobuild-...]
//                                                  # re-pin FFmpeg, refresh hashes
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

const BTBN = "BtbN/FFmpeg-Builds";
// "ffmpeg-n8.1.3-9-g29e619e767-win64-lgpl-8.1.zip" (dated) or
// "ffmpeg-n8.1-latest-win64-lgpl-8.1.zip": release builds start with a
// lowercase "n" (master builds are "N-..."); the variant ("lgpl-8.1") names
// the release branch.
const BTBN_ASSET = /^ffmpeg-n[\d.]+-.+-(win64|winarm64|linux64|linuxarm64)-(.+)\.(zip|tar\.xz)$/;

function curlText(url, headers = []) {
  return execFileSync("curl", ["-fsSL", "--proto", "=https", ...headers.flatMap((h) => ["-H", h]), url], {
    encoding: "utf8",
    maxBuffer: 64 << 20,
  });
}

// BtbN keeps the last build of each month for two years, but only 14 daily
// builds and a floating "latest" that changes every day. Pin the last build
// of the most recent completed month so the URLs and hashes stay valid.
function lastMonthlyBuild() {
  const headers = ["Accept: application/vnd.github+json"];
  if (process.env.GITHUB_TOKEN) headers.push(`Authorization: Bearer ${process.env.GITHUB_TOKEN}`);
  const releases = JSON.parse(curlText(`https://api.github.com/repos/${BTBN}/releases?per_page=100`, headers));
  const thisMonth = new Date().toISOString().slice(0, 7);
  const tags = releases
    .map((r) => r.tag_name)
    .filter((t) => /^autobuild-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}$/.test(t) && t.slice(10, 17) < thisMonth)
    .sort();
  if (!tags.length) fail(`no completed-month build found in ${BTBN}`);
  return tags.at(-1);
}

// `tag` pins a given build (it should be the last build of a month, or it
// disappears after 14 days); without it, the newest such build is looked up.
function pinMonthlyFfmpeg(lock, tag = lastMonthlyBuild()) {
  if (!/^autobuild-\d{4}-\d{2}-\d{2}-\d{2}-\d{2}$/.test(tag)) fail(`not a BtbN build tag: ${tag}`);
  const checksums = `https://github.com/${BTBN}/releases/download/${tag}/checksums.sha256`;
  const assets = curlText(checksums)
    .split("\n")
    .map((l) => l.trim().split(/\s+/)[1]?.replace(/^\*/, ""))
    .filter(Boolean);
  for (const tool of ["ffmpeg", "ffprobe"]) {
    lock.tools[tool].checksums = checksums;
    lock.tools[tool].version = lock.tools[tool].version.replace(/ \(BtbN.*$/, ` (BtbN LGPL static build, ${tag})`);
    for (const [target, entries] of Object.entries(lock.targets)) {
      const e = entries[tool];
      const m = e?.asset?.match(BTBN_ASSET);
      if (!m) continue;
      const [, platform, variant, ext] = m;
      const found = assets.filter((a) => {
        const n = a.match(BTBN_ASSET);
        return n && n[1] === platform && n[2] === variant && n[3] === ext;
      });
      if (found.length !== 1) fail(`${target} ${tool}: expected one ${platform} ${variant} asset in ${tag}, found ${found.length}`);
      const asset = found[0];
      const stem = asset.slice(0, -(ext.length + 1));
      const exe = path.basename(e.member);
      if (asset !== e.asset) console.log(`${target} ${tool}: ${e.asset} -> ${tag}/${asset}`);
      Object.assign(e, {
        url: `https://github.com/${BTBN}/releases/download/${tag}/${asset}`,
        asset,
        member: `${stem}/bin/${exe}`,
        license_member: `${stem}/LICENSE.txt`,
      });
    }
  }
}

function updateLock(lock, btbnTag) {
  pinMonthlyFfmpeg(lock, btbnTag);
  let changed = 0;
  for (const tool of TOOLS) {
    const list = curlText(lock.tools[tool].checksums);
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
  console.log(
    changed
      ? `${changed} entries updated; run \`node scripts/third-party-notices.mjs\`, then review and commit both files.`
      : "lock file is up to date",
  );
}

const args = process.argv.slice(2);
const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
if (args[0] === "update-lock") {
  const ti = args.indexOf("--btbn-tag");
  updateLock(lock, ti >= 0 ? args[ti + 1] : undefined);
} else {
  const ti = args.indexOf("--target");
  const target = ti >= 0 ? args[ti + 1] : hostTriple();
  prepareSidecars(lock, target);
  if (!args.includes("--skip-host")) buildHost(target);
  if (!args.includes("--skip-extensions")) run("pnpm", ["--filter", "extensions", "build"], { cwd: root });
  console.log(`ready: build with \`pnpm --filter desktop tauri build --target ${target} --config src-tauri/tauri.release.conf.json\``);
}
