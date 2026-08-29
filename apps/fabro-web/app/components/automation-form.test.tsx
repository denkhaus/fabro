import { describe, expect, test } from "bun:test";

import {
  EMPTY_AUTOMATION_FORM,
  automationFormValuesFromRun,
  automationToFormValues,
  isFormValid,
  workflowSourceFromFormValues,
} from "./automation-form";

describe("automation workflow source form values", () => {
  test("the default and create-from-run forms inherit the target checkout", () => {
    expect(workflowSourceFromFormValues(EMPTY_AUTOMATION_FORM)).toBeUndefined();

    const values = automationFormValuesFromRun({
      title: "Release",
      workflow: { name: "Release", graph_name: "release", slug: "release" },
      repository: {
        name:       "fabro-sh/fabro",
        origin_url: "https://github.com/fabro-sh/fabro.git",
      },
      sandbox: null,
    } as any);
    expect(values.usesSeparateWorkflowSource).toBe(false);
    expect(workflowSourceFromFormValues(values)).toBeUndefined();
  });

  test("branch, tag, and commit sources serialize unambiguously", () => {
    const base = {
      ...EMPTY_AUTOMATION_FORM,
      usesSeparateWorkflowSource: true,
      workflowSourceRepository: " fabro-sh/workflows ",
    };

    expect(workflowSourceFromFormValues({
      ...base,
      workflowSourceKind: "branch",
      workflowSourceRef:  " main ",
    })).toEqual({ repo: "fabro-sh/workflows", kind: "branch", ref: "main" });
    expect(workflowSourceFromFormValues({
      ...base,
      workflowSourceKind: "tag",
      workflowSourceRef:  " v1.2.3 ",
    })).toEqual({ repo: "fabro-sh/workflows", kind: "tag", ref: "v1.2.3" });
    expect(workflowSourceFromFormValues({
      ...base,
      workflowSourceKind: "commit",
      workflowSourceRef:  "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
    })).toEqual({
      repo: "fabro-sh/workflows",
      kind: "commit",
      ref:  "abcdef0123456789abcdef0123456789abcdef01",
    });
  });

  test("separate source fields are required and commits need 40 hex characters", () => {
    const validBase = {
      ...EMPTY_AUTOMATION_FORM,
      id:                       "nightly",
      name:                     "Nightly",
      environmentId:            "daytona-smoke",
      targetRepository:         "fabro-sh/app",
      targetBranch:             "main",
      workflow:                 "release",
      usesSeparateWorkflowSource: true,
      workflowSourceRepository: "fabro-sh/workflows",
      workflowSourceKind:       "commit" as const,
      workflowSourceRef:        "0123456789abcdef0123456789abcdef01234567",
    };

    expect(isFormValid(validBase)).toBe(true);
    expect(isFormValid({ ...validBase, workflowSourceRepository: "" })).toBe(false);
    expect(isFormValid({ ...validBase, workflowSourceRef: "main" })).toBe(false);
  });

  test("editing preserves an explicit source even when it equals the target", () => {
    const values = automationToFormValues({
      id:          "nightly",
      revision:    "revision",
      name:        "Nightly",
      description: null,
      target:      { kind: "git", repo: "fabro-sh/fabro", branch: "main" },
      workflow:    "release",
      workflow_source: { repo: "fabro-sh/fabro", kind: "branch", ref: "main" },
      triggers:    [],
    });

    expect(values.usesSeparateWorkflowSource).toBe(true);
    expect(values.workflowSourceRepository).toBe("fabro-sh/fabro");
    expect(values.workflowSourceKind).toBe("branch");
    expect(values.workflowSourceRef).toBe("main");
    expect(workflowSourceFromFormValues({
      ...values,
      usesSeparateWorkflowSource: false,
    })).toBeUndefined();
  });
});
