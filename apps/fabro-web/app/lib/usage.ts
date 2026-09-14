import type { Cost, TokenCounts, Usage } from "@qltysh/fabro-api-client";

export interface UsageTokenBucket {
  label: string;
  value: number;
}

/** The sum of the five disjoint token buckets. */
export function totalTokens(usage: Usage): number {
  const { input, output, reasoning, cache_read, cache_write } = usage.tokens;
  return input + output + reasoning + cache_read + cache_write;
}

/** Output tokens as they are priced: completion plus reasoning. */
export function billableOutputTokens(tokens: TokenCounts): number {
  return tokens.output + tokens.reasoning;
}

/** The disjoint token buckets shown in every usage breakdown. */
export function usageTokenBuckets(usage: Usage): UsageTokenBucket[] {
  return [
    { label: "Cache read", value: usage.tokens.cache_read },
    { label: "Cache creation", value: usage.tokens.cache_write },
    { label: "Uncached", value: usage.tokens.input },
    { label: "Output", value: billableOutputTokens(usage.tokens) },
  ];
}

/** Whether the usage carries any tokens or a cost. */
export function hasUsage(usage: Usage): boolean {
  return totalTokens(usage) !== 0 || (usage.cost?.usd_micros ?? 0) !== 0;
}

/** The cost in USD micros, when the usage carries one. */
export function costUsdMicros(usage: Usage | null | undefined): number | undefined {
  return usage?.cost?.usd_micros;
}

/**
 * A short tag for a cost that did not come from the catalog: `reported`
 * when the provider gave the figure, `summed` when it was assembled from
 * differently sourced parts. Catalog estimates carry no tag.
 */
export function costSourceTag(cost: Cost | null | undefined): string | null {
  switch (cost?.source) {
    case "provider":
      return "reported";
    case "application":
      return "summed";
    default:
      return null;
  }
}
