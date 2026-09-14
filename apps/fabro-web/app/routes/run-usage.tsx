import { Fragment, useMemo } from "react";

import { EmptyState } from "../components/state";
import { Tooltip } from "../components/ui";
import {
  formatDurationMs,
  formatTokenCount,
  formatUsdMicros,
} from "../lib/format";
import { useRunUsage } from "../lib/queries";
import { IN_FLIGHT_STAGE_STATES } from "../lib/stage-sidebar";
import { useTickingNow } from "../lib/time";
import {
  billableOutputTokens,
  costSourceTag,
  hasUsage,
  usageTokenBuckets,
} from "../lib/usage";
import type {
  RunUsage,
  RunUsageStage,
  Usage,
  UsageModelRef,
} from "@qltysh/fabro-api-client";

const EMPTY_VALUE = "—";

function formatTokens(n: number | null | undefined) {
  if (n == null) return EMPTY_VALUE;
  return formatTokenCount(n, { compactDecimal: true });
}

function formatModelRef(model?: UsageModelRef | null): string | null {
  if (!model) return null;
  const speed = model.speed ? ` · ${model.speed}` : "";
  return `${model.provider}:${model.model_id}${speed}`;
}

function isInFlight(stage: RunUsageStage): boolean {
  return stage.state != null && IN_FLIGHT_STAGE_STATES.has(stage.state);
}

function isVisibleRow(row: MappedStageRow): boolean {
  if (row.inFlight) return true;
  return row.usage != null && hasUsage(row.usage);
}

interface MappedStageRow {
  stage:      string;
  model:      string | null;
  usage:      Usage | null;
  wallTimeMs: number;
  inFlight:   boolean;
}

function liveWallTimeMs(stage: RunUsageStage, now: number): number {
  if (stage.started_at) {
    const startedMs = new Date(stage.started_at).getTime();
    if (Number.isFinite(startedMs)) {
      return Math.max(0, now - startedMs);
    }
  }
  return stage.timing.wall_time_ms;
}

export const handle = { wide: true };

function mapStageRow(stage: RunUsageStage, wallTimeMs: number): MappedStageRow {
  const hasModel = stage.model != null;
  return {
    stage: stage.stage.name,
    model: formatModelRef(stage.model),
    usage: hasModel ? stage.usage : null,
    wallTimeMs,
    inFlight: isInFlight(stage),
  };
}

/** Hover breakdown of the disjoint token buckets behind an `in / out` count. */
function TokenBreakdown({ usage }: { usage: Usage }) {
  const buckets = usageTokenBuckets(usage);
  return (
    <div className="min-w-44 py-0.5">
      <div className="border-line text-fg-2 mb-1.5 border-b pb-1 font-medium">
        Tokens in / out
      </div>
      <dl className="grid grid-cols-[1fr_auto] gap-x-6 gap-y-1">
        {buckets.map((bucket) => (
          <Fragment key={bucket.label}>
            <dt className="text-fg-3">{bucket.label}</dt>
            <dd className="text-fg text-right font-mono tabular-nums">
              {formatTokens(bucket.value)}
            </dd>
          </Fragment>
        ))}
      </dl>
      <p className="border-line text-fg-3 mt-1.5 border-t pt-1">
        Includes subagent tokens, priced at each subagent&apos;s model.
      </p>
    </div>
  );
}

/**
 * Renders an `input / output` token count. When the row has model usage,
 * hovering the count reveals the cache breakdown.
 */
function TokensCell({ usage }: { usage: Usage | null }) {
  const display = (
    <>
      {formatTokens(usage?.tokens.input)} <span className="text-fg-muted">/</span>{" "}
      {formatTokens(usage ? billableOutputTokens(usage.tokens) : null)}
    </>
  );
  if (!usage) return display;
  return (
    <Tooltip label={<TokenBreakdown usage={usage} />}>
      <span>{display}</span>
    </Tooltip>
  );
}

/**
 * Renders a cost, or a dash when the usage carries none. A cost that did not
 * come from the catalog is tagged with where it came from.
 */
function CostCell({ usage }: { usage: Usage | null | undefined }) {
  const cost = usage?.cost;
  const formatted = formatUsdMicros(cost?.usd_micros);
  if (formatted == null) return <>{EMPTY_VALUE}</>;
  const tag = costSourceTag(cost);
  return (
    <>
      {formatted}
      {tag ? (
        <span className="text-fg-muted ml-1.5 font-sans text-[10px] uppercase tracking-wide">
          {tag}
        </span>
      ) : null}
    </>
  );
}

