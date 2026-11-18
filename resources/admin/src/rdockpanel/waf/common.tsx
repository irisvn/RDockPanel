// The WAF page's shared parts: its data's types, its state with the
// progress of a start (the reconcile's steps), and the log of an action.
import { useEffect, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Button } from "@rustcms/admin-sdk/ui";
import { StatusBadge } from "../lib/ui";
import { DockerLog } from "../actions/Actions";

export type Mode = "off" | "detect" | "block";
export type Level = "basic" | "strict";
export type Exception = { rule: string; host: string; since?: string | null };
export type WafSettings = {
  mode: Mode;
  level: Level;
  allowlist: string[];
  sharing: boolean;
  blocklist: boolean;
  exceptions: Exception[];
  rules: "bundled" | "latest";
};
export type WafStatus = {
  state: "off" | "starting" | "stopping" | "on" | "unavailable" | string;
  detail?: string | null;
  checked_at?: string | null;
  applied?: boolean;
  step?: number | null;
  started_at?: string | null;
  action?: number | null;
};
export type WafData = { settings: WafSettings; status: WafStatus; services_off: number };

/** The reconcile's steps, as the server names them (`waf_docker_service::STEPS`). */
export const STEPS = [
  "Copying the WAF's plugin and rules",
  "Starting CrowdSec",
  "Giving Traefik the plugin",
  "Checking (CrowdSec loads its rules: about half a minute)",
];

export function stateTone(state: string): "ok" | "busy" | "bad" | "off" {
  if (state === "on") return "ok";
  if (state === "starting" || state === "stopping") return "busy";
  if (state === "unavailable") return "bad";
  return "off";
}

export function stateLabel(state: string, t: (s: string) => string): string {
  const words: Record<string, string> = { on: t("On"), off: t("Off"), starting: t("Starting"), stopping: t("Stopping"), unavailable: t("Unavailable") };
  return words[state] ?? state;
}

export function StateBadge(props: { status: WafStatus; t: (s: string) => string }) {
  return <StatusBadge tone={stateTone(props.status.state)} label={stateLabel(props.status.state, props.t)} />;
}

/** Seconds since `from`, ticking. */
function useElapsed(from: string | null | undefined): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!from) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [from]);
  return from ? Math.max(0, Math.floor((now - new Date(from).getTime()) / 1000)) : 0;
}

/** A start under way: its four steps (done, running, to come), the time it
 * has taken, and its action's log. */
export function Progress(props: PageProps<unknown> & { status: WafStatus }) {
  const t = props.t;
  const [log, setLog] = useState(false);
  const elapsed = useElapsed(props.status.started_at);
  const step = props.status.step ?? 1;
  const time = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  return (
    <div className="rdp2-waf-progress" aria-live="polite">
      <div className="rdp2-row">
        <StatusBadge tone="busy" busy label={t("Starting")} />
        <span className="rdp2-muted rdp2-small rdp2-num">{time}</span>
        {props.status.action != null && (
          <Button size="sm" variant="ghost" onClick={() => setLog(true)}>
            {t("View log")}
          </Button>
        )}
      </div>
      <ol className="rdp2-waf-steps">
        {STEPS.map((label, i) => {
          const n = i + 1;
          const state = n < step ? "done" : n === step ? "running" : "next";
          return (
            <li key={label} className={`rdp2-waf-step rdp2-waf-step-${state}`}>
              <span className={`rdp2-dot ${state === "done" ? "rdp2-dot-ok" : state === "running" ? "rdp2-dot-busy" : "rdp2-dot-off"}`} aria-hidden />
              <span>{t(label)}</span>
              <span className="rdp2-sr-only">{state === "done" ? t("done") : state === "running" ? t("under way") : t("to come")}</span>
            </li>
          );
        })}
      </ol>
      {log && props.status.action != null && <DockerLog {...props} id={props.status.action} url={`/apps/waf/actions/${props.status.action}`} onClose={() => setLog(false)} />}
    </div>
  );
}
