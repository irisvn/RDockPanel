// WAF → Cloudflare: the ranges trusted to name the visitor (Traefik, the
// rate limit, the WAF), checked daily against Cloudflare's API: fewer are
// applied at once, more are offered here to apply.
import { useState } from "react";
import { type PageProps } from "@rustcms/admin-sdk";
import { Alert, Button, CodeBlock, ErrorState, Loading } from "@rustcms/admin-sdk/ui";
import { useAdminQuery } from "../lib/query";
import { ErrorNote, Facts, Section, useAsk, when } from "../lib/ui";

type Cloudflare = {
  applied: { ranges: string[]; applied_at: string | null; source: "builtin" | "cloudflare" };
  check: { checked_at: string | null; error?: string | null; offered: string[]; added: string[]; removed: string[] };
  source: string;
};

export function WafCloudflare(props: PageProps<unknown>) {
  const t = props.t;
  const data = useAdminQuery<Cloudflare>(props, "/apps/waf/cloudflare");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [show, setShow] = useState(false);
  const ask = useAsk();
  if (data.isLoading) return <Loading />;
  if (data.error || !data.data) return <ErrorState error={data.error} />;
  const { applied, check } = data.data;
  const fours = applied.ranges.filter((r) => !r.includes(":")).length;
  return (
    <Section
      title={t("Cloudflare's ranges")}
      description={t("Requests from these ranges name their visitor (X-Forwarded-For): Traefik, the rate limit and the WAF see the visitor's own address. Checked daily against Cloudflare's list.")}
      actions={
        <Button
          variant="outline"
          disabled={busy !== null}
          onClick={async () => {
            setBusy("check");
            setError(null);
            try {
              await props.api.post("/apps/waf/cloudflare/check", {});
              data.refetch();
            } catch (err) {
              setError(err);
            } finally {
              setBusy(null);
            }
          }}
        >
          {busy === "check" ? t("Checking…") : t("Check now")}
        </Button>
      }
    >
      <Facts
        items={[
          [t("In use"), applied.source === "builtin" ? t("Built into the panel") : t("Updated {when}").replace("{when}", applied.applied_at ? when(applied.applied_at, props.locale) : "–")],
          [t("Ranges"), t("{v4} IPv4, {v6} IPv6").replace("{v4}", String(fours)).replace("{v6}", String(applied.ranges.length - fours))],
          [t("Checked"), check.checked_at ? when(check.checked_at, props.locale) : t("Not yet")],
        ]}
      />
      <ErrorNote error={error} />
      {check.error && <Alert variant="warning">{t("The last check failed (the ranges in use are kept):")} {check.error}</Alert>}
      {check.offered.length > 0 ? (
        <Alert variant="warning">
          {t("Cloudflare added {n} ranges: apply them so their visitors are seen by their own address.").replace("{n}", String(check.added.length))}
          <CodeBlock code={check.added.join("\n")} />
          <Button
            size="sm"
            disabled={busy !== null}
            onClick={async () => {
              const ok = await ask({ title: t("Apply Cloudflare's new ranges?"), body: t("Traefik starts again with them: domains stop answering for a few seconds."), action: t("Apply") });
              if (!ok) return;
              setBusy("apply");
              setError(null);
              try {
                await props.api.post("/apps/waf/cloudflare/apply", {});
                props.toast.success(t("Applied: Traefik restarts"));
                data.refetch();
              } catch (err) {
                setError(err);
              } finally {
                setBusy(null);
              }
            }}
          >
            {busy === "apply" ? t("Applying…") : t("Apply")}
          </Button>
        </Alert>
      ) : (
        check.checked_at && !check.error && <p className="rdp2-muted rdp2-small">{check.removed.length > 0 ? t("Cloudflare dropped {n} ranges: applied at once.").replace("{n}", String(check.removed.length)) : t("Up to date with Cloudflare's list.")}</p>
      )}
      <div>
        <Button variant="ghost" size="sm" aria-expanded={show} onClick={() => setShow((v) => !v)}>
          {show ? t("Hide the list") : t("Show the list")}
        </Button>
        {show && <CodeBlock code={applied.ranges.join("\n")} />}
      </div>
    </Section>
  );
}
