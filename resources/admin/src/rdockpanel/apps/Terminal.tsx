// A service's terminal (`/apps/services/{id}/exec/{container}`) and its
// logs (`/apps/services/{id}/logs/{tail}`), for whoever may use them in its
// project — no access to Docker's pages needed. The terminal is xterm.js
// (the panel's extension scripts), its bytes over the panel's socket.
import { useEffect, useRef, useState } from "react";
import type { PageProps } from "@rustcms/admin-sdk";
import { Dialog } from "@rustcms/admin-sdk/ui";
import { Logs } from "../settings/Traefik";

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

export function ServiceLogs(props: PageProps<unknown> & { serviceId: number; onClose: () => void }) {
  const t = props.t;
  return (
    <Dialog open onClose={props.onClose} title={t("Logs")} className="rdp2-wide">
      <Logs {...props} url={`/apps/services/${props.serviceId}/logs/300`} />
    </Dialog>
  );
}

export function ServiceTerminal(props: PageProps<unknown> & { serviceId: number; container: string; onClose: () => void }) {
  const t = props.t;
  const screen = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState(t("Connecting…"));
  useEffect(() => {
    if (!screen.current || !window.Terminal || !window.FitAddon) {
      setStatus(t("The terminal could not load."));
      return;
    }
    const term = new window.Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 13,
      scrollback: 5000,
      theme: { background: "#0b0b0e", foreground: "#e4e4e7" },
      disableStdin: true,
    });
    const fit = new window.FitAddon.FitAddon();
    term.loadAddon(fit);
    term.open(screen.current);
    fit.fit();
    const encoder = new TextEncoder();
    const ended = (text: string) => {
      setStatus(text);
      term.options.disableStdin = true;
    };
    const socket = props.realtime.socket(
      `/apps/services/${props.serviceId}/exec/${encodeURIComponent(props.container)}`,
      {
        onOpen() {
          socket.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
          setStatus("");
          term.options.disableStdin = false;
          term.focus();
        },
        onMessage(data) {
          if (typeof data !== "string") return term.write(new Uint8Array(data as ArrayBuffer));
          const message = JSON.parse(data) as { type: string; message?: string };
          if (message.type === "exit") ended(t("The shell exited."));
          else if (message.type === "idle") ended(t("Closed after 30 minutes without activity."));
          else if (message.type === "error") ended(message.message ?? "");
        },
        onState(state) {
          if (state.state !== "closed") return;
          const code = (state as { code?: number }).code;
          if (code === 4403 || code === 4401) ended(t("You may no longer use this terminal."));
          else setStatus((s) => s || t("Disconnected."));
        },
      },
      { reconnect: false },
    );
    const typing = term.onData((keys) => {
      const bytes = encoder.encode(keys);
      for (let at = 0; at < bytes.length; at += 32 * 1024) socket.send(bytes.subarray(at, at + 32 * 1024));
    });
    return () => {
      typing.dispose();
      socket.close();
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.serviceId, props.container]);
  return (
    <Dialog open onClose={props.onClose} title={t("Shell")} className="rdp2-wide">
      {status && <p className="rdp2-small rdp2-muted">{status}</p>}
      <div ref={screen} className="rdp2-terminal" />
    </Dialog>
  );
}
