// WAF → Overview: the WAF's state (a start's progress while it runs), the
// requests checked and blocked (24 hours or 7 days, hour by hour; hours the
// panel did not sample show no data) and the top rules, addresses and
// domains of its alerts.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, ErrorState, Skeleton } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "../lib/query";
import { EmptyState, Facts, Metric, Section, when } from "../lib/ui";
import { Progress, StateBadge, type WafData } from "./common";

type Point = { hour: string; checked: number; blocked: number; samples: number };
type Top = Array<{ name: string; count: number }>;
type Overview = { from: string; checked: number; blocked: number; recorded: number; series: Point[]; top_rules: Top; top_ips: Top; top_hosts: Top };

export function WafOverview(props: PageProps<unknown> & { data: WafData; onRepair: () => void; repairing: boolean }) {
  const t = props.t;
  const [hours, setHours] = useState(24);
  const overview = useAdminQuery<Overview>(props, "/apps/waf/overview", { query: { hours }, refetchInterval: 60000 });
  const { status, settings } = props.data;
  const mode = settings.mode === "block" ? t("Block") : settings.mode === "detect" ? t("Detect only") : t("Off");
  const level = settings.level === "strict" ? t("Strict") : t("Basic");
  const number = (n: number) => n.toLocaleString(props.locale);
  return (
    <>
      <Section title={t("State")}>
        {status.state === "starting" ? (
          <Progress {...props} status={status} />
        ) : (
          <Facts
            items={[
              [t("State"), <StateBadge key="s" status={status} t={t} />],
              [t("Mode"), settings.mode === "off" ? t("Off") : `${mode} · ${level}`],
              [t("Rules"), settings.rules === "latest" ? t("Latest (updated)") : t("Bundled with the panel")],
              [t("Checked"), status.checked_at ? when(status.checked_at, props.locale) : "–"],
              [t("Services with it off"), String(props.data.services_off)],
            ]}
          />
        )}
        {status.state === "unavailable" && status.detail && (
          <Alert variant="warning">
            {t(status.detail)}{" "}
            <Button size="sm" variant="outline" disabled={props.repairing} onClick={props.onRepair}>
              {props.repairing ? t("Repairing…") : t("Repair")}
            </Button>
          </Alert>
        )}
      </Section>
      <Section
        title={t("Requests")}
        description={t("Requests to the services' domains the WAF checked, and those it blocked (the panel's own test requests left out).")}
        actions={
          <div className="rdp2-segmented" role="group" aria-label={t("Period")}>
            {[24, 168].map((h) => (
              <Button key={h} size="sm" variant={hours === h ? "secondary" : "ghost"} aria-pressed={hours === h} onClick={() => setHours(h)}>
                {h === 24 ? t("24 hours") : t("7 days")}
              </Button>
            ))}
          </div>
        }
      >
        {overview.isLoading ? (
          <Skeleton className="rdp2-skeleton-chart" />
        ) : overview.error || !overview.data ? (
          <ErrorState error={overview.error} />
        ) : (
          <>
            <div className="rdp2-metrics">
              <Metric plain label={t("Requests checked")} value={number(overview.data.checked)} />
              <Metric
                plain
                label={t("Requests blocked")}
                value={number(overview.data.blocked)}
                sub={overview.data.checked > 0 ? `${((overview.data.blocked / overview.data.checked) * 100).toFixed(1)} %` : undefined}
              />
              <Metric plain label={t("Recorded (Detect mode)")} value={number(overview.data.recorded)} />
              <Metric plain label={t("Top rule")} value={<span className="rdp2-metric-text">{overview.data.top_rules[0]?.name ?? "–"}</span>} />
            </div>
            <Chart points={overview.data.series} hours={hours} from={overview.data.from} t={t} locale={props.locale} />
          </>
        )}
      </Section>
      {overview.data && (
        <div className="rdp2-grid3">
          <TopList title={t("Rules")} rows={overview.data.top_rules} t={t} mono={false} />
          <TopList title={t("Addresses")} rows={overview.data.top_ips} t={t} mono />
          <TopList title={t("Domains")} rows={overview.data.top_hosts} t={t} mono />
        </div>
      )}
    </>
  );
}

/** Hour by hour: a bar the requests checked, its blocked part filled; an
 * hour not sampled (the panel stopped) is a gap. */
function Chart(props: { points: Point[]; hours: number; from: string; t: (s: string) => string; locale: string }) {
  const t = props.t;
  const start = new Date(props.from).getTime();
  const byHour = new Map(props.points.map((p) => [new Date(p.hour).getTime(), p]));
  const slots = Array.from({ length: props.hours }, (_, i) => {
    const at = start + i * 3600_000;
    return { at, point: byHour.get(at) };
  });
  const max = Math.max(1, ...props.points.map((p) => p.checked));
  const width = 100 / slots.length;
  if (props.points.length === 0) return <EmptyState title={t("No data yet.")} hint={t("The panel samples the WAF every minute while it is on.")} />;
  return (
    <figure className="rdp2-waf-chart">
      <svg viewBox="0 0 100 40" preserveAspectRatio="none" role="img" aria-label={t("Requests checked and blocked, hour by hour")}>
        {slots.map(({ at, point }, i) =>
          point ? (
            <g key={at}>
              <title>{`${new Date(at).toLocaleString(props.locale, { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" })}: ${point.checked} ${t("checked")}, ${point.blocked} ${t("blocked")}${point.samples < 60 ? ` (${t("part of the hour sampled")})` : ""}`}</title>
              <rect className="rdp2-waf-bar" x={i * width + width * 0.1} width={width * 0.8} y={40 - (point.checked / max) * 38} height={(point.checked / max) * 38} />
              <rect className="rdp2-waf-bar-blocked" x={i * width + width * 0.1} width={width * 0.8} y={40 - (point.blocked / max) * 38} height={(point.blocked / max) * 38} />
            </g>
          ) : (
            <rect key={at} className="rdp2-waf-bar-gap" x={i * width + width * 0.1} width={width * 0.8} y={38} height={2} />
          ),
        )}
      </svg>
      <figcaption className="rdp2-row rdp2-small rdp2-muted">
        <span className="rdp2-row">
          <span className="rdp2-waf-key" aria-hidden /> {t("Requests checked")}
        </span>
        <span className="rdp2-row">
          <span className="rdp2-waf-key rdp2-waf-key-blocked" aria-hidden /> {t("Requests blocked")}
        </span>
        <span className="rdp2-row">
          <span className="rdp2-waf-key rdp2-waf-key-gap" aria-hidden /> {t("No data (the panel was stopped)")}
        </span>
      </figcaption>
    </figure>
  );
}

function TopList(props: { title: string; rows: Array<{ name: string; count: number }>; t: (s: string) => string; mono: boolean }) {
  return (
    <Section title={props.title} description={props.t("From the alerts (grouped: one a minute per address and path in Detect mode).")} flush>
      {props.rows.length === 0 ? (
        <EmptyState title={props.t("No alert yet.")} />
      ) : (
        <table className="rdp2-table">
          <tbody>
            {props.rows.map((row) => (
              <tr key={row.name}>
                <td className={props.mono ? "rdp2-mono rdp2-truncate" : "rdp2-truncate"} title={row.name}>
                  {row.name}
                </td>
                <td className="rdp2-num rdp2-right">{row.count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Section>
  );
}
