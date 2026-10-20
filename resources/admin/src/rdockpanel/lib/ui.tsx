// The pieces every page is built from (docs/ui-guidelines.md §7), on the
// panel's UI kit (ui 2, RustCMS 0.6+) and theme variables (light and dark).
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { PageProps } from "@rustcms/admin-sdk";
import {
  Alert,
  Button,
  cn,
  Dialog,
  DropdownMenu,
  DropdownMenuItem,
  DropdownMenuSeparator,
  Input,
  PageHeader,
  Tabs as KitTabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Tooltip,
} from "@rustcms/admin-sdk/ui";
import { Icon, type IconName } from "./icons";
import { statusOf, type StatusKind, type Tone } from "./status";

// ---------------------------------------------------------------- Layout

/** A page: its header (eyebrow, title, description, actions), then its
 * content in the page rhythm. The shell's own heading is hidden: this one
 * replaces it. */
export function Page(
  props: Pick<PageProps<unknown>, "title"> & {
    eyebrow?: ReactNode;
    heading: ReactNode;
    description?: ReactNode;
    actions?: ReactNode;
    children?: ReactNode;
  },
) {
  const { title } = props;
  useEffect(() => {
    title(null);
  }, [title]);
  return (
    <div className="rdp2-page">
      <PageHeader eyebrow={props.eyebrow} title={props.heading} description={props.description} actions={props.actions} />
      {props.children}
    </div>
  );
}

/** The vertical rhythm of a page's sections (24px). */
export function Stack(props: { children?: ReactNode; className?: string }) {
  return <div className={cn("rdp2-stack", props.className)}>{props.children}</div>;
}

/** A section: a card with a header (title, description, actions) and a body.
 * `flush`: the body reaches the card's edges (a table). */
export function Section(props: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  flush?: boolean;
  /** Kept for older callers: sections are always `h2` under the page's `h1`. */
  level?: 2 | 3;
}) {
  return (
    <section className="rdp2-section">
      <header className="rdp2-section-head">
        <div className="rdp2-section-text">
          <h2 className="rdp2-section-title">{props.title}</h2>
          {props.description && <p className="rdp2-section-description">{props.description}</p>}
        </div>
        {props.actions && <div className="rdp2-actions">{props.actions}</div>}
      </header>
      {props.children != null && <div className={props.flush ? "rdp2-section-flush" : "rdp2-section-body"}>{props.children}</div>}
    </section>
  );
}

/** Tabs of the page's sub-path (the URL keeps the open tab), on the kit's
 * Tabs (keyboard, ARIA), in one row that scrolls sideways on a narrow screen;
 * `children` is the open tab's panel, in the page rhythm. */
