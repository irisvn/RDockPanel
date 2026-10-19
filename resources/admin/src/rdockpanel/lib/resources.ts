// The panel's resource API from React pages, through the SDK's
// `props.resources` (RustCMS 0.6): records and actions go through the
// resources' policies, validation, audit and encrypted fields; a write whose
// answer is lost pauses that resource (the panel's banner offers a safe
// retry) and nothing is sent for another session.
import { fieldErrors, type Resources } from "@rustcms/admin-sdk";

let bound: Resources | null = null;

/** Binds the writes to the view's resource API (each page calls it with its props). */
export function bindResources(api: Resources) {
  bound = api;
}

function api(): Resources {
  if (!bound) throw new ResourceError(0, "The page is not ready", null);
  return bound;
}

/** An error answer: its status, stable code, and field errors (422). */
export class ResourceError extends Error {
  readonly status: number;
  readonly body: unknown;
  readonly fieldErrors: Record<string, string[]>;
  constructor(status: number, message: string, body: unknown, fields: Record<string, string[]> = {}) {
    super(message);
    this.status = status;
    this.body = body;
    this.fieldErrors = fields;
  }
  get code(): string | undefined {
    return (this.body as { code?: string } | null)?.code;
  }
}

/** The SDK's error as a [`ResourceError`] (same status, message and field errors). */
async function call<T>(run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (err) {
    if (err instanceof ResourceError) throw err;
    const e = err as { status?: number; body?: unknown; message?: string };
    const fields = fieldErrors(err);
    // The fields' messages too, for callers that show only the message.
    const summary = Object.entries(fields).map(([field, list]) => `${field}: ${list.join(", ")}`).join("; ");
    throw new ResourceError(e.status ?? 0, summary || (e.message ?? String(err)), e.body ?? null, fields);
  }
}

export const resources = {
  create: (slug: string, data: Record<string, unknown>) => call(() => api().create(slug, data)),
  update: (slug: string, key: string | number, data: Record<string, unknown>) =>
    call(() => api().update(slug, String(key), data)),
  remove: (slug: string, key: string | number) => call(() => api().delete(slug, String(key))),
  get: (slug: string, key: string | number) => call(() => api().get(slug, String(key))),
  /** Runs action `name` on `keys`; a key the policy refuses is an error. */
  action: async (slug: string, name: string, keys: Array<string | number>) => {
    const out = await call(() =>
      api().action<{ denied?: unknown[]; missing?: unknown[] }>(slug, name, keys.map(String)),
    );
    if (out?.denied?.length) throw new ResourceError(403, "Not allowed", out);
    return out;
  },
};
