import type { BilledTokenCounts, Principal } from "@qltysh/fabro-api-client";

export const TEST_PRINCIPAL: Principal = {
  kind:        "user",
  identity:    { issuer: "fabro:test", subject: "test-user" },
  login:       "test",
  auth_method: "dev_token",
};

export function testBilledTokenCounts(
  overrides: Partial<BilledTokenCounts> = {},
): BilledTokenCounts {
  return {
    input_tokens: 0,
    output_tokens: 0,
    total_tokens: 0,
    reasoning_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    ...overrides,
  };
}
