import { describe, expect, it, vi } from "vitest";
import { AUTO_CHECK_INTERVAL_MS, autoCheckDue, createAutoChecker, type FoundUpdate } from "./updates";

describe("autoCheckDue", () => {
  const base = { configured: true, enabled: true, lastCheck: null, now: 1_000_000 };

  it("checks once at start when configured and enabled", () => {
    expect(autoCheckDue(base)).toBe(true);
  });

  it("never checks without a signing key or when switched off", () => {
    expect(autoCheckDue({ ...base, configured: false })).toBe(false);
    expect(autoCheckDue({ ...base, enabled: false })).toBe(false);
  });

  it("waits a day between checks", () => {
    const lastCheck = base.now;
    expect(autoCheckDue({ ...base, lastCheck, now: lastCheck + AUTO_CHECK_INTERVAL_MS - 1 })).toBe(false);
    expect(autoCheckDue({ ...base, lastCheck, now: lastCheck + AUTO_CHECK_INTERVAL_MS })).toBe(true);
  });
});

describe("createAutoChecker", () => {
  const on = { configured: true, enabled: true };

  function setup(results: Array<FoundUpdate | null | Error>) {
    let clock = 0;
    const check = vi.fn(async () => {
      const r = results.shift() ?? null;
      if (r instanceof Error) throw r;
      return r;
    });
    const notify = vi.fn();
    const onError = vi.fn();
    const tick = createAutoChecker({ check, notify, onError, now: () => clock });
    return { tick, check, notify, onError, advance: (ms: number) => (clock += ms) };
  }

  it("checks at start and then only once a day", async () => {
    const t = setup([null, null]);
    await t.tick(on);
    await t.tick(on);
    expect(t.check).toHaveBeenCalledTimes(1);
    t.advance(AUTO_CHECK_INTERVAL_MS);
    await t.tick(on);
    expect(t.check).toHaveBeenCalledTimes(2);
  });

  it("retries at the next wake-up after a failed check", async () => {
    const t = setup([new Error("offline"), { version: "0.2.0" }]);
    await t.tick(on);
    expect(t.onError).toHaveBeenCalledTimes(1);
    t.advance(60 * 60 * 1000);
    await t.tick(on);
    expect(t.check).toHaveBeenCalledTimes(2);
    expect(t.notify).toHaveBeenCalledWith({ version: "0.2.0" });
  });

  it("announces each version once", async () => {
    const t = setup([{ version: "0.2.0" }, { version: "0.2.0" }, { version: "0.3.0" }]);
    for (let i = 0; i < 3; i++) {
      await t.tick(on);
      t.advance(AUTO_CHECK_INTERVAL_MS);
    }
    expect(t.notify.mock.calls.map((c) => c[0].version)).toEqual(["0.2.0", "0.3.0"]);
  });

  it("does nothing when unconfigured or switched off, and never overlaps", async () => {
    const t = setup([null]);
    await t.tick({ configured: false, enabled: true });
    await t.tick({ configured: true, enabled: false });
    expect(t.check).not.toHaveBeenCalled();
    await Promise.all([t.tick(on), t.tick(on)]);
    expect(t.check).toHaveBeenCalledTimes(1);
  });
});
