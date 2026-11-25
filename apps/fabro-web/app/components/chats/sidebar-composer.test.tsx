import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { createElement } from "react";
import TestRenderer, { act } from "react-test-renderer";

import { setupReactTestEnv } from "../../lib/test-utils";

// Partial-mock: bun's mock.module is process-wide (restore does not undo
// it) — spread the REAL module so every export stays available to later
// files in the same run; only the composer primitives are replaced. The
// Input capture records the props SidebarComposer hands the autosizing
// textarea, which is exactly the behavior contract under test
// (work#1535): the default height and the growth clamp are the minRows/
// maxRows contract the composer gives react-textarea-autosize — the
// autosizer grows with typed content between them, and the
// ComposerPrimitive integration itself stays untouched.
const realAssistantUi = await import("@assistant-ui/react");
let composerInputProps: Record<string, unknown> | null = null;
let composerRootClassName: string | null = null;
mock.module("@assistant-ui/react", () => ({
  ...realAssistantUi,
  ComposerPrimitive: {
    ...realAssistantUi.ComposerPrimitive,
    Root: (props: { className?: string; children?: React.ReactNode }) => {
      composerRootClassName = props.className ?? null;
      return createElement("div", null, props.children);
    },
    Input: (props: Record<string, unknown>) => {
      composerInputProps = props;
      return createElement("textarea", { "data-probe": "composer-input" });
    },
    // The mocked ThreadPrimitive.If always renders its children, so the
    // run-state buttons would mount their real provider-bound selves —
    // inert stand-ins keep the render provider-free (neither is under
    // test; the props capture is).
    Cancel: () => createElement("button", { type: "button" }),
    Send: () => createElement("button", { type: "submit" }),
  },
  ThreadPrimitive: {
    ...realAssistantUi.ThreadPrimitive,
    If: (props: { children?: React.ReactNode }) =>
      createElement("div", null, props.children),
  },
}));

const { default: SidebarComposer } = await import("./sidebar-composer");
mock.restore();

describe("SidebarComposer height (work#1535)", () => {
  let teardown: () => void = () => {};
  let renderer: TestRenderer.ReactTestRenderer | null = null;

  beforeEach(() => {
    teardown = setupReactTestEnv();
    composerInputProps = null;
    composerRootClassName = null;
  });

  afterEach(() => {
    act(() => {
      renderer?.unmount();
    });
    renderer = null;
    teardown();
  });

  test("the input starts four rows tall and leaves auto-grow to the autosizer", () => {
    act(() => {
      renderer = TestRenderer.create(createElement(SidebarComposer));
    });

    expect(composerInputProps).toBeDefined();
    // ~4 rows tall by default — the operator's minimum working surface for
    // a multi-line prompt.
    expect(composerInputProps!.minRows).toBe(4);
    // No fixed `rows` pin: the autosizer owns the height so the input
    // keeps growing as more lines are typed.
    expect(composerInputProps!.rows).toBeUndefined();
  });

  test("the pill stretches to its container instead of shrink-wrapping (work#1687)", () => {
    act(() => {
      renderer = TestRenderer.create(createElement(SidebarComposer));
    });

    // The composer sits in assistant-ui's column-flex footer — a
    // `w-auto` pill shrink-wraps to the textarea's intrinsic width
    // (~20ch), which starved the operator's input surface.
    expect(composerRootClassName).toContain("w-full");
    expect(composerRootClassName).not.toContain("w-auto");
  });
});
