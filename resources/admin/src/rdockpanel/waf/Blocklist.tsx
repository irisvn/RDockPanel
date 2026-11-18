// WAF → IP lists: the blocklist's state (CrowdSec's community list, with
// sharing on, and the addresses blocked by hand) and every blocked address,
// the firewall's too (`lib/blocks`): where each is blocked, until when.
import { type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { BlockedAddresses, useBlocks } from "../lib/blocks";
import { Facts, Section, StatusBadge } from "../lib/ui";
import { type WafData } from "./common";

export function WafBlocklist(props: PageProps<unknown> & { data: WafData; onSettings: () => void }) {
  const t = props.t;
  const blocks = useBlocks(props);
  if (blocks.isLoading) return <Loading />;
  if (blocks.error || !blocks.data) return <ErrorState error={blocks.error} />;
  const b = blocks.data;
  const settings = props.data.settings;
  const active = b.waf.active;
  return (
    <>
      <Section
        title={t("Blocklist")}
        description={t("Addresses that get 403 on every domain: CrowdSec's community list (with sharing on) and those you block. If CrowdSec stops answering, the addresses already blocked stay blocked until their ban ends, and no other one is.")}
        actions={
          <Button variant="outline" onClick={props.onSettings}>
            {t("Settings")}
          </Button>
        }
      >
        <Facts
          items={[
            [t("Blocklist"), <StatusBadge key="b" tone={active ? "ok" : "off"} label={active ? t("On") : t("Off")} />],
            [t("Community"), settings.sharing ? (b.waf.connected ? t("Connected") : t("Not connected")) : t("Sharing off")],
            [t("Community addresses"), Number(b.waf.community ?? 0).toLocaleString(props.locale)],
            [t("Blocked by you"), String(b.blocked.filter((r) => r.waf).length)],
          ]}
        />
        {!active && <Alert>{settings.mode !== "block" ? t("The blocklist works in Block mode only.") : t("The blocklist is off: turn it on in Settings.")}</Alert>}
        {active && (
          <Alert variant="warning">{t("Many people share one address on mobile networks: a reported address can block real visitors. Unblock it here, or turn the blocklist off.")}</Alert>
        )}
      </Section>
      <BlockedAddresses
        {...props}
        layer="waf"
        heading={t("Blocked addresses")}
        blurb={t("On the websites (WAF) and on the whole server (Firewall): the same list as the Firewall page's. Block one from an alert's menu too.")}
      />
    </>
  );
}
