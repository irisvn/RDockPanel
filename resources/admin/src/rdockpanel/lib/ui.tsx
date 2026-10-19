// Small pieces shared by the apps pages, on the panel's UI kit (ui 2,
// RustCMS 0.6) and theme variables (light and dark).
import { useState, type ReactNode } from "react";
import { Alert, Button, cn, Tabs as KitTabs, TabsContent, TabsList, TabsTrigger } from "@rustcms/admin-sdk/ui";

/** How a status reads: ok, busy, off or bad. */
export function statusTone(status: string | null | undefined, busy = false): "ok" | "busy" | "off" | "bad" {
  if (busy || status === "deploying" || status === "queued" || status === "running_deployment") return "busy";
  if (status === "running" || status === "succeeded") return "ok";
  if (status === "failed" || status === "interrupted") return "bad";
  if (status === "rolled_back") return "busy";
  return "off";
}

export function StatusDot(props: { status?: string | null; busy?: boolean; title?: string }) {
  const tone = statusTone(props.status, props.busy);
  return <span className={cn("rdp2-dot", `rdp2-dot-${tone}`)} title={props.title ?? props.status ?? ""} aria-hidden />;
}

export function StatusBadge(props: { status?: string | null; busy?: boolean; label: string }) {
  const tone = statusTone(props.status, props.busy);
  return (
    <span className={cn("rdp2-badge", `rdp2-badge-${tone}`)}>
      <StatusDot status={props.status} busy={props.busy} />
      {props.label}
    </span>
  );
}

/** Tabs of the page's sub-path (the URL keeps the open tab), on the kit's Tabs (keyboard, ARIA);
 * `children` is the open tab's panel. */
export function Tabs(props: {
  tabs: Array<{ id: string; label: string }>;
  value: string;
  onSelect: (id: string) => void;
  label?: string;
  children?: ReactNode;
}) {
  return (
    <KitTabs value={props.value} onValueChange={props.onSelect}>
      <TabsList aria-label={props.label ?? "Sections"}>
        {props.tabs.map((tab) => (
          <TabsTrigger key={tab.id} value={tab.id}>
            {tab.label}
          </TabsTrigger>
        ))}
      </TabsList>
      <TabsContent value={props.value} className="rdp2-tab-body">
        {props.children}
      </TabsContent>
    </KitTabs>
  );
}

export function CopyButton(props: { text: string; label: string; copied: string }) {
  const [done, setDone] = useState(false);
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(props.text);
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        } catch {
          // Clipboard refused (insecure origin): the text stays selectable.
        }
      }}
    >
      {done ? props.copied : props.label}
    </Button>
  );
}

/** A titled block of a tab, with its actions on the right; `level` 2 directly under the page's title. */
export function Section(props: { title: ReactNode; description?: ReactNode; actions?: ReactNode; children?: ReactNode; level?: 2 | 3 }) {
  const Title = props.level === 2 ? "h2" : "h3";
  return (
    <section className="rdp2-section">
      <header className="rdp2-section-head">
        <div>
          <Title className="rdp2-section-title">{props.title}</Title>
          {props.description && <p className="rdp2-muted">{props.description}</p>}
        </div>
        {props.actions && <div className="rdp2-row">{props.actions}</div>}
      </header>
      {props.children}
    </section>
  );
}

/** A label and its control, with the field's error. */
export function Field(props: { label: string; help?: string; error?: string[]; children: ReactNode }) {
  return (
    <label className="rdp2-field">
      <span className="rdp2-field-label">{props.label}</span>
      {props.children}
      {props.help && <span className="rdp2-muted rdp2-small">{props.help}</span>}
      {props.error?.map((e) => (
        <span key={e} className="rdp2-error rdp2-small">
          {e}
        </span>
      ))}
    </label>
  );
}

/** An error, in words. */
export function ErrorNote(props: { error: unknown }) {
  if (!props.error) return null;
  const message = props.error instanceof Error ? props.error.message : String(props.error);
  return <Alert variant="destructive">{message}</Alert>;
}

/** Facts as a two-column list. */
export function Facts(props: { items: Array<[string, ReactNode]> }) {
  return (
    <dl className="rdp2-facts">
      {props.items.map(([label, value]) => (
        <div key={label} className="rdp2-fact">
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** A sparkline of `values` (0..max). */
export function Sparkline(props: { values: number[]; max?: number }) {
  const values = props.values.slice(-60);
  if (values.length < 2) return <svg className="rdp2-spark" viewBox="0 0 100 24" aria-hidden />;
  const max = props.max ?? Math.max(1, ...values);
  const points = values
    .map((v, i) => `${((i / (values.length - 1)) * 100).toFixed(2)},${(24 - (Math.min(v, max) / max) * 22 - 1).toFixed(2)}`)
    .join(" ");
  return (
    <svg className="rdp2-spark" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden>
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u += 1;
  }
  return `${v.toFixed(v < 10 && u > 0 ? 1 : 0)} ${units[u]}`;
}

export function when(at: string | null | undefined, locale: string): string {
  return at ? new Date(at).toLocaleString(locale) : "";
}
