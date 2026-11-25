import { describe, expect, test } from "bun:test";

import {
  recallAskSessionId,
  rememberAskSessionId,
  restoreAskSelection,
} from "./ask-index";
import type { AskSessionSummary } from "./ask-index";

function session(id: string, updatedMinutesAgo: number): AskSessionSummary {
  const updated_at = new Date(
    Date.now() - updatedMinutesAgo * 60_000,
  ).toISOString();
  return {
    id,
    run_id: null,
    status: "idle",
    active_turn: null,
    capability: "full_access",
    created_at: updated_at,
    updated_at,
  };
}

describe("restoreAskSelection (work#1686)", () => {
  const index = [session("ses_new", 1), session("ses_old", 90)];

  test("the remembered conversation wins when the index still carries it", () => {
    expect(restoreAskSelection("ses_old", index)?.id).toBe("ses_old");
  });

  test("a deleted memory falls back to the most recently updated session", () => {
    expect(restoreAskSelection("ses_gone", index)?.id).toBe("ses_new");
  });

  test("no memory at all lands on the most recently updated session", () => {
    expect(restoreAskSelection(null, index)?.id).toBe("ses_new");
  });

  test("an empty index restores nothing", () => {
    expect(restoreAskSelection("ses_old", [])).toBeNull();
    expect(restoreAskSelection(null, [])).toBeNull();
  });
});

describe("the last-active session memory", () => {
  function fakeStorage(): {
    store: Map<string, string>;
    storage: {
      getItem: (key: string) => string | null;
      setItem: (key: string, value: string) => void;
    };
  } {
    const store = new Map<string, string>();
    return {
      store,
      storage: {
        getItem: (key) => store.get(key) ?? null,
        setItem: (key, value) => void store.set(key, value),
      },
    };
  }

  test("recall degrades to null when storage is unavailable", () => {
    expect(recallAskSessionId(undefined)).toBeNull();
    expect(() => rememberAskSessionId("ses_x", undefined)).not.toThrow();
  });

  test("remembering then recalling round-trips the id", () => {
    const { storage } = fakeStorage();
    rememberAskSessionId("ses_round_trip", storage);
    expect(recallAskSessionId(storage)).toBe("ses_round_trip");
  });
});