export function Tabs(props: {
  tabs: Array<{ id: string; label: string }>;
  value: string;
  onSelect: (id: string) => void;
  label?: string;
  children?: ReactNode;
}) {
  return (
    <KitTabs value={props.value} onValueChange={props.onSelect}>
      <div className="rdp2-tabs">
        <TabsList aria-label={props.label ?? "Sections"} className="rdp2-tablist">
          {props.tabs.map((tab) => (
            <TabsTrigger key={tab.id} value={tab.id}>
              {tab.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      <TabsContent value={props.value} className="rdp2-tab-body">
        {props.children}
      </TabsContent>
    </KitTabs>
  );
}

/** The panel's URL of a page path (`/p/dashboard/demo` → `/admin/p/dashboard/demo`),
 * from where this page lives (the admin path is configurable). */
export function adminHref(path: string): string {
  const here = window.location.pathname;
  for (const marker of ["/p/", "/system/", "/r/"]) {
    const at = here.indexOf(marker);
    if (at >= 0) return here.slice(0, at) + path;
  }
  return path;
}

/** A link to a panel page: a plain click stays in the panel, a modified or
 * middle click opens the right URL in a new tab. */
export function NavLink(props: { to: string; navigate: (path: string) => void; className?: string; children: ReactNode }) {
  return (
    <a
      className={props.className ?? "rdp2-link"}
      href={adminHref(props.to)}
      onClick={(e) => {
        if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        e.preventDefault();
        props.navigate(props.to);
      }}
    >
      {props.children}
    </a>
  );
}

/** Whether the screen is phone-narrow (640px or less), kept up to date. */
export function useNarrow(): boolean {
  const query = "(max-width: 640px)";
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const list = window.matchMedia(query);
    const change = () => setNarrow(list.matches);
    list.addEventListener("change", change);
    return () => list.removeEventListener("change", change);
  }, []);
  return narrow;
}

// ---------------------------------------------------------------- Buttons

/** An icon-only button (32px, ghost) with its words in a tooltip and for
 * screen readers. */
export function IconButton(props: {
  icon: IconName;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  href?: string;
  variant?: "ghost" | "outline";
}) {
  const button = props.href ? (
    <a
      href={props.href}
      target="_blank"
      rel="noreferrer"
      aria-label={props.label}
      className={cn("rdp2-icon-button", props.variant === "outline" && "rdp2-icon-outline")}
    >
      <Icon name={props.icon} />
    </a>
  ) : (
    <Button
      type="button"
      variant={props.variant ?? "ghost"}
      size="icon"
      className="rdp2-icon-sm"
      aria-label={props.label}
      disabled={props.disabled}
      onClick={props.onClick}
    >
      <Icon name={props.icon} />
    </Button>
  );
  return <Tooltip content={props.label}>{button}</Tooltip>;
}

/** A link drawn as a button (`buttonVariants`), with its icon. */
export function LinkButton(props: { href: string; children: ReactNode; external?: boolean; size?: "default" | "sm" }) {
  return (
    <a
      href={props.href}
      target={props.external ? "_blank" : undefined}
      rel={props.external ? "noreferrer" : undefined}
      className={cn("rdp2-link-button", props.size === "sm" && "rdp2-link-button-sm")}
    >
      {props.children}
      {props.external && <Icon name="externalLink" size={14} />}
    </a>
  );
}

export type MenuItem = { label: string; onSelect: () => void; destructive?: boolean; disabled?: boolean } | "separator";

/** A row's actions: up to two buttons, then the rest in a `⋯` menu, its
 * destructive items last after a separator. */
export function RowActions(props: { children?: ReactNode; menu?: MenuItem[]; label: string; header?: boolean }) {
  const items = (props.menu ?? []).filter(Boolean);
  // Every item disabled (a deployment runs): no menu, which could not be
  // left with Escape (the kit handles keys on enabled items only).
  const usable = items.some((item) => item !== "separator" && !item.disabled);
  return (
    <div className="rdp2-row-actions">
      {props.children}
      {usable && (
        <DropdownMenu
          variant={props.header ? "outline" : "ghost"}
          size="icon"
          align="end"
          className={props.header ? undefined : "rdp2-icon-sm"}
          aria-label={props.label}
          trigger={<Icon name="more" />}
        >
          {items.map((item, i) =>
            item === "separator" ? (
              <DropdownMenuSeparator key={`s${i}`} />
            ) : (
              <DropdownMenuItem key={item.label} onSelect={item.onSelect} destructive={item.destructive} disabled={item.disabled}>
                {item.label}
              </DropdownMenuItem>
            ),
          )}
        </DropdownMenu>
      )}
    </div>
  );
}

/** Copies `text`; its label says "Copied" for a moment. Icon-only by default. */
export function CopyButton(props: { text: string; label: string; copied: string; withLabel?: boolean }) {
  const [done, setDone] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.text);
      setDone(true);
      setTimeout(() => setDone(false), 1500);
    } catch {
      // Clipboard refused (insecure origin): the text stays selectable.
    }
  };
  if (props.withLabel) {
    return (
      <Button type="button" variant="outline" size="sm" onClick={copy}>
        <Icon name={done ? "check" : "copy"} size={14} />
        {done ? props.copied : props.label}
      </Button>
    );
  }
  return <IconButton icon={done ? "check" : "copy"} label={done ? props.copied : props.label} onClick={copy} />;
}

// ---------------------------------------------------------------- Status

/** How an older status reads (pages not yet on `Status`). */
export function statusTone(status: string | null | undefined, busy = false): Tone {
  if (busy || status === "deploying" || status === "queued" || status === "running_deployment") return "busy";
  if (status === "running" || status === "succeeded" || status === "ok") return "ok";
  if (status === "failed" || status === "interrupted") return "bad";
  return "off";
}

export function StatusDot(props: { status?: string | null; busy?: boolean; title?: string; tone?: Tone }) {
  const tone = props.tone ?? statusTone(props.status, props.busy);
  return <span className={cn("rdp2-dot", `rdp2-dot-${tone}`)} title={props.title} aria-hidden />;
}

/** A status: a dot in its tone and its word (the word in the text color). */
export function StatusBadge(props: { status?: string | null; busy?: boolean; label: string; tone?: Tone; title?: string }) {
  const tone = props.tone ?? statusTone(props.status, props.busy);
  return (
    <span className="rdp2-badge" title={props.title}>
      <StatusDot tone={tone} />
      {props.label}
    </span>
  );
}

/** The status of a thing of `kind`, in its words (docs/ui-guidelines.md §4);
 * the raw value in the tooltip when it says more. */
