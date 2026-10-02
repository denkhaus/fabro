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
