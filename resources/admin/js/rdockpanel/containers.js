// The `containers` page: every container with its state, start/stop/…
// buttons, and a panel with its logs, its live stats or its inspect
// document. The list refreshes every 5 s; logs and stats are server-sent
// events.
window.rustcms.registerPage("rdockpanel/containers", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, bytes, date, badge, button, act, poll, table, page, showState } = window.rdockPanel;
    const t = ctx.t;
    const can = ctx.data?.can ?? {};
    let alive = true;
    let containers = [];
    let open = null; // { id, name, tab }

    const search = h("input", { type: "search", className: "rdp-input", placeholder: t("Search"), oninput: () => renderList() });
    const running = h("input", { type: "checkbox", onchange: () => renderList() });
    const refresh = button(t("Refresh"), () => loadList());
    const { body, note } = page(element, t, "Containers", [search, h("label", { className: "rdp-check" }, running, t("Only running")), refresh]);
    const list = h("div");
    const detail = h("section", { className: "rdp-detail", hidden: true });
    body.append(list, detail);

    const stateKind = (state) => (state === "running" ? "ok" : state === "paused" || state === "restarting" ? "warn" : "muted");
    const path = (id, rest = "") => `/docker/containers/${encodeURIComponent(id)}${rest}`;

    function actions(c) {
      if (!can["docker.manage"]) return [];
      const run = (action, label) =>
        button(t(label), (event) => act(event.target, note, () => api("POST", path(c.id, `/${action}`)), t("Done.")).then(loadList));
      const buttons = [];
      if (c.state === "running") buttons.push(run("restart", "Restart"), run("stop", "Stop"), run("pause", "Pause"));
      else if (c.state === "paused") buttons.push(run("unpause", "Unpause"));
      else buttons.push(run("start", "Start"));
      buttons.push(
        button(
          t("Remove"),
          (event) => {
            if (!window.confirm(t("Remove {name}?").replace("{name}", c.name))) return;
            act(event.target, note, () => api("DELETE", path(c.id, "?force=true")), t("Removed.")).then(() => {
              if (open?.id === c.id) closeDetail();
              loadList();
            });
          },
          "danger",
        ),
      );
      return h("div", { className: "rdp-actions" }, buttons);
    }

    function renderList() {
      const term = search.value.trim().toLowerCase();
      const shown = containers.filter(
        (c) => (!running.checked || c.state === "running") && (!term || c.name.toLowerCase().includes(term) || c.image.toLowerCase().includes(term)),
      );
      const rows = shown.map((c) => [
        h("button", { type: "button", className: "rdp-link", onclick: () => openDetail(c, can["docker.logs"] ? "logs" : "inspect") }, c.name),
        h("span", { className: "rdp-mono" }, c.image),
        badge(c.state, stateKind(c.state)),
        h("span", { title: date(c.created) }, c.status),
        h("span", { className: "rdp-mono" }, c.ports.join(", ")),
        actions(c),
      ]);
      list.replaceChildren(table([t("Name"), t("Image"), t("State"), t("Status"), t("Ports"), t("Actions")], rows, t("No containers.")));
    }

    // `?open=<id>&tab=logs|terminal|stats|inspect`: a service's Logs and
    // Shell buttons open its container here.
    const wanted = new URLSearchParams(window.location.search);
    let deepLink = wanted.get("open");
    async function loadList() {
      try {
        containers = (await api("GET", "/docker/containers")).containers;
        containers.sort((a, b) => a.name.localeCompare(b.name));
        if (alive) renderList();
        if (alive && deepLink) {
          const c = containers.find((x) => x.id === deepLink || x.id.startsWith(deepLink));
          deepLink = null;
          if (c) openDetail(c, wanted.get("tab") || "logs");
        }
      } catch (err) {
        note.error(err);
      }
    }

    // The detail panel: tabs, and the logs (with their own polling) or inspect.
    let stopLogs = () => {};
    function closeDetail() {
      stopLogs();
      open = null;
      detail.hidden = true;
      detail.replaceChildren();
    }

    function openDetail(c, tab) {
      stopLogs();
      open = { id: c.id, name: c.name, tab };
      const tabs = [];
      if (can["docker.logs"]) tabs.push(["logs", "Logs"]);
      if (c.state === "running") tabs.push(["stats", "Stats"]);
      if (c.state === "running" && can["docker.exec"] && window.Terminal) tabs.push(["terminal", "Terminal"]);
      tabs.push(["inspect", "Inspect"]);
      const nav = h(
        "div",
        { className: "rdp-tabs", role: "tablist" },
        tabs.map(([key, label]) =>
          h("button", { type: "button", role: "tab", "aria-selected": String(key === tab), className: `rdp-tab${key === tab ? " rdp-tab-active" : ""}`, onclick: () => openDetail(c, key) }, t(label)),
        ),
      );
      const content = h("div", { className: "rdp-detail-body" });
      detail.replaceChildren(
        h("header", { className: "rdp-detail-header" }, h("h2", { className: "rdp-card-title" }, c.name), nav, button(t("Close"), closeDetail)),
        content,
      );
      detail.hidden = false;
      if (tab === "logs") showLogs(c, content);
      else if (tab === "stats") showStats(c, content);
      else if (tab === "terminal") showTerminal(c, content);
      else showInspect(c, content);
      detail.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }

    // Logs: the last lines, then new ones as they come while "Follow" is on.
    function showLogs(c, content) {
      const MAX_LINES = 5000;
      const lines = h("select", { className: "rdp-input" }, [100, 500, 2000].map((n) => h("option", { value: n }, String(n))));
      lines.value = "500";
      const follow = h("input", { type: "checkbox", checked: true });
      const output = h("pre", { className: "rdp-log", tabindex: "0", "aria-live": "off" });
      content.replaceChildren(
        h("div", { className: "rdp-toolbar" }, h("label", { className: "rdp-check" }, t("Lines"), lines), h("label", { className: "rdp-check" }, follow, t("Follow"))),
        output,
      );
      const append = (batch) => {
        const stick = output.scrollTop + output.clientHeight >= output.scrollHeight - 8;
        output.querySelector(".rdp-log-empty")?.remove();
        output.append(...batch.map((l) => h("span", { className: l.stream === "stderr" ? "rdp-log-err" : "" }, l.text.endsWith("\n") ? l.text : `${l.text}\n`)));
        while (output.childNodes.length > MAX_LINES) output.firstChild.remove();
        if (stick) output.scrollTop = output.scrollHeight;
      };
      let stream = null;
      const start = () => {
        stream?.close();
        stream = ctx.realtime.events(path(c.id, `/logs/follow?tail=${lines.value}`), {
          onEvent(name, data) {
            if (name === "lines") append(data.lines);
          },
          onState(state) {
            // The server sends the last lines again after each (re)connect.
            if (state.state === "open") output.replaceChildren(h("span", { className: "rdp-muted rdp-log-empty" }, t("No output.")));
            showState(note, t, state);
          },
        });
      };
      follow.addEventListener("change", () => (follow.checked ? start() : stream?.close()));
      lines.addEventListener("change", () => (follow.checked ? start() : null));
      start();
      stopLogs = () => stream?.close();
    }

    // Stats: CPU, memory, network and disk I/O, live.
    function showStats(c, content) {
      const value = (label) => {
        const out = h("div", { className: "rdp-card-value" }, "–");
        return [h("section", { className: "rdp-card" }, h("h3", { className: "rdp-card-title" }, t(label)), out), out];
      };
      const [cpuCard, cpu] = value("CPU");
      const [memCard, mem] = value("Memory");
      const [netCard, net] = value("Network");
      const [diskCard, disk] = value("Disk");
      content.replaceChildren(h("div", { className: "rdp-cards" }, cpuCard, memCard, netCard, diskCard));
      const stream = ctx.realtime.events(path(c.id, "/stats"), {
        onEvent(name, s) {
          if (name !== "stats") return;
          cpu.textContent = `${s.cpu_percent.toFixed(1)}%`;
          mem.textContent = s.mem_limit ? `${bytes(s.mem_used)} / ${bytes(s.mem_limit)}` : bytes(s.mem_used);
          net.textContent = `${t("In")} ${bytes(s.net_rx)} · ${t("Out")} ${bytes(s.net_tx)}`;
          disk.textContent = `${t("Read")} ${bytes(s.blk_read)} · ${t("Written")} ${bytes(s.blk_write)}`;
        },
        onState: (state) => showState(note, t, state),
      });
      stopLogs = () => stream.close();
    }

    // Terminal: a shell in the container (xterm.js over a WebSocket). Keys go
    // as bytes, sizes as `{"type":"resize"}`; a closed socket is not reopened
    // (a new one is a new shell).
    function showTerminal(c, content) {
      const screen = h("div", { className: "rdp-terminal" });
      const status = h("p", { className: "rdp-muted" });
      // The old terminal goes first: a new session is a new shell.
      const again = button(t("New session"), () => {
        stopLogs();
        showTerminal(c, content);
      });
      again.hidden = true;
      content.replaceChildren(h("div", { className: "rdp-toolbar" }, status, again), screen);
      const style = getComputedStyle(element);
      const term = new window.Terminal({
        cursorBlink: true,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        fontSize: 13,
        scrollback: 5000,
        theme: { background: "#0b0b0e", foreground: "#e4e4e7", cursor: style.getPropertyValue("--primary").trim() || "#e4e4e7" },
        // No typing until the shell is there: nothing is queued meanwhile.
        disableStdin: true,
      });
      const fit = new window.FitAddon.FitAddon();
      term.loadAddon(fit);
      term.open(screen);
      fit.fit();
      const encoder = new TextEncoder();
      // Output not yet drawn: past this, the shell is faster than the page
      // and the session is closed rather than the tab running out of memory.
      const MAX_PENDING = 8 * 1024 * 1024;
      let pending = 0;
      const ended = (text) => {
        status.textContent = text;
        again.hidden = false;
        term.options.disableStdin = true;
      };
      status.textContent = t("Connecting…");
      const socket = ctx.realtime.socket(
        path(c.id, `/exec?cols=${term.cols}&rows=${term.rows}`),
        {
          onOpen() {
            status.textContent = "";
            term.options.disableStdin = false;
            term.focus();
          },
          onMessage(data) {
            if (typeof data !== "string") {
              const bytes = new Uint8Array(data);
              pending += bytes.length;
              if (pending > MAX_PENDING) {
                socket.close();
                return ended(t("Closed: the output was too fast to show."));
              }
              return term.write(bytes, () => (pending -= bytes.length));
            }
            const message = JSON.parse(data);
            if (message.type === "exit") ended(t("The shell exited."));
            else if (message.type === "idle") ended(t("Closed after 30 minutes without activity."));
          },
          onState(state) {
            if (state.state !== "closed") return;
            if (state.code === 4403 || state.code === 4401) ended(t("You may no longer use this terminal."));
            else if (!again.hidden) return;
            else ended(t("Disconnected."));
          },
        },
        { reconnect: false },
      );
      // Keys and pastes, in messages under the 64 KiB limit; what the
      // socket refuses (closed, or 1 MiB already waiting) is reported.
      const CHUNK = 32 * 1024;
      const typing = term.onData((keys) => {
        const bytes = encoder.encode(keys);
        for (let at = 0; at < bytes.length; at += CHUNK) {
          if (!socket.send(bytes.subarray(at, at + CHUNK))) {
            status.textContent = t("Some input was not sent: type or paste less at once.");
            return;
          }
        }
      });
      let resizeTimer = null;
      const observer = new ResizeObserver(() => {
        clearTimeout(resizeTimer);
        resizeTimer = setTimeout(() => {
          fit.fit();
          socket.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
        }, 150);
      });
      observer.observe(screen);
      stopLogs = () => {
        clearTimeout(resizeTimer);
        observer.disconnect();
        typing.dispose();
        socket.close();
        term.dispose();
      };
    }

    async function showInspect(c, content) {
      const output = h("pre", { className: "rdp-log", tabindex: "0" });
      content.replaceChildren(output);
      try {
        const doc = await api("GET", path(c.id));
        output.textContent = JSON.stringify(doc, null, 2);
      } catch (err) {
        note.error(err);
      }
    }

    const stopList = poll(loadList, 5000);
    return {
      update() {},
      unmount() {
        alive = false;
        stopList();
        stopLogs();
        element.replaceChildren();
      },
    };
  },
});