export function Status(props: { kind: StatusKind; status: string | null | undefined; t: (s: string) => string; busy?: boolean; detail?: string }) {
  const [tone, word] = props.busy ? (["busy", props.kind === "service" ? "Deploying" : "Running"] as const) : statusOf(props.kind, props.status);
  return <StatusBadge tone={tone} label={props.t(word)} title={props.detail} />;
}

// ---------------------------------------------------------------- Data

/** Label-over-value facts of one thing; `mono` per item for values that are code. */
export function Facts(props: { items: Array<[string, ReactNode] | [string, ReactNode, { mono?: boolean }]> }) {
  return (
    <dl className="rdp2-facts">
      {props.items.map(([label, value, options]) => (
        <div key={label} className="rdp2-fact">
          <dt>{label}</dt>
          <dd className={options?.mono ? "rdp2-mono" : undefined}>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** A metric tile: label, value, a sub-line and a sparkline. `plain`: no
 * border (inside a section). */
export function Metric(props: { label: string; value: ReactNode; sub?: ReactNode; values?: number[]; max?: number; plain?: boolean }) {
  return (
    <div className={cn("rdp2-metric", props.plain && "rdp2-metric-plain")}>
      <span className="rdp2-metric-label">{props.label}</span>
      <span className="rdp2-metric-value">{props.value}</span>
      {props.sub != null && <span className="rdp2-metric-sub">{props.sub}</span>}
      {props.values && <Sparkline values={props.values} max={props.max} />}
    </div>
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

/** Nothing to show: a line, an optional hint and an optional action. */
export function EmptyState(props: { title: string; hint?: string; action?: ReactNode }) {
  return (
    <div className="rdp2-empty">
      <p className="rdp2-empty-title">{props.title}</p>
      {props.hint && <p className="rdp2-empty-hint">{props.hint}</p>}
      {props.action}
    </div>
  );
}

/** The current page of `rows` (25 a page), back to the first page when
 * `filter` (what chose the rows) changes, and kept in range when rows come
 * and go; draw `Pager` under the table. */
export function usePaged<T>(rows: T[], filter = "", size = 25): { page: T[]; at: number; set: (n: number) => void } {
  const [at, set] = useState(0);
  const pages = Math.max(1, Math.ceil(rows.length / size));
  const chosen = useRef(filter);
  useEffect(() => {
    if (chosen.current !== filter) {
      chosen.current = filter;
      set(0);
    }
  }, [filter]);
  const current = Math.min(at, pages - 1);
  return { page: rows.slice(current * size, current * size + size), set, at: current };
}

/** `1–25 of 132` with Previous and Next. */
export function Pager(props: { total: number; at: number; size?: number; onPage: (n: number) => void; t: (s: string) => string; note?: ReactNode }) {
  const size = props.size ?? 25;
  const pages = Math.max(1, Math.ceil(props.total / size));
  const from = props.total === 0 ? 0 : props.at * size + 1;
  const to = Math.min(props.total, (props.at + 1) * size);
  return (
    <div className="rdp2-table-foot">
      <span>
        {props.t("{from}–{to} of {total}").replace("{from}", String(from)).replace("{to}", String(to)).replace("{total}", String(props.total))}
        {props.note && <> · {props.note}</>}
      </span>
      {pages > 1 && (
        <span className="rdp2-actions">
          <Button size="sm" variant="outline" disabled={props.at === 0} onClick={() => props.onPage(props.at - 1)}>
            {props.t("Previous")}
          </Button>
          <Button size="sm" variant="outline" disabled={props.at >= pages - 1} onClick={() => props.onPage(props.at + 1)}>
            {props.t("Next")}
          </Button>
        </span>
      )}
    </div>
  );
}

/** A table that scrolls sideways inside its card on a narrow screen. */
export function TableScroll(props: { children: ReactNode }) {
  return <div className="rdp2-table-scroll">{props.children}</div>;
}

// ---------------------------------------------------------------- Forms

/** A label and its control, with its help and the field's error. */
export function Field(props: { label: string; help?: string; error?: string[]; children: ReactNode }) {
  return (
    <label className="rdp2-field">
      <span className="rdp2-field-label">{props.label}</span>
      {props.children}
      {props.help && <span className="rdp2-field-help">{props.help}</span>}
      {props.error?.map((e) => (
        <span key={e} className="rdp2-field-error">
          {e}
        </span>
      ))}
    </label>
  );
}

/** A file picker drawn as a button; `onFile` gets the chosen file. */
export function FilePicker(props: { label: string; ariaLabel?: string; accept?: string; disabled?: boolean; onFile: (file: File) => void }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <input
        ref={input}
        type="file"
        accept={props.accept}
        hidden
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) props.onFile(file);
        }}
      />
      <Button type="button" variant="outline" size="sm" disabled={props.disabled} aria-label={props.ariaLabel} onClick={() => input.current?.click()}>
        <Icon name="upload" size={14} />
        {props.label}
      </Button>
    </>
  );
}

/** A color: its swatch (the browser's picker) and its hex value. */
export function ColorField(props: { value: string; onChange: (value: string) => void; label: string; placeholder?: string }) {
  const hex = /^#[0-9a-f]{6}$/i.test(props.value) ? props.value : (props.placeholder ?? "#000000");
  return (
    <div className="rdp2-color">
      <input type="color" className="rdp2-swatch" aria-label={props.label} value={hex} onChange={(e) => props.onChange(e.target.value)} />
      <Input className="rdp2-mono" value={props.value} placeholder={props.placeholder} onChange={(e) => props.onChange(e.target.value)} />
    </div>
  );
}

// ---------------------------------------------------------------- Feedback

/** An error, in words. */
export function ErrorNote(props: { error: unknown }) {
  if (!props.error) return null;
  const message = props.error instanceof Error ? props.error.message : String(props.error);
  return <Alert variant="destructive">{message}</Alert>;
}

export type ConfirmOptions = {
  title: string;
  body?: ReactNode;
  /** The confirm button's words ("Delete service"). */
  action: string;
  /** Destructive or disruptive: the button is red. */
  destructive?: boolean;
  /** Irreversible: this text must be typed first. */
  typed?: string;
  /** Only telling: one button, no Cancel. */
  notice?: boolean;
};

/** The panel's confirmation (never `window.confirm`): `confirm(…)` resolves
 * to whether it was confirmed; render `dialog` once in the page. */
export function useConfirm(t: (s: string) => string): [(options: ConfirmOptions) => Promise<boolean>, ReactNode, () => void] {
  const [open, setOpen] = useState<(ConfirmOptions & { resolve: (ok: boolean) => void }) | null>(null);
  const [typed, setTyped] = useState("");
  // The question waiting for an answer: a newer one, or leaving the page,
  // answers it "no" (nothing is left pending).
  const waiting = useRef<((ok: boolean) => void) | null>(null);
  useEffect(() => () => waiting.current?.(false), []);
  const confirm = (options: ConfirmOptions) =>
    new Promise<boolean>((resolve) => {
      waiting.current?.(false);
      waiting.current = resolve;
      setTyped("");
      setOpen({ ...options, resolve });
    });
  const close = (ok: boolean) => {
    if (open && waiting.current === open.resolve) waiting.current = null;
    open?.resolve(ok);
    setOpen(null);
  };
  const dialog = open && (
    <Dialog open onClose={() => close(false)} title={open.title} className="rdp2-dialog-compact">
      <form
        className="rdp2-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!open.typed || typed.trim() === open.typed) close(true);
        }}
      >
        {open.body && <div className="rdp2-dialog-body">{open.body}</div>}
        {open.typed && (
          <Field label={t("Type {name} to confirm").replace("{name}", open.typed)}>
            <Input autoFocus autoComplete="off" value={typed} onChange={(e) => setTyped(e.target.value)} />
          </Field>
        )}
        <div className="rdp2-dialog-foot">
          {!open.notice && (
            <Button type="button" variant="ghost" autoFocus={!open.typed} onClick={() => close(false)}>
              {t("Cancel")}
            </Button>
          )}
          <Button type="submit" variant={open.destructive ? "destructive" : "default"} disabled={!!open.typed && typed.trim() !== open.typed}>
            {open.action}
          </Button>
        </div>
      </form>
    </Dialog>
  );
  const cancel = () => {
    if (waiting.current) {
      waiting.current(false);
      waiting.current = null;
    }
    setOpen(null);
  };
  return [confirm, dialog || null, cancel];
}

const AskContext = createContext<((options: ConfirmOptions) => Promise<boolean>) | null>(null);

/** One confirmation dialog for a whole page: every component under it asks
 * with `useAsk()` (set once per registered page). */
export function ConfirmProvider(props: { t: (s: string) => string; path?: string; children: ReactNode }) {
  const [confirm, dialog, cancel] = useConfirm(props.t);
  // Another sub-path (Back, a link): the question asked here goes unanswered.
  useEffect(() => cancel(), [props.path]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <AskContext.Provider value={confirm}>
      {props.children}
      {dialog}
    </AskContext.Provider>
  );
}

/** The page's confirmation: `if (!(await ask({…}))) return;`. */
export function useAsk(): (options: ConfirmOptions) => Promise<boolean> {
  const ask = useContext(AskContext);
  if (!ask) throw new Error("useAsk needs a ConfirmProvider around the page");
  return ask;
}

// ---------------------------------------------------------------- Older helpers

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
