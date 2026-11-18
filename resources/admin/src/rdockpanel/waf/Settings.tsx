// WAF → Settings: its mode (off, detect: recorded only, block), level
// (basic: known exploits; strict: + the OWASP Core Rule Set), trusted
// addresses, community sharing and the blocklist. Saving starts the WAF
// again: the Overview tab follows its progress.
import { useEffect, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, NativeSelect, Switch, Textarea } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section } from "../lib/ui";
import { type Level, type Mode, type WafData } from "./common";

export function WafSettings(props: PageProps<unknown> & { data: WafData; onSaved: () => void }) {
  const t = props.t;
  const loaded = props.data.settings;
  const [mode, setMode] = useState<Mode>(loaded.mode);
  const [level, setLevel] = useState<Level>(loaded.level);
  const [allowlist, setAllowlist] = useState(loaded.allowlist.join("\n"));
  const [sharing, setSharing] = useState(loaded.sharing);
  const [blocklist, setBlocklist] = useState(loaded.blocklist);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // Changed here and not saved yet: what the server says meanwhile (another
  // administrator's save) does not overwrite it.
  const [dirty, setDirty] = useState(false);
  const loadedKey = JSON.stringify(loaded);
  useEffect(() => {
    if (dirty) return;
    setMode(loaded.mode);
    setLevel(loaded.level);
    setAllowlist(loaded.allowlist.join("\n"));
    setSharing(loaded.sharing);
    setBlocklist(loaded.blocklist);
    // The server's settings, each time they change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadedKey, dirty]);
  const fields = error ? fieldErrors(error) : {};
  const change = <T,>(set: (v: T) => void) => (v: T) => {
    set(v);
    setDirty(true);
  };
  return (
    <Section title={t("Settings")} description={t("Saving starts the WAF again: the Overview tab shows its progress.")}>
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
              blocklist,
              allowlist: allowlist.split(/[\n,]/).map((a) => a.trim()).filter(Boolean),
            });
            props.toast.success(mode === "off" ? t("Saved: the WAF is turning off") : t("Saved: the WAF is starting (about a minute)"));
            props.onSaved();
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
            <NativeSelect value={mode} onChange={(e) => change(setMode)(e.target.value as Mode)}>
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
            <NativeSelect value={level} onChange={(e) => change(setLevel)(e.target.value as Level)} disabled={mode === "off"}>
              <option value="basic">{t("Basic")}</option>
              <option value="strict">{t("Strict")}</option>
            </NativeSelect>
          </Field>
        </div>
        <Field
          label={t("Trusted addresses")}
          help={t("IPs or networks never checked, one a line (your office, a monitor). Trusted means not inspected at all. Never Cloudflare's: its visitors are already seen by their own address.")}
          error={fields.allowlist}
        >
          <Textarea
            className="rdp2-mono"
            rows={3}
            value={allowlist}
            placeholder={"203.0.113.7\n10.8.0.0/24"}
            onChange={(e) => change(setAllowlist)(e.target.value)}
            disabled={mode === "off"}
          />
        </Field>
        <label className="rdp2-row">
          <Switch checked={sharing} onCheckedChange={change(setSharing)} disabled={mode === "off"} />{" "}
          {t("Share attack signals with CrowdSec's community (the attacker's IP, the rule and the time; never your requests)")}
        </label>
        <label className="rdp2-row">
          <Switch checked={blocklist} onCheckedChange={change(setBlocklist)} disabled={mode === "off"} />{" "}
          {t("Block the addresses CrowdSec's community reports, and those you block (Block mode)")}
        </label>
        {blocklist && (
          <Alert variant="warning">
            {t("Many people share one address on mobile networks: a reported address can block real visitors. The IP lists tab shows what is blocked.")}
            {!sharing && ` ${t("Without sharing, only the addresses you block are blocked.")}`}
          </Alert>
        )}
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          <Button type="submit" disabled={busy}>
            {busy ? t("Saving…") : t("Save")}
          </Button>
        </div>
      </form>
    </Section>
  );
}
