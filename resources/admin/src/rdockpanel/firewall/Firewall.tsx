// The Firewall page (`/admin/p/firewall`): the server's listeners and its
// firewall. Default deny: SSH, 80 and 443 open; every other port closed
// unless opened, for the sources listed. A change is applied, then kept
// within its time or reverted by the server itself (docs/firewall.md).
import { useEffect, useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, CodeBlock, Dialog, ErrorState, Input, Loading, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Facts, Field, Page, Pager, RowActions, Section, StatusBadge, TableScroll, type MenuItem, useConfirm, usePaged } from "../lib/ui";
import { fieldErrors } from "@rustcms/admin-sdk";

type Port = {
  port: number;
  protocol: string;
  addresses: string[];
  owner: string;
  published: boolean;
  swarm: boolean;
  state: "local" | "always_open" | "opened" | "closed" | "open";
  openings: Array<{ id: number; anywhere: boolean; sources: string[] }>;
};

type OpeningRow = {
  id: number;
  port_start: number;
  port_end: number;
  protocol: "tcp" | "udp";
  target: "host" | "published";
  anywhere: boolean;
  sources: string;
  note: string | null;
  enabled: boolean;
};

type Report =
  | { available: false; reason: string }
  | {
      available: true;
      enabled: boolean;
      changed: boolean;
      drift: boolean;
      pending: { txid: string; state: string; deadline: string | null } | null;
      unsupported: string[];
      ssh_ports: number[];
      host: { nft: boolean; systemd: boolean; ufw: boolean; firewalld: boolean; nodes: number | null; boot_failed: string | null };
      ports: Port[];
      openings: OpeningRow[];
      blocked: Array<{ id: number; cidr: string; note: string | null }>;
    };

type Preview = { ruleset: string; closes: Port[] };

const ports = (o: { port_start: number; port_end: number }) => (o.port_start === o.port_end ? `${o.port_start}` : `${o.port_start}-${o.port_end}`);

