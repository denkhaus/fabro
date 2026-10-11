import { afterEach, describe, expect, mock, test } from "bun:test";
import { createElement } from "react";
import TestRenderer, { act } from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";

import { ToastProvider } from "../components/toast";
import { setupReactTestEnv } from "../lib/test-utils";

let currentAutomations: any[] = [];
const replaceAutomationMock = mock(
  (_id: string, _revision: string, _body: unknown) =>
    Promise.resolve({ data: {} }),
);
const swrMutateMock = mock((_key: unknown) => Promise.resolve(undefined));
let teardownReactEnv: (() => void) | undefined;

mock.module("@headlessui/react", () => ({
  Dialog: ({ open, children }: any) =>
    open ? createElement("div", { role: "dialog" }, children) : null,
  DialogPanel: ({ children, ...props }: any) =>
    createElement("div", props, children),
  DialogTitle: ({ children, ...props }: any) =>
    createElement("h2", props, children),
  Switch: ({ checked, onChange, children, ...props }: any) =>
    createElement(
      "button",
      {
        ...props,
        type:           "button",
        role:           "switch",
        "aria-checked": checked,
        onClick:        () => onChange(!checked),
      },
      children,
    ),
  Listbox: ({ children, ...props }: any) =>
    createElement("div", props, children),
  ListboxButton: ({ children, ...props }: any) =>
    createElement("button", props, children),
  ListboxOptions: ({ children, ...props }: any) =>
    createElement("div", props, children),
  ListboxOption: ({ children, ...props }: any) =>
    createElement("div", props, children),
  Menu: ({ children }: any) => createElement("div", children),
  MenuButton: ({ children, ...props }: any) =>
    createElement("button", props, children),
  MenuItems: ({ children, ...props }: any) =>
    createElement("div", props, children),
  MenuItem: ({ children, ...props }: any) =>
    createElement("div", props, children),
}));

// Partial-mock: bun's mock.module is process-wide (restore does not
// undo it) — spread the REAL module so every export stays available
// to later files in the same run; override only what this test needs.
const realQueries = await import("../lib/queries");
mock.module("../lib/queries", () => ({
  ...realQueries,
  useAutomations: () => ({
    data: {
      data: currentAutomations,
      meta: { total: currentAutomations.length },
    },
    error: null,
    isLoading: false,
  }),
  useAutomation: (_id: unknown) => ({
    data: currentAutomations[0] ?? null,
    error: null,
    isLoading: false,
  }),
  useAutomationRuns: () => ({
    data: { data: [], meta: { total: 0 } },
    error: null,
    isLoading: false,
  }),
}));

// Partial-mock: bun's mock.module is process-wide (restore does not
// undo it) — spread the REAL module so every export stays available
// to later files in the same run; override only what this test needs.
const realApiClient = await import("../lib/api-client");
mock.module("../lib/api-client", () => ({
  ...realApiClient,
  // Only the API object this route hits is replaced; apiData/ApiError stay
  // REAL so later files in the same run test the real adapter behavior.
  automationsApi: {
    ...realApiClient.automationsApi,
    replaceAutomation: replaceAutomationMock,
    createAutomationRun: mock(() => Promise.resolve({ data: { id: "run_1" } })),
    deleteAutomation: mock(() => Promise.resolve({ data: {} })),
  },
}));

const realSwr = await import("swr");
mock.module("swr", () => ({
  ...realSwr,
  useSWRConfig: () => ({ mutate: swrMutateMock }),
}));

const { default: Automations } = await import("./automations");
const { default: AutomationDetail } = await import("./automation-detail");
mock.restore();

function makeAutomation(overrides: Record<string, unknown> = {}) {
  return {
    id:             "loop-fabro",
    revision:       "rev1",
    name:           "Loop",
    description:    null,
    environment_id: "toolchain",
    last_error:     null,
    target:         {
      kind:   "git",
      repo:   "denkhaus/fabro",
      branch: "denkhaus",
    },
    workflow:       "loop",
    on_overlap:     "skip",
    triggers:       [
      { type: "api", id: "manual", enabled: true },
      {
        type:       "schedule",
        id:         "schedule",
        enabled:    true,
        expression: "0 */2 * * *",
      },
    ],
    ...overrides,
  };
}

