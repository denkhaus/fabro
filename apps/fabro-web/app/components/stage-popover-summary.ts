import type { EventEnvelope } from "@qltysh/fabro-api-client";

import {
  getArray,
  getNumber,
  getObject,
  getString,
} from "../lib/unknown";

export interface StageSummary {
  attempt?: number;
  maxAttempts?: number;
  failureMessage?: string;
  notes?: string;
  inputTokens?: number;
  outputTokens?: number;
  filesTouchedCount?: number;
  systemActor?: string;
  exitCode?: number;
}

export function deriveStageSummary(events: EventEnvelope[]): StageSummary {
  const summary: StageSummary = {};
  for (const e of events) {
    const props = e.properties ?? {};
    switch (e.event) {
      case "stage.started": {
        const attempt = getNumber(props, "attempt");
        const max = getNumber(props, "max_attempts");
        if (attempt !== undefined) summary.attempt = attempt;
        if (max !== undefined) summary.maxAttempts = max;
        break;
      }
      case "stage.completed": {
        readFailure(summary, getObject(props, "failure"));
        readUsage(summary, getObject(props, "usage"));
        readTermination(summary, getObject(props, "termination"));
        const notes = getString(props, "notes");
        if (notes !== undefined) summary.notes = notes;
        const files = getArray(props, "files_touched");
        if (files !== undefined) summary.filesTouchedCount = files.length;
        break;
      }
      case "stage.failed": {
        readFailure(summary, getObject(props, "failure"));
        readUsage(summary, getObject(props, "usage"));
        break;
      }
    }
  }
  return summary;
}

function readFailure(summary: StageSummary, failure: unknown) {
  if (!failure) return;
  const message = getString(failure, "message");
  if (message !== undefined) summary.failureMessage = message;
  const actor = getString(failure, "system_actor");
  if (actor !== undefined) summary.systemActor = actor;
}

/** `stage.completed.usage` is a `ModelUsage`: the model, then the usage. */
function readUsage(summary: StageSummary, modelUsage: unknown) {
  if (!modelUsage) return;
  const tokens = getObject(getObject(modelUsage, "usage"), "tokens");
  if (!tokens) return;
  const input = getNumber(tokens, "input");
  const output = getNumber(tokens, "output");
  if (input !== undefined) summary.inputTokens = input;
  if (output !== undefined) summary.outputTokens = output;
}

function readTermination(summary: StageSummary, termination: unknown) {
  if (!termination) return;
  const exitCode = getNumber(termination, "exit_code");
  if (exitCode !== undefined) summary.exitCode = exitCode;
}
