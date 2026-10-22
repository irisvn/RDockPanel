// Whether the server has room for what is about to run (`/apps/capacity`):
// a notice in the forms that create services, and a question before a
// service is deployed, started, restarted or rebuilt. Advisory: "not
// enough" asks to be confirmed; only a reservation Swarm cannot place stops.
import { type ReactNode, useEffect, useRef, useState } from "react";
import { type AdminApi } from "@rustcms/admin-sdk";
import { Button, Checkbox, Dialog } from "@rustcms/admin-sdk/ui";
import { bytes } from "../lib/ui";

type T = (s: string) => string;
type Source = "observed" | "limit" | "default" | "unknown";
export type Capacity = {
  level: "ok" | "tight" | "short" | "impossible" | "unknown";
  needs: Array<{ name: string; kind: string; memory: number; temporary: number; disk: number; source: Source }>;
  memory: number;
  disk: number;
  reasons: string[];
  /** The server's figures (who sees the server only). */
  host: { mem_total: number; mem_available: number; swap_used: number; swap_total: number; disk_free: number; cpu_percent: number | null } | null;
  reservation?: { total: number | null; used: number | null; more: number };
};

/** Whether `report` is worth a word (and a question before deploying). */
export const worthSaying = (report: Capacity | null | undefined): report is Capacity =>
  !!report && (report.level === "tight" || report.level === "short" || report.level === "impossible");

const SOURCES: Record<Source, string> = {
  observed: "In use now",
  limit: "Memory limit",
  default: "Typical",
  unknown: "Unknown",
};

function sourceNote(t: T, need: Capacity["needs"][number]): string {
  switch (need.source) {
    case "observed":
      return t("Estimate: what it uses now.");
    case "limit":
      return t("Estimate: its memory limit.");
    case "unknown":
      return t("Not known before it runs: its Compose file is in a repository.");
    default:
      return t("Estimate: a typical {kind} service. Set a memory limit on its Advanced tab for a closer one.").replace("{kind}", need.kind);
  }
}

/** The notice (nothing when there is room); `ack` and `onAck`: the
 * "deploy anyway" box when it does not fit. */
