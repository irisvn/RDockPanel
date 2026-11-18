// The WAF page (`/admin/p/waf`): the web application firewall in front of
// the services' domains (CrowdSec's AppSec through Traefik). Its tabs
// (`/p/waf/<tab>`): Overview (state, progress, statistics), Alerts, Rules
// (versions, update, exceptions), IP lists (the blocklist), Cloudflare (its
// ranges) and Settings. The panel's administrators' (`system.manage`).
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Button, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "../lib/query";
import { Page, Tabs, useAsk } from "../lib/ui";
import { WafAlerts } from "./Alerts";
import { WafBlocklist } from "./Blocklist";
import { WafCloudflare } from "./Cloudflare";
import { type WafData } from "./common";
import { WafOverview } from "./Overview";
import { WafRules } from "./Rules";
import { WafSettings } from "./Settings";

const TABS = [
  { id: "overview", label: "Overview" },
  { id: "alerts", label: "Alerts" },
  { id: "rules", label: "Rules" },
  { id: "lists", label: "IP lists" },
  { id: "cloudflare", label: "Cloudflare" },
  { id: "settings", label: "Settings" },
];

export function WafPage(props: PageProps<unknown>) {
  const t = props.t;
  // Every 3 s while starting (its steps), else every 10 s.
  const data = useAdminQuery<WafData>(props, "/apps/waf", {
    refetchInterval: (q) => (["starting", "stopping"].includes(q.state.data?.status.state ?? "") ? 3000 : 10000),
  });
  const [repairing, setRepairing] = useState(false);
  const [rotating, setRotating] = useState(false);
  const ask = useAsk();
  const wanted = (props.path ?? "").split("/")[0];
  const tab = TABS.find((x) => x.id === wanted)?.id ?? "overview";
  const go = (id: string) => props.navigate(`/p/waf/${id}`);
  const repair = async () => {
    setRepairing(true);
    try {
      await props.api.post("/apps/waf/repair", {});
      props.toast.success(t("Repair started: the Overview shows its progress"));
      data.refetch();
      go("overview");
    } catch (err) {
      props.toast.error(err instanceof Error ? err.message : t("Repair failed"));
    } finally {
      setRepairing(false);
    }
  };
  const on = data.data && data.data.settings.mode !== "off";
  const header = {
    title: props.title,
    eyebrow: t("Workspace"),
    heading: t("WAF"),
    description: t("The web application firewall: it checks every request to the services' domains and stops known attacks (CrowdSec)."),
    actions: on ? (
      <>
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
            try {
              await props.api.post("/apps/waf/rotate-key", {});
              props.toast.success(t("New key started: the state shows when it works"));
              data.refetch();
              go("overview");
            } catch (err) {
              props.toast.error(err instanceof Error ? err.message : t("Failed"));
            } finally {
              setRotating(false);
            }
          }}
        >
          {rotating ? t("Starting…") : t("New key")}
        </Button>
        <Button variant="outline" disabled={repairing} onClick={() => void repair()}>
          {repairing ? t("Repairing…") : t("Repair")}
        </Button>
      </>
    ) : undefined,
  };
  if (data.isLoading)
    return (
      <Page {...header}>
        <Loading />
      </Page>
    );
  if (data.error || !data.data)
    return (
      <Page {...header}>
        <ErrorState error={data.error} />
      </Page>
    );
  const d = data.data;
  return (
    <Page {...header}>
      <Tabs label={t("WAF")} value={tab} onSelect={go} tabs={TABS.map((x) => ({ id: x.id, label: t(x.label) }))}>
        {tab === "overview" && <WafOverview {...props} data={d} onRepair={() => void repair()} repairing={repairing} />}
        {tab === "alerts" && <WafAlerts {...props} />}
        {tab === "rules" && <WafRules {...props} data={d} onChanged={() => data.refetch()} />}
        {tab === "lists" && <WafBlocklist {...props} data={d} onSettings={() => go("settings")} />}
        {tab === "cloudflare" && <WafCloudflare {...props} />}
        {tab === "settings" && <WafSettings {...props} data={d} onSaved={() => (data.refetch(), go("overview"))} />}
      </Tabs>
    </Page>
  );
}
