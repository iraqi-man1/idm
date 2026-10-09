import { describe, expect, it } from "vitest";
import ar from "./ar.json";
import en from "./en.json";

type Tree = { [k: string]: string | Tree };

function keys(t: Tree, prefix = ""): string[] {
  return Object.entries(t).flatMap(([k, v]) => (typeof v === "string" ? [prefix + k] : keys(v, `${prefix}${k}.`)));
}

/** Strip i18next plural suffixes so `files_one` and `files_few` compare as `files`. */
const base = (k: string) => k.replace(/_(zero|one|two|few|many|other)$/, "");

describe("translations", () => {
  it("Arabic covers every English key", () => {
    const arKeys = new Set(keys(ar as Tree).map(base));
    const missing = keys(en as Tree).map(base).filter((k) => !arKeys.has(k));
    expect(missing).toEqual([]);
  });
  it("Arabic has no stale keys", () => {
    const enKeys = new Set(keys(en as Tree).map(base));
    expect(keys(ar as Tree).map(base).filter((k) => !enKeys.has(k))).toEqual([]);
  });
  it("interpolation placeholders match", () => {
    const flat = (t: Tree) => Object.fromEntries(keys(t).map((k) => [k, k.split(".").reduce<Tree | string>((o, p) => (o as Tree)[p], t) as string]));
    const e = flat(en as Tree);
    const a = flat(ar as Tree);
    const vars = (s: string) => (s.match(/{{\w+}}/g) ?? []).filter((v) => v !== "{{count}}").sort().join(",");
    for (const [k, v] of Object.entries(e)) {
      const match = k in a ? k : Object.keys(a).find((ak) => base(ak) === base(k));
      if (match) expect(vars(a[match]), k).toBe(vars(v));
    }
  });
});
