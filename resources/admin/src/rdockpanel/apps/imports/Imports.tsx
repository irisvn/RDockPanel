// Importing existing containers (docs/import.md): the dialog that picks a
// source, and the import's page (its plan, what it will lose, what it is
// doing, and the buttons of each state). The plan holds names and findings,
// never a value of an environment variable.
import type { PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { Fragment, useState } from "react";
import { Icon } from "../../lib/icons";
import { useAdminQuery } from "../../lib/query";
import { EmptyState, Field, Metric, type MenuItem, Pager, Page, RowActions, Section, Status, StatusBadge, TableScroll, useAsk, usePaged } from "../../lib/ui";

type T = (s: string) => string;

type Finding = { code: string; level: "BLOCK" | "CONFIRM" | "WARN" | "INFO"; field: string; text: string; how_to_unblock: string | null; name?: string | null };
type PlanItem = {
  container_id: string;
  name: string;
  compose_project: string | null;
  compose_service: string | null;
  choice: "bring" | "leave" | "blocked";
  env_names: string[];
  command_secrets: number;
  tunnel_token: boolean;
  as_tunnel: boolean;
  ports: Array<{ host_ip: string; host_port: number; container_port: number; protocol: string }>;
  domains: Array<{ host: string; path: string; port: number | null; https: boolean; skip: boolean }>;
  findings: Finding[];
};
type Edge = { from: string; to: string; kind: string; signal: string; hardness: string };
type Plan = {
  project_name: string;
  items: PlanItem[];
  edges: Edge[];
  levels: string[][];
  truncated: boolean;
  compose?: Array<{ project: string; without_container: string[]; notes: string[] }>;
};
type ImportVolume = { id: number; original_name: string; mode: string; panel_volume: string | null; copied_at: string | null; released_at: string | null };
type ImportView = {
  id: number;
  state: string;
  source_kind: string;
  source_ref: string;
  project_name: string;
  error: string | null;
  plan: Plan | null;
  volumes: ImportVolume[];
};

/** The states in which a step runs: the page looks again every 2 seconds. */
const RUNNING = ["discovering", "queued", "preparing", "stopping", "draining", "copying", "editing_compose", "deploying", "observing", "undoing", "draining_panel", "restoring"];

const LEVELS: Record<string, string> = { BLOCK: "Blocks", CONFIRM: "Confirm", WARN: "Lost", INFO: "Note" };

/** What kind of edge joins two containers (English source, translated). */
const EDGE_KINDS: Record<string, string> = {
  dns: "Called by name",
  depends_on: "Depends on",
  links: "Linked",
  ip: "Fixed address",
  network_mode: "Network mode",
  volumes_from: "Volumes from",
  volume: "Shared data",
  bind: "Shared data",
};

/** An error's message, translated when the panel knows it. */
function message(t: T, error: unknown): string {
  // A refused field says why (the form's errors); the message alone says "invalid".
  const fields = (error as { fieldErrors?: Record<string, string[]> } | null)?.fieldErrors;
  const said = fields ? Object.values(fields).flat() : [];
  if (said.length > 0) return said.map((m) => t(m)).join(" ");
  return t(error instanceof Error ? error.message : String(error));
}

/** The finding codes of a domain the panel leaves out: their field is the domain. */
const DOMAIN_LEFT = ["domain_taken", "domain_invalid"];

/** A finding's words (translated), with the container it names. */
function said(t: T, f: Finding, text: string) {
  return t(text).replaceAll("{name}", f.name ?? "");
}

/** A finding's text (translated), named by its domain when it is a domain's. */
function findingText(t: T, f: Finding) {
  return DOMAIN_LEFT.includes(f.code) ? `${f.field}: ${said(t, f, f.text)}` : t(f.text);
}

/** Words of a finding's level (translated). */
function levelWord(t: T, level: Finding["level"]) {
  return t(LEVELS[level]);
}

// ------------------------------------------------------------ Source dialog

type SourceGroup = { project: string; compose: boolean; containers: Array<{ id: string; name: string; image: string; state: string; compose_service: string | null; excluded: boolean }> };

/** Picks a container or a Compose project; reading it makes the import. */
export function SourceDialog(props: PageProps<unknown> & { onClose: () => void }) {
  const t = props.t;
  const [page, setPage] = useState(0);
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const sources = useAdminQuery<{ groups: SourceGroup[]; total: number }>(props, "/imports/sources", { query: { page: page + 1 }, refetchOnMount: "always" });
  const groups = (sources.data?.groups ?? []).filter((g) => g.project.toLowerCase().includes(search.trim().toLowerCase()));
  const read = async (group: SourceGroup) => {
    setBusy(group.project);
    setError(null);
    try {
      const first = group.containers.find((c) => !c.excluded);
      const made = await props.api.post<{ id: number }>("/imports", group.compose ? { source_kind: "project", source_ref: group.project } : { source_kind: "container", source_ref: first?.id });
      props.toast.success(t("Reading queued"));
      props.onClose();
      props.navigate(`/p/dashboard/_import/${made.id}`);
    } catch (err) {
      setError(err);
      setBusy(null);
    }
  };
  return (
    <Dialog open onClose={props.onClose} title={t("Bring existing containers into the panel")} className="rdp2-wide">
      <div className="rdp2-form">
        <p className="rdp2-muted">{t("Choose a container or a Compose project. The panel reads the containers it needs with it.")}</p>
        <Input aria-label={t("Search")} placeholder={t("Search")} value={search} onChange={(e) => setSearch(e.target.value)} />
        {error ? <Alert variant="destructive">{message(t, error)}</Alert> : null}
        {sources.isLoading ? (
          <Loading />
        ) : sources.error ? (
          <ErrorState error={sources.error} />
        ) : groups.length === 0 ? (
          <EmptyState title={t("No containers.")} />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Project or container")}</th>
                    <th>{t("Containers")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {groups.map((g) => {
                    const usable = g.containers.filter((c) => !c.excluded);
                    return (
                      <tr key={g.project}>
                        <td>
                          <span className="rdp2-strong-cell">{g.project}</span>
                          <div className="rdp2-muted rdp2-small">{g.compose ? t("Compose project") : t("Container")}</div>
                        </td>
                        <td>
                          {usable.length === 0 ? (
                            <span className="rdp2-muted">{t("Belongs to the panel")}</span>
                          ) : (
                            <span className="rdp2-mono rdp2-small">{usable.map((c) => c.compose_service ?? c.name).join(", ")}</span>
                          )}
                        </td>
                        <td>
                          {usable.length > 0 && (
                            <Button size="sm" disabled={busy !== null} onClick={() => read(g)}>
                              {busy === g.project ? t("Reading…") : t("Read")}
                            </Button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={sources.data?.total ?? 0} at={page} onPage={setPage} t={t} />
          </>
        )}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Close")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------ The import page

/** The plan's container `id` by name, for the edges. */
function nameOf(plan: Plan, id: string) {
  return plan.items.find((i) => i.container_id === id)?.name ?? id.slice(0, 12);
}

export function ImportPage(props: PageProps<unknown> & { id: number }) {
  const t = props.t;
  const ask = useAsk();
  const [busy, setBusy] = useState<string | null>(null);
  const [detail, setDetail] = useState<string | null>(null);
  const [name, setName] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const view = useAdminQuery<ImportView>(props, `/imports/${props.id}`, {
    refetchOnMount: "always",
    refetchInterval: (query: { state: { data?: ImportView } }) => (RUNNING.includes(query.state.data?.state ?? "") ? 2000 : false),
  });
  if (view.isLoading) return <Loading />;
  if (view.error) return <ErrorState error={view.error} />;
  const imp = view.data;
  if (!imp) return <EmptyState title={t("This import does not exist.")} />;
  const plan = imp.plan;
  const state = imp.state;
  const isMove = imp.source_kind === "move";
  const items = plan?.items ?? [];
  const blocked = items.filter((i) => i.choice === "blocked").length;
  const confirms = items.filter((i) => i.choice !== "blocked" && i.findings.some((f) => f.level === "CONFIRM")).length;
  const run = async (key: string, path: string, body: unknown, done: string, method: "post" | "delete" = "post") => {
    setBusy(key);
    setError(null);
    try {
      if (method === "delete") await props.api.delete(path);
      else await props.api.post(path, body ?? {});
      props.toast.success(t(done));
      await view.refetch();
      return true;
    } catch (err) {
      setError(err);
      return false;
    } finally {
      setBusy(null);
    }
  };
  const start = async () => {
    const confirmations = items
      .filter((i) => i.choice === "bring")
      .flatMap((i) => i.findings.filter((f) => f.level === "CONFIRM").map((f) => ({ name: i.compose_service ?? i.name, text: findingText(t, f) })));
    const ok = await ask({
      title: t("Start importing {project}?").replace("{project}", imp.project_name),
      body: (
        <>
          <p>{t("The panel stops the originals, copies their data, changes their Compose file and deploys the services. They are kept to undo to.")}</p>
          {confirmations.length > 0 && (
            <>
              <p>{t("By starting you confirm:")}</p>
              <ul className="rdp2-findings">
                {confirmations.map((c, n) => (
                  <li key={n}>
                    <span>
                      <strong>{c.name}</strong>: {c.text}
                    </span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      ),
      action: t("Start importing"),
      destructive: true,
      typed: imp.project_name,
    });
    if (ok) await run("start", `/imports/${imp.id}/start`, { confirm: imp.project_name }, "Import queued");
  };
  const undo = async () => {
    const ok = await ask({
      title: t("Undo this import?"),
      body: t("The new services are removed and the originals run again. What the new services wrote stays in their volumes."),
      action: t("Undo"),
      destructive: true,
    });
    if (ok) await run("undo", isMove ? `/services/${imp.source_ref}/move-data/undo` : `/imports/${imp.id}/undo`, {}, "Undo queued");
  };
  const finish = async () => {
    const ok = await ask({
      title: t("Finish the import?"),
      body: t("The original containers are removed (not their volumes). It can no longer be undone."),
      action: t("Remove the originals"),
      destructive: true,
      typed: imp.project_name,
    });
    if (ok) await run("finish", `/imports/${imp.id}/finish`, {}, "Finished");
  };
  const header = (
    <>
      {state === "planned" && (
        <>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("rescan", `/imports/${imp.id}/rescan`, {}, "Reading queued")}>
            {t("Read again")}
          </Button>
          <Button disabled={busy !== null || blocked > 0 || items.every((i) => i.choice !== "bring")} onClick={start}>
            {busy === "start" ? t("Starting…") : t("Start importing…")}
          </Button>
        </>
      )}
      {state === "discover_failed" && (
        <Button variant="outline" disabled={busy !== null} onClick={() => run("rescan", `/imports/${imp.id}/rescan`, {}, "Reading queued")}>
          {t("Read again")}
        </Button>
      )}
      {state === "needs_attention" && (
        <>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("retry", `/imports/${imp.id}/retry`, {}, "Retry queued")}>
            {t("Retry the step")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("continue", `/imports/${imp.id}/continue`, {}, "Continuing")}>
            {t("Continue")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("accept", `/imports/${imp.id}/accept`, {}, "Accepted")}>
            {t("Accept as it is")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={undo}>
            {t("Undo")}
          </Button>
        </>
      )}
      {(RUNNING.includes(state) && !["discovering", "undoing", "draining_panel", "restoring"].includes(state)) && (
        <Button variant="outline" disabled={busy !== null} onClick={undo}>
          {t("Undo")}
        </Button>
      )}
      {state === "active" && (
        <>
          <Button variant="outline" disabled={busy !== null} onClick={undo}>
            {t("Undo")}
          </Button>
          {!isMove && (
            <Button disabled={busy !== null} onClick={finish}>
              {t("Finish")}
            </Button>
          )}
        </>
      )}
    </>
  );
  const menu: MenuItem[] = [];
  if (!isMove && ["active", "needs_attention", "finished"].includes(state)) {
    menu.push({ label: t("Stop the originals that run again"), onSelect: () => void run("stop", `/imports/${imp.id}/stop-originals`, {}, "Stopped") });
    menu.push({ label: t("Stop watching the originals"), onSelect: () => void run("watch", `/imports/${imp.id}/stop-watching`, {}, "Not watching any more") });
  }
  if (["planned", "discover_failed"].includes(state)) {
    menu.push("separator" as const, {
      label: t("Delete…"),
      destructive: true,
      onSelect: async () => {
        const ok = await ask({ title: t("Delete this import?"), action: t("Delete import"), destructive: true });
        if (ok && (await run("delete", `/imports/${imp.id}`, null, "Deleted", "delete"))) props.navigate("/p/dashboard");
      },
    });
  }
  return (
    <Page
      title={props.title}
      eyebrow={isMove ? t("Move") : t("Import")}
      heading={
        <>
          {imp.project_name} <Status kind="import" status={state} t={t} />
        </>
      }
      description={isMove ? t("Moves the folders a service uses in place into volumes of the panel's.") : t("Brings containers started outside the panel into a project. The originals are kept to undo to.")}
      actions={
        <>
          {header}
          {menu.length > 0 && <RowActions label={t("Actions")} menu={menu} header />}
        </>
      }
    >
      {error ? <Alert variant="destructive">{message(t, error)}</Alert> : null}
      {imp.error && <Alert variant="destructive">{t(imp.error)}</Alert>}
      {plan && (
        <>
          <Section title={t("Overview")}>
            <div className="rdp2-metrics">
              <Metric plain label={t("Read")} value={items.length} />
              <Metric plain label={t("To bring")} value={items.filter((i) => i.choice === "bring").length} />
              <Metric plain label={t("Blocked")} value={blocked} />
            </div>
            {plan.truncated && <Alert>{t("More than 50 containers are tied together: the rest is left out.")}</Alert>}
            {blocked > 0 ? (
              <Alert variant="destructive">{t("Something blocks the import: see the containers below.")}</Alert>
            ) : confirms > 0 && state === "planned" ? (
              <Alert>{t("{n} containers need your attention before the import starts.").replace("{n}", String(confirms))}</Alert>
            ) : null}
            {state === "planned" && (
              <form
                className="rdp2-actions"
                onSubmit={async (e) => {
                  e.preventDefault();
                  if (name === null) return;
                  try {
                    await props.api.patch(`/imports/${imp.id}/plan`, { project_name: name.trim() });
                    props.toast.success(t("Saved"));
                    setName(null);
                    await view.refetch();
                  } catch (err) {
                    setError(err);
                  }
                }}
              >
                <Field label={t("Project")}>
                  <Input value={name ?? plan.project_name} onChange={(e) => setName(e.target.value)} />
                </Field>
                <Button type="submit" variant="outline" disabled={name === null}>
                  {t("Save")}
                </Button>
              </form>
            )}
          </Section>
          <Section title={t("Order and relations")} description={t("A level starts when the one before is running.")}>
            <div className="rdp2-levels">
              {plan.levels.map((level, i) => (
                <Fragment key={i}>
                  {i > 0 && <Icon name="arrowRight" className="rdp2-muted" />}
                  <span className="rdp2-level">{level.map((id) => nameOf(plan, id)).join(" · ")}</span>
                </Fragment>
              ))}
            </div>
            {plan.edges.length > 0 && (
              <TableScroll>
                <table className="rdp2-table">
                  <thead>
                    <tr>
                      <th>{t("From")}</th>
                      <th>{t("To")}</th>
                      <th>{t("Kind")}</th>
                      <th>{t("Seen in")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {plan.edges.map((e, i) => (
                      <tr key={i}>
                        <td className="rdp2-strong-cell">{nameOf(plan, e.from)}</td>
                        <td>{nameOf(plan, e.to)}</td>
                        <td>{t(EDGE_KINDS[e.kind] ?? e.kind)}</td>
                        <td className="rdp2-mono rdp2-small">{e.signal}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </TableScroll>
            )}
          </Section>
          <Section title={t("Containers")} flush>
            <PlanTable {...props} imp={imp} plan={plan} onDetail={setDetail} />
          </Section>
          <Lost t={t} plan={plan} />
        </>
      )}
      {imp.volumes.length > 0 && <Volumes {...props} imp={imp} run={run} busy={busy} />}
      {detail && plan && (
        <Detail {...props} imp={imp} plan={plan} id={detail} onClose={() => setDetail(null)} onSaved={() => void view.refetch()} />
      )}
    </Page>
  );
}

function PlanTable(props: PageProps<unknown> & { imp: ImportView; plan: Plan; onDetail: (id: string) => void }) {
  const { t, plan } = props;
  const paged = usePaged(plan.items);
  return (
    <>
      <TableScroll>
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Name")}</th>
              <th>{t("Data")}</th>
              <th>{t("State")}</th>
              <th className="rdp2-right">
                <span className="rdp2-sr">{t("Actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {paged.page.map((i) => (
              <tr key={i.container_id}>
                <td>
                  <span className="rdp2-strong-cell">{i.compose_service ?? i.name}</span>
                  <div className="rdp2-muted rdp2-small">{i.name}</div>
                </td>
                <td className="rdp2-small">
                  {i.as_tunnel ? <StatusBadge tone="ok" label={t("Tunnel of the panel")} /> : i.env_names.length + i.command_secrets > 0 ? `${i.env_names.length + i.command_secrets} ${t("environment variables")}` : "–"}
                </td>
                <td>
                  <ChoiceBadge t={t} item={i} />
                </td>
                <td>
                  <Button size="sm" variant="ghost" onClick={() => props.onDetail(i.container_id)}>
                    {t("Details")}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </TableScroll>
      <Pager total={plan.items.length} at={paged.at} onPage={paged.set} t={t} />
    </>
  );
}

function ChoiceBadge(props: { t: T; item: PlanItem }) {
  const { t, item } = props;
  if (item.choice === "blocked") return <StatusBadge tone="bad" label={t("Blocked")} />;
  if (item.choice === "leave") return <StatusBadge tone="off" label={t("Left behind")} />;
  const confirm = item.findings.some((f) => f.level === "CONFIRM");
  return <StatusBadge tone={confirm ? "busy" : "ok"} label={confirm ? t("Needs confirming") : t("Ready")} />;
}

/** What the import loses or asks about, container by container. */
function Lost(props: { t: T; plan: Plan }) {
  const { t, plan } = props;
  const rows = plan.items.flatMap((i) => i.findings.filter((f) => f.level !== "BLOCK").map((f) => ({ item: i, finding: f })));
  if (rows.length === 0) return null;
  return (
    <Section title={t("What is lost or changes")}>
      <ul className="rdp2-findings">
        {rows.map((r, n) => (
          <li key={n}>
            <span className="rdp2-finding-level">{levelWord(t, r.finding.level)}</span>
            <span>
              <strong>{r.item.compose_service ?? r.item.name}</strong>: {t(r.finding.text)}
              {r.finding.how_to_unblock && <div className="rdp2-muted rdp2-small">{t(r.finding.how_to_unblock)}</div>}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

/** The volumes an import copied: released by typing their name; a move's old folders cleaned. */
function Volumes(props: PageProps<unknown> & { imp: ImportView; run: (key: string, path: string, body: unknown, done: string) => Promise<boolean>; busy: string | null }) {
  const { t, imp } = props;
  const ask = useAsk();
  const over = ["finished", "reverted"].includes(imp.state);
  const isMove = imp.source_kind === "move";
  return (
    <Section title={isMove ? t("Old folders") : t("Volumes")} description={isMove ? t("The folders copied into volumes. They stay until you clean them.") : t("The originals' volumes stay until you release them.")} flush>
      <TableScroll>
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Original")}</th>
              <th>{t("Copy")}</th>
              <th className="rdp2-right">
                <span className="rdp2-sr">{t("Actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {imp.volumes.map((v) => (
              <tr key={v.id}>
                <td className="rdp2-mono rdp2-ellipsis" title={v.original_name}>
                  {v.original_name}
                </td>
                <td className="rdp2-mono">{v.panel_volume ?? t("used in place")}</td>
                <td>
                  <div className="rdp2-actions">
                    {isMove && imp.state === "active" && !v.released_at && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={props.busy !== null}
                        onClick={async () => {
                          const folder = v.original_name.split("/").pop() ?? v.original_name;
                          const ok = await ask({
                            title: t("Clean the old folder?"),
                            body: t("The folder {path} is deleted. The data is in the volume.").replace("{path}", v.original_name),
                            action: t("Clean the old folder"),
                            destructive: true,
                            typed: folder,
                          });
                          if (ok) await props.run("clean", `/services/${imp.source_ref}/clean-old-folder`, { folder: v.original_name, confirm: folder }, "Cleaned");
                        }}
                      >
                        {t("Clean the old folder")}
                      </Button>
                    )}
                    {!isMove && over && !v.released_at && v.mode === "copy" && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={props.busy !== null}
                        onClick={async () => {
                          const ok = await ask({
                            title: t("Release the original volume?"),
                            body: t("{name} is removed from Docker. Its copy stays."). replace("{name}", v.original_name),
                            action: t("Remove the original volume"),
                            destructive: true,
                            typed: v.original_name,
                          });
                          if (ok) await props.run("release", `/imports/${imp.id}/volumes/${v.id}/release`, { remove: true }, "Released");
                        }}
                      >
                        {t("Remove the original volume")}
                      </Button>
                    )}
                    {v.released_at && <span className="rdp2-muted rdp2-small">{t("Released")}</span>}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </TableScroll>
    </Section>
  );
}

function Detail(props: PageProps<unknown> & { imp: ImportView; plan: Plan; id: string; onClose: () => void; onSaved: () => void }) {
  const { t } = props;
  const item = props.plan.items.find((i) => i.container_id === props.id);
  const ask = useAsk();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (!item) return null;
  const editable = props.imp.state === "planned" && item.choice !== "blocked";
  // The container becomes the panel's own tunnel, or a service again.
  const setTunnel = async (on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await props.api.patch(`/imports/${props.imp.id}/plan`, { tunnel: { [item.container_id]: on } });
      props.toast.success(t("Saved"));
      props.onSaved();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  // A domain ticked or not: the hosts left out are sent.
  const setDomain = async (host: string, on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      const skipped = item.domains.filter((d) => (d.host === host ? !on : d.skip)).map((d) => d.host);
      await props.api.patch(`/imports/${props.imp.id}/plan`, { domains: { [item.container_id]: skipped } });
      props.toast.success(t("Saved"));
      props.onSaved();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  // Why the panel leaves a domain out (it cannot be ticked then).
  const leftOut = (d: PlanItem["domains"][number]) => item.findings.find((f) => DOMAIN_LEFT.includes(f.code) && f.field === (d.path === "/" ? d.host : d.host + d.path));
  const choose = async (choice: "bring" | "leave") => {
    setBusy(true);
    setError(null);
    try {
      try {
        await props.api.patch(`/imports/${props.imp.id}/plan`, { choices: { [item.container_id]: choice } });
      } catch (err) {
        const fields = (err as { fieldErrors?: Record<string, string[]> }).fieldErrors ?? {};
        if (!fields.confirm) throw err;
        // Leaving it behind breaks the calls to it: said, then confirmed.
        const ok = await ask({
          title: t("Leave {name} behind?").replace("{name}", item.name),
          body: fields.confirm.join(" "),
          action: t("Leave it behind"),
          destructive: true,
        });
        if (!ok) return;
        await props.api.patch(`/imports/${props.imp.id}/plan`, { choices: { [item.container_id]: choice }, confirm: true });
      }
      props.toast.success(t("Saved"));
      props.onSaved();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open onClose={props.onClose} title={item.compose_service ?? item.name} >
      <div className="rdp2-form">
        {error ? <Alert variant="destructive">{message(t, error)}</Alert> : null}
        <Field label={t("Container")}>
          <span className="rdp2-mono">{item.name}</span>
        </Field>
        <Field label={t("Environment variables (names only)")}>
          <span className="rdp2-mono rdp2-small">{item.env_names.join(" · ") || (item.command_secrets > 0 ? "" : "–")}</span>
          {item.command_secrets > 0 && (
            <span className="rdp2-small rdp2-muted">
              {t("{n} more from the command line (RDP_SECRET_1…): a password written there moves to the environment.").replace("{n}", String(item.command_secrets))}
            </span>
          )}
        </Field>
        {item.tunnel_token && (
          <Field
            label={t("How it runs")}
            help={t("A Cloudflare tunnel can become a tunnel of the panel (Settings → Cloudflare Tunnel), named after the container, instead of a service. Every public hostname then points to Traefik, and each service needs a domain.")}
          >
            {editable ? (
              <div>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => setTunnel(!item.as_tunnel)}
                >
                  {item.as_tunnel ? t("Run it as a service") : t("Make it a tunnel of the panel")}
                </Button>
              </div>
            ) : (
              <span>{item.as_tunnel ? t("A tunnel of the panel") : t("A service")}</span>
            )}
          </Field>
        )}
        <Field label={t("Ports")}>
          <span className="rdp2-mono rdp2-small">
            {item.ports.length === 0 ? "–" : item.ports.map((p) => `${p.host_ip || "*"}:${p.host_port} → ${p.container_port}/${p.protocol}`).join(", ")}
          </span>
        </Field>
        {item.domains.length > 0 && (
          <Field label={t("Domains")} help={t("Ticked domains are added to the service when it deploys.")}>
            <div className="rdp2-form">
              {item.domains.map((d) => {
                const why = leftOut(d);
                return (
                  <label key={`${d.host}${d.path}`} className="rdp2-row">
                    <Checkbox checked={!d.skip} disabled={!editable || busy || !!why} onChange={(e) => setDomain(d.host, e.target.checked)} />
                    <span className="rdp2-mono">
                      {d.host}
                      {d.path === "/" ? "" : d.path} → {d.port ?? 80}
                    </span>
                    {why && <span className="rdp2-muted rdp2-small">{said(t, why, why.text)}</span>}
                  </label>
                );
              })}
            </div>
          </Field>
        )}
        <div>
          <div className="rdp2-field-label">{t("Findings")}</div>
          {item.findings.length === 0 ? (
            <p className="rdp2-muted">{t("Nothing to say about this container.")}</p>
          ) : (
            <ul className="rdp2-findings">
              {item.findings.map((f, n) => (
                <li key={n}>
                  <span className="rdp2-finding-level">{levelWord(t, f.level)}</span>
                  <span>
                    {findingText(t, f)}
                    {f.how_to_unblock && <div className="rdp2-muted rdp2-small">{said(t, f, f.how_to_unblock)}</div>}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Close")}
          </Button>
          {editable && (
            <Button type="button" variant="outline" disabled={busy} onClick={() => choose(item.choice === "bring" ? "leave" : "bring")}>
              {busy ? t("Saving…") : item.choice === "bring" ? t("Leave it behind") : t("Bring it")}
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}
