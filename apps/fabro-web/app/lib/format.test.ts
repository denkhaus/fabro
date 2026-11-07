import { describe, expect, test } from "bun:test";

import { formatTileTimestamp } from "./format";

describe("formatTileTimestamp", () => {
  test("zero-pads single-digit day, month, hours and minutes", () => {
    // 5 March 2026, 07:08:45 local — every two-digit slot is single-digit.
    const iso = new Date(2026, 2, 5, 7, 8, 45).toISOString();
    expect(formatTileTimestamp(iso)).toBe("05.03.2026 07:08");
  });

  test("renders dd.MM.yyyy HH:mm and drops the seconds", () => {
    const iso = new Date(2026, 11, 20, 18, 33, 12).toISOString();
    expect(formatTileTimestamp(iso)).toBe("20.12.2026 18:33");
  });

  test("is locale-independent: exactly dd.MM.yyyy HH:mm, no comma/second variant", () => {
    const iso = new Date(2026, 0, 9, 9, 5, 5).toISOString();
    expect(formatTileTimestamp(iso)).toMatch(/^\d{2}\.\d{2}\.\d{4} \d{2}:\d{2}$/);
  });

  test("falls back to the input when unparseable", () => {
    expect(formatTileTimestamp("not-a-timestamp")).toBe("not-a-timestamp");
  });
});
