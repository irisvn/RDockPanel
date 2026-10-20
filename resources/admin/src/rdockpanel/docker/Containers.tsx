// The Containers page (`/admin/p/containers`): every container with its
// state and actions; one opens at `/p/containers/<id>/<tab>` with its logs,
// live stats, a terminal and its inspect document. Old links
// (`?open=<id>&tab=…`) still open it.
import { useEffect, useState } from "react";
import { type PageProps, useAdminQuery, useEvents } from "@rustcms/admin-sdk";
import { Button, Checkbox, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import {
  EmptyState,
  ErrorNote,
  IconButton,
  type MenuItem,
  Metric,
  NavLink,
  Page,
  Pager,
  RowActions,
  Section,
  Status,
  TableScroll,
  Tabs,
  useAsk,
  usePaged,
} from "../lib/ui";
import { LogViewer, TerminalView } from "../lib/viewers";
import { dateTime, percent, relative, size } from "../lib/format";
import { Icon } from "../lib/icons";

type Container = { id: string; name: string; image: string; state: string; status: string; created: number; ports: string[] };
type Can = { "docker.manage"?: boolean; "docker.logs"?: boolean; "docker.exec"?: boolean; "system.manage"?: boolean };

const path = (id: string, rest = "") => `/docker/containers/${encodeURIComponent(id)}${rest}`;

/** A Swarm task's container (`<service>.<slot>.<task>`): its service and slot. */
function taskOf(name: string): string | null {
  const m = /^(.+\.\d+)\.[a-z0-9]{20,}$/.exec(name);
  return m ? m[1] : null;
}

/** Exited containers of a task slot that has a running one: old deployments'. */
function oldTasks(list: Container[]): Set<string> {
  const live = new Set(list.filter((c) => c.state === "running").map((c) => taskOf(c.name)).filter(Boolean) as string[]);
  return new Set(list.filter((c) => c.state !== "running" && live.has(taskOf(c.name) ?? "")).map((c) => c.id));
}

export function Containers(props: PageProps<{ can?: Can }>) {
  const can = props.data?.can ?? {};
  const [id, tab] = props.path ? props.path.split("/") : [];
  // The old deep link: `?open=<id>&tab=…` becomes the sub-path.
  useEffect(() => {
    const query = new URLSearchParams(window.location.search);
    const open = query.get("open");
    if (open && !id) props.navigate(`/p/containers/${encodeURIComponent(open)}/${query.get("tab") || "logs"}`);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (id) return <ContainerDetail {...props} id={decodeURIComponent(id)} tab={tab ?? ""} perm={can} />;
  return <ContainerList {...props} perm={can} />;
}

function ContainerList(props: PageProps<unknown> & { perm: Can }) {
  const t = props.t;
  const can = props.perm;
  const list = useAdminQuery<{ containers: Container[] }>(props, "/docker/containers", { refetchInterval: 5000 });
  const [search, setSearch] = useState("");
  const [running, setRunning] = useState(false);
  const [showOld, setShowOld] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const all = [...(list.data?.containers ?? [])].sort((a, b) => a.name.localeCompare(b.name));
  const old = oldTasks(all);
  const term = search.trim().toLowerCase();
  const shown = all.filter(
    (c) =>
      (showOld || !old.has(c.id)) &&
      (!running || c.state === "running") &&
      (!term || c.name.toLowerCase().includes(term) || c.image.toLowerCase().includes(term)),
  );
  const paged = usePaged(shown, `${term}|${running}|${showOld}`);
  const header = {
    title: props.title,
    eyebrow: "Docker",
    heading: t("Containers"),
    description: t("Every container on this server, the panel's own included."),
  };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const act = async (c: Container, action: string, done: string) => {
    setBusy(c.id);
    setError(null);
    try {
      await props.api.post(path(c.id, `/${action}`), {});
      props.toast.success(t(done));
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(null);
    }
  };
  const remove = async (c: Container) => {
    const ok = await ask({
      title: t("Remove the container {name}?").replace("{name}", c.name),
      body: t("It is stopped first if it runs; a service's task is started again by Swarm."),
      action: t("Remove container"),
      destructive: true,
    });
    if (!ok) return;
    setBusy(c.id);
    setError(null);
    try {
      await props.api.delete(path(c.id), { query: { force: true } });
      props.toast.success(t("Removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(null);
    }
  };
  const detail = (c: Container, which: string) => `/p/containers/${encodeURIComponent(c.id)}/${which}`;
  return (
    <Page {...header}>
      <ErrorNote error={error} />
      <Section
        title={t("All containers")}
        flush
        actions={
          <Button size="sm" variant="outline" onClick={() => list.refetch()}>
            <Icon name="refresh" size={14} />
            {t("Refresh")}
          </Button>
        }
      >
        <div className="rdp2-toolbar">
          <Input type="search" className="rdp2-search" placeholder={t("Search")} aria-label={t("Search")} value={search} onChange={(e) => setSearch(e.target.value)} />
          <label className="rdp2-row rdp2-small">
            <Checkbox checked={running} onChange={(e) => setRunning(e.target.checked)} />
            {t("Running only")}
          </label>
          {old.size > 0 && (
            <label className="rdp2-row rdp2-small">
              <Checkbox checked={showOld} onChange={(e) => setShowOld(e.target.checked)} />
              {t("Show {n} old tasks").replace("{n}", String(old.size))}
            </label>
          )}
        </div>
        {shown.length === 0 ? (
          <EmptyState
            title={all.length === 0 ? t("No containers.") : t("No container matches.")}
            action={
              all.length > 0 && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setSearch("");
                    setRunning(false);
                  }}
                >
                  {t("Clear filters")}
                </Button>
              )
            }
          />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Name")}</th>
                    <th>{t("Image")}</th>
                    <th>{t("State")}</th>
                    <th className="rdp2-hide-sm">{t("Ports")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((c) => {
                    const manage = !!can["docker.manage"];
                    const disabled = busy === c.id;
                    const menu: MenuItem[] = [];
                    if (manage && c.state === "running") {
                      menu.push({ label: t("Restart"), disabled, onSelect: () => void act(c, "restart", "Restarted") });
                      menu.push({ label: t("Pause"), disabled, onSelect: () => void act(c, "pause", "Paused") });
                    }
                    menu.push({ label: t("Inspect"), onSelect: () => props.navigate(detail(c, "inspect")) });
                    if (manage) menu.push("separator", { label: t("Remove…"), destructive: true, disabled, onSelect: () => void remove(c) });
                    return (
                      <tr key={c.id}>
                        <td>
                          <NavLink to={detail(c, can["docker.logs"] ? "logs" : "inspect")} navigate={props.navigate} className="rdp2-link rdp2-strong-cell">
                            {c.name}
                          </NavLink>
                          <div className="rdp2-muted rdp2-small" title={dateTime(c.created * 1000, props.locale)}>
                            {c.id.slice(0, 12)} · {c.status}
                          </div>
                        </td>
                        <td>
                          <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={c.image}>
                            {c.image.split("@")[0]}
                          </span>
                        </td>
                        <td>
                          <Status kind="container" status={c.state} t={t} detail={c.status} />
                        </td>
                        <td className="rdp2-hide-sm">
                          <span className="rdp2-mono rdp2-small">{c.ports.join(", ") || "–"}</span>
                        </td>
                        <td>
                          <RowActions label={t("Actions for {host}").replace("{host}", c.name)} menu={menu}>
                            {manage && c.state === "running" && (
                              <Button size="sm" variant="ghost" disabled={disabled} onClick={() => act(c, "stop", "Stopped")}>
                                {t("Stop")}
                              </Button>
                            )}
                            {manage && c.state === "paused" && (
                              <Button size="sm" variant="ghost" disabled={disabled} onClick={() => act(c, "unpause", "Resumed")}>
                                {t("Unpause")}
                              </Button>
                            )}
                            {manage && c.state !== "running" && c.state !== "paused" && (
                              <Button size="sm" variant="ghost" disabled={disabled} onClick={() => act(c, "start", "Started")}>
                                {t("Start")}
                              </Button>
                            )}
                            {can["docker.logs"] && <IconButton icon="logs" label={t("Logs of {name}").replace("{name}", c.name)} onClick={() => props.navigate(detail(c, "logs"))} />}
                            {can["docker.exec"] && c.state === "running" && (
                              <IconButton icon="terminal" label={t("Terminal of {name}").replace("{name}", c.name)} onClick={() => props.navigate(detail(c, "terminal"))} />
                            )}
                          </RowActions>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </TableScroll>
            <Pager
              total={shown.length}
              at={paged.at}
              onPage={paged.set}
              t={t}
              note={!showOld && old.size > 0 ? t("{n} old tasks hidden").replace("{n}", String(old.size)) : undefined}
            />
          </>
        )}
      </Section>
    </Page>
  );
}

type Stats = { cpu_percent: number; mem_used: number; mem_limit: number; net_rx: number; net_tx: number; blk_read: number; blk_write: number };

function LiveStats(props: PageProps<unknown> & { id: string }) {
  const t = props.t;
  const [s, setS] = useState<Stats | null>(null);
  const [cpu, setCpu] = useState<number[]>([]);
  const [mem, setMem] = useState<number[]>([]);
  const [state, setState] = useState("");
  useEvents(props, path(props.id, "/stats"), {
    onState(next) {
      const st = next as { state: string; reason?: string; status?: number };
      if (st.state === "open") setState("");
      else if (st.reason === "forbidden" || st.status === 403) setState(t("You may no longer see this."));
      else if (st.reason === "signed_out" || st.status === 401) setState(t("Signed out."));
      else if (st.status === 404) setState(t("It no longer exists."));
      else if (st.state === "reconnecting" || st.state === "closed") setState(t("Reconnecting…"));
    },
    onEvent(name, data) {
      if (name !== "stats") return;
      const next = data as Stats;
      setS(next);
      setCpu((v) => [...v.slice(-59), next.cpu_percent]);
      setMem((v) => [...v.slice(-59), next.mem_used]);
    },
  });
  const l = props.locale;
  return (
    <>
    {state && <p className="rdp2-small rdp2-muted">{state}</p>}
    <div className="rdp2-metrics">
      <Metric plain label={t("CPU")} value={s ? percent(s.cpu_percent, l) : "–"} values={cpu} />
      <Metric
        plain
        label={t("Memory")}
        value={s ? size(s.mem_used, l) : "–"}
        sub={s && s.mem_limit ? `${t("of")} ${size(s.mem_limit, l)}` : undefined}
        values={mem}
      />
      <Metric plain label={t("Network")} value={s ? `↓ ${size(s.net_rx, l)}` : "–"} sub={s ? `↑ ${size(s.net_tx, l)}` : undefined} />
      <Metric plain label={t("Disk")} value={s ? `${t("Read")} ${size(s.blk_read, l)}` : "–"} sub={s ? `${t("Written")} ${size(s.blk_write, l)}` : undefined} />
    </div>
    </>
  );
}

function Inspect(props: PageProps<unknown> & { id: string }) {
  const doc = useAdminQuery<unknown>(props, path(props.id));
  if (doc.isLoading) return <Loading />;
  if (doc.error) return <ErrorState error={doc.error} />;
  return <pre className="rdp2-log rdp2-log-tall">{JSON.stringify(doc.data, null, 2)}</pre>;
}

function ContainerDetail(props: PageProps<unknown> & { id: string; tab: string; perm: Can }) {
  const t = props.t;
  const list = useAdminQuery<{ containers: Container[] }>(props, "/docker/containers", { refetchInterval: 5000 });
  // By id (or its start), by name, or by a service's name (its running task first).
  const all = list.data?.containers ?? [];
  const c =
    all.find((x) => x.id === props.id || x.name === props.id) ??
    all.find((x) => x.id.startsWith(props.id)) ??
    [...all].sort((a, b) => Number(b.state === "running") - Number(a.state === "running")).find((x) => x.name.startsWith(`${props.id}.`));
  const tabs = [
    ...(props.perm["docker.logs"] ? [{ id: "logs", label: t("Logs") }] : []),
    ...(c?.state === "running" ? [{ id: "stats", label: t("Stats") }] : []),
    ...(c?.state === "running" && props.perm["docker.exec"] ? [{ id: "terminal", label: t("Terminal") }] : []),
    { id: "inspect", label: t("Inspect") },
  ];
  const tab = tabs.find((x) => x.id === props.tab)?.id ?? tabs[0].id;
  if (list.isLoading) return <Loading />;
  if (list.error) return <ErrorState error={list.error} />;
  if (!c)
    return (
      <Page title={props.title} eyebrow="Docker" heading={props.id.slice(0, 12)}>
        <EmptyState title={t("This container no longer exists.")} />
      </Page>
    );
  return (
    <Page
      title={props.title}
      eyebrow="Docker"
      heading={
        <span className="rdp2-heading">
          {c.name}
          <Status kind="container" status={c.state} t={t} detail={c.status} />
        </span>
      }
      description={
        <>
          <span className="rdp2-mono">{c.image.split("@")[0]}</span> · {relative(c.created * 1000, props.locale)}
        </>
      }
    >
      <Tabs label={c.name} value={tab} onSelect={(next) => props.navigate(`/p/containers/${encodeURIComponent(c.id)}/${next}`)} tabs={tabs}>
        {tab === "logs" && (
          <Section title={t("Logs")} description={t("The latest lines, then new ones while Follow is on.")}>
            <LogViewer key={c.id} {...props} url={(n) => path(c.id, `/logs/follow/${n}`)} />
          </Section>
        )}
        {tab === "stats" && (
          <Section title={t("Stats")} description={t("Live, every few seconds.")}>
            <LiveStats key={c.id} {...props} id={c.id} />
          </Section>
        )}
        {tab === "terminal" && (
          <Section title={t("Terminal")} description={t("A shell in the container; it closes after 30 minutes without activity.")}>
            <TerminalView key={c.id} {...props} path={path(c.id, "/exec")} />
          </Section>
        )}
        {tab === "inspect" && (
          <Section title={t("Inspect")} description={t("Docker's document of this container.")}>
            <Inspect {...props} id={c.id} />
          </Section>
        )}
      </Tabs>
    </Page>
  );
}

