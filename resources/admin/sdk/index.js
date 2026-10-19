// @rustcms/admin-sdk: hooks and registration helpers for host API v2 (ADR
// 0032). Bundled into the extension; `react` and `@tanstack/react-query` stay
// external (the panel's own instances, mapped by the preset).
import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

function host() {
  const v2 = globalThis.rustcms && globalThis.rustcms.v2;
  if (!v2) throw new Error("rustcms: this script needs the panel's host API v2");
  return v2;
}

/** The panel level this SDK is built for (`Requires.ui` must be at least this). */
export const SDK_UI = 2;

function registering(kind) {
  return (name, component) => {
    const v2 = host();
    // An older panel ignores what it does not know: refuse here, before it runs.
    const level = v2.versions && v2.versions.ui;
    if (typeof level !== "number" || level < SDK_UI) {
      throw new Error(`rustcms: ${kind} ${name} needs a newer panel (ui ${SDK_UI}, the panel has ${level})`);
    }
    return v2[`register${kind[0].toUpperCase()}${kind.slice(1)}`](name, component, { ui: SDK_UI });
  };
}

/** Registers a page component for `PageView::Script(name)`. */
export const registerPage = registering("page");
/** Registers a slot component. */
export const registerSlot = registering("slot");
/** Registers a table cell component (`TableColumn::cell(name)`). */
export const registerCell = registering("cell");

/** `query` as sorted `[name, value]` pairs (duplicates kept): the key part and what is sent. */
export function queryPairs(query) {
  const pairs = [];
  for (const name of Object.keys(query || {}).sort()) {
    const value = query[name];
    if (value === undefined) continue;
    for (const item of Array.isArray(value) ? value : [value]) pairs.push([name, String(item)]);
  }
  return pairs;
}

/** `path` with each segment decoded once and re-encoded, as the panel sends it. */
function canonicalPath(path) {
  try {
    return path
      .split("/")
      .map((segment) => encodeURIComponent(decodeURIComponent(segment)))
      .join("/");
  } catch {
    return path; // refused by `api` anyway
  }
}

/** The cache key of `path` + `query` in the view's scope. */
export function adminKey(props, path, query) {
  return ["rustcms.x", props.scope.namespace, props.scope.epoch, props.locale, canonicalPath(path), queryPairs(query)];
}

/** `GET {admin}/api/x{path}` with React Query, in the view's scope. */
export function useAdminQuery(props, path, options = {}) {
  const { query, ...rest } = options;
  return useQuery({
    queryKey: adminKey(props, path, query),
    queryFn: () => props.api.get(path, { query }),
    ...rest,
  });
}

/**
 * A write to `{admin}/api/x{path}` (`post`, `put`, `patch`, `delete`):
 * invalidates the view's queries on success; never retried; callbacks of a
 * stale view (another session, unmounted) are dropped.
 */
export function useAdminMutation(props, path, method = "post", options = {}) {
  const client = useQueryClient();
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const { onSuccess, onError, onSettled, ...rest } = options;
  return useMutation({
    mutationFn: (body) => (method === "delete" ? props.api.delete(path) : props.api[method](path, body)),
    retry: false,
    ...rest,
    // Every argument passes through, and a returned promise is awaited.
    onSuccess: (...args) => {
      if (!alive.current) return undefined;
      void client.invalidateQueries({ queryKey: ["rustcms.x", props.scope.namespace, props.scope.epoch] });
      return onSuccess?.(...args);
    },
    onError: (...args) => (alive.current ? onError?.(...args) : undefined),
    onSettled: (...args) => (alive.current ? onSettled?.(...args) : undefined),
  });
}

/** Keeps the latest handlers without reopening the stream. */
function useLatest(value) {
  const ref = useRef(value);
  ref.current = value;
  return ref;
}

/** Server-sent events of `AdminRoutes::events` at `path` while mounted. */
export function useEvents(props, path, handlers) {
  const latest = useLatest(handlers);
  const realtime = props.realtime;
  useEffect(() => {
    const handle = realtime.events(path, {
      onEvent: (name, data) => latest.current.onEvent?.(name, data),
      onState: (state) => latest.current.onState?.(state),
    });
    let closed = false;
    return () => {
      if (closed) return;
      closed = true;
      handle.close();
    };
  }, [realtime, path, latest]);
}

