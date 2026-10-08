// Builds the Chromium and Firefox extensions from shared sources.
//   node build.mjs          -> extensions/{chromium,firefox}/dist
//   node build.mjs --zip    -> also extensions/velox-{chromium,firefox}-<version>.zip
import { build } from "esbuild";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const root = path.dirname(fileURLToPath(import.meta.url));
const shared = path.join(root, "shared");
const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const targets = ["chromium", "firefox"];
const zip = process.argv.includes("--zip");

function merge(a, b) {
  const out = { ...a };
  for (const [k, v] of Object.entries(b)) {
    out[k] = v && typeof v === "object" && !Array.isArray(v) && a[k] && typeof a[k] === "object" ? merge(a[k], v) : v;
  }
  return out;
}

function copyDir(from, to) {
  fs.mkdirSync(to, { recursive: true });
  for (const e of fs.readdirSync(from, { withFileTypes: true })) {
    const s = path.join(from, e.name);
    const d = path.join(to, e.name);
    if (e.isDirectory()) copyDir(s, d);
    else fs.copyFileSync(s, d);
  }
}

// --- minimal ZIP writer (deflate), enough for store uploads -------------------
const CRC_TABLE = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function listFiles(dir, base = dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    return e.isDirectory() ? listFiles(p, base) : [path.relative(base, p).split(path.sep).join("/")];
  }).sort();
}
function writeZip(dir, out) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const name of listFiles(dir)) {
    const data = fs.readFileSync(path.join(dir, name));
    const comp = zlib.deflateRawSync(data, { level: 9 });
    const nameBuf = Buffer.from(name, "utf8");
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6); // UTF-8 names
    local.writeUInt16LE(8, 8); // deflate
    local.writeUInt16LE(0, 10); // time 00:00 (fixed for reproducible archives)
    local.writeUInt16LE(0x21, 12); // date 1980-01-01
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(comp.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    parts.push(local, nameBuf, comp);
    const cen = Buffer.alloc(46);
    cen.writeUInt32LE(0x02014b50, 0);
    cen.writeUInt16LE(20, 4);
    cen.writeUInt16LE(20, 6);
    cen.writeUInt16LE(0x0800, 8);
    cen.writeUInt16LE(8, 10);
    cen.writeUInt16LE(0, 12);
    cen.writeUInt16LE(0x21, 14);
    cen.writeUInt32LE(crc, 16);
    cen.writeUInt32LE(comp.length, 20);
    cen.writeUInt32LE(data.length, 24);
    cen.writeUInt16LE(nameBuf.length, 28);
    cen.writeUInt32LE(offset, 42);
    central.push(cen, nameBuf);
    offset += local.length + nameBuf.length + comp.length;
  }
  const cenBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(central.length / 2, 8);
  end.writeUInt16LE(central.length / 2, 10);
  end.writeUInt32LE(cenBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  fs.writeFileSync(out, Buffer.concat([...parts, cenBuf, end]));
}

const base = JSON.parse(fs.readFileSync(path.join(shared, "manifest.base.json"), "utf8"));
for (const target of targets) {
  const dist = path.join(root, target, "dist");
  fs.rmSync(dist, { recursive: true, force: true });
  fs.mkdirSync(dist, { recursive: true });
  await build({
    entryPoints: {
      background: path.join(shared, "src/background.ts"),
      content: path.join(shared, "src/content.ts"),
      popup: path.join(shared, "src/popup.ts"),
      options: path.join(shared, "src/options.ts"),
    },
    outdir: dist,
    bundle: true,
    format: "iife",
    target: target === "firefox" ? "firefox115" : "chrome110",
    minify: true,
    sourcemap: false,
    legalComments: "none",
    alias: { "@bindings": path.join(root, "../apps/desktop/src/bindings") },
    define: { "process.env.NODE_ENV": '"production"' },
    logLevel: "warning",
  });
  for (const f of ["popup.html", "options.html", "ui.css"]) fs.copyFileSync(path.join(shared, "pages", f), path.join(dist, f));
  copyDir(path.join(shared, "icons"), path.join(dist, "icons"));
  copyDir(path.join(shared, "_locales"), path.join(dist, "_locales"));
  const override = JSON.parse(fs.readFileSync(path.join(root, target, "manifest.json"), "utf8"));
  const manifest = merge(base, override);
  manifest.version = pkg.version;
  fs.writeFileSync(path.join(dist, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");
  const digest = createHash("sha256");
  for (const f of listFiles(dist)) digest.update(f).update(fs.readFileSync(path.join(dist, f)));
  console.log(`built ${target} -> ${path.relative(process.cwd(), dist)} (sha256 ${digest.digest("hex").slice(0, 16)})`);
  if (zip) {
    const out = path.join(root, `velox-${target}-${pkg.version}.zip`);
    writeZip(dist, out);
    console.log(`packed ${path.relative(process.cwd(), out)}`);
  }
}