export function Firewall(props: PageProps<unknown>) {
  const t = props.t;
  const report = useAdminQuery<Report>(props, "/apps/firewall");
  const [error, setError] = useState<unknown>(null);
  const [opening, setOpening] = useState<OpeningRow | "new" | { port: number; protocol: string; published: boolean } | null>(null);
  const [blocking, setBlocking] = useState(false);
  const [applying, setApplying] = useState(false);
  const [confirm, confirmDialog] = useConfirm(t);
  // Up to a thousand: 25 a page.
  const paged = usePaged(report.data?.available ? report.data.blocked : []);
  const header = {
    title: props.title,
    eyebrow: t("Workspace"),
    heading: t("Firewall"),
    description: t("Every port is closed to the internet, except SSH, the web (80, 443) and the ports you open for the addresses you choose."),
  };
  if (report.isLoading) return <Loading />;
  if (report.error || !report.data) return <ErrorState error={report.error} />;
  const r = report.data;
  if (!r.available)
    return (
      <Page {...header}>
        <Alert>{t(r.reason)}</Alert>
      </Page>
    );
  const refresh = () => report.refetch();
  const call = async (run: () => Promise<unknown>, done?: string) => {
    setError(null);
    try {
      await run();
      if (done) props.toast.success(t(done));
    } catch (err) {
      setError(err);
    }
    refresh();
  };
  const saveOpening = (o: OpeningRow, enabled: boolean) =>
    call(
      () =>
      props.api.patch(`/apps/firewall/openings/${o.id}`, {
        ports: ports(o),
        protocol: o.protocol,
        target: o.target,
        anywhere: o.anywhere,
        sources_v4: o.sources.split(",").filter((s) => s && !s.includes(":")).join(", "),
        sources_v6: o.sources.split(",").filter((s) => s.includes(":")).join(", "),
        note: o.note,
        enabled,
      }),
      enabled ? "Opening turned on: apply the rules." : "Opening turned off: apply the rules.",
    );
  const removeOpening = async (o: OpeningRow) => {
    const ok = await confirm({ title: t("Remove this opening?"), body: t("The port closes at the next apply."), action: t("Remove"), destructive: true });
    if (ok) await call(() => props.api.delete(`/apps/firewall/openings/${o.id}`), "Opening removed: apply the rules.");
  };
  const disable = async () => {
    const ok = await confirm({
      title: t("Turn the firewall off?"),
      body: t("The panel's table is removed: every port is open again as Docker publishes it, and Docker no longer waits for the firewall at boot."),
      action: t("Turn off"),
      destructive: true,
    });
    if (ok) await call(() => props.api.post("/apps/firewall/disable", {}), "Firewall turned off.");
  };
  const unsupported = r.unsupported.length > 0;
  return (
    <Page
      {...header}
      actions={
        !r.pending && (
          <>
            {r.enabled && (
              <Button variant="outline" onClick={() => void disable()}>
                {t("Turn off")}
              </Button>
            )}
            {!unsupported && (r.changed || !r.enabled) && <Button onClick={() => setApplying(true)}>{r.enabled ? t("Apply…") : t("Turn on…")}</Button>}
          </>
        )
      }
    >
      {confirmDialog}
      <ErrorNote error={error} />

      <Section
        title={t("Firewall")}
        description={t("The panel's own nftables table (inet rdockpanel). It closes every new inbound connection except SSH, 80, 443 and the ports opened below; Docker's, ufw's and firewalld's rules still apply alongside.")}
      >
      {r.pending && <Pending t={t} pending={r.pending} api={props.api} onDone={refresh} onError={setError} toast={props.toast} />}
      {!r.pending && r.enabled && r.changed && <Alert>{t("The rules changed since they were applied: apply them to use them.")}</Alert>}
      {r.host.boot_failed && (
        <Alert variant="destructive">
          {r.host.boot_failed === "both"
            ? t("At boot, neither the confirmed table nor the fallback loaded: Docker is held until it does. See the server's journal (rdockpanel-firewall.service).")
            : t("At boot, the confirmed table did not load: the fallback (SSH and the web only) is on. Apply again.")}
        </Alert>
      )}
      {r.drift && !r.pending && <Alert variant="warning">{t("The server's table differs from the confirmed one (changed outside the panel): it is reloaded within two minutes.")}</Alert>}
        <Facts
          items={[
            [t("State"), r.enabled ? <StatusBadge tone="ok" label={t("On")} /> : <StatusBadge tone="off" label={t("Off")} />],
            [t("SSH"), r.ssh_ports.length ? r.ssh_ports.join(", ") : t("not found")],
            [t("Other firewalls"), [r.host.ufw && "ufw", r.host.firewalld && "firewalld"].filter(Boolean).join(", ") || t("none active")],
            [t("Swarm nodes"), r.host.nodes == null ? t("unknown") : String(r.host.nodes)],
          ]}
        />
        {unsupported && (
          <Alert variant="warning">
            {t("The firewall cannot be turned on here:")} {r.unsupported.map((u) => t(u)).join("; ")}
          </Alert>
        )}
        {!r.enabled && !unsupported && (
          <Alert>{t("Off: every port the server listens on, and every port a service publishes, is reachable as it is. Open what must stay reachable, then apply.")}</Alert>
        )}
      </Section>

      <Section title={t("Listening ports")} description={t("On the server, and the ports services publish. Whether the internet reaches them also depends on your provider's firewall.")} flush>
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Port")}</th>
                <th>{t("Protocol")}</th>
                <th className="rdp2-hide-sm">{t("Listens on")}</th>
                <th>{t("Belongs to")}</th>
                <th>{t("Firewall")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {r.ports.map((p) => (
                <tr key={`${p.port}/${p.protocol}`}>
                  <td className="rdp2-mono">{p.port}</td>
                  <td>{p.protocol}</td>
                  <td className="rdp2-hide-sm rdp2-mono rdp2-ellipsis">{p.addresses.join(", ")}</td>
                  <td>{t(p.owner)}</td>
                  <td>
                    <PortState t={t} p={p} enabled={r.enabled} />
                  </td>
                  <td>
                    {(p.state === "closed" || p.state === "open") && !p.swarm && (
                      <RowActions label={t("Actions for port {port}").replace("{port}", String(p.port))}>
                        <Button size="sm" variant="ghost" onClick={() => setOpening({ port: p.port, protocol: p.protocol, published: p.published })}>
                          {t("Open for…")}
                        </Button>
                      </RowActions>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
      </Section>

      <Section
        title={t("Open ports")}
        description={t("Everything not listed here is closed (except SSH and the web). Only the sources listed connect; IPv4 and IPv6 are listed apart. Existing connections continue until they close.")}
        actions={
          <Button size="sm" onClick={() => setOpening("new")}>
            {t("Open a port")}
          </Button>
        }
        flush
      >
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Port")}</th>
                <th>{t("Applies to")}</th>
                <th>{t("Open for")}</th>
                <th className="rdp2-hide-sm">{t("Note")}</th>
                <th>{t("On")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td className="rdp2-mono">{[...r.ssh_ports, 80, 443].join(", ")}/tcp</td>
                <td>{t("This server")}</td>
                <td>{t("Anywhere")}</td>
                <td className="rdp2-hide-sm rdp2-muted">{t("SSH and the web (always open)")}</td>
                <td>–</td>
                <td />
              </tr>
              {r.openings.map((o) => {
                const menu: MenuItem[] = [
                  { label: t("Edit"), onSelect: () => setOpening(o) },
                  "separator",
                  { label: t("Remove…"), destructive: true, onSelect: () => void removeOpening(o) },
                ];
                return (
                  <tr key={o.id}>
                    <td className="rdp2-mono">
                      {ports(o)}/{o.protocol}
                    </td>
                    <td>{o.target === "host" ? t("This server") : t("A published port")}</td>
                    <td className="rdp2-mono rdp2-ellipsis">{o.anywhere ? t("Anywhere") : o.sources.split(",").join(", ")}</td>
                    <td className="rdp2-hide-sm">{o.note ?? ""}</td>
                    <td>
                      <Switch checked={o.enabled} aria-label={t("Opening {ports} on").replace("{ports}", `${ports(o)}/${o.protocol}`)} onCheckedChange={(on) => void saveOpening(o, on)} />
                    </td>
                    <td>
                      <RowActions label={t("Actions for port {port}").replace("{port}", ports(o))} menu={menu} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </TableScroll>
      </Section>

      <Section
        title={t("Blocked addresses")}
        description={t("These addresses reach nothing on this server (every site, the panel included) but SSH, so nobody locks themselves out. Visitors through the Cloudflare tunnel are not affected: the server does not see their address.")}
        actions={
          <Button size="sm" variant="outline" onClick={() => setBlocking(true)}>
            {t("Block an address")}
          </Button>
        }
        flush={r.blocked.length > 0}
      >
        {r.blocked.length === 0 ? (
          <EmptyState title={t("No blocked address")} />
        ) : (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("Address or network")}</th>
                  <th>{t("Note")}</th>
                  <th className="rdp2-right">
                    <span className="rdp2-sr">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {paged.page.map((b) => (
                  <tr key={b.id}>
                    <td className="rdp2-mono">{b.cidr}</td>
                    <td className="rdp2-muted">{b.note ?? ""}</td>
                    <td>
                      <RowActions label={t("Actions for {cidr}").replace("{cidr}", b.cidr)}>
                        <Button
                          size="sm"
                          variant="ghost"
                          aria-label={t("Unblock {cidr}").replace("{cidr}", b.cidr)}
                          onClick={() => void call(() => props.api.delete(`/apps/firewall/blocked/${b.id}`), "Address unblocked: apply the rules.")}
                        >
                          {t("Unblock")}
                        </Button>
                      </RowActions>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableScroll>
        )}
        {r.blocked.length > 25 && <Pager total={r.blocked.length} at={paged.at} onPage={paged.set} t={t} />}
      </Section>

      {opening && (
        <OpeningDialog
          t={t}
          api={props.api}
          initial={opening}
          onClose={() => setOpening(null)}
          onSaved={() => {
            setOpening(null);
            refresh();
          }}
        />
      )}
      {blocking && (
        <BlockDialog
          t={t}
          api={props.api}
          onClose={() => setBlocking(false)}
          onSaved={() => {
            setBlocking(false);
            refresh();
          }}
        />
      )}
      {applying && (
        <ApplyDialog
          {...props}
          enabled={r.enabled}
          onClose={() => setApplying(false)}
          onApplied={() => {
            setApplying(false);
            refresh();
          }}
        />
      )}
    </Page>
  );
}

function PortState(props: { t: (s: string) => string; p: Port; enabled: boolean }) {
  const { t, p } = props;
  switch (p.state) {
    case "local":
      return <span className="rdp2-muted">{t("This server only")}</span>;
    case "always_open":
      return <StatusBadge tone="off" label={t("Always open")} />;
    case "opened":
      return (
        <StatusBadge
          tone={p.openings.some((o) => o.anywhere) ? "bad" : "ok"}
          label={
            p.openings.some((o) => o.anywhere)
              ? t("Open to everyone")
              : t("Open for {sources}").replace("{sources}", p.openings.flatMap((o) => o.sources).join(", "))
          }
        />
      );
    case "closed":
      return <StatusBadge tone="off" label={t("Closed")} />;
    default:
      return <StatusBadge tone="bad" label={t("Open to everyone")} title={t("The firewall is off.")} />;
  }
}

/** The change waiting to be kept: kept, reverted now, or by the server at its time. */
function Pending(props: {
  t: (s: string) => string;
  pending: { txid: string; deadline: string | null; state: string };
  api: PageProps<unknown>["api"];
  toast: PageProps<unknown>["toast"];
  onDone: () => void;
  onError: (e: unknown) => void;
}) {
  const { t, pending } = props;
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState<"keep" | "revert" | null>(null);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const left = pending.deadline ? Math.max(0, Math.round((new Date(pending.deadline).getTime() - now) / 1000)) : 0;
  const act = async (what: "keep" | "revert", path: string, done: string) => {
    setBusy(what);
    props.onError(null);
    try {
      await props.api.post(path, { txid: pending.txid });
      props.toast.success(t(done));
    } catch (err) {
      props.onError(err);
    } finally {
      setBusy(null);
      props.onDone();
    }
  };
  useEffect(() => {
    if (left === 0) {
      const timer = setTimeout(props.onDone, 15000);
      return () => clearTimeout(timer);
    }
  }, [left, props.onDone]);
  return (
    <Alert variant="warning" role="status">
      <div className="rdp2-form">
        <div>
          <strong className="rdp2-strong">{t("Trying the new rules.")}</strong>{" "}
          {left > 0
            ? t("Unless you keep them, the server goes back to the previous rules in {s} s.").replace("{s}", String(left))
            : t("Time is up: the server is going back to the previous rules.")}
        </div>
        <p>{t("Before keeping them, check that you still reach the panel and SSH from a new connection (a private window, a new SSH session).")}</p>
        <div className="rdp2-row rdp2-wrap rdp2-end">
          <Button variant="outline" disabled={busy !== null} onClick={() => void act("revert", "/apps/firewall/revert", "Previous rules restored.")}>
            {busy === "revert" ? t("Going back…") : t("Go back now")}
          </Button>
          <Button disabled={busy !== null || left === 0} onClick={() => void act("keep", "/apps/firewall/keep", "Rules kept.")}>
            {busy === "keep" ? t("Keeping…") : t("Keep these rules")}
          </Button>
        </div>
      </div>
    </Alert>
  );
}

function OpeningDialog(props: {
  t: (s: string) => string;
  api: PageProps<unknown>["api"];
  initial: OpeningRow | "new" | { port: number; protocol: string; published: boolean };
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t, initial } = props;
  const row = typeof initial === "object" && "id" in initial ? initial : null;
  const preset = typeof initial === "object" && "port" in initial ? initial : null;
  const sources = row ? row.sources.split(",").filter(Boolean) : [];
  const [portsText, setPorts] = useState(row ? ports(row) : preset ? String(preset.port) : "");
  const [protocol, setProtocol] = useState<string>(row?.protocol ?? preset?.protocol ?? "tcp");
  const [target, setTarget] = useState<string>(row?.target ?? (preset && !preset.published ? "host" : "published"));
  const [anywhere, setAnywhere] = useState(row?.anywhere ?? false);
  const [v4, setV4] = useState(sources.filter((s) => !s.includes(":")).join(", "));
  const [v6, setV6] = useState(sources.filter((s) => s.includes(":")).join(", "));
  const [note, setNote] = useState(row?.note ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = fieldErrors(error);
  return (
    <Dialog open onClose={props.onClose} title={row ? t("Edit opening") : t("Open a port")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          const body = { ports: portsText, protocol, target, anywhere, sources_v4: anywhere ? "" : v4, sources_v6: anywhere ? "" : v6, note: note || null, enabled: row?.enabled ?? true };
          try {
            if (row) await props.api.patch(`/apps/firewall/openings/${row.id}`, body);
            else await props.api.post("/apps/firewall/openings", body);
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Port or range")} help={t("5432, or 8000-8100")} error={fields.ports}>
          <Input autoFocus value={portsText} onChange={(e) => setPorts(e.target.value)} placeholder="5432" />
        </Field>
        <div className="rdp2-grid2">
          <Field label={t("Protocol")} error={fields.protocol}>
            <NativeSelect value={protocol} onChange={(e) => setProtocol(e.target.value)}>
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
            </NativeSelect>
          </Field>
          <Field label={t("Applies to")} error={fields.target}>
            <NativeSelect value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="published">{t("A port a service publishes")}</option>
              <option value="host">{t("This server (a program outside Docker)")}</option>
            </NativeSelect>
          </Field>
        </div>
        <Field label={t("Open for")}>
          <NativeSelect value={anywhere ? "anywhere" : "listed"} onChange={(e) => setAnywhere(e.target.value === "anywhere")}>
            <option value="listed">{t("Only the addresses below")}</option>
            <option value="anywhere">{t("Anywhere (everyone on the internet)")}</option>
          </NativeSelect>
        </Field>
        {anywhere ? (
          <Alert variant="warning">{t("Everyone on the internet can connect to this port.")}</Alert>
        ) : (
          <>
            <Field label={t("IPv4 sources")} help={t("Addresses or networks, separated by commas: 10.8.0.0/24, 203.0.113.25")} error={fields.sources_v4}>
              <Input value={v4} onChange={(e) => setV4(e.target.value)} placeholder="10.8.0.0/24" className="rdp2-mono" />
            </Field>
            <Field label={t("IPv6 sources")} help={t("Empty: no IPv6 address connects.")} error={fields.sources_v6}>
              <Input value={v6} onChange={(e) => setV6(e.target.value)} placeholder="2001:db8::/48" className="rdp2-mono" />
            </Field>
          </>
        )}
        <Field label={t("Note")} error={fields.note}>
          <Input value={note} onChange={(e) => setNote(e.target.value)} placeholder={t("Through the VPN")} />
        </Field>
        <p className="rdp2-small rdp2-muted">{t("SSH, 80 and 443 are always open: nothing to add for them. The change applies when you apply the rules.")}</p>
        <ErrorNote error={Object.keys(fields).length ? null : error} />
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy}>
            {busy ? t("Saving…") : row ? t("Save") : t("Open the port")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function BlockDialog(props: { t: (s: string) => string; api: PageProps<unknown>["api"]; onClose: () => void; onSaved: () => void }) {
  const { t } = props;
  const [cidr, setCidr] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = fieldErrors(error);
  return (
    <Dialog open onClose={props.onClose} title={t("Block an address")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await props.api.post("/apps/firewall/blocked", { cidr, note: note || null });
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Address or network")} help={t("198.51.100.7, 203.0.113.0/24 or 2001:db8::/32")} error={fields.cidr}>
          <Input autoFocus value={cidr} onChange={(e) => setCidr(e.target.value)} className="rdp2-mono" />
        </Field>
        <Field label={t("Note")} error={fields.note}>
          <Input value={note} onChange={(e) => setNote(e.target.value)} />
        </Field>
        <ErrorNote error={Object.keys(fields).length ? null : error} />
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy}>
            {busy ? t("Blocking…") : t("Block")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** What applying closes, the table itself, then the change (kept in time or reverted). */
function ApplyDialog(props: PageProps<unknown> & { enabled: boolean; onClose: () => void; onApplied: () => void }) {
  const t = props.t;
  const preview = useAdminQuery<Preview>(props, "/apps/firewall/preview");
  const [busy, setBusy] = useState(false);
  const [ruleset, setRuleset] = useState(false);
  const [error, setError] = useState<unknown>(null);
  return (
    <Dialog open onClose={props.onClose} title={props.enabled ? t("Apply the rules?") : t("Turn the firewall on?")} className={ruleset ? "rdp2-wide" : undefined}>
      <div className="rdp2-form">
        {preview.isLoading && <Loading />}
        <ErrorNote error={preview.error ?? error} />
        {preview.data && (
          <>
            {preview.data.closes.length > 0 ? (
              <Alert variant="warning">
                {t("These ports close to the internet:")}{" "}
                {preview.data.closes.map((p) => `${p.port}/${p.protocol} (${t(p.owner)})`).join(", ")}
              </Alert>
            ) : (
              <p>{t("No listening port closes: only SSH, the web and the opened ports stay reachable.")}</p>
            )}
            <p>
              {t("The server tries the new rules and goes back to the previous ones by itself in two minutes unless you keep them: check from a new connection, then press Keep.")}
            </p>
            <div className="rdp2-row">
              <Button type="button" size="sm" variant="outline" onClick={() => setRuleset(!ruleset)}>
                {ruleset ? t("Hide the table") : t("Show the table loaded on the server")}
              </Button>
            </div>
            {ruleset && <CodeBlock code={preview.data.ruleset} label={t("Ruleset")} />}
          </>
        )}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" autoFocus onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button
            disabled={busy || !preview.data}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await props.api.post("/apps/firewall/apply", {});
                props.onApplied();
              } catch (err) {
                setError(err);
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? t("Applying…") : props.enabled ? t("Apply") : t("Turn on")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