function buttonsOf(root: TestRenderer.ReactTestRenderer) {
  return root.root.findAll((node) => node.type === "button");
}

function textFromInstance(node: TestRenderer.ReactTestInstance): string {
  return node.children
    .map((child) => {
      if (typeof child === "string") return child;
      if (typeof child === "number") return String(child);
      return textFromInstance(child);
    })
    .join("");
}

function buttonByTitle(
  root: TestRenderer.ReactTestRenderer,
  title: string,
) {
  return buttonsOf(root).find((node) => node.props.title === title);
}

async function renderRoute() {
  const router = createMemoryRouter(
    [
      {
        path: "/automations",
        element: createElement(Automations),
      },
    ],
    { initialEntries: ["/automations"] },
  );
  let renderer!: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(
      createElement(
        ToastProvider,
        null,
        createElement(RouterProvider, { router }),
      ),
    );
  });
  return renderer;
}

async function renderDetail() {
  const router = createMemoryRouter(
    [
      {
        path: "/automations/:id",
        element: createElement(AutomationDetail),
      },
    ],
    { initialEntries: ["/automations/loop-fabro"] },
  );
  let renderer!: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(
      createElement(
        ToastProvider,
        null,
        createElement(RouterProvider, { router }),
      ),
    );
  });
  return renderer;
}

function allText(root: TestRenderer.ReactTestRenderer): string {
  const parts: string[] = [];
  const walk = (node: unknown, depth = 0) => {
    if (depth > 80 || node == null) return;
    if (typeof node === "string" || typeof node === "number") {
      parts.push(String(node));
      return;
    }
    if (Array.isArray(node)) {
      for (const child of node) walk(child, depth + 1);
      return;
    }
    if (typeof node === "object" && "children" in (node as Record<string, unknown>)) {
      walk((node as { children?: unknown }).children, depth + 1);
    }
  };
  walk(root.toJSON());
  return parts.join(" ");
}

describe("Automation breaker surface (fabro-3d97)", () => {
  afterEach(() => {
    teardownReactEnv?.();
    teardownReactEnv = undefined;
  });

  test("a tripped breaker renders its state and the reset keeps the threshold", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [
      makeAutomation({
        triggers: [
          { type: "api", id: "manual", enabled: true },
          {
            type:              "schedule",
            id:                "schedule",
            enabled:           false,
            expression:        "0 */2 * * *",
            breaker_threshold: 3,
            breaker:           {
              signature:         "api_transient|zai|rate_limited",
              consecutive_count: 3,
              last_run_id:       "01M4KMWYRQDJ54ZSS08NJNH74N",
              paused_at:         "2026-10-11T05:04:33Z",
            },
          },
        ],
      }),
    ];
    replaceAutomationMock.mockClear();
    swrMutateMock.mockClear();

    const renderer = await renderDetail();
    expect(allText(renderer)).toContain("Breaker paused this schedule after 3 consecutive failures");

    const reset = buttonsOf(renderer).find((node) =>
      textFromInstance(node).includes("Reset breaker"),
    );
    expect(reset).toBeDefined();
    await act(async () => {
      reset!.props.onClick();
    });

    expect(replaceAutomationMock).toHaveBeenCalledTimes(1);
    const [id, revision, body] = replaceAutomationMock.mock.calls[0] as [
      string,
      string,
      { triggers: Array<Record<string, unknown>> },
    ];
    expect(id).toBe("loop-fabro");
    expect(revision).toBe("rev1");
    const schedule = body.triggers.find((trigger) => trigger.type === "schedule");
    expect(schedule?.enabled).toBe(true);
    expect(schedule?.breaker_threshold).toBe(3);
  });

  test("a counting breaker shows progress without a pause", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [
      makeAutomation({
        triggers: [
          {
            type:              "schedule",
            id:                "schedule",
            enabled:           true,
            expression:        "0 */2 * * *",
            breaker_threshold: 5,
            breaker:           {
              signature:         "api_transient|zai|rate_limited",
              consecutive_count: 2,
              last_run_id:       "01M4KMWYRQDJ54ZSS08NJNH74N",
            },
          },
        ],
      }),
    ];
    const renderer = await renderDetail();
    expect(allText(renderer)).toContain("Breaker: 2 of 5 consecutive failures");
  });
});

