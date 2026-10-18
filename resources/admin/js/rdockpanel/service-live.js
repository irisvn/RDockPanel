// Beside a service (`ServiceLiveSlot`): its state, whether its configuration
// changed since the last deployment, and its latest deployment with the log
// followed live (`/deploy/deployments/{id}/log`). The Deploy, Start, Stop…
// buttons are the resource's actions, above.
window.rustcms.registerSlot("rdockpanel/service-live", {
  apiVersion: 1,
  mount(element, first) {
    const { h, badge, button, showState, resourceAction } = window.rdockPanel;
    let stream = null;
    // Drawn again on `update` (after an action, `ctx.reload()` brings the
    // record's new state and latest deployment).
    const draw = (ctx) => {
      stream?.close();
      stream = null;
      const t = ctx.t;
      const data = ctx.data ?? {};

      const kind = (status) =>
        ({
          running: "ok",
          succeeded: "ok",
          deploying: "warn",
          queued: "warn",
          stopped: "muted",
          new: "muted",
          rolled_back: "warn",
        })[status] ?? (status === "failed" || status === "interrupted" ? "error" : "muted");

      const note = h("p", { className: "rdp-notice", hidden: true });
      const facts = h("dl", { className: "rdp-facts rdp-facts-tight" });
      const deploymentHead = h("div", { className: "rdp-toolbar" });
      const error = h("p", { className: "rdp-notice rdp-notice-error", hidden: true });
      const output = h("pre", { className: "rdp-log rdp-log-short", tabindex: "0" });
      // The service's actions (the resource's, through its action route).
      const busy = data.busy || (data.latest && !data.latest.finished);
      const run = (name, label, kind, question) =>
        button(
          t(label),
          async (event) => {
            if (question && !window.confirm(t(question))) return;
            event.target.disabled = true;
            try {
              await resourceAction("services", name, [String(data.id)]);
              ctx.reload();
            } catch (err) {
              error.hidden = false;
              error.textContent = err.message;
              event.target.disabled = false;
            }
          },
          kind,
        );
      const buttons = [];
      if (data.can?.deploy && !busy) {
        buttons.push(run("deploy", "Deploy", "primary"));
        if (data.status === "stopped") buttons.push(run("start", "Start"));
        if (data.status === "running" || data.status === "failed") {
          buttons.push(
            run("restart", "Restart"),
            run("stop", "Stop", null, "Stop this service? Its tasks are removed."),
          );
        }
      }
      const unpublished = data.latest?.finished && !data.latest.published;
      if (data.can?.deploy && !busy && unpublished) buttons.push(run("republish", "Republish routes"));
      if (data.can?.destroy && !busy && data.status && data.status !== "new")
        buttons.push(run("destroy", "Destroy", "danger", "Remove this service from Docker? Its volumes are kept."));
      const actions = h("div", { className: "rdp-actions" }, buttons);
      element.replaceChildren(
        h("div", { className: "rdp-stack rdp-slot" }, actions, facts, note, deploymentHead, error, output),
      );

      const fact = (label, value) => [h("dt", null, t(label)), h("dd", null, value)];
      facts.replaceChildren(
        ...fact("Status", badge(t(data.status ?? "new"), kind(data.status))),
        ...fact("Swarm service", h("span", { className: "rdp-mono" }, data.swarm_name ?? "")),
        ...fact("First deployed", data.deployed_at ? new Date(data.deployed_at).toLocaleString() : t("never")),
      );
      const notes = [];
      if (data.pending && data.status !== "new")
        notes.push(t("The configuration changed since the last deployment: deploy to apply it."));
      if (unpublished)
        notes.push(t("Traefik's routes were not written: the domains may not follow this deployment. Republish them."));
      if (data.zero_downtime && !data.zero_downtime_possible)
        notes.push(t("Zero downtime needs a health check and no host-mode port: updates stop the old task first."));
      if (notes.length) {
        note.textContent = notes.join(" ");
        note.className = "rdp-notice rdp-notice-warn";
        note.hidden = false;
      }

      const showDeployment = (d) => {
        // (replaceChildren would print a null as "null")
        const parts = [
          h("strong", null, `${t("Deployment")} #${d.id}`),
          badge(t(d.trigger), "muted"),
          badge(t(d.status), kind(d.status)),
          d.image ? h("span", { className: "rdp-mono rdp-muted" }, d.image) : null,
        ];
        deploymentHead.replaceChildren(...parts.filter(Boolean));
        error.hidden = !d.error;
        error.textContent = d.error ?? "";
      };

      const latest = data.latest;
      if (!data.can?.deployments) {
        // Deployments (their log) need their own permission.
        deploymentHead.hidden = true;
        output.hidden = true;
      } else if (!latest) {
        deploymentHead.replaceChildren(h("span", { className: "rdp-muted" }, t("Not deployed yet: use Deploy above.")));
        output.hidden = true;
      } else {
        showDeployment(latest);
        output.textContent = latest.log ?? "";
        output.scrollTop = output.scrollHeight;
        if (!latest.finished && data.can?.deployments && ctx.realtime) {
          stream = ctx.realtime.events(`/deploy/deployments/${encodeURIComponent(latest.id)}/log`, {
            onEvent(name, value) {
              if (name === "log") {
                // The whole log comes first on each connection (`reset`).
                const stick = output.scrollTop + output.clientHeight >= output.scrollHeight - 8;
                if (value.reset !== undefined) output.textContent = value.reset;
                else output.append(value.append);
                if (stick) output.scrollTop = output.scrollHeight;
              } else if (name === "status") {
                showDeployment(value);
                // Over: the record (state, pending changes) changed too.
                if (value.finished) {
                  stream?.close();
                  ctx.reload();
                }
              }
            },
            onState: (state) =>
              showState(
                { clear() {}, error: (e) => ((error.hidden = false), (error.textContent = String(e))) },
                t,
                state,
              ),
          });
        }
      }
    };
    draw(first);
    return {
      update: draw,
      unmount() {
        stream?.close();
        element.replaceChildren();
      },
    };
  },
});

