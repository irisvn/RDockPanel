// Importing existing containers (docs/import.md): the dialog that picks a
// source, and the import's page (its plan, what it will lose, what it is
// doing, and the buttons of each state). The plan holds names and findings,
// never a value of an environment variable.
import type { PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { Fragment, useRef, useState } from "react";
import { Icon } from "../../lib/icons";
import { dateTime, size } from "../../lib/format";
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
  /** The database engine it can become (`postgres`…), its configuration kept. */
  database?: string | null;
  /** Chosen to become that database service. */
  as_database?: boolean;
  ports: Array<{ host_ip: string; host_port: number; container_port: number; protocol: string }>;
  domains: Array<{ host: string; path: string; port: number | null; https: boolean; skip: boolean; auth?: string; guard?: string; lost?: string[]; allow?: { ipAllowList?: { sourceRange?: string[] } } }>;
  findings: Finding[];
  /** Bytes of its volumes to copy (null: Docker did not say). */
  copy_bytes?: number | null;
  /** It writes data (a volume or folder not read only). */
  writes?: boolean;
  /** Ports published on another, as the admin chose (`"80/tcp": 8081`). */
  port_remap?: Record<string, number>;
  /** The admin's choice for a health check the panel could not cut. */
  healthcheck_choice?: "drop" | { command: string } | null;
};
type Edge = { from: string; to: string; kind: string; signal: string; hardness: string };
type Plan = {
  project_name: string;
  items: PlanItem[];
  edges: Edge[];
  levels: string[][];
  truncated: boolean;
  /** Free room where Docker keeps volumes (null: not measured). */
  free_bytes?: number | null;
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
  /** What it did, newest first (names only). */
  events?: Array<{ text: string; subject: string | null; at: string }>;
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
  // Searched by the server: a group on another page is found too.
  const sources = useAdminQuery<{ groups: SourceGroup[]; total: number }>(props, "/imports/sources", { query: { page: page + 1, search: search.trim() }, refetchOnMount: "always" });
  const groups = sources.data?.groups ?? [];
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
        <Input
          aria-label={t("Search")}
          placeholder={t("Search")}
          autoFocus
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(0);
          }}
        />
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
                          <div className="rdp2-muted rdp2-small">{g.compose ? t("Compose project") : t("A single container")}</div>
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
  const [naming, setNaming] = useState(false);
  const [nameError, setNameError] = useState<string[] | undefined>(undefined);
  const nameInput = useRef<HTMLInputElement>(null);
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
  // What the brought containers copy, and how long they may be down: the
  // stops, the copy (~100 MB/s), each level's deployment (~30 s), the watch (60 s).
  const brought = items.filter((i) => i.choice === "bring" && !i.as_tunnel);
  const toCopy = brought.some((i) => i.copy_bytes == null) ? null : brought.reduce((n, i) => n + (i.copy_bytes ?? 0), 0);
  // The stops run one after the other: one that writes data gets up to 120 s,
  // another up to 30 s (most stop in seconds: the estimate is the most).
  const stopS = brought.reduce((n, i) => n + (i.writes ? 120 : 30), 0);
  const downtimeS = stopS + (toCopy ?? 0) / 100_000_000 + (plan?.levels.length ?? 1) * 30 + 60;
  const downtimeMin = Math.max(1, Math.ceil(downtimeS / 60));
  const noRoom = plan?.free_bytes != null && toCopy !== null && toCopy > 0 && toCopy + Math.max(toCopy / 10, 2 * 1024 ** 3) > plan.free_bytes;
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
            {busy === "rescan" ? t("Reading…") : t("Read again")}
          </Button>
          <Button disabled={busy !== null || blocked > 0 || items.every((i) => i.choice !== "bring")} onClick={start}>
            {busy === "start" ? t("Starting…") : t("Start importing…")}
          </Button>
        </>
      )}
      {state === "discover_failed" && (
        <Button variant="outline" disabled={busy !== null} onClick={() => run("rescan", `/imports/${imp.id}/rescan`, {}, "Reading queued")}>
          {busy === "rescan" ? t("Reading…") : t("Read again")}
        </Button>
      )}
      {state === "needs_attention" && (
        <>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("retry", `/imports/${imp.id}/retry`, {}, "Retry queued")}>
            {busy === "retry" ? t("Retrying…") : t("Retry the step")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("continue", `/imports/${imp.id}/continue`, {}, "Continue queued")}>
            {busy === "continue" ? t("Continuing…") : t("Continue")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={() => run("accept", `/imports/${imp.id}/accept`, {}, "Accepted")}>
            {busy === "accept" ? t("Accepting…") : t("Accept as it is")}
          </Button>
          <Button variant="outline" disabled={busy !== null} onClick={undo}>
            {busy === "undo" ? t("Undoing…") : t("Undo")}
          </Button>
        </>
      )}
      {(RUNNING.includes(state) && !["discovering", "undoing", "draining_panel", "restoring"].includes(state)) && (
        <Button variant="outline" disabled={busy !== null} onClick={undo}>
          {busy === "undo" ? t("Undoing…") : t("Undo")}
        </Button>
      )}
      {state === "active" && (
        <>
          <Button variant="outline" disabled={busy !== null} onClick={undo}>
            {busy === "undo" ? t("Undoing…") : t("Undo")}
          </Button>
          {!isMove && (
            <Button disabled={busy !== null} onClick={finish}>
              {busy === "finish" ? t("Finishing…") : t("Finish")}
            </Button>
          )}
        </>
      )}
      {state === "finishing" && (
        // Stopped halfway (Docker refused a removal): run again.
        <Button disabled={busy !== null} onClick={() => run("finish", `/imports/${imp.id}/finish`, {}, "Finished")}>
          {busy === "finish" ? t("Finishing…") : t("Finish again")}
        </Button>
      )}
    </>
  );
  const menu: MenuItem[] = [];
  if (!isMove && ["active", "needs_attention", "finishing"].includes(state)) {
    menu.push({ label: t("Stop the originals that run again"), onSelect: () => void run("stop", `/imports/${imp.id}/stop-originals`, {}, "Stopped") });
  }
  if (!isMove && ["active", "needs_attention", "finishing", "finished"].includes(state)) {
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
      {state === "finishing" && <Alert>{t("Some originals could not be removed: the import can no longer be undone. Finish again once Docker lets it.")}</Alert>}
      {plan && (
        <>
          <Section title={t("Overview")}>
            <div className="rdp2-metrics">
              <Metric plain label={t("Read")} value={items.length} />
              <Metric plain label={t("To bring")} value={items.filter((i) => i.choice === "bring").length} />
              <Metric plain label={t("Blocked")} value={blocked} />
              <Metric plain label={t("Data to copy")} value={toCopy === null ? t("unknown") : size(toCopy, props.locale)} />
              <Metric plain label={t("Free room")} value={plan.free_bytes == null ? t("unknown") : size(plan.free_bytes, props.locale)} />
              <Metric plain label={t("Downtime (estimate)")} value={t("up to {n} min").replace("{n}", String(downtimeMin))} />
            </div>
            {noRoom && <Alert variant="destructive">{t("There is not enough free room for the data to copy: free some, then read the plan again.")}</Alert>}
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
                  if (name === null || naming) return;
                  setNaming(true);
                  setNameError(undefined);
                  try {
                    await props.api.patch(`/imports/${imp.id}/plan`, { project_name: name.trim() });
                    props.toast.success(t("Saved"));
                    setName(null);
                    await view.refetch();
                  } catch (err) {
                    const fields = (err as { fieldErrors?: Record<string, string[]> } | null)?.fieldErrors;
                    if (fields?.project_name) {
                      setNameError(fields.project_name.map((m) => t(m)));
                      nameInput.current?.focus();
                    } else {
                      setError(err);
                    }
                  } finally {
                    setNaming(false);
                  }
                }}
              >
                <Field label={t("Project")} error={nameError}>
                  <Input ref={nameInput} value={name ?? plan.project_name} aria-invalid={nameError ? true : undefined} onChange={(e) => setName(e.target.value)} />
                </Field>
                <Button type="submit" variant="outline" disabled={name === null || naming}>
                  {naming ? t("Saving…") : t("Save")}
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
            {plan.edges.length > 0 && <Edges t={t} plan={plan} />}
          </Section>
          <Section title={t("Containers")} flush>
            <PlanTable {...props} imp={imp} plan={plan} onDetail={setDetail} />
          </Section>
          <Lost t={t} plan={plan} />
        </>
      )}
      {(imp.events?.length ?? 0) > 0 && <Progress t={t} locale={props.locale} events={imp.events ?? []} />}
      {imp.volumes.length > 0 && <Volumes {...props} imp={imp} run={run} busy={busy} />}
      {detail && plan && (
        <Detail {...props} imp={imp} plan={plan} id={detail} onClose={() => setDetail(null)} onSaved={() => void view.refetch()} />
      )}
    </Page>
  );
}

