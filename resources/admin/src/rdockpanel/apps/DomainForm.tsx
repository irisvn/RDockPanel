// A domain's dialog, adding one or changing it: the service's Domains tab
// and the Domains page. Written through the domains' resource (its policy:
// a certificate of its own, a wildcard, are the panel's administrators').
import { useEffect, useRef, useState } from "react";
import { Alert, Button, Dialog, Input, NativeSelect, Switch } from "@rustcms/admin-sdk/ui";
import { ErrorNote, Field } from "../lib/ui";
import { ResourceError, resources } from "../lib/resources";
import type { Domain } from "./types";

type T = (s: string) => string;

export type ServiceChoice = { id: number; project: string; name: string; kind: string; writable: boolean };

export function DomainForm(props: {
  t: T;
  /** Changed (else added). */
  domain?: Domain;
  /** The service it is added to (else chosen among `services`). */
  service?: { id: number; kind: string };
  services?: ServiceChoice[];
  /** A host to start with (the generated domain). */
  host?: string;
  cert_resolvers: Array<{ id: number; name: string; kind: string }>;
  certificates: Array<{ id: number; name: string; names: string[] }>;
  /** The installer's resolver (none: Traefik's own certificate). */
  defaultResolver: string | null;
  /** `system.manage`: a certificate of the domain's own is chosen. */
  system: boolean;
  /** Where a host leads (`cloudflare`: its HTTPS is Cloudflare's). */
  lookup?: (host: string) => Promise<{ via: string; tunnel: boolean }>;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t, domain } = props;
  const [serviceId, setServiceId] = useState(String(props.service?.id ?? domain?.service_id ?? ""));
  const [host, setHost] = useState(domain?.host ?? props.host ?? "");
  const [path, setPath] = useState(domain?.path ?? "/");
  const [port, setPort] = useState(String(domain?.port ?? 80));
  const [container, setContainer] = useState(domain?.compose_service ?? "");
  const [middlewares, setMiddlewares] = useState(domain?.middlewares ?? "");
  const [https, setHttps] = useState(domain?.https ?? !(props.host ?? "").endsWith(".localhost"));
  const [resolver, setResolver] = useState(
    domain?.cert_resolver_id != null ? `r:${domain.cert_resolver_id}` : domain?.certificate_id != null ? `c:${domain.certificate_id}` : "",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // Where the host leads, looked up once typing stops (a name, no wildcard).
  const [via, setVia] = useState<{ via: string; tunnel: boolean } | null>(null);
  // The caller's function (a new one each render): read when it runs.
  const lookupRef = useRef(props.lookup);
  lookupRef.current = props.lookup;
  useEffect(() => {
    setVia(null);
    const name = host.trim().toLowerCase();
    const lookup = lookupRef.current;
    if (!lookup || !/^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(name)) return;
    let gone = false;
    const timer = setTimeout(() => {
      lookup(name)
        .then((r) => gone || setVia(r))
        .catch(() => undefined);
    }, 600);
    return () => {
      gone = true;
      clearTimeout(timer);
    };
  }, [host]);
  const fields = error instanceof ResourceError ? error.fieldErrors : {};
  const kind = props.service?.kind ?? props.services?.find((s) => String(s.id) === serviceId)?.kind;
  const isCompose = kind === "compose";
  const choosable = props.system;
  const none = props.cert_resolvers.length === 0 && props.certificates.length === 0;
  const fallback = props.defaultResolver
    ? `${t("The panel's default")} (${props.defaultResolver})`
    : `${t("The panel's default")} (${t("Traefik's own certificate")})`;
  return (
    <Dialog open onClose={props.onClose} title={domain ? t("Edit domain") : t("Add domain")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          const data: Record<string, unknown> = {
            host: host.trim().toLowerCase(),
            path: path.trim() || "/",
            port: Number(port),
            https,
            middlewares: middlewares.trim() || null,
          };
          if (isCompose) data.compose_service = container.trim();
          // A certificate is the administrators' (they alone see the choice).
          if (choosable) {
            data.cert_resolver_id = https && resolver.startsWith("r:") ? resolver.slice(2) : null;
            data.certificate_id = https && resolver.startsWith("c:") ? resolver.slice(2) : null;
          }
          try {
            if (domain) await resources.update("service_domains", domain.id, data);
            else await resources.create("service_domains", { service_id: serviceId, ...data });
            props.onSaved();
          } catch (err) {
            setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        {!domain && props.services && (
          <Field label={t("Service")} error={fields.service_id}>
            <NativeSelect value={serviceId} onChange={(e) => setServiceId(e.target.value)}>
              <option value="">{t("Choose a service")}</option>
              {props.services.filter((s) => s.writable).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.project} / {s.name}
                </option>
              ))}
            </NativeSelect>
          </Field>
        )}
        <Field label={t("Host")} help={t("e.g. app.example.com, or *.example.com for every name under it (DNS pointing to this server)")} error={fields.host}>
          <Input autoFocus value={host} onChange={(e) => setHost(e.target.value)} placeholder="app.example.com" />
        </Field>
        <div className="rdp2-grid2">
          <Field label={t("Path")} error={fields.path}>
            <Input value={path} onChange={(e) => setPath(e.target.value)} />
          </Field>
          <Field label={t("Container port")} error={fields.port}>
            <Input type="number" value={port} onChange={(e) => setPort(e.target.value)} />
          </Field>
        </div>
        <Field label={t("Traefik middlewares")} help={t("Optional, comma-separated, defined in Traefik's files, e.g. ratelimit@file")} error={fields.middlewares}>
          <Input className="rdp2-mono" value={middlewares} onChange={(e) => setMiddlewares(e.target.value)} />
        </Field>
        {isCompose && (
          <Field label={t("Compose service")} help={t("The service of the Compose file it leads to")} error={fields.compose_service}>
            <Input value={container} onChange={(e) => setContainer(e.target.value)} placeholder="web" />
          </Field>
        )}
        <label className="rdp2-row">
          <Switch checked={https} onCheckedChange={setHttps} /> {t("HTTPS")}
        </label>
        {/* Through the tunnel with the installer's resolver (an HTTP challenge):
            it cannot pass Cloudflare. A DNS resolver or an uploaded
            certificate works; without a resolver Traefik's own one is used. */}
        {https && via?.via === "cloudflare" && via.tunnel && !resolver && props.defaultResolver && (
          <Alert variant="warning">
            <span className="rdp2-row rdp2-wrap">
              {t("This name leads to Cloudflare. Through the Cloudflare Tunnel, Cloudflare gives its visitors HTTPS and this server's certificate cannot be issued (Let's Encrypt's check stops at Cloudflare): turn HTTPS off. Proxied to this server instead (Cloudflare's Full mode), keep it, with a DNS resolver or an uploaded certificate.")}
              <Button type="button" size="sm" variant="outline" onClick={() => setHttps(false)}>
                {t("Turn HTTPS off")}
              </Button>
            </span>
          </Alert>
        )}
        {https && choosable && (
          <Field
            label={t("Certificate")}
            help={
              none
                ? t("No resolver or uploaded certificate yet: add one in Settings → Certificates. A wildcard needs a DNS resolver or a wildcard certificate.")
                : t("A resolver, or an uploaded certificate covering the host (Settings). A wildcard needs a DNS resolver or a wildcard certificate.")
            }
            error={fields.cert_resolver_id ?? fields.certificate_id}
          >
            <NativeSelect value={resolver} onChange={(e) => setResolver(e.target.value)}>
              <option value="">{fallback}</option>
              {props.cert_resolvers.length > 0 && (
                <optgroup label={t("Certificate resolvers")}>
                  {props.cert_resolvers.map((r) => (
                    <option key={`r${r.id}`} value={`r:${r.id}`}>
                      {r.name} ({r.kind === "dns" ? "DNS" : "HTTP"})
                    </option>
                  ))}
                </optgroup>
              )}
              {props.certificates.length > 0 && (
                <optgroup label={t("Uploaded certificates")}>
                  {props.certificates.map((c) => (
                    <option key={`c${c.id}`} value={`c:${c.id}`}>
                      {c.name} ({c.names.join(", ")})
                    </option>
                  ))}
                </optgroup>
              )}
            </NativeSelect>
          </Field>
        )}
        {https && !choosable && (
          <p className="rdp2-small rdp2-muted">
            {t("Certificate")}: {fallback}
          </p>
        )}
        {(fields.cert_resolver_id || fields.certificate_id) && !choosable && (
          <Alert variant="destructive">{(fields.cert_resolver_id ?? fields.certificate_id ?? []).join(" ")}</Alert>
        )}
        {Object.keys(fields).length === 0 && <ErrorNote error={error} />}
        <p className="rdp2-small rdp2-muted">{t("Applied when saved, while the service runs.")}</p>
        <div className="rdp2-row rdp2-end">
          <Button type="button" variant="ghost" onClick={props.onClose}>
            {t("Cancel")}
          </Button>
          <Button type="submit" disabled={busy || !host.trim() || (!domain && !serviceId)}>
            {domain ? t("Save") : t("Add")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** Whether `domain` may be changed by someone without `system.manage`. */
export function plainDomain(domain: Pick<Domain, "host" | "cert_resolver_id" | "certificate_id">): boolean {
  return !domain.host.startsWith("*.") && domain.cert_resolver_id == null && domain.certificate_id == null;
}