window.rustcms.registerTranslations("vi", {
  Deployment: "Lần deploy",
  "Swarm service": "Swarm service",
  "First deployed": "Deploy lần đầu",
  never: "chưa",
  new: "mới",
  running: "đang chạy",
  stopped: "đã dừng",
  deploying: "đang deploy",
  failed: "lỗi",
  queued: "đang chờ",
  succeeded: "thành công",
  rolled_back: "đã rollback",
  interrupted: "bị ngắt",
  deploy: "deploy",
  start: "start",
  stop: "stop",
  restart: "restart",
  destroy: "destroy",
  rollback: "rollback",
  "The configuration changed since the last deployment: deploy to apply it.":
    "Cấu hình đã thay đổi so với lần deploy trước: bấm Deploy để áp dụng.",
  "Zero downtime needs a health check and no host-mode port: updates stop the old task first.":
    "Zero downtime cần healthcheck và không dùng port host-mode: khi update, task cũ sẽ dừng trước.",
  "Not deployed yet: use Deploy above.": "Chưa deploy: bấm Deploy ở trên.",
  Deploy: "Deploy",
  Start: "Chạy",
  Stop: "Dừng",
  Restart: "Khởi động lại",
  "Republish routes": "Ghi lại route",
  "Traefik's routes were not written: the domains may not follow this deployment. Republish them.":
    "Chưa ghi được route Traefik: domain có thể chưa theo lần deploy này. Bấm Ghi lại route.",
  Destroy: "Gỡ khỏi Docker",
  "Stop this service? Its tasks are removed.": "Dừng service này? Các task sẽ bị gỡ.",
  "Remove this service from Docker? Its volumes are kept.": "Gỡ service khỏi Docker? Volume được giữ lại.",
});
