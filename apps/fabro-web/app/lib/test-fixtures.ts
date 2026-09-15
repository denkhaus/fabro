import type { Cost, Principal, TokenCounts, Usage } from "@qltysh/fabro-api-client";

export const TEST_PRINCIPAL: Principal = {
  kind:        "user",
  identity:    { issuer: "fabro:test", subject: "test-user" },
  login:       "test",
  auth_method: "dev_token",
};

export function makeTokenCounts(overrides: Partial<TokenCounts> = {}): TokenCounts {
  return {
    input: 0,
    output: 0,
    reasoning: 0,
    cache_read: 0,
    cache_write: 0,
    ...overrides,
  };
}

/** A usage with the given token buckets and, when `cost` is given, a catalog cost. */
export function makeUsage(
  tokens: Partial<TokenCounts> = {},
  cost?: number | Cost,
): Usage {
  const usage: Usage = { tokens: makeTokenCounts(tokens) };
  if (typeof cost === "number") usage.cost = { usd_micros: cost, source: "catalog" };
  else if (cost) usage.cost = cost;
  return usage;
}
