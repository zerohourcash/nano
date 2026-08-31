import { describe, expect, it } from "vitest";
import { firstUsableInvite, inviteExpiryLabel } from "./invite";

describe("firstUsableInvite", () => {
  it("skips expired and exhausted codes", () => {
    const list = [
      { token: "expired", usable: false },
      { token: "spent", usable: false },
      { token: "good", usable: true },
    ];
    expect(firstUsableInvite(list)?.token).toBe("good");
  });

  it("falls back to counters when the server omits `usable`", () => {
    const list = [
      { token: "spent", usedCount: 5, maxUses: 5 },
      { token: "fresh", usedCount: 0, maxUses: 5 },
    ];
    expect(firstUsableInvite(list)?.token).toBe("fresh");
  });

  it("returns nothing when every code is dead", () => {
    expect(firstUsableInvite([{ token: "x", revoked: true }])).toBeUndefined();
    expect(firstUsableInvite([])).toBeUndefined();
    expect(firstUsableInvite(undefined)).toBeUndefined();
  });
});

describe("inviteExpiryLabel", () => {
  it("has no label without an expiry", () => {
    expect(inviteExpiryLabel(null)).toBeNull();
    expect(inviteExpiryLabel(undefined)).toBeNull();
  });

  it("ignores an unparsable date instead of printing «Invalid Date»", () => {
    expect(inviteExpiryLabel("не дата")).toBeNull();
  });

  it("formats a real timestamp", () => {
    expect(inviteExpiryLabel("2026-09-02T10:00:00Z")).toContain("2026");
  });
});
