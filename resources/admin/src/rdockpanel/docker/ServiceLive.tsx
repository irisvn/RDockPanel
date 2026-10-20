// Beside a service's record (`ServiceLiveSlot`): its state, whether its
// configuration changed since the last deployment, and its latest deployment
// with the log followed live (`/deploy/deployments/{id}/log`). The actions
// are the resource's, through its action route.
import { useEffect, useRef, useState } from "react";
import { type SlotProps, useEvents } from "@rustcms/admin-sdk";
import { Alert, Button } from "@rustcms/admin-sdk/ui";
import { ConfirmProvider, ErrorNote, Facts, Status, useAsk } from "../lib/ui";
import { bindResources, resources } from "../lib/resources";
import { dateTime } from "../lib/format";

type Deployment = { id: number; trigger: string; status: string; error: string | null; image: string | null; log: string | null; finished: boolean; published: boolean };
type Data = {
  id: number;
  status: string | null;
  busy: boolean;
  pending: boolean;
  swarm_name: string | null;
  deployed_at: string | null;
  zero_downtime: boolean;
  zero_downtime_possible: boolean;
  latest: Deployment | null;
  can: { deploy?: boolean; destroy?: boolean; deployments?: boolean };
};

function LiveLog(props: SlotProps<Data> & { deployment: Deployment; onStatus: (d: Deployment) => void }) {
  const [log, setLog] = useState(props.deployment.log ?? "");
  const [error, setError] = useState<string | null>(null);
  const output = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    const el = output.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [log]);
  useEvents(props, `/deploy/deployments/${encodeURIComponent(props.deployment.id)}/log`, {
    onEvent(name, value) {
      if (name === "log") {
        // The whole log comes first on each connection (`reset`).
        const v = value as { reset?: string; append?: string };
        if (v.reset !== undefined) setLog(v.reset);
        else setLog((old) => old + (v.append ?? ""));
      } else if (name === "status") props.onStatus(value as Deployment);
    },
    onState(state) {
      const s = state as { reason?: string; status?: number };
      if (s.reason === "forbidden" || s.status === 403) setError(props.t("You may no longer see this."));
      else if (s.reason === "signed_out" || s.status === 401) setError(props.t("Signed out."));
    },
  });
  return (
    <>
      {error && <Alert variant="destructive">{error}</Alert>}
      <pre
        ref={output}
        className="rdp2-log"
        tabIndex={0}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
        }}
      >
        {log || " "}
      </pre>
    </>
  );
}

function Live(props: SlotProps<Data>) {
  const t = props.t;
  const data = props.data;
  const ask = useAsk();
  const [error, setError] = useState<unknown>(null);
  const [running, setRunning] = useState(false);
  const [latest, setLatest] = useState<Deployment | null>(data.latest);
  useEffect(() => setLatest(data.latest), [data.latest]);
  const busy = data.busy || (latest != null && !latest.finished);
  const unpublished = latest?.finished && !latest.published;
  const run = async (name: string, question?: { title: string; body: string; action: string }) => {
    if (question && !(await ask({ title: t(question.title), body: t(question.body), action: t(question.action), destructive: true }))) return;
    setRunning(true);
    setError(null);
    try {
      await resources.action("services", name, [String(data.id)]);
      props.reload();
    } catch (err) {
      setError(err);
    } finally {
      setRunning(false);
    }
  };
  const notes: string[] = [];
  if (data.pending && data.status !== "new") notes.push(t("The configuration changed since the last deployment: deploy to apply it."));
  if (unpublished) notes.push(t("Traefik's routes were not written: the domains may not follow this deployment. Republish them."));
  if (data.zero_downtime && !data.zero_downtime_possible)
    notes.push(t("Zero downtime needs a health check and no host-mode port: updates stop the old task first."));
  const can = data.can ?? {};
  return (
    <div className="rdp2-form">
      {(can.deploy || can.destroy) && !busy && (
        <div className="rdp2-actions">
          {can.deploy && (
            <Button size="sm" disabled={running} onClick={() => run("deploy")}>
              {t("Deploy")}
            </Button>
          )}
          {can.deploy && data.status === "stopped" && (
            <Button size="sm" variant="outline" disabled={running} onClick={() => run("start")}>
              {t("Start")}
            </Button>
          )}
          {can.deploy && (data.status === "running" || data.status === "failed") && (
            <>
              <Button size="sm" variant="outline" disabled={running} onClick={() => run("restart")}>
                {t("Restart")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={running}
                onClick={() => run("stop", { title: "Stop this service?", body: "Its tasks are removed; Start brings it back.", action: "Stop service" })}
              >
                {t("Stop")}
              </Button>
            </>
          )}
          {can.deploy && unpublished && (
            <Button size="sm" variant="outline" disabled={running} onClick={() => run("republish")}>
              {t("Republish routes")}
            </Button>
          )}
          {can.destroy && data.status && data.status !== "new" && (
            <Button
              size="sm"
              variant="ghost"
              disabled={running}
              onClick={() =>
                run("destroy", { title: "Remove this service from Docker?", body: "Its volumes are kept; Deploy creates it again.", action: "Remove from Docker" })
              }
            >
              {t("Remove from Docker…")}
            </Button>
          )}
        </div>
      )}
      <Facts
        items={[
          [t("State"), <Status kind="service" status={data.status ?? "new"} busy={busy} t={t} />],
          [t("Swarm service"), data.swarm_name ?? "", { mono: true }],
          [t("First deployed"), data.deployed_at ? dateTime(data.deployed_at, props.locale) : t("never")],
        ]}
      />
      {notes.length > 0 && <Alert variant="warning">{notes.join(" ")}</Alert>}
      <ErrorNote error={error} />
      {can.deployments &&
        (!latest ? (
          <p className="rdp2-muted">{t("Not deployed yet: use Deploy above.")}</p>
        ) : (
          <>
            <div className="rdp2-row rdp2-wrap">
              <strong>{`${t("Deployment")} #${latest.id}`}</strong>
              <span className="rdp2-badge">{t(latest.trigger)}</span>
              <Status kind="deployment" status={latest.status} t={t} />
              {latest.image && <span className="rdp2-mono rdp2-muted rdp2-ellipsis">{latest.image}</span>}
            </div>
            {latest.error && <Alert variant="destructive">{latest.error}</Alert>}
            {latest.finished ? (
              <pre className="rdp2-log">{latest.log || " "}</pre>
            ) : (
              <LiveLog
                key={latest.id}
                {...props}
                deployment={latest}
                onStatus={(d) => {
                  setLatest((old) => (old ? { ...old, ...d } : old));
                  // Over: the record (state, pending changes) changed too.
                  if (d.finished) props.reload();
                }}
              />
            )}
          </>
        ))}
    </div>
  );
}

export function ServiceLive(props: SlotProps<Data>) {
  bindResources(props.resources);
  return (
    <ConfirmProvider t={props.t}>
      <Live {...props} />
    </ConfirmProvider>
  );
}
