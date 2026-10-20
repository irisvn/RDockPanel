// The Actions page (`/admin/p/actions`), like EasyPanel's: what the panel did
// lately — deployments, database backups and restores, Docker actions — with
// their status; a row opens its service's tab, or its log.
import { useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { EmptyState, Page, Pager, RowActions, Section, Status, TableScroll, usePaged } from "../lib/ui";
import { dateTime, relative } from "../lib/format";

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
  const items = (feed.data?.actions ?? []).filter((i) => kind === "all" || i.type === kind);
  const paged = usePaged(items, kind);
  const header = {
    title: props.title,
    eyebrow: t("Workspace"),
    heading: t("Actions"),
    description: t("Deployments, backups and Docker actions, newest first."),
  };
  if (feed.isLoading) return <Loading />;
  if (feed.error || !feed.data) return <ErrorState error={feed.error} />;
  const where = (i: Item) => (i.target ? `/p/dashboard/${i.target.project}/${i.target.service}/${i.type === "backup" ? "backups" : "deployments"}` : null);
  return (
    <Page {...header}>
      <Section
        title={t("Latest actions")}
        description={t("The latest 100: older ones stay in each service's history.")}
        flush
        actions={
          <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)} aria-label={t("Kind")}>
            <option value="all">{t("All")}</option>
            <option value="deployment">{t("Deployments")}</option>
            <option value="backup">{t("Backups")}</option>
            <option value="docker">Docker</option>
          </NativeSelect>
        }
      >
        {items.length === 0 ? (
          <EmptyState
            title={kind === "all" ? t("Nothing yet") : t("Nothing of this kind")}
            hint={t("Deployments, backups and Docker actions appear here as they run.")}
            action={kind !== "all" && <Button size="sm" variant="outline" onClick={() => setKind("all")}>{t("Show all")}</Button>}
          />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Action")}</th>
                    <th>{t("Where")}</th>
                    <th>{t("State")}</th>
                    <th className="rdp2-hide-sm">{t("Started")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((i) => {
                    const path = where(i);
                    return (
                      <tr key={`${i.type}-${i.id}`}>
                        <td>
                          <div className="rdp2-strong-cell">{t(i.what)}</div>
                          {i.detail && (
                            <div className="rdp2-muted rdp2-small rdp2-ellipsis rdp2-cell-clip" title={i.detail}>
                              {i.detail}
                            </div>
                          )}
                        </td>
                        <td className="rdp2-nowrap">{i.target ? `${i.target.project} / ${i.target.service}` : <span className="rdp2-muted">–</span>}</td>
                        <td>
                          <Status kind={i.type === "deployment" ? "deployment" : "action"} status={i.status} t={t} detail={i.error ?? undefined} />
                          {i.error && <div className="rdp2-error rdp2-small rdp2-clamp">{i.error}</div>}
                        </td>
                        <td className="rdp2-hide-sm" title={dateTime(i.created_at, props.locale)}>
                          {relative(i.created_at, props.locale)}
                        </td>
                        <td>
                          <RowActions label={t("Actions for {host}").replace("{host}", t(i.what))}>
                            {i.type === "docker" ? (
                              <Button size="sm" variant="ghost" onClick={() => setOpen(i.id)}>
                                {t("Log")}
                              </Button>
                            ) : (
                              path && (
                                <Button size="sm" variant="ghost" onClick={() => props.navigate(path)}>
                                  {t("Open")}
                                </Button>
                              )
                            )}
                          </RowActions>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={items.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
      {open !== null && <DockerLog {...props} id={open} onClose={() => setOpen(null)} />}
    </Page>
  );
}