/** A WebSocket of `AdminRoutes::socket` at `path` while mounted; returns `send`. */
export function useSocket(props, path, handlers, options = {}) {
  const latest = useLatest(handlers);
  const handleRef = useRef(null);
  const realtime = props.realtime;
  const reconnect = options.reconnect === true;
  useEffect(() => {
    const handle = realtime.socket(
      path,
      {
        onOpen: () => latest.current.onOpen?.(),
        onMessage: (data) => latest.current.onMessage?.(data),
        onState: (state) => latest.current.onState?.(state),
      },
      { reconnect },
    );
    handleRef.current = handle;
    let closed = false;
    return () => {
      if (closed) return;
      closed = true;
      if (handleRef.current === handle) handleRef.current = null;
      handle.close();
    };
  }, [realtime, path, reconnect, latest]);
  const send = useCallback((data) => (handleRef.current ? handleRef.current.send(data) : false), []);
  return useMemo(() => ({ send }), [send]);
}

/** Field errors of a refused write (422): `{ field: [messages] }`, `{}` otherwise. */
export function fieldErrors(error) {
  return error && typeof error === "object" && error.fieldErrors && typeof error.fieldErrors === "object" ? error.fieldErrors : {};
}

/** The cache key of a resource read in the view's scope (refreshed after any write to `slug`). */
export function resourceKey(props, slug, ...rest) {
  return ["rustcms.r", slug, props.scope.namespace, props.scope.epoch, props.locale, ...rest];
}

/** Marks `slug` as used by the view (its paused-write banner shows there). */
function useTrack(props, slug) {
  const resources = props.resources;
  useEffect(() => {
    resources.track(slug);
  }, [resources, slug]);
}

/** A resource's records (`props.resources.list`), as `{ data, meta }`. */
export function useResource(props, slug, options = {}) {
  const { query, ...rest } = options;
  useTrack(props, slug);
  return useQuery({
    queryKey: resourceKey(props, slug, "list", queryPairs(query)),
    queryFn: () => props.resources.list(slug, query),
    ...rest,
  });
}

/** One record (`props.resources.get`), as an envelope. */
export function useRecord(props, slug, key, options = {}) {
  useTrack(props, slug);
  return useQuery({
    queryKey: resourceKey(props, slug, "one", key),
    queryFn: () => props.resources.get(slug, key),
    ...options,
  });
}

const OPERATIONS = {
  create: (r, slug, v) => r.create(slug, v.data),
  update: (r, slug, v) => r.update(slug, v.key, v.data),
  delete: (r, slug, v) => r.delete(slug, v.key),
  restore: (r, slug, v) => r.restore(slug, v.key),
  forceDelete: (r, slug, v) => r.forceDelete(slug, v.key),
  action: (r, slug, v) => r.action(slug, v.name, v.keys, v.data),
};

/**
 * A write to resource `slug`: `op` is `create` (`{ data }`), `update`
 * (`{ key, data }`), `delete` / `restore` / `forceDelete` (`{ key }`) or
 * `action` (`{ name, keys, data? }`). Never retried; the panel refreshes
 * every view of the resource; callbacks of a stale view are dropped.
 */
export function useResourceMutation(props, slug, op, options = {}) {
  const run = OPERATIONS[op];
  if (!run) throw new Error(`rustcms: unknown resource operation ${op}`);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useTrack(props, slug);
  const { onSuccess, onError, onSettled, ...rest } = options;
  return useMutation({
    mutationFn: (variables) => run(props.resources, slug, variables || {}),
    retry: false,
    ...rest,
    onSuccess: (...args) => (alive.current ? onSuccess?.(...args) : undefined),
    onError: (...args) => (alive.current ? onError?.(...args) : undefined),
    onSettled: (...args) => (alive.current ? onSettled?.(...args) : undefined),
  });
}

function subscribeScheme(listener) {
  const observer = new MutationObserver(listener);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-scheme"] });
  return () => observer.disconnect();
}

const readScheme = () => (document.documentElement.dataset.scheme === "dark" ? "dark" : "light");

/**
 * The panel's color scheme now, `"light"` or `"dark"`, re-rendering when the
 * user or the system changes it (e.g. to redraw a canvas chart). Styles need
 * nothing: the theme's CSS variables follow it.
 */
export function useColorScheme() {
  return useSyncExternalStore(subscribeScheme, readScheme, () => "light");
}

