import { describe, expect, it } from "vitest";
import { parseDueInput, toDateTimeLocal } from "./due-date";

describe("parseDueInput", () => {
  it("treats an empty value as «без срока»", () => {
    expect(parseDueInput("   ")).toEqual({ ok: true });
  });

  it("accepts the prompt format with a space separator", () => {
    const result = parseDueInput("2026-09-01 18:00");
    expect(result.ok).toBe(true);
    expect(result.ok && result.iso).toBe(
      new Date("2026-09-01T18:00").toISOString(),
    );
  });

  it("reports garbage instead of throwing on toISOString", () => {
    expect(parseDueInput("завтра")).toEqual({ ok: false });
    expect(parseDueInput("2026-13-45 99:99")).toEqual({ ok: false });
  });
});

describe("toDateTimeLocal", () => {
  it("keeps local wall-clock time instead of shifting to UTC", () => {
    const local = new Date(2026, 8, 1, 9, 30, 0, 0);
    expect(toDateTimeLocal(local)).toBe("2026-09-01T09:30");
  });

  it("round-trips through the datetime-local input value", () => {
    const local = new Date(2026, 0, 5, 7, 5, 0, 0);
    const asInput = toDateTimeLocal(local);
    const parsed = parseDueInput(asInput);
    expect(parsed.ok && parsed.iso).toBe(local.toISOString());
  });
});
