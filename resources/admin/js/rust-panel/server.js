// The `server` page: the host's CPU, memory, network and disk over the last
// minutes, like EasyPanel's dashboard, live from `/server/live` (server-sent
// events: the history first, then each sample).
window.rustcms.registerPage("rust-panel/server", {
  apiVersion: 1,
  mount(element, ctx) {
    const { api, h, bytes, percent, duration, page, showState } = window.rustPanel;
    const t = ctx.t;
    const { body, note } = page(element, t, "Server");

    const info = h("dl", { className: "rp-facts" });
    const cards = h("div", { className: "rp-cards" });
    body.append(info, cards);

    // One chart card per metric: a value line and a canvas of the history.
    function card(title, series) {
      const value = h("div", { className: "rp-card-value" });
      const detail = h("div", { className: "rp-card-detail rp-muted" });
      const canvas = h("canvas", { className: "rp-chart", width: 600, height: 160, "aria-hidden": "true" });
      const element = h("section", { className: "rp-card" }, h("h2", { className: "rp-card-title" }, t(title)), value, detail, canvas);
      cards.append(element);
      return { value, detail, canvas, series };
    }
    const cpu = card("CPU", [{ key: "cpu", color: "--primary" }]);
    const memory = card("Memory", [{ key: "mem", color: "--primary" }]);
    const network = card("Network", [
      { key: "rx", color: "--success" },
      { key: "tx", color: "--warning" },
    ]);
    const disk = card("Disk", [{ key: "disk", color: "--primary" }]);

    function color(variable) {
      return getComputedStyle(element).getPropertyValue(variable).trim() || "#888";
    }

    // Lines of `points` (arrays of numbers, one per series) scaled to `max`
    // (0: the largest value). Drawn at the canvas's displayed size.
    function draw(chart, points, max) {
      const canvas = chart.canvas;
      const ratio = window.devicePixelRatio || 1;
      const width = canvas.clientWidth || 600;
      const height = canvas.clientHeight || 160;
      if (canvas.width !== Math.round(width * ratio)) canvas.width = Math.round(width * ratio);
      if (canvas.height !== Math.round(height * ratio)) canvas.height = Math.round(height * ratio);
      const g = canvas.getContext("2d");
      g.setTransform(ratio, 0, 0, ratio, 0, 0);
      g.clearRect(0, 0, width, height);
      g.strokeStyle = color("--border");
      g.lineWidth = 1;
      for (const y of [0.25, 0.5, 0.75]) {
        g.beginPath();
        g.moveTo(0, Math.round(height * y) + 0.5);
        g.lineTo(width, Math.round(height * y) + 0.5);
        g.stroke();
      }
      const top = max || Math.max(1, ...points.flat());
      chart.series.forEach((series, s) => {
        if (points.length < 2) return;
        g.strokeStyle = color(series.color);
        g.lineWidth = 2;
        g.beginPath();
        points.forEach((p, i) => {
          const x = (i / (points.length - 1)) * width;
          const y = height - 2 - (Math.min(p[s], top) / top) * (height - 4);
          if (i === 0) g.moveTo(x, y);
          else g.lineTo(x, y);
        });
        g.stroke();
      });
    }

    function render(samples) {
      const last = samples[samples.length - 1];
      if (!last) {
        cpu.value.textContent = t("Collecting samples…");
        return;
      }
      cpu.value.textContent = `${last.cpu_percent.toFixed(1)}%`;
      draw(cpu, samples.map((s) => [s.cpu_percent]), 100);

      memory.value.textContent = `${bytes(last.mem_used)} / ${bytes(last.mem_total)}`;
      memory.detail.textContent = `${percent(last.mem_used, last.mem_total).toFixed(0)}% · ${t("Swap")} ${bytes(last.swap_used)} / ${bytes(last.swap_total)}`;
      draw(memory, samples.map((s) => [s.mem_used]), last.mem_total);

      network.value.textContent = `${t("In")} ${bytes(last.net_rx_rate)}/s · ${t("Out")} ${bytes(last.net_tx_rate)}/s`;
      draw(network, samples.map((s) => [s.net_rx_rate, s.net_tx_rate]), 0);

      disk.value.textContent = `${bytes(last.disk_used)} / ${bytes(last.disk_total)}`;
      disk.detail.textContent = `${percent(last.disk_used, last.disk_total).toFixed(0)}%`;
      draw(disk, samples.map((s) => [s.disk_used]), last.disk_total);
    }

    function fact(label, value) {
      return [h("dt", null, t(label)), h("dd", null, value)];
    }

    async function loadInfo() {
      try {
        const host = await api("GET", "/server/info");
        info.replaceChildren(
          ...fact("Hostname", host.hostname),
          ...fact("Operating system", host.os),
          ...fact("Kernel", host.kernel),
          ...fact("Architecture", host.architecture),
          ...fact("Cores", host.cpus),
          ...fact("Uptime", duration(host.uptime_secs)),
        );
      } catch (err) {
        note.error(err);
      }
    }

    // The samples shown: the server's history, then each new one live.
    const KEEP = 150;
    let samples = [];
    // Samples by time, once each: the history and the live ones can overlap.
    const merge = (more) => {
      const byTime = new Map(samples.map((s) => [s.at, s]));
      for (const s of more) byTime.set(s.at, s);
      samples = [...byTime.values()].sort((a, b) => a.at - b.at).slice(-KEEP);
    };
    loadInfo();
    const live = ctx.realtime.events("/server/live", {
      onEvent(name, data) {
        if (name === "history") merge(data.samples);
        else if (name === "sample") merge([data]);
        else if (name === "resync") {
          // Samples were missed: read the history again (merged, so what came meanwhile stays).
          api("GET", "/server/samples").then((r) => render((merge(r.samples), samples)), (err) => note.error(err));
          return;
        } else return;
        render(samples);
      },
      onState: (state) => showState(note, t, state),
    });

    return {
      update() {},
      unmount() {
        live.close();
        element.replaceChildren();
      },
    };
  },
});
