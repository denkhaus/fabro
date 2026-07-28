import { describe, expect, test } from "bun:test";

import {
  billingTokenBuckets,
  formatBillingTokenCount,
  hasBillingUsage,
} from "./billing";
import { testBilledTokenCounts } from "./test-fixtures";

describe("billingTokenBuckets", () => {
  test("returns the shared display order and folds reasoning into output", () => {
    expect(
      billingTokenBuckets(
        testBilledTokenCounts({
          input_tokens: 10,
          output_tokens: 20,
          reasoning_tokens: 5,
          cache_read_tokens: 30,
          cache_write_tokens: 40,
        }),
      ),
    ).toEqual([
      { label: "Cache read", value: 30 },
      { label: "Cache creation", value: 40 },
      { label: "Uncached", value: 10 },
      { label: "Output", value: 25 },
    ]);
  });
});

describe("hasBillingUsage", () => {
  test("includes cache-only and cost-only usage", () => {
    expect(hasBillingUsage(testBilledTokenCounts())).toBe(false);
    expect(
      hasBillingUsage(testBilledTokenCounts({ cache_read_tokens: 1 })),
    ).toBe(true);
    expect(
      hasBillingUsage(testBilledTokenCounts({ total_usd_micros: 1 })),
    ).toBe(true);
  });
});

describe("formatBillingTokenCount", () => {
  test("formats zero without a fractional suffix", () => {
    expect(formatBillingTokenCount(0)).toBe("0");
    expect(formatBillingTokenCount(1_200)).toBe("1.2k");
  });
});
