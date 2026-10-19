// The Actions page (`/admin/p/actions`), like EasyPanel's: what the panel did
// lately — deployments, database backups and restores, Docker actions — with
// their status; a row opens its service's tab, or its log.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { StatusBadge, when } from "../lib/ui";

type Item = {
  type: "deployment" | "backup" | "docker";
  id: number;
  what: string;
  status: string;
  error: string | null;
  detail?: string;
  target?: { project: string; service: string } | null;
  created_at: string;
  finished_at: string | null;
};

function DockerLog(props: PageProps<unknown> & { id: number; onClose: () => void }) {
  const t = props.t;
  const action = useAdminQuery<{ action: { log: string | null; error: string | null; status: string } }>(props, `/docker/actions/${props.id}`, {
    refetchInterval: (q) => (q.state.data && ["queued", "running"].includes(q.state.data.action.status) ? 1500 : false),
  });
  return (
    <Dialog open onClose={props.onClose} title={`${t("Action")} #${props.id}`} className="rdp2-wide">
      {action.isLoading && <Loading />}
      {action.data?.action.error && <Alert variant="destructive">{action.data.action.error}</Alert>}
      {action.data && <pre className="rdp2-log">{action.data.action.log || ""}</pre>}
    </Dialog>
  );
}

export function Actions(props: PageProps<unknown>) {
  const t = props.t;
  const feed = useAdminQuery<{ actions: Item[] }>(props, "/apps/actions", { refetchInterval: 3000 });
  const [kind, setKind] = useState("all");
  const [open, setOpen] = useState<number | null>(null);
  if (feed.isLoading) return <Loading />;
  if (feed.error || !feed.data) return <ErrorState error={feed.error} />;
  const items = feed.data.actions.filter((i) => kind === "all" || i.type === kind);
  const go = (i: Item) => {
    if (i.type === "docker") return setOpen(i.id);
    if (!i.target) return;
    props.navigate(`/p/dashboard/${i.target.project}/${i.target.service}/${i.type === "backup" ? "backups" : "deployments"}`);
  };
  return (
    <div className="rdp2-page">
      <div className="rdp2-head">
        <p className="rdp2-muted">{t("Deployments, backups and Docker actions, newest first.")}</p>
        <NativeSelect className="rdp2-narrow" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="all">{t("All")}</option>
          <option value="deployment">{t("Deployments")}</option>
          <option value="backup">{t("Backups")}</option>
          <option value="docker">Docker</option>
        </NativeSelect>
      </div>
      {items.length === 0 ? (
        <p className="rdp2-muted">{t("Nothing yet.")}</p>
      ) : (
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Action")}</th>
              <th>{t("Where")}</th>
              <th>{t("Status")}</th>
              <th>{t("Started")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {items.map((i) => (
              <tr key={`${i.type}-${i.id}`}>
                <td>
                  {t(i.what)}
                  {i.detail && <span className="rdp2-muted rdp2-small"> · {i.detail}</span>}
                </td>
                <td className="rdp2-small">{i.target ? `${i.target.project}/${i.target.service}` : ""}</td>
                <td>
                  <StatusBadge status={i.status} label={t(i.status)} />
                  {i.error && <div className="rdp2-error rdp2-small rdp2-clamp">{i.error}</div>}
                </td>
                <td className="rdp2-small">{when(i.created_at, props.locale)}</td>
                <td className="rdp2-row rdp2-end">
                  <Button size="sm" variant="ghost" onClick={() => go(i)} disabled={i.type !== "docker" && !i.target}>
                    {i.type === "docker" ? t("Log") : t("Open")}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {open !== null && <DockerLog {...props} id={open} onClose={() => setOpen(null)} />}
    </div>
  );
}
