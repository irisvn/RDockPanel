// Helpers shared by rdockpanel's pages (`window.rdockPanel`), loaded before
// them. Pages build elements with `h` (never innerHTML: names, labels and
// logs come from Docker) and call the panel's admin routes with `api`.
(() => {
  const base = () => document.baseURI.replace(/\/$/, ""); // e.g. https://host/admin
  let csrfToken = null;
  // The user the token was issued to: a write is never replayed for another
  // one (signed out and in as someone else in another tab).
  let csrfUser = null;

  async function csrf(refresh) {
    if (!csrfToken || refresh) {
      const response = await fetch(`${base()}/api/auth/csrf`, { credentials: "same-origin" });
      if (!response.ok) throw new ApiError(response.status, "Could not start a secure session");
      const { csrfToken: token, userId } = await response.json();
      const user = String(userId ?? "");
      if (csrfUser !== null && user !== csrfUser) {
        csrfToken = null;
        csrfUser = null;
        throw new ApiError(401, "The session changed: reload the page");
      }
      csrfToken = token;
      csrfUser = user;
    }
    return csrfToken;
  }

  class ApiError extends Error {
    constructor(status, message, body) {
      super(message);
      this.status = status;
      this.body = body;
    }
  }

  // The message of an error answer: validation issues joined, else `message`.
  function messageOf(status, body) {
    const issues = body?.errors ?? body?.issues;
    if (issues && typeof issues === "object") {
      const parts = Object.entries(issues).map(([field, list]) => {
        const texts = (Array.isArray(list) ? list : [list]).map((i) => i?.message ?? String(i));
        return `${field}: ${texts.join(", ")}`;
      });
      if (parts.length) return parts.join("; ");
    }
    return body?.message ?? body?.code ?? `HTTP ${status}`;
  }

  // `path` below /admin/api/x, e.g. "/docker/containers".
  async function api(method, path, body) {
    const write = method !== "GET";
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const headers = { Accept: "application/json" };
      if (write) {
        headers["Content-Type"] = "application/json";
        headers["X-CSRF-Token"] = await csrf(attempt > 0);
      }
      const response = await fetch(`${base()}/api/x${path}`, {
        method,
        credentials: "same-origin",
        headers,
        body: write ? JSON.stringify(body ?? {}) : undefined,
      });
      const text = await response.text();
      const json = text ? JSON.parse(text) : null;
      if (response.ok) return json;
      // A token from an older session: fetch a fresh one once.
      if (write && attempt === 0 && json?.code === "csrf_mismatch") continue;
      throw new ApiError(response.status, messageOf(response.status, json), json);
    }
    return null;
  }

  // Runs the action `name` of the resource `slug` on `keys` (the panel's own
  // action route: same permissions, locks and audit as from its lists).
  async function resourceAction(slug, name, keys) {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const response = await fetch(`${base()}/api/resources/${slug}/actions/${name}`, {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json", Accept: "application/json", "X-CSRF-Token": await csrf(attempt > 0) },
        body: JSON.stringify({ keys }),
      });
      const text = await response.text();
      const json = text ? JSON.parse(text) : null;
      if (response.ok) {
        if (json?.denied?.length) throw new ApiError(403, "Not allowed");
        return json;
      }
      if (attempt === 0 && json?.code === "csrf_mismatch") continue;
      throw new ApiError(response.status, messageOf(response.status, json), json);
    }
    return null;
  }

  // h("button", { className: "rdp-btn", onclick }, "Text", child, …)
  function h(tag, props, ...children) {
    const element = document.createElement(tag);
    for (const [key, value] of Object.entries(props ?? {})) {
      if (value === undefined || value === null || value === false) continue;
      if (key.startsWith("on")) element.addEventListener(key.slice(2), value);
      else if (key === "className" || key === "value" || key === "checked" || key === "disabled" || key === "title")
        element[key] = value;
      else element.setAttribute(key, value === true ? "" : String(value));
    }
    for (const child of children.flat()) {
      if (child === undefined || child === null || child === false) continue;
      element.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return element;
  }

  function bytes(value) {
    const n = Number(value) || 0;
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v >= 10 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
  }

  // Unix seconds → local date and time.
  function date(seconds) {
    return seconds ? new Date(seconds * 1000).toLocaleString() : "";
  }

  function shortId(id) {
    return String(id ?? "").replace(/^sha256:/, "").slice(0, 12);
  }

  // A line under the page title for errors and confirmations.
  function notice() {
    const element = h("p", { className: "rdp-notice", role: "status", hidden: true });
    let timer = null;
    const show = (text, kind) => {
      clearTimeout(timer);
      element.textContent = text;
      element.className = `rdp-notice rdp-notice-${kind}`;
      element.hidden = false;
      if (kind === "ok") timer = setTimeout(() => (element.hidden = true), 4000);
    };
    return {
      element,
      ok: (text) => show(text, "ok"),
      error: (err) => show(err instanceof Error ? err.message : String(err), "error"),
      clear: () => (element.hidden = true),
    };
  }

  // Calls `tick` now and every `ms` while the tab is visible, until stopped.
  function poll(tick, ms) {
    let stopped = false;
    let timer = null;
    const run = async () => {
      if (stopped) return;
      if (!document.hidden) {
        try {
          await tick();
        } catch {
          // The tick reports its own errors.
        }
      }
      if (!stopped) timer = setTimeout(run, ms);
    };
    run();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }

  function table(columns, rows, empty) {
    const head = h("thead", null, h("tr", null, columns.map((c) => h("th", { scope: "col" }, c))));
    const body = h(
      "tbody",
      null,
      rows.length ? rows.map((cells) => h("tr", null, cells.map((c) => h("td", null, c)))) : h("tr", null, h("td", { colspan: columns.length, className: "rdp-muted" }, empty)),
    );
    return h("div", { className: "rdp-table-wrap" }, h("table", { className: "rdp-table" }, head, body));
  }

  function badge(text, kind) {
    return h("span", { className: `rdp-badge rdp-badge-${kind ?? "muted"}` }, text);
  }

  function button(label, onclick, kind) {
    return h("button", { type: "button", className: `rdp-btn${kind ? ` rdp-btn-${kind}` : ""}`, onclick }, label);
  }

  // Runs `work` with `trigger` disabled; reports the outcome on `note`.
  async function act(trigger, note, work, done) {
    if (trigger) trigger.disabled = true;
    try {
      const result = await work();
      if (done) note.ok(done);
      return result;
    } catch (err) {
      note.error(err);
      return undefined;
    } finally {
      if (trigger) trigger.disabled = false;
    }
  }

  // The page frame below the panel's own title: toolbar, notice, body.
  function page(element, t, title, toolbar) {
    const note = notice();
    const body = h("div", { className: "rdp-body" });
    const bar = toolbar?.length ? h("div", { className: "rdp-toolbar", role: "toolbar", "aria-label": t(title) }, toolbar) : null;
    element.replaceChildren(h("div", { className: "rdp-page" }, bar, note.element, body));
    return { body, note };
  }

  // A live stream's state on the page's notice line (`ctx.realtime.events`).
  function showState(note, t, state) {
    if (state.state === "open") note.clear();
    else if (state.state === "reconnecting") note.error(t("Live updates interrupted, reconnecting…"));
    else if (state.reason === "forbidden" || state.status === 403) note.error(t("You may no longer see this."));
    else if (state.reason === "signed_out" || state.status === 401) note.error(t("Signed out."));
    else if (state.status === 404) note.error(t("It no longer exists."));
    else if (state.status === 429) note.error(t("Too many live views open: close some tabs."));
    else note.error(t("Live updates stopped."));
  }

  window.rdockPanel = { showState, resourceAction, api, ApiError, h, bytes, date, shortId, poll, table, badge, button, act, page };

  window.rustcms.registerTranslations("vi", {
    Server: "Máy chủ",
    Containers: "Container",
    Images: "Image",
    Volumes: "Volume",
    Networks: "Network",
    CPU: "CPU",
    Memory: "Bộ nhớ",
    Swap: "Swap",
    Network: "Mạng",
    Disk: "Ổ đĩa",
    "In": "Vào",
    "Out": "Ra",
    Docker: "Docker",
    Swarm: "Swarm",
    "Docker is unavailable": "Không kết nối được Docker",
    "Collecting samples…": "Đang thu thập số liệu…",
    Refresh: "Làm mới",
    Name: "Tên",
    Image: "Image",
    State: "Trạng thái",
    Status: "Tình trạng",
    Ports: "Cổng",
    Actions: "Thao tác",
    Start: "Chạy",
    Stop: "Dừng",
    Restart: "Khởi động lại",
    Pause: "Tạm dừng",
    Unpause: "Tiếp tục",
    Remove: "Xoá",
    Logs: "Log",
    Inspect: "Chi tiết",
    Close: "Đóng",
    Search: "Tìm kiếm",
    "Only running": "Chỉ đang chạy",
    "No containers.": "Không có container nào.",
    Lines: "Số dòng",
    "Follow": "Tự cập nhật",
    "No output.": "Không có log.",
    "Output cut: too large.": "Log quá lớn, đã cắt bớt.",
    Tags: "Tag",
    ID: "ID",
    Size: "Dung lượng",
    Created: "Tạo lúc",
    "In use": "Đang dùng",
    Pull: "Pull",
    "Image to pull, e.g. nginx:1.27": "Image cần pull, ví dụ nginx:1.27",
    "Pulling…": "Đang pull…",
    "Pulled.": "Đã pull xong.",
    "Pull failed": "Pull thất bại",
    Prune: "Dọn dẹp",
    "Remove stopped containers and dangling images? The panel's own are kept.":
      "Xoá các container đã dừng và image không còn dùng? Các object của panel được giữ lại.",
    "Removed {n} containers and {m} images, freed {size}.": "Đã xoá {n} container và {m} image, giải phóng {size}.",
    "No images.": "Không có image nào.",
    "Remove {name}?": "Xoá {name}?",
    "Force (also when in use)": "Bắt buộc (kể cả khi đang dùng)",
    Driver: "Driver",
    Mountpoint: "Thư mục",
    Scope: "Phạm vi",
    Create: "Tạo",
    "New volume name": "Tên volume mới",
    "New network name": "Tên network mới",
    "No volumes.": "Không có volume nào.",
    "No networks.": "Không có network nào.",
    "Created.": "Đã tạo.",
    "Removed.": "Đã xoá.",
    "Done.": "Xong.",
    "bridge (this server)": "bridge (máy này)",
    "Live updates interrupted, reconnecting…": "Mất kết nối cập nhật trực tiếp, đang kết nối lại…",
    "You may no longer see this.": "Bạn không còn quyền xem mục này.",
    "Signed out.": "Đã đăng xuất.",
    "It no longer exists.": "Mục này không còn tồn tại.",
    "Too many live views open: close some tabs.": "Mở quá nhiều luồng trực tiếp: hãy đóng bớt tab.",
    "Live updates stopped.": "Cập nhật trực tiếp đã dừng.",
    Live: "Trực tiếp",
    Stats: "Tài nguyên",
    Terminal: "Terminal",
    "New session": "Phiên mới",
    "Connecting…": "Đang kết nối…",
    "The shell exited.": "Shell đã thoát.",
    "Closed after 30 minutes without activity.": "Đã đóng sau 30 phút không hoạt động.",
    "You may no longer use this terminal.": "Bạn không còn quyền dùng terminal này.",
    "Disconnected.": "Mất kết nối.",
    "Closed: the output was too fast to show.": "Đã đóng: output ra quá nhanh, không hiển thị kịp.",
    "Some input was not sent: type or paste less at once.": "Một phần nội dung chưa được gửi: hãy gõ hoặc dán ít hơn mỗi lần.",
    Read: "Đọc",
    Written: "Ghi",
    "CPU (%)": "CPU (%)",
    "overlay (the swarm)": "overlay (cả swarm)",
  });
})();
