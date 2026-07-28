import type { BilledTokenCounts } from "@qltysh/fabro-api-client";

import { formatTokenCount } from "./format";

export interface BillingTokenBucket {
  label: "Cache read" | "Cache creation" | "Uncached" | "Output";
  value: number;
}

export function billableOutputTokens(billing: BilledTokenCounts): number {
  return billing.output_tokens + billing.reasoning_tokens;
}

/**
 * Return the disjoint billing buckets in their shared display order.
 * Reasoning tokens are billed as output tokens.
 */
export function billingTokenBuckets(billing: BilledTokenCounts): BillingTokenBucket[] {
  return [
    { label: "Cache read", value: billing.cache_read_tokens },
    { label: "Cache creation", value: billing.cache_write_tokens },
    { label: "Uncached", value: billing.input_tokens },
    { label: "Output", value: billableOutputTokens(billing) },
  ];
}

export function hasBillingUsage(billing: BilledTokenCounts): boolean {
  return (
    billing.input_tokens !== 0 ||
    billing.output_tokens !== 0 ||
    billing.reasoning_tokens !== 0 ||
    billing.cache_read_tokens !== 0 ||
    billing.cache_write_tokens !== 0 ||
    billing.total_tokens !== 0 ||
    (billing.total_usd_micros ?? 0) !== 0
  );
}

export function formatBillingTokenCount(value: number): string {
  return value === 0 ? "0" : formatTokenCount(value, { compactDecimal: true });
}
