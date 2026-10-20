// A domain's dialog, adding one or changing it: the service's Domains tab
// and the Domains page. Written through the domains' resource (its policy:
// a certificate of its own, a wildcard, are the panel's administrators').
import { useState } from "react";
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
