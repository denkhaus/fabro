import { describe, expect, test } from "bun:test";

import {
  EMPTY_ENVIRONMENT_FORM,
  createRequestFromForm,
  isEnvironmentFormValid,
  replaceRequestFromForm,
  type EnvironmentFormValues,
} from "./environment-form";

function form(overrides: Partial<EnvironmentFormValues>): EnvironmentFormValues {
  return { ...EMPTY_ENVIRONMENT_FORM, id: "docker", ...overrides };
}

describe("environment image source", () => {
  test("image source requires a non-empty image reference", () => {
    expect(isEnvironmentFormValid(form({ imageSource: "image", dockerRef: "" }))).toBe(false);
    expect(
      isEnvironmentFormValid(form({ imageSource: "image", dockerRef: "ubuntu:24.04" })),
    ).toBe(true);
  });

  test("dockerfile source requires non-empty Dockerfile contents", () => {
    expect(isEnvironmentFormValid(form({ imageSource: "dockerfile", dockerfile: "" }))).toBe(false);
    expect(
      isEnvironmentFormValid(form({ imageSource: "dockerfile", dockerfile: "FROM ubuntu" })),
    ).toBe(true);
  });

  test("an empty Dockerfile does not satisfy the image-reference source", () => {
    expect(
      isEnvironmentFormValid(form({ imageSource: "image", dockerRef: "", dockerfile: "FROM x" })),
    ).toBe(false);
  });

  test("image source sends only the docker reference", () => {
    const request = createRequestFromForm(
      form({ imageSource: "image", dockerRef: "ubuntu:24.04", dockerfile: "FROM leftover" }),
    );
    expect(request.image.docker).toBe("ubuntu:24.04");
    expect(request.image.dockerfile).toBeNull();
  });

  test("dockerfile source sends only the inline Dockerfile", () => {
    const request = createRequestFromForm(
      form({ imageSource: "dockerfile", dockerRef: "leftover", dockerfile: "FROM ubuntu" }),
    );
    expect(request.image.docker).toBeNull();
    expect(request.image.dockerfile?.value).toBe("FROM ubuntu");
  });
});

describe("environment resources by provider", () => {
  test("docker environments never offer or submit a disk limit", () => {
    const request = createRequestFromForm(
      form({ provider: "docker", disk: 16, dockerRef: "ubuntu:24.04" }),
    );
    expect(request.resources.disk).toBeNull();
    expect(request.resources.memory).toBe("8GB");
  });

  test("editing a docker environment sends no disk limit on replace", () => {
    const request = replaceRequestFromForm(
      form({ id: "toolchain", provider: "docker", dockerRef: "fabro-toolchain:v2" }),
    );
    expect(request.resources.disk).toBeNull();
  });

  test("providers that enforce disk still submit the slider value", () => {
    const request = createRequestFromForm(
      form({ provider: "daytona", disk: 20, dockerRef: "fabro-snapshot" }),
    );
    expect(request.resources.disk).toBe("20GB");
  });
});
