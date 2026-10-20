// Settings → Cloudflare Tunnel: the panel runs cloudflared with a tunnel's
// token, so domains are reached through Cloudflare without opening ports 80
// and 443. Each public hostname of the tunnel (set in Cloudflare) goes to the
// origin shown here; every domain of the panel is served there too.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, CodeBlock, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field, Section, StatusDot, useAsk } from "../lib/ui";

type Status = {
  enabled: boolean;
  token_set: boolean;
  service: boolean;
  running: boolean;
  connected: boolean;
  error: string | null;
  image: string;
  origin: string;
};

export function Tunnel(props: PageProps<unknown>) {
  const ask = useAsk();
  const t = props.t;
  const status = useAdminQuery<Status>(props, "/apps/tunnel", { refetchInterval: 10000 });
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  if (status.isLoading) return <Loading />;
  if (status.error || !status.data) return <ErrorState error={status.error} />;
  const s = status.data;
  const fields = error ? fieldErrors(error) : {};
  const act = async (path: string, body: unknown) => {
    setBusy(true);
    setError(null);
    try {
      await props.api.post(path, body);
      setToken("");
      status.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  const state = !s.enabled ? "stopped" : s.connected ? "running" : s.error ? "failed" : "queued";
  return (
    <Section
      title={t("Cloudflare Tunnel")}
      description={t("Reach your domains through Cloudflare, without opening ports 80 and 443 (cloudflared runs beside Traefik).")}
    >
      <div className="rdp2-form">
        <p className="rdp2-row">
          <StatusDot status={state} />
          <strong>{s.enabled ? (s.connected ? t("Connected") : s.error ? t("Not connected") : t("Starting")) : t("Off")}</strong>
          <span className="rdp2-small rdp2-muted">{s.image}</span>
        </p>
        {s.enabled && s.error && <Alert variant="warning">{s.error}</Alert>}
        <ol className="rdp2-small">
          <li>{t("In Cloudflare: Zero Trust → Networks → Tunnels → Create a tunnel (cloudflared), copy its token.")}</li>
          <li>{t("Paste it below and turn the tunnel on (Traefik restarts once: a few seconds).")}</li>
          <li>{t("For each domain, add a public hostname to the tunnel with this service:")}</li>
        </ol>
        <CodeBlock code={s.origin} label={t("Service (origin)")} />
        <Alert>
          {t("Cloudflare serves HTTPS (domains' certificates are not used through the tunnel); a visitor over plain HTTP is redirected. The visitor's address reaches apps as Cloudflare's Cf-Connecting-Ip header (the panel sees cloudflared's).")}
        </Alert>
        <Field label={t("Tunnel token")} error={fields.token}>
          <Input
            type="password"
            autoComplete="off"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={s.token_set ? t("Kept (paste a new one to replace it)") : "eyJ…"}
          />
        </Field>
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <div className="rdp2-row rdp2-end">
          {s.enabled && (
            <Button
              variant="ghost"
              disabled={busy}
              onClick={async () => {
                const ok = await ask({
                  title: t("Turn the tunnel off?"),
                  body: t("Domains are then reached only through ports 80 and 443."),
                  action: t("Turn off"),
                  destructive: true,
                });
                if (ok) void act("/apps/tunnel/disable", {});
              }}
            >
              {t("Turn off")}
            </Button>
          )}
          <Button disabled={busy || (!token.trim() && !s.token_set)} onClick={() => void act("/apps/tunnel/enable", { token: token.trim() || null })}>
            {s.enabled ? t("Apply") : t("Turn on")}
          </Button>
        </div>
      </div>
    </Section>
  );
}
