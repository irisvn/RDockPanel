// The log viewer and the terminal, shared by the service page and the Docker
// pages (docs/ui-redesign.md, appendix): every behaviour the plain-JS pages
// had is kept.
import { useEffect, useRef, useState } from "react";
import { type PageProps, useEvents } from "@rustcms/admin-sdk";
import { Button, Checkbox, NativeSelect } from "@rustcms/admin-sdk/ui";

type Line = { text: string; err: boolean };

/** Lines kept at most: older ones are dropped. */
const MAX_LINES = 5000;
const CHOICES = [100, 500, 2000];
// Coloured output (ANSI escapes): shown plain.
const plain = (text: string) => text.replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "");

/** A stream of log lines (server-sent `lines` events), mounted while followed. */
function Stream(props: PageProps<unknown> & { url: string; onLines: (batch: Line[]) => void; onReset: () => void; onState: (text: string) => void }) {
  const t = props.t;
  useEvents(props, props.url, {
    onEvent(name, data) {
      if (name !== "lines") return;
      const batch = (data as { lines: Array<{ stream: string; text: string }> }).lines;
      props.onLines(batch.map((l) => ({ text: plain(l.text), err: l.stream === "stderr" })));
    },
    onState(state) {
      const s = state as { state: string; reason?: string; status?: number };
      // The server sends the last lines again after each (re)connect.
      if (s.state === "open") {
        props.onReset();
        props.onState("");
      } else if (s.reason === "forbidden" || s.status === 403) props.onState(t("You may no longer see this."));
      else if (s.reason === "signed_out" || s.status === 401) props.onState(t("Signed out."));
      else if (s.status === 404) props.onState(t("It no longer exists."));
      else if (s.status === 429) props.onState(t("Too many live views open: close some tabs."));
      else if (s.state === "connecting") props.onState(t("Connecting…"));
      else if (s.state === "reconnecting") props.onState(t("Reconnecting…"));
    },
  });
  return null;
}

/** Logs, followed live: `url(lines)` is the stream of the last `lines` lines
 * (none: one fixed stream, no line choice). Follow off keeps what was read. */
export function LogViewer(props: PageProps<unknown> & { url: (lines: number) => string; choices?: boolean }) {
  const t = props.t;
  const [count, setCount] = useState(500);
  const [follow, setFollow] = useState(true);
  const [lines, setLines] = useState<Line[]>([]);
  const [state, setState] = useState(t("Connecting…"));
  const output = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useEffect(() => {
    const el = output.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);
  return (
    <div className="rdp2-form">
      {props.choices !== false && (
        <div className="rdp2-row rdp2-wrap">
          <label className="rdp2-row rdp2-small">
            {t("Lines")}
            <NativeSelect value={String(count)} onChange={(e) => setCount(Number(e.target.value))} aria-label={t("Lines")}>
              {CHOICES.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label className="rdp2-row rdp2-small">
            <Checkbox checked={follow} onChange={(e) => setFollow(e.target.checked)} />
            {t("Follow")}
          </label>
          {state && <span className="rdp2-small rdp2-muted">{state}</span>}
        </div>
      )}
      {props.choices === false && state && <span className="rdp2-small rdp2-muted">{state}</span>}
      {follow && (
        <Stream
          key={count}
          {...props}
          url={props.url(count)}
          onReset={() => setLines([])}
          onState={setState}
          onLines={(batch) => setLines((old) => [...old, ...batch].slice(-MAX_LINES))}
        />
      )}
      <pre
        ref={output}
        className="rdp2-log rdp2-log-tall"
        tabIndex={0}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 8;
        }}
      >
        {lines.length === 0 ? (
          <span className="rdp2-muted">{t("No output.")}</span>
        ) : (
          lines.map((l, i) => (
            <span key={i} className={l.err ? "rdp2-log-err" : undefined}>
              {l.text.endsWith("\n") ? l.text : `${l.text}\n`}
            </span>
          ))
        )}
      </pre>
    </div>
  );
}

type XTerm = {
  cols: number;
  rows: number;
  options: { disableStdin: boolean };
  open(el: HTMLElement): void;
  write(data: Uint8Array | string, done?: () => void): void;
  onData(f: (keys: string) => void): { dispose(): void };
  loadAddon(addon: unknown): void;
  focus(): void;
  dispose(): void;
};

