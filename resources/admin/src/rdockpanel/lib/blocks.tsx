// Blocked addresses, the WAF's and the firewall's together (`/apps/blocks`):
// the dialog that blocks one (on the websites, the server, or both) and the
// list both pages show (WAF → IP lists, Firewall → Blocked addresses). The
// rules are the server's (`ip_block_service`): what each layer can do here
// comes with the list.
import { useRef, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, Checkbox, Dialog, ErrorState, Input, Loading, NativeSelect } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "./query";
import { EmptyState, ErrorNote, Field, Pager, RowActions, Section, TableScroll, useAsk, usePaged, when, type MenuItem } from "./ui";

export type Blocks = {
  blocked: Array<{
    ip: string;
    waf: { until: string | null; reason: string | null } | null;
    firewall: { id: number; until: string | null; reason: string | null; since: string; on_server: boolean } | null;
  }>;
  you: string | null;
  waf: { enabled: boolean; on: boolean; active: boolean; running: boolean; sharing: boolean; community: number; connected: boolean };
  firewall: { available: boolean; reason: string | null; enabled: boolean };
};

/** `/apps/blocks`, refreshed every 30 s. */
export function useBlocks(props: PageProps<unknown>) {
  return useAdminQuery<Blocks>(props, "/apps/blocks", { refetchInterval: 30000 });
}

type Answer = { ip: string; waf?: string; firewall?: string | { waiting: string } };

/** What the toast says of a block or an unblock, by layer. */
function outcome(t: (s: string) => string, answer: Answer, removed: boolean): string {
  const parts: string[] = [];
  if (answer.waf === "saved") parts.push(t("websites: saved, blocked once the blocklist is on in Block mode"));
  else if (answer.waf) parts.push(t("websites: within ten seconds"));
  if (answer.firewall === "applied") parts.push(t("server: applied"));
  else if (answer.firewall === "off") parts.push(t("server: saved, applied once the firewall is on"));
  else if (answer.firewall && typeof answer.firewall === "object") parts.push(t("server: saved, not applied yet ({why})").replace("{why}", answer.firewall.waiting));
  return `${removed ? t("Unblocked") : t("Blocked")} ${answer.ip}: ${parts.join("; ")}`;
}