/** The ties between containers, 25 a page. */
function Edges(props: { t: T; plan: Plan }) {
  const { t, plan } = props;
  const paged = usePaged(plan.edges);
  return (
    <>
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
            {paged.page.map((e, i) => (
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
      <Pager total={plan.edges.length} at={paged.at} onPage={paged.set} t={t} />
    </>
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
                  {i.as_tunnel ? <StatusBadge tone="ok" label={t("Tunnel of the panel")} /> : i.env_names.length + i.command_secrets > 0 ? t("{n} environment variables").replace("{n}", String(i.env_names.length + i.command_secrets)) : "–"}
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
  const paged = usePaged(rows);
  if (rows.length === 0) return null;
  return (
    <Section title={t("What is lost or changes")}>
      <ul className="rdp2-findings">
        {paged.page.map((r, n) => (
          <li key={n}>
            <span className="rdp2-finding-level">{levelWord(t, r.finding.level)}</span>
            <span>
              <strong>{r.item.compose_service ?? r.item.name}</strong>: {t(r.finding.text)}
              {r.finding.how_to_unblock && <div className="rdp2-muted rdp2-small">{t(r.finding.how_to_unblock)}</div>}
            </span>
          </li>
        ))}
      </ul>
      <Pager total={rows.length} at={paged.at} onPage={paged.set} t={t} />
    </Section>
  );
}

/** What the import did, newest first, 25 a page. */
function Progress(props: { t: T; locale: string; events: NonNullable<ImportView["events"]> }) {
  const { t, events } = props;
  const paged = usePaged(events);
  return (
    <Section title={t("Progress")} flush>
      <TableScroll>
        <table className="rdp2-table">
          <thead>
            <tr>
              <th>{t("Step")}</th>
              <th className="rdp2-right">{t("When")}</th>
            </tr>
          </thead>
          <tbody>
            {paged.page.map((e, n) => (
              <tr key={n}>
                <td>{t(e.text).replace("{name}", e.subject ?? "")}</td>
                <td className="rdp2-right rdp2-muted rdp2-small">{dateTime(e.at, props.locale)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </TableScroll>
      <Pager total={events.length} at={paged.at} onPage={paged.set} t={t} />
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
                    {isMove && ["active", "finished"].includes(imp.state) && !v.released_at && v.copied_at && (
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
  const chosen = item?.healthcheck_choice;
  const [portDraft, setPortDraft] = useState<Record<string, string>>({});
  const [portError, setPortError] = useState<string[] | undefined>(undefined);
  const [healthCommand, setHealthCommand] = useState(typeof chosen === "object" && chosen ? chosen.command : "");
  const [healthError, setHealthError] = useState<string[] | undefined>(undefined);
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
  // A database service of the panel's, or an app.
  const setDatabase = async (on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await props.api.patch(`/imports/${props.imp.id}/plan`, { database: { [item.container_id]: on } });
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
  // Ports the panel keeps, or on the loopback: they may go on another port.
  const movable = item.ports.filter(
    (p, n, all) =>
      all.findIndex((q) => q.host_port === p.host_port && q.protocol === p.protocol) === n &&
      item.findings.some((f) => (f.code === "port_reserved" || f.code === "port_loopback") && f.field === `port ${p.host_port}`),
  );
  const movePort = async (key: string, to: number | null) => {
    setBusy(true);
    setError(null);
    setPortError(undefined);
    try {
      await props.api.patch(`/imports/${props.imp.id}/plan`, { ports: { [item.container_id]: { [key]: to } } });
      props.toast.success(t("Saved"));
      setPortDraft({});
      props.onSaved();
    } catch (err) {
      const fields = (err as { fieldErrors?: Record<string, string[]> } | null)?.fieldErrors;
      if (fields?.ports) setPortError(fields.ports.map((m) => t(m)));
      else setError(err);
    } finally {
      setBusy(false);
    }
  };
  // A health check the panel could not cut: dropped, or the admin's command.
  const healthStuck = item.findings.some((f) => f.code === "healthcheck_secret" || f.code === "healthcheck_long" || f.code === "healthcheck_chosen");
  const setHealth = async (choice: "drop" | { command: string } | null) => {
    setBusy(true);
    setError(null);
    setHealthError(undefined);
    try {
      await props.api.patch(`/imports/${props.imp.id}/plan`, { healthcheck: { [item.container_id]: choice } });
      props.toast.success(t("Saved"));
      props.onSaved();
    } catch (err) {
      const fields = (err as { fieldErrors?: Record<string, string[]> } | null)?.fieldErrors;
      if (fields?.healthcheck) setHealthError(fields.healthcheck.map((m) => t(m)));
      else setError(err);
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
          body: fields.confirm.map((m) => t(m)).join(" "),
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
        <Field label={t("Original container")}>
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
        {item.database && (
          <Field
            label={t("Database")}
            help={t("As a database service it gets the panel's connection details and dumps; its image, environment, command and data stay as they are. The credentials shown are those of its environment: if they were changed inside the database since, update them there.")}
          >
            {editable ? (
              <div>
                <Button type="button" size="sm" variant="outline" disabled={busy} onClick={() => setDatabase(!item.as_database)}>
                  {busy ? t("Saving…") : item.as_database ? t("Import it as an app") : t("Import it as a database service")}
                </Button>
              </div>
            ) : null}
            <span className="rdp2-small">
              {item.as_database ? t("A database service ({engine})").replace("{engine}", item.database) : t("An app")}
            </span>
          </Field>
        )}
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
        {healthStuck && (
          <Field
            label={t("Health check")}
            help={t("Its health check holds a secret the panel cannot set apart. Drop it, or type a command that reads the secret from a variable (\"$NAME\").")}
            error={healthError}
          >
            {item.healthcheck_choice === "drop" ? (
              <span>{t("Dropped")}</span>
            ) : null}
            {props.imp.state === "planned" && (
              <div className="rdp2-form">
                <Input
                  className="rdp2-mono"
                  aria-label={t("Health check")}
                  value={healthCommand}
                  aria-invalid={healthError ? true : undefined}
                  onChange={(e) => setHealthCommand(e.target.value)}
                  placeholder={'mysqladmin ping -h 127.0.0.1 -p"$MYSQL_ROOT_PASSWORD"'}
                />
                <div className="rdp2-actions">
                  <Button type="button" size="sm" variant="outline" disabled={busy || healthCommand.trim() === ""} onClick={() => setHealth({ command: healthCommand.trim() })}>
                    {busy ? t("Saving…") : t("Use this command")}
                  </Button>
                  <Button type="button" size="sm" variant="outline" disabled={busy || item.healthcheck_choice === "drop"} onClick={() => setHealth("drop")}>
                    {t("Drop the health check")}
                  </Button>
                  {item.healthcheck_choice ? (
                    <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => setHealth(null)}>
                      {t("Choose again")}
                    </Button>
                  ) : null}
                </div>
              </div>
            )}
          </Field>
        )}
        <Field label={t("Ports")} help={movable.length > 0 ? t("A port the panel keeps, or one on the loopback, can be published on another.") : undefined} error={portError}>
          <span className="rdp2-mono rdp2-small">
            {item.ports.length === 0
              ? "–"
              : item.ports.map((p, n) => (
                  <span key={n} className="rdp2-levels">
                    <span>{`${p.host_ip || "*"}:${item.port_remap?.[`${p.host_port}/${p.protocol}`] ?? p.host_port}`}</span>
                    <Icon name="arrowRight" className="rdp2-muted" />
                    <span>{`${p.container_port}/${p.protocol}`}</span>
                  </span>
                ))}
          </span>
          {props.imp.state === "planned" &&
            movable.map((p) => {
              const key = `${p.host_port}/${p.protocol}`;
              return (
                <div key={key} className="rdp2-actions">
                  <Input
                    type="number"
                    aria-label={t("Publish {port} on").replace("{port}", key)}
                    placeholder={t("Publish {port} on").replace("{port}", key)}
                    value={portDraft[key] ?? String(item.port_remap?.[key] ?? "")}
                    onChange={(e) => setPortDraft({ ...portDraft, [key]: e.target.value })}
                  />
                  <Button type="button" size="sm" variant="outline" disabled={busy || !(portDraft[key] ?? "").trim()} onClick={() => movePort(key, Number(portDraft[key]))}>
                    {busy ? t("Saving…") : t("Publish")}
                  </Button>
                  {item.port_remap?.[key] ? (
                    <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => movePort(key, null)}>
                      {t("Not published")}
                    </Button>
                  ) : null}
                </div>
              );
            })}
        </Field>
        {item.domains.length > 0 && (
          <Field label={t("Domains")} help={t("Ticked domains are added to the service when it deploys.")}>
            <div className="rdp2-form">
              {item.domains.map((d) => {
                const why = leftOut(d);
                return (
                  <label key={`${d.host}${d.path}`} className="rdp2-row">
                    <Checkbox checked={!d.skip} disabled={!editable || busy || !!why || !!d.guard} onChange={(e) => setDomain(d.host, e.target.checked)} />
                    <span className="rdp2-mono rdp2-levels">
                      <span>
                        {d.host}
                        {d.path === "/" ? "" : d.path}
                      </span>
                      <Icon name="arrowRight" className="rdp2-muted" />
                      <span>{d.port ?? 80}</span>
                    </span>
                    {why && <span className="rdp2-muted rdp2-small">{said(t, why, why.text)}</span>}
                    {d.guard && <span className="rdp2-muted rdp2-small">{t("Protected by a Traefik middleware the panel cannot carry over: add it after protecting the service.")}</span>}
                    {d.auth && !d.skip && <span className="rdp2-muted rdp2-small">{t("Its basic auth users come along.")}</span>}
                    {d.allow && !d.skip && <span className="rdp2-muted rdp2-small">{t("Its IP allow list comes along: {ranges}.").replace("{ranges}", (d.allow.ipAllowList?.sourceRange ?? []).join(", "))}</span>}
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
          <Button type="button" variant="ghost" autoFocus onClick={props.onClose}>
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
