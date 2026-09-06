import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { createElement } from "react";
import TestRenderer, { act } from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";

import { ToastProvider } from "../components/toast";
import { setupReactTestEnv } from "../lib/test-utils";

const replaceAutomationMock = mock(
  (id: string, ifMatch: string, body: { triggers: Array<{ type: string; enabled: boolean }> }) =>
    Promise.resolve({ data: {} }),
);
const swrMutateMock = mock((_key: unknown) => Promise.resolve(undefined));

let currentAutomations: any[] = [];
let currentLoading = false;
let currentError: unknown = null;
let teardownReactEnv: (() => void) | undefined;
const mountedRenderers: TestRenderer.ReactTestRenderer[] = [];

mock.module("@headlessui/react", () => ({
  Menu: ({ children }: any) => createElement("div", children),
  MenuButton: ({ children, ...props }: any) =>
    createElement("button", { ...props, type: "button" }, children),
  MenuItems: ({ children, ...props }: any) => createElement("div", props, children),
  MenuItem: ({ children }: any) => createElement("div", children),
  Dialog: ({ open, children }: any) =>
    open ? createElement("div", { role: "dialog" }, children) : null,
  DialogPanel: ({ children, ...props }: any) => createElement("div", props, children),
  DialogTitle: ({ children, ...props }: any) => createElement("h2", props, children),
}));

mock.module("../lib/queries", () => ({
  useAutomations: () => ({
    data:      { data: currentAutomations, meta: { total: currentAutomations.length } },
    error:     currentError,
    isLoading: currentLoading,
  }),
}));

mock.module("../lib/api-client", () => ({
  ApiError: class ApiError extends Error {},
  apiData:   <T,>(run: () => Promise<T>) => run(),
  automationsApi: {
    replaceAutomation: (...args: unknown[]) => replaceAutomationMock(...args as any),
  },
}));

mock.module("swr", () => ({
  useSWRConfig: () => ({ mutate: swrMutateMock }),
}));

function scheduledAutomation(enabled: boolean) {
  return {
    id:            "auto_conductor",
    revision:      "32ea7dfb",
    name:          "Conductor line",
    description:   "Serialized fabro line",
    environment_id: "env_main",
    target:        {
      kind:     "git",
      repo:     "denkhaus/fabro",
      branch:   "denkhaus",
    },
    workflow:      "conductor",
    workflow_source: null,
    triggers:      [
      { id: "t_sched", type: "schedule", enabled, expression: "*/30 * * * *" },
    ],
  };
}

let routeModule: typeof import("./automations");

async function renderAutomations() {
  routeModule ??= await import("./automations");
  const router = createMemoryRouter([
    { path: "/automations", element: createElement(routeModule.default) },
  ], { initialEntries: ["/automations"] });
  let renderer: TestRenderer.ReactTestRenderer;
  await act(async () => {
    renderer = TestRenderer.create(
      createElement(ToastProvider, null, createElement(RouterProvider, { router })),
    );
  });
  mountedRenderers.push(renderer!);
  return renderer!;
}

function buttonByTitle(renderer: TestRenderer.ReactTestRenderer, title: string) {
  const hits = renderer.root.findAll(
    (node) => node.props?.title === title && node.type === "button",
  );
  expect(hits.length).toBe(1);
  return hits[0];
}

beforeEach(() => {
  teardownReactEnv = setupReactTestEnv();
  replaceAutomationMock.mockClear();
  swrMutateMock.mockClear();
  currentAutomations = [];
  currentLoading = false;
  currentError = null;
});

afterEach(async () => {
  for (const renderer of mountedRenderers.splice(0)) {
    await act(async () => {
      renderer.unmount();
    });
  }
  teardownReactEnv?.();
});

describe("automation schedule pause/resume", () => {
  test("pause sends PUT with schedule enabled:false and If-Match revision", async () => {
    currentAutomations = [scheduledAutomation(true)];
    const renderer = await renderAutomations();
    const pause = buttonByTitle(renderer, "Pause schedule");
    await act(async () => {
      pause.props.onClick();
    });
    expect(replaceAutomationMock).toHaveBeenCalledTimes(1);
    const [id, ifMatch, body] = replaceAutomationMock.mock.calls[0];
    expect(id).toBe("auto_conductor");
    expect(ifMatch).toBe("32ea7dfb");
    expect(body.triggers[0]?.enabled).toBe(false);
    expect(body.triggers[0]?.type).toBe("schedule");
  });

  test("resume sends PUT with schedule enabled:true", async () => {
    currentAutomations = [scheduledAutomation(false)];
    const renderer = await renderAutomations();
    const resume = buttonByTitle(renderer, "Resume schedule");
    await act(async () => {
      resume.props.onClick();
    });
    expect(replaceAutomationMock).toHaveBeenCalledTimes(1);
    const [id, ifMatch, body] = replaceAutomationMock.mock.calls[0];
    expect(id).toBe("auto_conductor");
    expect(ifMatch).toBe("32ea7dfb");
    expect(body.triggers[0]?.enabled).toBe(true);
  });

  test("paused schedule renders amber paused badge", async () => {
    currentAutomations = [scheduledAutomation(false)];
    const renderer = await renderAutomations();
    const texts = renderer.root.findAllByType("span").map((n) => n.children?.join(""));
    expect(texts.some((text) => String(text).includes("paused"))).toBe(true);
  });
});
