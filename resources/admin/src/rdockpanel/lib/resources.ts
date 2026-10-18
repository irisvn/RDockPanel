// The panel's resource API (`{admin}/api/resources/<slug>/…`) from React
// pages: records and actions go through the resources' policies, validation,
// audit and encrypted fields. `props.api` only reaches `{admin}/api/x`; this
// stands in until the SDK's `props.resources` (RustCMS 0.6).

const base = () => document.baseURI.replace(/\/$/, ""); // e.g. https://host/admin

let token: string | null = null;
// The user the page was opened for (`bindSession`); once another session
// shows up, every write is refused until the page is reloaded.
let pageUser: string | null = null;
let foreign = false;

/** Binds the writes to the page's user (the page's `data.user`). */
export function bindSession(user: string) {
  if (pageUser !== null && pageUser !== user) foreign = true;
  pageUser = user;
}

async function csrf(refresh: boolean): Promise<string> {
  if (foreign) throw new ResourceError(401, "The session changed: reload the page", null);
  if (!token || refresh) {
    const response = await fetch(`${base()}/api/auth/csrf`, { credentials: "same-origin" });
    if (!response.ok) throw new ResourceError(response.status, "Could not start a secure session", null);
    const body = (await response.json()) as { csrfToken: string; userId: unknown };
    const user = String(body.userId ?? "");
    // Never a write for another user (signed in again elsewhere).
    if (pageUser === null || user !== pageUser) {
      foreign = true;
      token = null;
      throw new ResourceError(401, "The session changed: reload the page", null);
    }
    token = body.csrfToken;
  }
  return token;
}

/** An error answer: its status, stable code, and field errors (422). */
export class ResourceError extends Error {
  readonly status: number;
  readonly body: unknown;
  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.status = status;
    this.body = body;
  }
  get code(): string | undefined {
    return (this.body as { code?: string } | null)?.code;
  }
  /** `{field: [message, …]}` of a validation error. */
  get fieldErrors(): Record<string, string[]> {
    const issues = (this.body as { errors?: unknown; issues?: unknown } | null)?.errors ??
      (this.body as { issues?: unknown } | null)?.issues;
    const out: Record<string, string[]> = {};
    if (issues && typeof issues === "object") {
      for (const [field, list] of Object.entries(issues as Record<string, unknown>)) {
        const items = Array.isArray(list) ? list : [list];
        out[field] = items.map((i) => (i as { message?: string })?.message ?? String(i));
      }
    }
    return out;
  }
}

function messageOf(status: number, body: unknown): string {
  const errors = new ResourceError(status, "", body).fieldErrors;
  const parts = Object.entries(errors).map(([field, list]) => `${field}: ${list.join(", ")}`);
  if (parts.length) return parts.join("; ");
  const b = body as { message?: string; code?: string } | null;
  return b?.message ?? b?.code ?? `HTTP ${status}`;
}

async function send<T>(method: string, path: string, body?: unknown): Promise<T> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const response = await fetch(`${base()}/api${path}`, {
      method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
        "X-CSRF-Token": await csrf(attempt > 0),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    const json: unknown = text ? JSON.parse(text) : null;
    if (response.ok) return json as T;
    // A token of an older session: once with a fresh one.
    if (attempt === 0 && (json as { code?: string } | null)?.code === "csrf_mismatch") continue;
    throw new ResourceError(response.status, messageOf(response.status, json), json);
  }
  throw new ResourceError(0, "unreachable", null);
}

type RecordReply = { data: { key: string; data: Readonly<{ [field: string]: unknown }> } };

export const resources = {
  create: (slug: string, data: object) => send<RecordReply>("POST", `/resources/${slug}/records`, data),
  update: (slug: string, key: string | number, data: object) =>
    send<RecordReply>("PATCH", `/resources/${slug}/records/${encodeURIComponent(String(key))}`, data),
  remove: (slug: string, key: string | number) =>
    send<unknown>("DELETE", `/resources/${slug}/records/${encodeURIComponent(String(key))}`),
  get: async (slug: string, key: string | number) => {
    const response = await fetch(`${base()}/api/resources/${slug}/records/${encodeURIComponent(String(key))}`, {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    });
    const json: unknown = await response.json().catch(() => null);
    if (!response.ok) throw new ResourceError(response.status, messageOf(response.status, json), json);
    return json as RecordReply;
  },
  /** Runs action `name` on `keys`; a key the policy refuses is an error. */
  action: async (slug: string, name: string, keys: Array<string | number>) => {
    const out = await send<{ denied?: unknown[]; missing?: unknown[] }>(
      "POST",
      `/resources/${slug}/actions/${name}`,
      { keys: keys.map(String) },
    );
    if (out?.denied?.length) throw new ResourceError(403, "Not allowed", out);
    return out;
  },
};
