// WAF → Alerts: the requests a rule matched (kept 30 days by the panel),
// 25 a page, filtered by domain, address, rule, result and period; a row
// opens its detail, and its menu spares the domain the rule, blocks the
// address or trusts it.
import { useRef, useState } from "react";
import { fieldErrors, type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Input, NativeSelect, Skeleton } from "@rustcms/admin-sdk/ui";
import { BlockAddressDialog } from "../lib/blocks";
import { useAdminQuery } from "../lib/query";
import { EmptyState, ErrorNote, Facts, Field, Pager, RowActions, Section, TableScroll, useAsk, when, type MenuItem } from "../lib/ui";

export type AlertRow = {
  id: number;
  uuid: string;
  at: string;
  ip: string;
  country: string | null;
  asn: string | null;
  host: string | null;
  path: string | null;
  rule: string;
  rule_ids: string | null;
  blocked: boolean;
};

/** The rule an exception names, from an alert: a hub rule's name, or the
 * Core Rule Set's id that caught it (not its scoring rules). */
export function exceptionRule(a: AlertRow): string | null {
  if (a.rule.startsWith("crowdsecurity/") && a.rule !== "crowdsecurity/appsec-vpatch") return a.rule;
  const ids = (a.rule_ids ?? a.rule).match(/\d{6}/g) ?? [];
  const caught = ids.find((id) => !/^(901|949|959|980)/.test(id));
  return caught ?? null;
}

