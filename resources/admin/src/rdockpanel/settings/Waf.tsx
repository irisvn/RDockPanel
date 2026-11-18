// Settings → Security: the WAF (CrowdSec's AppSec in front of the services'
// domains). Its mode (off, detect: recorded only, block), level (basic:
// known exploits; strict: + the OWASP Core Rule Set), trusted addresses,
// community sharing, its state (the canary's), Repair, and the latest
// alerts. The panel's administrators' (`system.manage`).
import { useEffect, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { useAdminQuery } from "../lib/query";
import { Alert, Button, ErrorState, Loading, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Facts, Field, Pager, Section, StatusBadge, TableScroll, useAsk, usePaged, when } from "../lib/ui";

type Mode = "off" | "detect" | "block";
type Level = "basic" | "strict";
type WafState = { state: string; detail?: string | null; checked_at?: string | null };
type Data = {
  settings: { mode: Mode; level: Level; allowlist: string[]; sharing: boolean };
  status: WafState;
  services_off: number;
};
type AlertRow = { id: number; at: string; ip: string | null; rule: string | null; message: string | null; host: string | null; path: string | null; blocked: boolean };

function stateTone(state: string): "ok" | "busy" | "bad" | "off" {
  if (state === "on") return "ok";
  if (state === "starting") return "busy";
  if (state === "unavailable") return "bad";
  return "off";
}

export function Waf(props: PageProps<unknown>) {
  const t = props.t;
  const data = useAdminQuery<Data>(props, "/apps/waf", { refetchInterval: 10000 });
  const alerts = useAdminQuery<{ alerts: AlertRow[]; limit: number }>(props, "/apps/waf/alerts", { refetchInterval: 30000 });
  const [mode, setMode] = useState<Mode>("off");
  const [level, setLevel] = useState<Level>("basic");
  const [allowlist, setAllowlist] = useState("");
  const [sharing, setSharing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [repairing, setRepairing] = useState(false);
  const [rotating, setRotating] = useState(false);
  const ask = useAsk();
  const [error, setError] = useState<unknown>(null);
  // Changed here and not saved yet: what the server says meanwhile (another
  // administrator's save) does not overwrite it.
  const [dirty, setDirty] = useState(false);
  const loaded = data.data?.settings;
  const loadedKey = loaded ? JSON.stringify(loaded) : "";
  useEffect(() => {
    if (!loaded || dirty) return;
    setMode(loaded.mode);
    setLevel(loaded.level);
    setAllowlist(loaded.allowlist.join("\n"));
    setSharing(loaded.sharing);
    // The server's settings, each time they change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadedKey, dirty]);
  const rows = alerts.data?.alerts ?? [];
  const paged = usePaged(rows);
  if (data.isLoading) return <Loading />;
  if (data.error || !data.data) return <ErrorState error={data.error} />;
  const status = data.data.status;
  const fields = error ? fieldErrors(error) : {};
  const words: Record<string, string> = { on: t("On"), off: t("Off"), starting: t("Starting"), unavailable: t("Unavailable") };
  return (
    <>
      <Section
        title={t("Web application firewall")}
        description={t("Checks every request to the services' domains and stops known attacks before they reach the app (CrowdSec). If it fails, requests go through unchecked: a site is never taken down by it.")}
        actions={
          loaded && loaded.mode !== "off" ? (
            <Button
              variant="outline"
              disabled={rotating}
              onClick={async () => {
                const ok = await ask({
                  title: t("Give Traefik a new key for CrowdSec?"),
                  body: t("Traefik restarts with it: domains stop answering for a few seconds. The previous key is removed once the new one works."),
                  action: t("New key"),
                });
                if (!ok) return;
                setRotating(true);
                setError(null);
                try {
                  await props.api.post("/apps/waf/rotate-key", {});
                  props.toast.success(t("New key started: the state shows when it works"));
                  data.refetch();
                } catch (err) {
                  setError(err);
                } finally {
                  setRotating(false);
                }
              }}
            >
              {rotating ? t("Starting…") : t("New key")}
            </Button>
          ) : undefined
        }
      >
        <Facts
          items={[
            [t("State"), <StatusBadge key="s" tone={stateTone(status.state)} label={words[status.state] ?? status.state} />],
            [t("Checked"), status.checked_at ? when(status.checked_at, props.locale) : "–"],
            [t("Services with it off"), String(data.data.services_off)],
          ]}
        />
        {status.state === "unavailable" && status.detail && (
          <Alert variant="warning">
            {t(status.detail)}{" "}
            <Button
              size="sm"
              variant="outline"
              disabled={repairing}
              onClick={async () => {
                setRepairing(true);
                try {
                  await props.api.post("/apps/waf/repair", {});
                  props.toast.success(t("Repair started"));
                  data.refetch();
                } catch (err) {
                  setError(err);
                } finally {
                  setRepairing(false);
                }
              }}
            >
              {repairing ? t("Repairing…") : t("Repair")}
            </Button>
          </Alert>
        )}
        <form
          className="rdp2-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setError(null);
            setBusy(true);
            try {
              await props.api.put("/apps/waf", {
                mode,
                level,
                sharing,
                allowlist: allowlist.split(/[\n,]/).map((a) => a.trim()).filter(Boolean),
              });
              props.toast.success(mode === "off" ? t("Saved: the WAF is off") : t("Saved: the WAF is starting (about a minute)"));
              // The saved settings read back first: never the old ones over
              // what was just saved.
              await data.refetch();
              setDirty(false);
            } catch (err) {
              setError(err);
            } finally {
              setBusy(false);
            }
          }}
        >
          <div className="rdp2-grid2">
            <Field
              label={t("Mode")}
              help={
                mode === "detect"
                  ? t("Attacks are recorded, never stopped: start here and look at the alerts for a while.")
                  : mode === "block"
                    ? t("Attacks get 403 Forbidden before reaching the app.")
                    : t("No request is checked.")
              }
            >
              <NativeSelect
                value={mode}
                onChange={(e) => {
                  setMode(e.target.value as Mode);
                  setDirty(true);
                }}
              >
                <option value="off">{t("Off")}</option>
                <option value="detect">{t("Detect only")}</option>
                <option value="block">{t("Block")}</option>
              </NativeSelect>
            </Field>
            <Field
              label={t("Level")}
              help={
                level === "basic"
                  ? t("Known exploits and probes of secret files. It does not stop generic SQL injection or XSS.")
                  : t("Basic, plus the OWASP Core Rule Set (SQL injection, XSS, path traversal…). More false positives: try it in Detect first.")
              }
            >
              <NativeSelect
                value={level}
                onChange={(e) => {
                  setLevel(e.target.value as Level);
                  setDirty(true);
                }}
                disabled={mode === "off"}
              >
                <option value="basic">{t("Basic")}</option>
                <option value="strict">{t("Strict")}</option>
              </NativeSelect>
            </Field>
          </div>
          <Field
            label={t("Trusted addresses")}
            help={t("IPs or networks never checked, one a line (your office, a monitor). Trusted means not inspected at all.")}
            error={fields.allowlist}
          >
            <Textarea className="rdp2-mono" rows={3} value={allowlist} placeholder={"203.0.113.7\n10.8.0.0/24"} onChange={(e) => {
                setAllowlist(e.target.value);
                setDirty(true);
              }}
              disabled={mode === "off"}
            />
          </Field>
          <label className="rdp2-row">
            <Switch
              checked={sharing}
              onCheckedChange={(on) => {
                setSharing(on);
                setDirty(true);
              }}
              disabled={mode === "off"}
            />{" "}
            {t("Share attack signals with CrowdSec's community (the attacker's IP, the rule and the time; never your requests)")}
          </label>
          {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
          <div className="rdp2-row rdp2-end">
            <Button type="submit" disabled={busy}>
              {busy ? t("Saving…") : t("Save")}
            </Button>
          </div>
        </form>
      </Section>
      {mode !== "off" && (
        <Section
          title={t("Latest alerts")}
          description={t("The latest {count} requests a rule matched (blocked, or recorded in Detect mode). Request bodies are never kept.").replace("{count}", String(alerts.data?.limit ?? 100))}
          flush
        >
          {alerts.isLoading ? (
            <Loading />
          ) : alerts.error ? (
            <ErrorState error={alerts.error} />
          ) : rows.length === 0 ? (
            <EmptyState title={t("No alert yet.")} />
          ) : (
            <>
              <TableScroll>
                <table className="rdp2-table">
                  <thead>
                    <tr>
                      <th>{t("When")}</th>
                      <th>{t("Address")}</th>
                      <th>{t("Request")}</th>
                      <th>{t("Rule")}</th>
                      <th>{t("Result")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {paged.page.map((a) => (
                      <tr key={a.id}>
                        <td>{when(a.at, props.locale)}</td>
                        <td className="rdp2-mono">{a.ip ?? "–"}</td>
                        <td className="rdp2-mono">{[a.host, a.path].filter(Boolean).join("") || "–"}</td>
                        <td>
                          <div>{a.message ?? a.rule ?? "–"}</div>
                          {a.message && a.rule && <div className="rdp2-muted rdp2-small rdp2-mono">{a.rule}</div>}
                        </td>
                        <td>{a.blocked ? t("Blocked") : t("Recorded")}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </TableScroll>
              <Pager total={rows.length} at={paged.at} onPage={paged.set} t={t} />
            </>
          )}
        </Section>
      )}
    </>
  );
}
