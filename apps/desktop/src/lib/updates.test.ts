import { describe, expect, it } from "vitest";
import { AUTO_CHECK_INTERVAL_MS, autoCheckDue } from "./updates";

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