describe("Automations list breaker badge (fabro-3d97)", () => {
  afterEach(() => {
    teardownReactEnv?.();
    teardownReactEnv = undefined;
  });

  test("a breaker-paused schedule is flagged in the list row", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [
      makeAutomation({
        triggers: [
          {
            type:       "schedule",
            id:         "schedule",
            enabled:    false,
            expression: "0 */2 * * *",
            breaker:    {
              signature:         "api_transient|zai|rate_limited",
              consecutive_count: 3,
              last_run_id:       "01M4KMWYRQDJ54ZSS08NJNH74N",
              paused_at:         "2026-10-11T05:04:33Z",
            },
          },
        ],
      }),
    ];
    const renderer = await renderRoute();
    expect(allText(renderer)).toContain("Breaker paused");
  });

  test("a clean schedule shows no breaker badge", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [makeAutomation()];
    const renderer = await renderRoute();
    expect(allText(renderer)).not.toContain("Breaker paused");
  });
});

describe("Automations schedule toggle (fabro-2093)", () => {
  afterEach(() => {
    teardownReactEnv?.();
    teardownReactEnv = undefined;
  });

  test("pause sends a full replace that keeps on_overlap and the revision", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [makeAutomation()];
    replaceAutomationMock.mockClear();
    swrMutateMock.mockClear();

    const renderer = await renderRoute();
    const pause = buttonByTitle(renderer, "Pause schedule");
    expect(pause).toBeDefined();

    await act(async () => {
      pause!.props.onClick();
    });

    expect(replaceAutomationMock).toHaveBeenCalledTimes(1);
    const [id, revision, body] = replaceAutomationMock.mock.calls[0];
    expect(id).toBe("loop-fabro");
    expect(revision).toBe("rev1");
    expect(body.name).toBe("Loop");
    expect(body.environment_id).toBe("toolchain");
    expect(body.workflow).toBe("loop");
    expect(body.on_overlap).toBe("skip");
    const schedule = (body.triggers as Array<Record<string, unknown>>).find(
      (trigger) => trigger.type === "schedule",
    );
    expect(schedule?.enabled).toBe(false);
    expect(swrMutateMock).toHaveBeenCalled();
  });

  test("paused schedule shows resume and defaults a missing on_overlap to skip", async () => {
    const env = setupReactTestEnv();
    teardownReactEnv = env.teardown;
    currentAutomations = [
      makeAutomation({
        on_overlap: undefined,
        triggers:   [
          { type: "api", id: "manual", enabled: true },
          {
            type:       "schedule",
            id:         "schedule",
            enabled:    false,
            expression: "0 */2 * * *",
          },
        ],
      }),
    ];
    replaceAutomationMock.mockClear();

    const renderer = await renderRoute();
    const resume = buttonByTitle(renderer, "Resume schedule");
    expect(resume).toBeDefined();
    expect(buttonByTitle(renderer, "Pause schedule")).toBeUndefined();

    await act(async () => {
      resume!.props.onClick();
    });

    expect(replaceAutomationMock).toHaveBeenCalledTimes(1);
    const [, , body] = replaceAutomationMock.mock.calls[0];
    expect(body.on_overlap).toBe("skip");
    const schedule = (body.triggers as Array<Record<string, unknown>>).find(
      (trigger) => trigger.type === "schedule",
    );
    expect(schedule?.enabled).toBe(true);
  });
});
