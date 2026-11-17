// The Nodes page (`/admin/p/nodes`): the Swarm's servers. Listed for
// docker.view; adding one (the join command, an audited read), pausing,
// draining, activating and removing one for system.manage. The panel's own
// node is never paused, drained or removed (the server refuses it too).
import { useEffect, useRef, useState } from "react";
import type { PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, Dialog, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { bytes, CopyButton, EmptyState, ErrorNote, Page, Pager, RowActions, Section, StatusBadge, TableScroll, useAsk, usePaged } from "../lib/ui";

type Node = {
  id: string;
  name: string;
  role: string;
  availability: string;
  state: string;
  message: string;
  address: string;
  leader: boolean;
  engine: string;
  os: string;
  arch: string;
  cpus: number | null;
  memory: number | null;
  labels: Record<string, string>;
  panel: boolean;
  tasks: number;
};
type Impact = {
  services: Array<{ name: string; tasks: number; tied: boolean; constrained: boolean; volumes: boolean; folders: boolean; panel: boolean; system: boolean }>;
  placed: string[];
};
type T = (s: string) => string;

function stateTone(state: string): "ok" | "bad" | "off" {
  if (state === "ready") return "ok";
  if (state === "down" || state === "disconnected") return "bad";
  return "off";
}

function stateWord(t: T, state: string): string {
  if (state === "ready") return t("Ready");
  if (state === "down") return t("Down");
  if (state === "disconnected") return t("Disconnected");
  return t("Unknown");
}

function availabilityWord(t: T, availability: string): string {
  if (availability === "active") return t("Active");
  if (availability === "pause") return t("Paused");
  if (availability === "drain") return t("Drained");
  return availability;
}

/** What draining a node does, said before it is done. */
function ImpactBody(props: { t: T; impact: Impact }) {
  const { t, impact } = props;
  const notes = (s: Impact["services"][number]) =>
    [
      s.tied && t("tied to this node: stays pending"),
      s.constrained && t("has placement rules: may stay pending"),
      s.volumes && t("its volumes start empty on another node"),
      s.folders && t("uses a folder of this server"),
      !s.panel && t("not the panel's"),
    ].filter(Boolean) as string[];
  return (
    <div className="rdp2-stack">
      <p>{t("Swarm stops what runs on this node and starts it on another one when it can. Nothing comes back by itself when you activate it again.")}</p>
      {impact.services.length === 0 ? (
        <p className="rdp2-muted">{t("Nothing runs on this node now.")}</p>
      ) : (
        <ul className="rdp2-list">
          {impact.services.map((s) => (
            <li key={s.name} className="rdp2-list-row">
              <span className="rdp2-mono">{s.name}</span>
              <span className="rdp2-muted rdp2-small">{notes(s).join(" · ") || t("Swarm moves it when another node fits")}</span>
            </li>
          ))}
        </ul>
      )}
      {impact.placed.length > 0 && (
        <p className="rdp2-muted rdp2-small">
          {t("Placed on this node for their next deployment (Advanced → Runs on):")} {impact.placed.join(", ")}
        </p>
      )}
    </div>
  );
}

/** The command a new server joins with: asked for when the dialog opens
 * (an audited read), forgotten when it closes. */
function AddNodeDialog(props: PageProps<unknown> & { onClose: () => void }) {
  const t = props.t;
  const ask = useAsk();
  const [command, setCommand] = useState<string | null>(null);
  const [why, setWhy] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  // Each read is numbered: an answer older than the latest (a read begun
  // before a rotation) is dropped, never shown.
  const generation = useRef(0);
  const load = async () => {
    const mine = ++generation.current;
    setError(null);
    try {
      const r = await props.api.post<{ command: string | null; why?: string }>("/docker/nodes/join-command", {});
      if (mine !== generation.current) return;
      setCommand(r.command);
      setWhy(r.command ? null : (r.why ?? null));
    } catch (err) {
      if (mine === generation.current) setError(err);
    }
  };
  useEffect(() => {
    void load();
    return () => setCommand(null);
    // Once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Dialog open onClose={props.onClose} title={t("Add a node")}>
      <div className="rdp2-stack">
        <p>{t("On the new server, with Docker installed, run:")}</p>
        {command ? (
          <div className="rdp2-row">
            <code className="rdp2-notes rdp2-mono">{command}</code>
            <CopyButton text={command} label={t("Copy")} copied={t("Copied")} />
          </div>
        ) : why ? (
          <Alert variant="warning">{t(why)}</Alert>
        ) : error ? null : (
          <Loading />
        )}
        <ErrorNote error={error} />
        <ul className="rdp2-list rdp2-small">
          <li>{t("Between the servers, open 2377/tcp, 7946/tcp and udp, and 4789/udp (the provider's firewall).")}</li>
          <li>{t("The panel's firewall works on one node only: turn it off before you add one.")}</li>
          <li>{t("Services may then run on the new node: their volumes stay on their node, and backups, scripts, the terminal and live stats work only on the panel's node.")}</li>
          <li>{t("The token lets any server join: keep it secret, rotate it if it leaks.")}</li>
        </ul>
        <div className="rdp2-dialog-foot">
          <Button
            type="button"
            variant="outline"
            disabled={busy}
            onClick={async () => {
              const ok = await ask({
                title: t("Rotate the join token?"),
                body: t("Servers already in the Swarm stay. The old command stops working."),
                action: t("Rotate token"),
              });
              if (!ok) return;
              setBusy(true);
              // A read under way returns the old command: dropped.
              generation.current++;
              setCommand(null);
              try {
                await props.api.post("/docker/nodes/rotate-token", {});
                // The old command no longer works: never left to copy.
                setCommand(null);
                props.toast.success(t("Token rotated"));
                await load();
              } catch (err) {
                setError(err);
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? t("Rotating…") : t("Rotate token")}
          </Button>
          <Button type="button" variant="ghost" autoFocus onClick={props.onClose}>
            {t("Close")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

export function Nodes(props: PageProps<{ can?: Record<string, boolean> }>) {
  const t = props.t;
  const list = useAdminQuery<{ nodes: Node[]; can_manage: boolean }>(props, "/docker/nodes", { refetchInterval: 10000 });
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // The node a change is under way for: its menu waits.
  const [pending, setPending] = useState<string | null>(null);
  const ask = useAsk();
  const nodes = list.data?.nodes ?? [];
  const paged = usePaged(nodes);
  const manage = !!list.data?.can_manage;
  const header = {
    title: props.title,
    eyebrow: "Docker",
    heading: t("Nodes"),
    description: t("The servers of this Swarm. Services run on them; the panel runs on its own node."),
  };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;

  const setAvailability = async (n: Node, availability: "active" | "pause" | "drain") => {
    // One change at a time, from the first request to the last.
    if (pending) return;
    setPending(n.id);
    setError(null);
    try {
      if (availability === "drain") {
        const impact = await props.api.get<Impact>(`/docker/nodes/${encodeURIComponent(n.id)}/impact`);
        const ok = await ask({
          title: t("Drain {name}?").replace("{name}", n.name),
          body: <ImpactBody t={t} impact={impact} />,
          action: t("Drain node"),
          destructive: true,
        });
        if (!ok) return;
      }
      await props.api.patch(`/docker/nodes/${encodeURIComponent(n.id)}`, { availability });
      props.toast.success(availability === "active" ? t("Node active") : availability === "pause" ? t("Node paused") : t("Node drained"));
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setPending(null);
    }
  };
  const remove = async (n: Node) => {
    if (pending) return;
    setPending(n.id);
    setError(null);
    try {
      const impact = await props.api.get<Impact>(`/docker/nodes/${encodeURIComponent(n.id)}/impact`);
      const ok = await ask({
        title: t("Remove {name} from the Swarm?").replace("{name}", n.name),
        body: (
          <div className="rdp2-stack">
            <p>{t("Only a server that has left (run docker swarm leave on it first: it shows Down). Services placed on it fail their next deployment until another node is chosen.")}</p>
            {impact.placed.length > 0 && (
              <p className="rdp2-muted rdp2-small">
                {t("Placed on this node for their next deployment (Advanced → Runs on):")} {impact.placed.join(", ")}
              </p>
            )}
          </div>
        ),
        action: t("Remove node"),
        destructive: true,
      });
      if (!ok) return;
      await props.api.delete(`/docker/nodes/${encodeURIComponent(n.id)}`);
      props.toast.success(t("Removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setPending(null);
    }
  };

  return (
    <Page {...header} actions={manage && <Button onClick={() => setAdding(true)}>{t("Add a node")}</Button>}>
      <ErrorNote error={error} />
      <Section
        title={t("All nodes")}
        flush
        actions={
          <Button size="sm" variant="outline" onClick={() => list.refetch()}>
            {t("Refresh")}
          </Button>
        }
      >
        {nodes.length === 0 ? (
          <EmptyState title={t("Docker is not a Swarm here.")} />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Name")}</th>
                    <th>{t("Role")}</th>
                    <th>{t("Status")}</th>
                    <th>{t("Availability")}</th>
                    <th className="rdp2-hide-sm">{t("Address")}</th>
                    <th className="rdp2-hide-sm">{t("CPU / RAM")}</th>
                    <th className="rdp2-right">{t("Tasks")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((n) => (
                    <tr key={n.id}>
                      <td>
                        <div className="rdp2-strong-cell">{n.name}</div>
                        <div className="rdp2-muted rdp2-small">
                          {[n.panel && t("this panel's node"), n.leader && t("leader"), `${n.os}/${n.arch}`, n.engine && `Docker ${n.engine}`].filter(Boolean).join(" · ")}
                        </div>
                        {Object.keys(n.labels).length > 0 && (
                          <div className="rdp2-muted rdp2-small rdp2-mono" title={Object.entries(n.labels).map(([k, v]) => `${k}=${v}`).join("\n")}>
                            {Object.entries(n.labels)
                              .map(([k, v]) => `${k}=${v}`)
                              .join(", ")}
                          </div>
                        )}
                      </td>
                      <td>{n.role === "manager" ? t("Manager") : t("Worker")}</td>
                      <td>
                        <StatusBadge tone={stateTone(n.state)} label={stateWord(t, n.state)} title={n.message || undefined} />
                      </td>
                      <td>{pending === n.id ? t("Working…") : availabilityWord(t, n.availability)}</td>
                      <td className="rdp2-hide-sm rdp2-mono">{n.address}</td>
                      <td className="rdp2-hide-sm rdp2-num">
                        {n.cpus ?? "–"} / {n.memory ? bytes(n.memory) : "–"}
                      </td>
                      <td className="rdp2-right rdp2-num">{n.tasks}</td>
                      <td>
                        <RowActions
                          label={t("Actions for {host}").replace("{host}", n.name)}
                          menu={
                            manage && !n.panel && pending === null
                              ? [
                                  { label: t("Activate"), disabled: n.availability === "active", onSelect: () => void setAvailability(n, "active") },
                                  { label: t("Pause"), disabled: n.availability === "pause", onSelect: () => void setAvailability(n, "pause") },
                                  { label: t("Drain…"), disabled: n.availability === "drain", onSelect: () => void setAvailability(n, "drain") },
                                  "separator",
                                  {
                                    label: n.role === "manager" ? t("A manager: kept") : t("Remove…"),
                                    destructive: true,
                                    disabled: n.role === "manager",
                                    onSelect: () => void remove(n),
                                  },
                                ]
                              : []
                          }
                        />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={nodes.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
      {adding && <AddNodeDialog {...(props as PageProps<unknown>)} onClose={() => setAdding(false)} />}
    </Page>
  );
}