export default function RunUsageRoute({ params }: { params: { id: string } }) {
  const usageQuery = useRunUsage(params.id);
  const runUsage: RunUsage | undefined = usageQuery.data;
  const hasInFlight = runUsage?.stages.some(isInFlight) ?? false;

  // Tick once per second only while a stage is in-flight.
  const now = useTickingNow(hasInFlight);

  // Completed rows don't depend on `now`; memoize them by `runUsage` so we
  // don't reallocate them every tick.
  const completedRows = useMemo<MappedStageRow[]>(() => {
    if (!runUsage) return [];
    return runUsage.stages.map((stage) => mapStageRow(stage, stage.timing.wall_time_ms));
  }, [runUsage]);

  // The model breakdown is server-derived and stable across ticks too.
  const modelBreakdown = useMemo(() => {
    if (!runUsage) return [];
    return runUsage.by_model
      .map((entry) => ({
        model:  formatModelRef(entry.model) ?? EMPTY_VALUE,
        stages: entry.stages,
        usage:  entry.usage,
      }))
      .sort(
        (a, b) =>
          (b.usage.cost?.usd_micros ?? -1) - (a.usage.cost?.usd_micros ?? -1),
      );
  }, [runUsage]);

  // Re-derive only the in-flight rows on each tick; everything else stays put.
  const rows = useMemo<MappedStageRow[]>(() => {
    if (!runUsage) return [];
    if (!hasInFlight) return completedRows;
    return runUsage.stages.map((stage, idx) =>
      isInFlight(stage)
        ? mapStageRow(stage, liveWallTimeMs(stage, now))
        : completedRows[idx],
    );
  }, [runUsage, completedRows, hasInFlight, now]);

  // While ticking, sum the displayed row runtimes so the footer updates in
  // lock-step. Otherwise trust the server's authoritative total.
  const totalWallTimeMs = hasInFlight
    ? rows.reduce((sum, row) => sum + row.wallTimeMs, 0)
    : (runUsage?.totals.timing.wall_time_ms ?? 0);

  const hasLlmStages = (runUsage?.by_model.length ?? 0) > 0;
  const totalUsage = hasLlmStages && runUsage ? runUsage.totals.usage : null;
  const modelStageCount = modelBreakdown.reduce((sum, row) => sum + row.stages, 0);
  const visibleRows = rows.filter(isVisibleRow);

  if (!visibleRows.length) {
    return (
      <div className="py-12">
        <EmptyState
          title={rows.length ? "No model usage" : "No stages yet"}
          description={
            rows.length
              ? "This run didn't call any AI models."
              : "Stages will appear as soon as the run starts executing."
          }
        />
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <div className="overflow-hidden rounded-md border border-line">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-line bg-panel/60 text-left text-xs font-medium text-fg-3">
              <th className="px-4 py-2.5 font-medium">Stage</th>
              <th className="px-4 py-2.5 font-medium">Model</th>
              <th className="px-4 py-2.5 font-medium text-right">Tokens</th>
              <th className="px-4 py-2.5 font-medium text-right">Run time</th>
              <th className="px-4 py-2.5 font-medium text-right">Cost</th>
            </tr>
          </thead>
          <tbody>
            {visibleRows.map((row) => (
              <tr key={row.stage} className="border-b border-line last:border-b-0">
                <td className="px-4 py-3 text-fg-2">{row.stage}</td>
                <td className="px-4 py-3 font-mono text-xs text-fg-3">
                  {row.model ?? EMPTY_VALUE}
                </td>
                <td className="px-4 py-3 text-right font-mono text-xs tabular-nums text-fg-3">
                  <TokensCell usage={row.usage} />
                </td>
                <td className="px-4 py-3 text-right font-mono text-xs text-fg-3">
                  {formatDurationMs(row.wallTimeMs)}
                </td>
                <td className="px-4 py-3 text-right font-mono text-xs text-fg-3">
                  <CostCell usage={row.usage} />
                </td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr className="border-t border-line-strong bg-overlay">
              <td className="px-4 py-3 font-medium text-fg">Total</td>
              <td className="px-4 py-3 text-xs text-fg-muted">All models</td>
              <td className="px-4 py-3 text-right font-mono text-xs tabular-nums font-medium text-fg">
                <TokensCell usage={totalUsage} />
              </td>
              <td className="px-4 py-3 text-right font-mono text-xs font-medium text-fg">
                {formatDurationMs(totalWallTimeMs)}
              </td>
              <td className="px-4 py-3 text-right font-mono text-xs font-medium text-fg">
                <CostCell usage={totalUsage} />
              </td>
            </tr>
          </tfoot>
        </table>
      </div>

      {modelBreakdown.length > 0 ? (
        <div>
          <h3 className="mb-3 text-sm font-semibold text-fg">By model</h3>
          <div className="overflow-hidden rounded-md border border-line">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-line bg-panel/60 text-left text-xs font-medium text-fg-3">
                  <th className="px-4 py-2.5 font-medium">Model</th>
                  <th className="px-4 py-2.5 font-medium text-right">Stages</th>
                  <th className="px-4 py-2.5 font-medium text-right">Tokens</th>
                  <th className="px-4 py-2.5 font-medium text-right">Cost</th>
                </tr>
              </thead>
              <tbody>
                {modelBreakdown.map((row) => (
                  <tr key={row.model} className="border-b border-line last:border-b-0">
                    <td className="px-4 py-3 font-mono text-xs text-fg-2">{row.model}</td>
                    <td className="px-4 py-3 text-right font-mono text-xs tabular-nums text-fg-3">
                      {row.stages}
                    </td>
                    <td className="px-4 py-3 text-right font-mono text-xs tabular-nums text-fg-3">
                      <TokensCell usage={row.usage} />
                    </td>
                    <td className="px-4 py-3 text-right font-mono text-xs text-fg-3">
                      <CostCell usage={row.usage} />
                    </td>
                  </tr>
                ))}
              </tbody>
              <tfoot>
                <tr className="border-t border-line-strong bg-overlay">
                  <td className="px-4 py-3 font-medium text-fg">Total</td>
                  <td className="px-4 py-3 text-right font-mono text-xs tabular-nums font-medium text-fg">
                    {modelStageCount}
                  </td>
                  <td className="px-4 py-3 text-right font-mono text-xs tabular-nums font-medium text-fg">
                    <TokensCell usage={totalUsage} />
                  </td>
                  <td className="px-4 py-3 text-right font-mono text-xs font-medium text-fg">
                    <CostCell usage={totalUsage} />
                  </td>
                </tr>
              </tfoot>
            </table>
          </div>
        </div>
      ) : null}
    </div>
  );
}
