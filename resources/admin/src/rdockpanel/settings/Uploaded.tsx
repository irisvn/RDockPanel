// Settings → Uploaded certificates: a certificate of your own (its PEM chain
// and key, the key sealed), served by Traefik for the names it covers, with
// no restart; "Replace" for a renewed one (no deployment needed). The
// panel's administrators' (`system.manage`); a domain chooses one in its
// dialog. The panel warns the notification channels 14, 7 and 1 days before
// it expires.
import { useState } from "react";
import { fieldErrors, type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, Dialog, ErrorState, Input, Loading, Textarea } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, Field, RowActions, Section, TableScroll, useAsk, when } from "../lib/ui";
import { dateTime } from "../lib/format";

type Certificate = {
  id: number;
  name: string;
  names: string[];
  not_after: string;
  days_left: number;
  fingerprint: string;
  in_use: boolean;
};

function UploadForm(props: PageProps<unknown> & { replacing: Certificate | null; onClose: () => void; onSaved: (selfSigned: boolean) => void }) {
  const t = props.t;
  const r = props.replacing;
  const [name, setName] = useState(r?.name ?? "");
  const [chain, setChain] = useState("");
  const [key, setKey] = useState("");
  const [fields, setFields] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  // A file's text into a field.
  const load = (set: (text: string) => void) => (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) file.text().then(set);
  };
  return (
    <Dialog open onClose={props.onClose} title={r ? t("Replace certificate") : t("Upload certificate")}>
      <form
        className="rdp2-form"
        onSubmit={async (e) => {
          e.preventDefault();
          setFields({});
          setError(null);
          setBusy(true);
          try {
            const body = { name: name.trim(), chain, key };
            const out = r
              ? await props.api.put<{ self_signed: boolean }>(`/apps/certificates/${r.id}`, body)
              : await props.api.post<{ self_signed: boolean }>("/apps/certificates", body);
            props.onSaved(out.self_signed);
          } catch (err) {
            const f = fieldErrors(err);
            if (Object.keys(f).length) setFields(f);
            else setError(err);
          } finally {
            setBusy(false);
          }
        }}
      >
        <Field label={t("Name")} help={t("Optional: its first name otherwise")} error={fields.name}>
          <Input value={name} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field
          label={t("Certificate (PEM)")}
          help={t("The certificate, then its intermediates (fullchain.pem)")}
          error={fields.chain}
        >
          <Input type="file" accept=".pem,.crt,.cer" onChange={load(setChain)} />
          <Textarea className="rdp2-mono" rows={5} value={chain} onChange={(e) => setChain(e.target.value)} placeholder="-----BEGIN CERTIFICATE-----" />
        </Field>
        <Field label={t("Private key (PEM)")} help={t("Not encrypted (privkey.pem); sealed in the panel")} error={fields.key}>
          <Input type="file" accept=".pem,.key" onChange={load(setKey)} />
          <Textarea className="rdp2-mono" rows={4} value={key} onChange={(e) => setKey(e.target.value)} placeholder="-----BEGIN PRIVATE KEY-----" />
        </Field>
        <ErrorNote error={error} />
        <div className="rdp2-row rdp2-end">
          <Button type="submit" disabled={busy || !chain.trim() || !key.trim()}>
            {r ? t("Replace") : t("Upload")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function UploadedCertificates(props: PageProps<unknown>) {
  const t = props.t;
  const list = useAdminQuery<{ certificates: Certificate[] }>(props, "/apps/certificates");
  const [editing, setEditing] = useState<Certificate | "new" | null>(null);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  return (
    <Section
      title={t("Uploaded certificates")}
      description={t("Certificates of your own (a CA's, or a wildcard), served for the names they cover; a domain chooses one in its dialog.")}
      actions={<Button onClick={() => setEditing("new")}>{t("Upload certificate")}</Button>}
    >
      <ErrorNote error={error} />
      {list.data.certificates.length === 0 ? (
        <EmptyState title={t("No certificate uploaded.")} hint={t("The notification channels hear 14, 7 and 1 days before one expires.")} />
      ) : (
        <TableScroll>
          <table className="rdp2-table">
            <thead>
              <tr>
                <th>{t("Name")}</th>
                <th>{t("Covers")}</th>
                <th>{t("Expires")}</th>
                <th className="rdp2-right">
                  <span className="rdp2-sr">{t("Actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {list.data.certificates.map((c) => (
                <tr key={c.id}>
                  <td className="rdp2-strong-cell">{c.name}</td>
                  <td>
                    <span className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={c.names.join(", ")}>
                      {c.names.join(", ")}
                    </span>
                  </td>
                  <td className={c.days_left < 14 ? "rdp2-error rdp2-nowrap" : "rdp2-nowrap"} title={dateTime(c.not_after, props.locale)}>
                    {when(c.not_after, props.locale)}
                    {c.days_left < 14 && ` · ${c.days_left <= 0 ? t("expired") : `${c.days_left} ${t("days left")}`}`}
                  </td>
                  <td>
                    <RowActions
                      label={t("Actions for {host}").replace("{host}", c.name)}
                      menu={[
                        {
                          label: c.in_use ? t("Delete (a domain uses it)") : t("Delete…"),
                          destructive: true,
                          disabled: c.in_use,
                          onSelect: async () => {
                            const ok = await ask({ title: t("Delete the certificate {name}?").replace("{name}", c.name), action: t("Delete certificate"), destructive: true });
                            if (!ok) return;
                            setError(null);
                            try {
                              await props.api.delete(`/apps/certificates/${c.id}`);
                              list.refetch();
                            } catch (err) {
                              setError(err);
                            }
                          },
                        },
                      ]}
                    >
                      <Button size="sm" variant="ghost" onClick={() => setEditing(c)}>
                        {t("Replace")}
                      </Button>
                    </RowActions>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
      )}
      {editing && (
        <UploadForm
          {...props}
          replacing={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={(selfSigned) => {
            setEditing(null);
            props.toast.success(t("Saved: Traefik serves it now."));
            if (selfSigned) props.toast.error(t("Its issuer is in the chain itself: browsers may not trust it."));
            list.refetch();
          }}
        />
      )}
    </Section>
  );
}