export function CapacityNotice(props: { t: T; report: Capacity | null | undefined; ack?: boolean; onAck?: (ack: boolean) => void; table?: boolean }) {
  const { t, report } = props;
  if (!worthSaying(report)) return null;
  const h = report.host;
  const bad = report.level !== "tight";
  const more = bytes(report.memory);
  const rolling = report.needs.length === 1 && report.needs[0].memory === 0 && report.needs[0].temporary > 0;
  let title: string;
  let detail: string;
  if (report.level === "impossible") {
    title = t("This service cannot run");
    const r = report.reservation;
    detail =
      r && r.total != null && r.used != null
        ? t("It reserves {more} of memory, but only {free} of the server's {total} is left to reserve. Docker Swarm would leave it pending.")
            .replace("{more}", bytes(r.more))
            .replace("{free}", bytes(Math.max(0, r.total - r.used)))
            .replace("{total}", bytes(r.total))
        : t("Its memory reservation is larger than the server can give. Docker Swarm would leave it pending.");
  } else if (report.level === "short") {
    title = report.reasons.includes("memory_short") ? t("Not enough memory for this") : t("Not enough disk space for this");
    detail = h
      ? t("It needs about {need}, the server has {free} free. Services may be stopped for lack of memory, or slow the others down.")
          .replace("{need}", more)
          .replace("{free}", bytes(h.mem_available))
      : t("It needs about {need}, more than the server has free.").replace("{need}", more);
  } else {
    title = t("Memory will be low");
    detail = rolling
      ? t("Without downtime, the new version starts before the old one stops: about {need} more while it deploys.").replace("{need}", more)
      : h
        ? t("About {left} stays free after it. It can still be deployed; keep an eye on it.").replace("{left}", bytes(Math.max(0, h.mem_available - report.memory)))
        : t("It needs about {need}; little memory stays free after it.").replace("{need}", more);
  }
  const reasons: string[] = [];
  for (const r of report.reasons) {
    if (r === "memory_short" && h) reasons.push(t("About {x} of memory missing").replace("{x}", bytes(report.memory - h.mem_available)));
    if (r === "memory_low") reasons.push(t("Less than 15% of the memory stays free"));
    if (r === "swap_high" && h && h.swap_total > 0) reasons.push(t("Swap is {p}% used").replace("{p}", String(Math.round((h.swap_used / h.swap_total) * 100))));
    if (r === "disk_short" || r === "disk_low") reasons.push(h ? t("{free} of disk space free").replace("{free}", bytes(h.disk_free)) : t("Little disk space free"));
    if (r === "cpu_busy" && h?.cpu_percent != null) reasons.push(t("The CPU is busy ({p}%)").replace("{p}", String(Math.round(h.cpu_percent))));
  }
  const used = h ? h.mem_total - h.mem_available : 0;
  const pct = (n: number) => (h ? `${Math.min(100, (n / h.mem_total) * 100)}%` : "0");
  const table = props.table || report.needs.length > 1;
  return (
    <div className={bad ? "rdp2-cap rdp2-cap-bad" : "rdp2-cap"} role="alert">
      <div className="rdp2-cap-head">
        <span className="rdp2-cap-icon">{report.level === "impossible" ? "✕" : "!"}</span>
        <div>
          <strong>{title}</strong>
          <p>{detail}</p>
        </div>
      </div>
      {h && report.level !== "impossible" && (
        <>
          <div className="rdp2-cap-bar" aria-hidden>
            <i className="rdp2-cap-used" style={{ width: pct(used) }} />
            <i className="rdp2-cap-more" style={{ width: pct(Math.min(report.memory, h.mem_available)) }} />
          </div>
          <div className="rdp2-cap-legend">
            <span>
              <i className="rdp2-cap-dot rdp2-cap-used" />
              {t("In use")} <b>{bytes(used)}</b>
            </span>
            <span>
              <i className="rdp2-cap-dot rdp2-cap-more" />
              {rolling ? t("For a while") : t("More")} <b>~{more}</b>
            </span>
            <span>
              {t("Total")} <b>{bytes(h.mem_total)}</b>
            </span>
          </div>
        </>
      )}
      {table && (
        <table className="rdp2-cap-table">
          <thead>
            <tr>
              <th>{t("Service")}</th>
              <th>{t("Estimated from")}</th>
              <th className="rdp2-num">{t("Memory")}</th>
            </tr>
          </thead>
          <tbody>
            {report.needs.map((n) => (
              <tr key={n.name}>
                <td>
                  {n.name} <span className="rdp2-muted">{n.kind}</span>
                </td>
                <td>
                  <span className="rdp2-cap-source">{t(SOURCES[n.source])}</span>
                </td>
                <td className="rdp2-num">{n.source === "unknown" ? "?" : bytes(n.memory + n.temporary)}</td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr>
              <td colSpan={2}>{t("Total")}</td>
              <td className="rdp2-num">{more}</td>
            </tr>
          </tfoot>
        </table>
      )}
      {reasons.length > 0 && (
        <ul className="rdp2-cap-reasons">
          {reasons.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      )}
      {!table && report.needs[0] && report.level !== "impossible" && <p className="rdp2-cap-note">{sourceNote(t, report.needs[0])}</p>}
      {report.level === "short" && props.onAck && (
        <label className="rdp2-cap-ack">
          <Checkbox checked={!!props.ack} onChange={(e) => props.onAck?.(e.target.checked)} /> {t("I understand, deploy anyway")}
        </label>
      )}
    </div>
  );
}

/** The report for `body`, asked again a moment after it changes (the
 * latest answer only); `null` while none (or when it failed: advisory). */
export function useCapacity(api: AdminApi, body: object | null): Capacity | null {
  const [report, setReport] = useState<Capacity | null>(null);
  const key = body ? JSON.stringify(body) : "";
  const latest = useRef(0);
  useEffect(() => {
    const ask = ++latest.current;
    setReport(null);
    if (!key) return;
    const timer = window.setTimeout(async () => {
      try {
        const r = await api.post<Capacity>("/apps/capacity", JSON.parse(key));
        if (latest.current === ask) setReport(r);
      } catch {
        // Not known: nothing said.
      }
    }, 400);
    return () => window.clearTimeout(timer);
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  return report;
}

/** A question before an action that runs more: `check(body, …)` resolves
 * to whether to go on (at once when there is room, or it is not known);
 * render `dialog` once. */
export function useCapacityCheck(t: T, api: AdminApi): [(body: object, title: string, action: string) => Promise<boolean>, ReactNode] {
  const [open, setOpen] = useState<{ report: Capacity; title: string; action: string; resolve: (ok: boolean) => void } | null>(null);
  const [ack, setAck] = useState(false);
  const waiting = useRef<((ok: boolean) => void) | null>(null);
  useEffect(() => () => waiting.current?.(false), []);
  const check = async (body: object, title: string, action: string) => {
    let report: Capacity;
    try {
      report = await api.post<Capacity>("/apps/capacity", body);
    } catch {
      return true;
    }
    if (!worthSaying(report)) return true;
    return new Promise<boolean>((resolve) => {
      waiting.current?.(false);
      waiting.current = resolve;
      setAck(false);
      setOpen({ report, title, action, resolve });
    });
  };
  const close = (ok: boolean) => {
    if (open && waiting.current === open.resolve) waiting.current = null;
    open?.resolve(ok);
    setOpen(null);
  };
  const impossible = open?.report.level === "impossible";
  const dialog = open && (
    <Dialog open onClose={() => close(false)} title={open.title} className="rdp2-dialog-compact">
      <div className="rdp2-form">
        <CapacityNotice t={t} report={open.report} ack={ack} onAck={setAck} />
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={() => close(false)}>
            {impossible ? t("Close") : t("Cancel")}
          </Button>
          {!impossible && (
            <Button
              type="button"
              variant={open.report.level === "short" ? "destructive" : "default"}
              disabled={open.report.level === "short" && !ack}
              onClick={() => close(true)}
            >
              {open.action}
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
  return [check, dialog || null];
}