declare global {
  interface Window {
    Terminal?: new (options: Record<string, unknown>) => XTerm;
    FitAddon?: { FitAddon: new () => { fit(): void } };
  }
}

/** Output not yet drawn: past this, the shell is faster than the page and
 * the session closes rather than the tab running out of memory. */
const MAX_PENDING = 8 * 1024 * 1024;
/** Input in messages under the socket's 64 KiB limit. */
const CHUNK = 32 * 1024;

function Session(props: PageProps<unknown> & { path: string; onEnded: () => void }) {
  const t = props.t;
  const screen = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState(t("Connecting…"));
  useEffect(() => {
    if (!screen.current || !window.Terminal || !window.FitAddon) {
      setStatus(t("The terminal could not load."));
      return;
    }
    const style = getComputedStyle(screen.current);
    const color = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
    const term = new window.Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 13,
      scrollback: 5000,
      theme: { background: color("--rdp-terminal-bg", "#0b0b0e"), foreground: color("--rdp-terminal-fg", "#e4e4e7"), cursor: color("--rdp-terminal-fg", "#e4e4e7") },
      // No typing until the shell is there: nothing is queued meanwhile.
      disableStdin: true,
    });
    const fit = new window.FitAddon.FitAddon();
    term.loadAddon(fit);
    term.open(screen.current);
    fit.fit();
    const encoder = new TextEncoder();
    let pending = 0;
    let over = false;
    const ended = (text: string) => {
      over = true;
      setStatus(text);
      term.options.disableStdin = true;
      props.onEnded();
    };
    const resize = () => socket.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
    const socket = props.realtime.socket(
      props.path,
      {
        onOpen() {
          resize();
          setStatus("");
          term.options.disableStdin = false;
          term.focus();
        },
        onMessage(data) {
          if (typeof data !== "string") {
            const bytes = new Uint8Array(data as ArrayBuffer);
            pending += bytes.length;
            if (pending > MAX_PENDING) {
              socket.close();
              return ended(t("Closed: the output was too fast to show."));
            }
            return term.write(bytes, () => {
              pending -= bytes.length;
            });
          }
          const message = JSON.parse(data) as { type: string; message?: string };
          if (message.type === "exit") ended(t("The shell exited."));
          else if (message.type === "idle") ended(t("Closed after 30 minutes without activity."));
          else if (message.type === "error") ended(message.message ?? "");
        },
        onState(state) {
          if (state.state !== "closed" || over) return;
          const code = (state as { code?: number }).code;
          if (code === 4403 || code === 4401) ended(t("You may no longer use this terminal."));
          else ended(t("Disconnected."));
        },
      },
      // A closed socket is the end of that shell: a new one is a new session.
      { reconnect: false },
    );
    const typing = term.onData((keys) => {
      const bytes = encoder.encode(keys);
      for (let at = 0; at < bytes.length; at += CHUNK) {
        if (!socket.send(bytes.subarray(at, at + CHUNK))) {
          setStatus(t("Some input was not sent: type or paste less at once."));
          return;
        }
      }
    });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        fit.fit();
        resize();
      }, 150);
    });
    observer.observe(screen.current);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
      typing.dispose();
      socket.close();
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.path]);
  return (
    <>
      {status && <p className="rdp2-small rdp2-muted">{status}</p>}
      <div ref={screen} className="rdp2-terminal" />
    </>
  );
}

/** A shell over the panel's socket at `path` (xterm.js); `New session` after
 * it ends opens another (the old one is never reopened). */
export function TerminalView(props: PageProps<unknown> & { path: string }) {
  const [session, setSession] = useState(0);
  const [ended, setEnded] = useState(false);
  return (
    <div className="rdp2-form">
      {ended && (
        <div className="rdp2-row rdp2-end">
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setEnded(false);
              setSession((n) => n + 1);
            }}
          >
            {props.t("New session")}
          </Button>
        </div>
      )}
      <Session key={session} {...props} onEnded={() => setEnded(true)} />
    </div>
  );
}
