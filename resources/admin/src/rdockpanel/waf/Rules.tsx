// WAF → Rules: the AppSec collections in use, bundled and out (Check
// asks CrowdSec's hub), updating them (an action: the bundle back if it
// fails) or going back to the bundle; and the exceptions (a rule a domain is
// spared).
import { useRef, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { Button, Dialog, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "../lib/query";
import { EmptyState, ErrorNote, Field, RowActions, Section, StatusBadge, TableScroll, useAsk, when, Pager, usePaged } from "../lib/ui";
import { DockerLog } from "../actions/Actions";
import { type Exception, type WafData } from "./common";

type Versions = {
  name: string;
  label: string;
  local: string | null;
  bundled: string | null;
  latest: string | null;
};
type Rules = {
  source: "bundled" | "latest";
  check: {
    checked_at: string | null;
    error?: string | null;
    collections: Versions[];
  };
  outdated: number;
  bundled: Record<string, string>;
  exceptions: Exception[];
};

export function WafRules(props: PageProps<unknown> & { data: WafData; onChanged: () => void }) {
  const t = props.t;
  const rules = useAdminQuery<Rules>(props, "/apps/waf/rules");
  const paged = usePaged(rules.data?.exceptions ?? []);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [action, setAction] = useState<number | null>(null);
  const [adding, setAdding] = useState(false);
  const ask = useAsk();
  const off = props.data.settings.mode === "off";
  if (rules.isLoading) return <Loading />;
  if (rules.error || !rules.data) return <ErrorState error={rules.error} />;
  const r = rules.data;
  const run = async (key: string, work: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await work();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(null);
    }
  };
  const rows: Versions[] = r.check.collections.length
    ? r.check.collections
    : [
        {
          name: "crowdsecurity/appsec-virtual-patching",
          label: "Virtual patching",
          local: null,
          bundled: r.bundled["crowdsecurity/appsec-virtual-patching"] ?? null,
          latest: null,
        },
        {
          name: "crowdsecurity/appsec-generic-rules",
          label: "Generic rules",
          local: null,
          bundled: r.bundled["crowdsecurity/appsec-generic-rules"] ?? null,
          latest: null,
        },
        {
          name: "crowdsecurity/appsec-crs",
          label: "OWASP Core Rule Set",
          local: null,
          bundled: r.bundled["crowdsecurity/appsec-crs"] ?? null,
          latest: null,
        },
      ];
  return (
    <>
      <Section
        title={t("Rules")}
        description={
          r.source === "latest"
            ? t("In use: CrowdSec's latest rules, updated here. Back to the bundle gives every server of this release the same ones again.")
            : t("In use: the rules bundled with this release of the panel (the same on every server). Update fetches CrowdSec's latest.")
        }
        actions={
          <Button
            variant="outline"
            disabled={off || busy !== null}
            onClick={() => void run("check", async () => (await props.api.post("/apps/waf/rules/check", {}), rules.refetch()))}
          >
            {busy === "check" ? t("Checking…") : t("Check for updates")}
          </Button>
        }
        flush
      >
        <ErrorNote error={error} />
        {r.check.error && <p className="rdp2-section-body rdp2-error rdp2-small">{r.check.error}</p>}
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Collection")}</th>
                <th>{t("In use")}</th>
                <th className="rdp2-hide-sm">{t("Bundled")}</th>
                <th>{t("Latest")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((c) => (
                <tr key={c.name}>
                  <td>
                    <div className="rdp2-strong-cell">{t(c.label)}</div>
                    <div className="rdp2-muted rdp2-small rdp2-mono">{c.name}</div>
                  </td>
                  <td className="rdp2-num">{c.local ?? "–"}</td>
                  <td className="rdp2-num rdp2-hide-sm">{c.bundled ?? "–"}</td>
                  <td>
                    {c.latest ? c.latest !== c.local ? <StatusBadge tone="busy" label={`${c.latest} · ${t("newer")}`} /> : <span className="rdp2-num">{c.latest}</span> : "–"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
        <div className="rdp2-section-body">
          <div className="rdp2-row rdp2-between rdp2-wrap">
            <span className="rdp2-muted rdp2-small">
              {r.check.checked_at ? t("Checked {when}").replace("{when}", when(r.check.checked_at, props.locale)) : t("Not checked yet.")}{" "}
              {r.outdated > 0 && t("{n} newer: update to use them.").replace("{n}", String(r.outdated))}
            </span>
            <div className="rdp2-row">
              {r.source === "latest" && (
                <Button
                  variant="outline"
                  disabled={off || busy !== null}
                  onClick={async () => {
                    const ok = await ask({
                      title: t("Back to the bundled rules?"),
                      body: t("CrowdSec starts again with the rules of this release (about a minute)."),
                      action: t("Back to the bundle"),
                    });
                    if (ok)
                      void run(
                        "bundled",
                        async () => (
                          await props.api.post("/apps/waf/rules/bundled", {}),
                          props.toast.success(t("Back to the bundled rules: the WAF is starting again")),
                          props.onChanged(),
                          rules.refetch()
                        ),
                      );
                  }}
                >
                  {busy === "bundled" ? t("Starting…") : t("Back to the bundle")}
                </Button>
              )}
              <Button
                disabled={off || busy !== null}
                onClick={async () => {
                  const ok = await ask({
                    title: t("Update the rules?"),
                    body: t(
                      "CrowdSec fetches its latest rules and starts again with them (about a minute; requests go through unchecked meanwhile). If it fails, the bundled rules come back.",
                    ),
                    action: t("Update"),
                  });
                  if (ok)
                    void run("update", async () => {
                      const answer = await props.api.post<{ action: number }>("/apps/waf/rules/update", {});
                      props.toast.success(t("Updating the rules: follow it in its log"));
                      setAction(answer.action);
                    });
                }}
              >
                {busy === "update" ? t("Starting…") : t("Update the rules")}
              </Button>
            </div>
          </div>
        </div>
      </Section>
      <Section
        title={t("Exceptions")}
        description={t("A rule a domain is spared, for a false positive (add one from an alert's menu). Each change starts CrowdSec again.")}
        actions={
          <Button variant="outline" disabled={off} onClick={() => setAdding(true)}>
            {t("Add an exception")}
          </Button>
        }
        flush
      >
        {r.exceptions.length === 0 ? (
          <EmptyState title={t("No exception: every rule checks every domain.")} />
        ) : (
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("Rule")}</th>
                  <th>{t("Domain")}</th>
                  <th className="rdp2-hide-sm">{t("Since")}</th>
                  <th className="rdp2-actions-col">
                    <span className="rdp2-sr-only">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {paged.page.map((e) => (
                  <tr key={`${e.rule}@${e.host}`}>
                    <td className="rdp2-mono">{e.rule}</td>
                    <td className="rdp2-mono">{e.host}</td>
                    <td className="rdp2-hide-sm">{e.since ? when(e.since, props.locale) : "–"}</td>
                    <td>
                      <RowActions label={t("Actions for {host}").replace("{host}", e.host)}>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={async () => {
                            const ok = await ask({
                              title: t("Remove this exception?"),
                              body: t("Rule {rule} checks {host} again (CrowdSec starts again).").replace("{rule}", e.rule).replace("{host}", e.host),
                              action: t("Remove"),
                              destructive: true,
                            });
                            if (ok)
                              void run(
                                "remove",
                                async () => (
                                  await props.api.post("/apps/waf/exceptions/remove", { rule: e.rule, host: e.host }),
                                  props.toast.success(t("Exception removed: the WAF is starting again")),
                                  props.onChanged(),
                                  rules.refetch()
                                ),
                              );
                          }}
                        >
                          {t("Remove")}
                        </Button>
                      </RowActions>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </TableScroll>
        )}
        {r.exceptions.length > 25 && <Pager total={r.exceptions.length} at={paged.at} onPage={paged.set} t={t} />}
      </Section>
      {adding && (
        <ExceptionDialog
          {...props}
          onClose={() => setAdding(false)}
          onDone={() => {
            setAdding(false);
            props.onChanged();
            rules.refetch();
          }}
        />
      )}
      {action !== null && <DockerLog {...props} id={action} url={`/apps/waf/actions/${action}`} onClose={() => (setAction(null), rules.refetch(), props.onChanged())} />}
    </>
  );
}

function ExceptionDialog(props: PageProps<unknown> & { onClose: () => void; onDone: () => void }) {
  const t = props.t;
  const [rule, setRule] = useState("");
  const [host, setHost] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const fields = error ? fieldErrors(error) : {};
  const ruleInput = useRef<HTMLInputElement>(null);
  const hostInput = useRef<HTMLInputElement>(null);
  return (
    <Dialog open onClose={props.onClose} title={t("Add an exception")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await props.api.post("/apps/waf/exceptions", {
              rule: rule.trim(),
              host: host.trim(),
            });
            props.toast.success(t("Exception added: the WAF is starting again"));
            props.onDone();
          } catch (err) {
            setError(err);
            // The first field refused, focused.
            const refused = fieldErrors(err);
            (refused.rule ? ruleInput : refused.host ? hostInput : null)?.current?.focus();
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Rule")} help={t("A rule's id (942100) or name (crowdsecurity/vpatch-env-access), as alerts show them.")} error={fields.rule}>
          <Input ref={ruleInput} autoFocus className="rdp2-mono" value={rule} aria-invalid={fields.rule ? true : undefined} onChange={(e) => setRule(e.target.value)} />
        </Field>
        <Field label={t("Domain")} error={fields.host}>
          <Input ref={hostInput} className="rdp2-mono" value={host} aria-invalid={fields.host ? true : undefined} onChange={(e) => setHost(e.target.value)} placeholder="shop.example.com" />
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-dialog-foot">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !rule.trim() || !host.trim()}>
            {busy ? t("Adding…") : t("Add")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