/** Blocks an address on the websites (WAF), the server (Firewall) or both. */
export function BlockAddressDialog(
  props: PageProps<unknown> & { ip?: string; reason?: string; layer?: "waf" | "firewall"; onClose: () => void; onDone?: () => void },
) {
  const t = props.t;
  const blocks = useBlocks(props);
  const [ip, setIp] = useState(props.ip ?? "");
  const [waf, setWaf] = useState(props.layer !== "firewall");
  const [firewall, setFirewall] = useState(props.layer === "firewall");
  const [days, setDays] = useState(props.layer === "firewall" ? "" : "7");
  const [reason, setReason] = useState(props.reason ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = error ? fieldErrors(error) : {};
  const ipInput = useRef<HTMLInputElement>(null);
  const reasonInput = useRef<HTMLInputElement>(null);
  const whereInput = useRef<HTMLDivElement>(null);
  const daysInput = useRef<HTMLSelectElement>(null);
  // Where the server's refusals are shown: unknown fields join the note.
  const shown = ["ip", "layers", "waf", "firewall", "days", "reason"];
  const whereErrors = [...(fields.layers ?? []), ...(fields.waf ?? []), ...(fields.firewall ?? [])];
  const b = blocks.data;
  const network = ip.includes("/");
  const wafPossible = !!b?.waf.on && !network;
  const firewallPossible = !!b?.firewall.available;
  const useWaf = waf && wafPossible;
  const useFirewall = firewall && firewallPossible;
  const own = !!b?.you && ip.trim() === b.you;
  return (
    <Dialog open onClose={props.onClose} title={t("Block an address")}>
      {blocks.isLoading ? (
        <Loading />
      ) : blocks.error || !b ? (
        <ErrorState error={blocks.error} />
      ) : (
        <form
          className="rdp2-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError(null);
            try {
              const answer = await props.api.post<Answer>("/apps/blocks", {
                ip: ip.trim(),
                waf: useWaf,
                firewall: useFirewall,
                days: days === "" ? null : Number(days),
                reason: reason.trim(),
              });
              props.toast.success(outcome(t, answer, false));
              props.onDone?.();
              props.onClose();
            } catch (err) {
              setError(err);
              // The first field refused, focused.
              const refused = fieldErrors(err);
              if (refused.ip) ipInput.current?.focus();
              else if (refused.layers || refused.waf || refused.firewall) whereInput.current?.querySelector<HTMLElement>("input:not(:disabled)")?.focus();
              else if (refused.days) daysInput.current?.focus();
              else if (refused.reason) reasonInput.current?.focus();
            } finally {
              setBusy(false);
            }
          }}
        >
          <Field label={t("Address")} help={t("One address; a network (203.0.113.0/24) on the server only.")} error={fields.ip}>
            <Input ref={ipInput} autoFocus className="rdp2-mono" value={ip} aria-invalid={fields.ip ? true : undefined} onChange={(e) => setIp(e.target.value)} placeholder="203.0.113.7" />
          </Field>
          {own && <Alert variant="destructive">{t("This is the address you are using: blocking it would lock you out.")}</Alert>}
          <Field label={t("Where")} error={whereErrors.length ? whereErrors : undefined}>
            <div className="rdp2-form" ref={whereInput}>
              <label className="rdp2-row rdp2-top">
                <Checkbox checked={useWaf} disabled={!wafPossible} onChange={(e) => setWaf(e.target.checked)} />
                <span>
                  <strong>{t("On the websites (WAF)")}</strong>
                  <span className="rdp2-field-help">
                    {!b.waf.on
                      ? t("The WAF is off: turn it on to block here.")
                      : network
                        ? t("The WAF blocks one address at a time.")
                        : b.waf.active
                          ? t("403 on every domain, also for visitors through Cloudflare and the tunnel. The panel and SSH stay reachable.")
                          : t("Saved, but the blocklist only blocks once it is on in Block mode (WAF → Settings).")}
                  </span>
                </span>
              </label>
              <label className="rdp2-row rdp2-top">
                <Checkbox
                  checked={useFirewall}
                  disabled={!firewallPossible}
                  onChange={(e) => {
                    setFirewall(e.target.checked);
                    if (!e.target.checked && days === "") setDays("7");
                  }}
                />
                <span>
                  <strong>{t("On the whole server (Firewall)")}</strong>
                  <span className="rdp2-field-help">
                    {!firewallPossible
                      ? t(b.firewall.reason ?? "The firewall is not available on this server.")
                      : b.firewall.enabled
                        ? t("Everything but SSH (every site and the panel), at once. It does not see visitors through Cloudflare's proxy or the tunnel: block them on the websites.")
                        : t("Saved, applied once the firewall is turned on (Firewall page).")}
                  </span>
                </span>
              </label>
            </div>
          </Field>
          <Field label={t("For")} error={fields.days}>
            <NativeSelect ref={daysInput} value={days} aria-invalid={fields.days ? true : undefined} onChange={(e) => setDays(e.target.value)}>
              <option value="1">{t("A day")}</option>
              <option value="7">{t("A week")}</option>
              <option value="30">{t("Thirty days")}</option>
              {useFirewall && !useWaf && <option value="">{t("Until removed")}</option>}
            </NativeSelect>
          </Field>
          <Field label={t("Reason")} error={fields.reason}>
            <Input ref={reasonInput} value={reason} maxLength={100} aria-invalid={fields.reason ? true : undefined} onChange={(e) => setReason(e.target.value)} />
          </Field>
          {Object.keys(fields).every((f) => !shown.includes(f)) && <ErrorNote error={error} />}
          <div className="rdp2-dialog-foot">
            <Button type="button" variant="ghost" onClick={props.onClose}>
              {t("Cancel")}
            </Button>
            <Button type="submit" disabled={busy || ip.trim() === "" || (!useWaf && !useFirewall) || own}>
              {busy ? t("Blocking…") : t("Block")}
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

/** The blocked addresses, both layers: where each is blocked and until when. */
export function BlockedAddresses(props: PageProps<unknown> & { heading: string; blurb: string; layer: "waf" | "firewall" }) {
  const t = props.t;
  const blocks = useBlocks(props);
  const [blocking, setBlocking] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const paged = usePaged(blocks.data?.blocked ?? []);
  const until = (at: string | null) => (at ? t("until {when}").replace("{when}", when(at, props.locale)) : t("until removed"));
  const unblock = async (ip: string, waf: boolean, firewall: boolean) => {
    const where = waf && firewall ? t("everywhere") : waf ? t("on the websites") : t("on the server");
    const ok = await ask({ title: t("Unblock {ip} {where}?").replace("{ip}", ip).replace("{where}", where), action: t("Unblock") });
    if (!ok) return;
    setError(null);
    try {
      const answer = await props.api.post<Answer>("/apps/blocks/remove", { ip, waf, firewall });
      props.toast.success(outcome(t, answer, true));
      blocks.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Section
      title={props.heading}
      description={props.blurb}
      actions={
        <Button variant="outline" onClick={() => setBlocking(true)}>
          {t("Block an address")}
        </Button>
      }
      flush
    >
      <ErrorNote error={error} />
      {blocks.isLoading ? (
        <Loading />
      ) : blocks.error || !blocks.data ? (
        <ErrorState error={blocks.error} />
      ) : blocks.data.blocked.length === 0 ? (
        <EmptyState title={t("No address blocked.")} />
      ) : (
        <>
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("Address")}</th>
                  <th>{t("Websites (WAF)")}</th>
                  <th>{t("Server (Firewall)")}</th>
                  <th className="rdp2-hide-sm">{t("Reason")}</th>
                  <th className="rdp2-actions-col">
                    <span className="rdp2-sr-only">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {paged.page.map((row) => {
                  const menu: MenuItem[] = [];
                  if (row.waf && row.firewall) menu.push({ label: t("Unblock everywhere…"), onSelect: () => void unblock(row.ip, true, true) });
                  if (row.waf) menu.push({ label: t("Unblock on the websites…"), onSelect: () => void unblock(row.ip, true, false) });
                  if (row.firewall) menu.push({ label: t("Unblock on the server…"), onSelect: () => void unblock(row.ip, false, true) });
                  const reason = row.waf?.reason || row.firewall?.reason || "";
                  return (
                    <tr key={row.ip}>
                      <td className="rdp2-mono">{row.ip}</td>
                      <td>{row.waf ? <span>{until(row.waf.until)}</span> : <span className="rdp2-muted">–</span>}</td>
                      <td>
                        {row.firewall ? (
                          <>
                            <div>{until(row.firewall.until)}</div>
                            {!row.firewall.on_server && (
                              <div className="rdp2-small rdp2-muted">{blocks.data?.firewall.enabled ? t("not on the server yet") : t("the firewall is off: applied once it is on")}</div>
                            )}
                          </>
                        ) : (
                          <span className="rdp2-muted">–</span>
                        )}
                      </td>
                      <td className="rdp2-hide-sm rdp2-truncate" title={reason}>
                        {reason || "–"}
                      </td>
                      <td>
                        <RowActions label={t("Actions for {host}").replace("{host}", row.ip)} menu={menu} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </TableScroll>
          {blocks.data.blocked.length > 25 && <Pager total={blocks.data.blocked.length} at={paged.at} onPage={paged.set} t={t} />}
        </>
      )}
      {blocking && <BlockAddressDialog {...props} layer={props.layer} onClose={() => setBlocking(false)} onDone={() => blocks.refetch()} />}
    </Section>
  );
}
