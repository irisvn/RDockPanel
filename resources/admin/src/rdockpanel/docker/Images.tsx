// The Images page (`/admin/p/images`): images on this server, pulling one (a
// queued job followed through its action until it ends), removing (with
// force for the panel's administrators when in use), pruning.
import { useEffect, useState } from "react";
import { type PageProps, useAdminQuery } from "@rustcms/admin-sdk";
import { Alert, Button, ErrorState, Input, Loading } from "@rustcms/admin-sdk/ui";
import { EmptyState, ErrorNote, type MenuItem, Page, Pager, RowActions, Section, StatusBadge, TableScroll, useAsk, usePaged } from "../lib/ui";
import { dateTime, relative, size } from "../lib/format";

type Image = { id: string; tags: string[]; size: number; created: number; containers: number };
type Can = { "docker.manage"?: boolean; "system.manage"?: boolean };
type Report = { containers_deleted: number; images_deleted: number; space_reclaimed: number };
type Action = { status: string; log: string | null; error: string | null };

const short = (id: string) => id.replace(/^sha256:/, "").slice(0, 12);

/** A pull's action, followed until it ends (or can no longer be read:
 * signed out, not allowed, gone); a failure stays shown with its log. */
function Pull(props: PageProps<unknown> & { id: number; onDone: (ok: boolean) => void }) {
  const t = props.t;
  const action = useAdminQuery<{ action: Action }>(props, `/docker/actions/${props.id}`, {
    refetchInterval: (q) => (q.state.error ? false : q.state.data && ["ok", "failed"].includes(q.state.data.action.status) ? false : 1500),
  });
  const a = action.data?.action;
  const lost = action.error != null;
  useEffect(() => {
    if (lost) props.onDone(false);
    else if (a && (a.status === "ok" || a.status === "failed")) props.onDone(a.status === "ok");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [a?.status, lost]);
  return (
    <div className="rdp2-form">
      {lost && <ErrorNote error={action.error} />}
      {a?.status === "failed" && <Alert variant="destructive">{`${t("Pull failed")}: ${a.error ?? ""}`}</Alert>}
      <pre className="rdp2-log">{a?.log || (lost ? " " : t("Pulling…"))}</pre>
    </div>
  );
}

export function Images(props: PageProps<{ can?: Can }>) {
  const t = props.t;
  const can = props.data?.can ?? {};
  const list = useAdminQuery<{ images: Image[] }>(props, "/docker/images", { refetchInterval: 10000 });
  const [reference, setReference] = useState("");
  const [pulling, setPulling] = useState<number | null>(null);
  // The last pull, shown until the next one (a failure keeps its log).
  const [shown, setShown] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const ask = useAsk();
  const images = [...(list.data?.images ?? [])].sort((a, b) => b.created - a.created);
  const paged = usePaged(images);
  const header = { title: props.title, eyebrow: "Docker", heading: t("Images"), description: t("Images on this server: pull one, remove what you no longer use.") };
  if (list.isLoading) return <Loading />;
  if (list.error || !list.data) return <ErrorState error={list.error} />;
  const prune = async () => {
    const ok = await ask({
      title: t("Remove stopped containers and dangling images?"),
      body: t("The panel's own are kept."),
      action: t("Prune"),
      destructive: true,
    });
    if (!ok) return;
    setBusy(true);
    setError(null);
    try {
      const r = (await props.api.post<{ report: Report }>("/docker/prune", {})).report;
      props.toast.success(
        t("Removed {n} containers and {m} images, freed {size}.")
          .replace("{n}", String(r.containers_deleted))
          .replace("{m}", String(r.images_deleted))
          .replace("{size}", size(r.space_reclaimed, props.locale)),
      );
      list.refetch();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };
  const remove = async (image: Image, tag: string) => {
    const ok = await ask({ title: t("Remove {name}?").replace("{name}", tag), action: t("Remove image"), destructive: true });
    if (!ok) return;
    // `containers` is -1 when Docker did not count: maybe in use.
    const force =
      image.containers !== 0 &&
      !!can["system.manage"] &&
      (await ask({
        title: t("Force the removal?"),
        body: t("A container may use this image: force removes it anyway."),
        action: t("Force (also when in use)"),
        destructive: true,
      }));
    setError(null);
    try {
      // A tag or a registry path in the query (no path segment takes `:` or `/`).
      await props.api.delete("/docker/images", { query: { image: tag, force } });
      props.toast.success(t("Removed"));
      list.refetch();
    } catch (err) {
      setError(err);
    }
  };
  return (
    <Page
      {...header}
      actions={
        can["docker.manage"] && (
          <Button variant="outline" disabled={busy} onClick={prune}>
            {t("Prune")}
          </Button>
        )
      }
    >
      <ErrorNote error={error} />
      {can["docker.manage"] && (
        <Section title={t("Pull an image")} description={t("From Docker Hub or a registry: the latest layers are downloaded.")}>
          <form
            className="rdp2-toolbar rdp2-toolbar-inline"
            onSubmit={async (e) => {
              e.preventDefault();
              setError(null);
              try {
                const queued = await props.api.post<{ action: { id: number } }>("/docker/images/pull", { image: reference.trim() });
                setPulling(queued.action.id);
                setShown(queued.action.id);
              } catch (err) {
                setError(err);
              }
            }}
          >
            <Input
              className="rdp2-mono rdp2-grow"
              placeholder={t("Image to pull, e.g. nginx:1.27")}
              aria-label={t("Image to pull, e.g. nginx:1.27")}
              value={reference}
              onChange={(e) => setReference(e.target.value)}
            />
            <Button type="submit" disabled={!reference.trim() || pulling !== null}>
              {pulling !== null ? t("Pulling…") : t("Pull")}
            </Button>
          </form>
          {shown !== null && (
            <Pull
              key={shown}
              {...props}
              id={shown}
              onDone={(ok) => {
                setPulling(null);
                if (ok) {
                  props.toast.success(t("Pulled."));
                  setReference("");
                  setShown(null);
                }
                list.refetch();
              }}
            />
          )}
        </Section>
      )}
      <Section
        title={t("All images")}
        flush
        actions={
          <Button size="sm" variant="outline" onClick={() => list.refetch()}>
            {t("Refresh")}
          </Button>
        }
      >
        {images.length === 0 ? (
          <EmptyState title={t("No images.")} />
        ) : (
          <>
            <TableScroll>
              <table className="rdp2-table">
                <thead>
                  <tr>
                    <th>{t("Tags")}</th>
                    <th className="rdp2-hide-sm">{t("ID")}</th>
                    <th className="rdp2-right">{t("Size")}</th>
                    <th className="rdp2-hide-sm">{t("Created")}</th>
                    <th>{t("State")}</th>
                    <th className="rdp2-right">
                      <span className="rdp2-sr">{t("Actions")}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {paged.page.map((image) => {
                    const tags = image.tags.length ? image.tags : [short(image.id)];
                    const menu: MenuItem[] = can["docker.manage"]
                      ? tags.map((tag) => ({
                          label: tags.length > 1 ? `${t("Remove")} ${tag}…` : t("Remove…"),
                          destructive: true,
                          onSelect: () => void remove(image, image.tags.length ? tag : image.id),
                        }))
                      : [];
                    return (
                      <tr key={image.id}>
                        <td>
                          {tags.map((tag) => (
                            <div key={tag} className="rdp2-mono rdp2-ellipsis rdp2-cell-clip" title={tag}>
                              {tag}
                            </div>
                          ))}
                        </td>
                        <td className="rdp2-hide-sm rdp2-mono" title={image.id}>
                          {short(image.id)}
                        </td>
                        <td className="rdp2-right rdp2-num rdp2-nowrap">{size(image.size, props.locale)}</td>
                        <td className="rdp2-hide-sm rdp2-nowrap" title={dateTime(image.created * 1000, props.locale)}>
                          {relative(image.created * 1000, props.locale)}
                        </td>
                        <td>{image.containers > 0 ? <StatusBadge tone="ok" label={t("In use")} /> : <StatusBadge tone="off" label={t("Unused")} />}</td>
                        <td>
                          <RowActions label={t("Actions for {host}").replace("{host}", tags[0])} menu={menu} />
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </TableScroll>
            <Pager total={images.length} at={paged.at} onPage={paged.set} t={t} />
          </>
        )}
      </Section>
    </Page>
  );
}