export function WafAlerts(props: PageProps<unknown>) {
  const t = props.t;
  const [filters, setFilters] = useState({
    host: "",
    ip: "",
    rule: "",
    result: "",
    hours: "24",
  });
  const [page, setPage] = useState(1);
  const [open, setOpen] = useState<AlertRow | null>(null);
  const [blocking, setBlocking] = useState<AlertRow | null>(null);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const list = useAdminQuery<{ alerts: AlertRow[]; total: number }>(props, "/apps/waf/alerts", { query: { ...filters, page }, refetchInterval: 30000 });
  const set = (key: keyof typeof filters) => (value: string) => {
    setFilters((f) => ({ ...f, [key]: value }));
    setPage(1);
  };
  const filtered = Object.entries(filters).some(([k, v]) => v !== "" && !(k === "hours" && v === "24"));
  const spare = async (a: AlertRow) => {
    const rule = exceptionRule(a);
    if (!rule || !a.host) return;
    const ok = await ask({
      title: t("Spare {host} this rule?").replace("{host}", a.host),
      body: t("Rule {rule} no longer checks {host}'s requests (a false positive). CrowdSec starts again: about a minute.").replace("{rule}", rule).replace("{host}", a.host),
      action: t("Spare the domain"),
    });
    if (!ok) return;
    try {
      await props.api.post("/apps/waf/exceptions", { rule, host: a.host });
      props.toast.success(t("Exception added: the WAF is starting again"));
    } catch (err) {
      setError(err);
    }
  };
  const trust = async (a: AlertRow) => {
    const ok = await ask({
      title: t("Trust {ip}?").replace("{ip}", a.ip),
      body: t("Its requests are never checked again, on any domain. Settings lists the trusted addresses."),
      action: t("Trust the address"),
      destructive: true,
    });
    if (!ok) return;
    try {
      const current = await props.api.get<{
        settings: {
          mode: string;
          level: string;
          allowlist: string[];
          sharing: boolean;
          blocklist: boolean;
        };
      }>("/apps/waf");
      const s = current.settings;
      await props.api.put("/apps/waf", {
        mode: s.mode,
        level: s.level,
        sharing: s.sharing,
        blocklist: s.blocklist,
        allowlist: [...s.allowlist, a.ip],
      });
      props.toast.success(t("Trusted: the WAF is starting again"));
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Section
      title={t("Alerts")}
      description={t("Requests a rule matched (blocked, or recorded in Detect mode), kept 30 days. Request bodies are never kept; secrets in queries are masked.")}
      flush
    >
      <div className="rdp2-toolbar">
        <Input aria-label={t("Domain")} placeholder={t("Domain")} value={filters.host} onChange={(e) => set("host")(e.target.value.trim())} className="rdp2-filter" />
        <Input aria-label={t("Address")} placeholder={t("Address")} value={filters.ip} onChange={(e) => set("ip")(e.target.value.trim())} className="rdp2-mono rdp2-filter" />
        <Input aria-label={t("Rule")} placeholder={t("Rule")} value={filters.rule} onChange={(e) => set("rule")(e.target.value)} className="rdp2-filter" />
        <div className="rdp2-filter">
          <NativeSelect aria-label={t("Result")} value={filters.result} onChange={(e) => set("result")(e.target.value)}>
            <option value="">{t("Blocked or recorded")}</option>
            <option value="blocked">{t("Blocked")}</option>
            <option value="recorded">{t("Recorded")}</option>
          </NativeSelect>
        </div>
        <div className="rdp2-filter">
          <NativeSelect aria-label={t("Period")} value={filters.hours} onChange={(e) => set("hours")(e.target.value)}>
            <option value="1">{t("1 hour")}</option>
            <option value="24">{t("24 hours")}</option>
            <option value="168">{t("7 days")}</option>
            <option value="720">{t("30 days")}</option>
          </NativeSelect>
        </div>
        {filtered && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => (
              setFilters({
                host: "",
                ip: "",
                rule: "",
                result: "",
                hours: "24",
              }),
              setPage(1)
            )}
          >
            {t("Clear filters")}
          </Button>
        )}
      </div>
      <ErrorNote error={error} />
      {list.isLoading ? (
        <Skeleton className="rdp2-skeleton-table" />
      ) : list.error || !list.data ? (
        <ErrorState error={list.error} />
      ) : list.data.alerts.length === 0 ? (
        <EmptyState title={filtered ? t("No alert matches.") : t("No alert yet.")} />
      ) : (
        <>
          <TableScroll>
            <table className="rdp2-table">
              <thead>
                <tr>
                  <th>{t("When")}</th>
                  <th>{t("Address")}</th>
                  <th>{t("Request")}</th>
                  <th className="rdp2-hide-sm">{t("Rule")}</th>
                  <th>{t("Result")}</th>
                  <th className="rdp2-actions-col">
                    <span className="rdp2-sr-only">{t("Actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {list.data.alerts.map((a) => {
                  const menu: MenuItem[] = [
                    { label: t("Details"), onSelect: () => setOpen(a) },
                    {
                      label: t("Spare the domain this rule…"),
                      disabled: !a.host || !exceptionRule(a),
                      onSelect: () => void spare(a),
                    },
                    {
                      label: t("Block the address…"),
                      onSelect: () => setBlocking(a),
                    },
                    {
                      label: t("Trust the address…"),
                      onSelect: () => void trust(a),
                    },
                  ];
                  const request = [a.host, a.path].filter(Boolean).join("") || "–";
                  return (
                    <tr key={a.id}>
                      <td className="rdp2-nowrap">{when(a.at, props.locale)}</td>
                      <td>
                        <div className="rdp2-mono">{a.ip}</div>
                        {(a.country || a.asn) && <div className="rdp2-muted rdp2-small">{[a.country, a.asn].filter(Boolean).join(" · ")}</div>}
                      </td>
                      <td className="rdp2-mono rdp2-truncate" title={request}>
                        {request}
                      </td>
                      <td className="rdp2-hide-sm rdp2-truncate" title={a.rule}>
                        {a.rule}
                      </td>
                      <td className="rdp2-nowrap">{a.blocked ? t("Blocked") : t("Recorded")}</td>
                      <td>
                        <RowActions label={t("Actions for {host}").replace("{host}", a.ip)} menu={menu} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </TableScroll>
          <Pager total={list.data.total} at={(page - 1) * 25} onPage={(at) => setPage(Math.floor(at / 25) + 1)} t={t} />
        </>
      )}
      {open && (
        <Dialog open onClose={() => setOpen(null)} title={t("Alert")}>
          <Facts
            items={[
              [t("When"), when(open.at, props.locale)],
              [t("Address"), open.ip, { mono: true }],
              [t("Country"), open.country ?? "–"],
              [t("Network"), open.asn ?? "–"],
              [t("Domain"), open.host ?? "–", { mono: true }],
              [t("Path"), open.path ?? "–", { mono: true }],
              [t("Rule"), open.rule],
              [t("Rule ids"), open.rule_ids ?? "–", { mono: true }],
              [t("Result"), open.blocked ? t("Blocked") : t("Recorded")],
            ]}
          />
          <div className="rdp2-dialog-foot">
            <Button variant="ghost" onClick={() => setOpen(null)}>
              {t("Close")}
            </Button>
          </div>
        </Dialog>
      )}
      {blocking && <BlockAddressDialog {...props} ip={blocking.ip} reason={blocking.rule} layer="waf" onClose={() => setBlocking(null)} />}
    </Section>
  );
}
