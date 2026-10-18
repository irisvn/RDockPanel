// @rustcms/admin-sdk: hooks and registration helpers for host API v2 (ADR
// 0032). Bundled into the extension; `react` and `@tanstack/react-query` stay
// external (the panel's own instances, mapped by the preset).
import { useCallback, useEffect, useMemo, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

function host() {
  const v2 = globalThis.rustcms && globalThis.rustcms.v2;
  if (!v2) throw new Error("rustcms: this script needs the panel's host API v2");
  return v2;
}

/** Registers a page component for `PageView::Script(name)`. */
export const registerPage = (name, component) => host().registerPage(name, component);
/** Registers a slot component. */
export const registerSlot = (name, component) => host().registerSlot(name, component);
/** Registers a table cell component (`TableColumn::cell(name)`). */
export const registerCell = (name, component) => host().registerCell(name, component);

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
    onSuccess: (data, variables, context) => {
      if (!alive.current) return;
      void client.invalidateQueries({ queryKey: ["rustcms.x", props.scope.namespace, props.scope.epoch] });
      onSuccess?.(data, variables, context);
    },
    onError: (error, variables, context) => {
      if (alive.current) onError?.(error, variables, context);
    },
    onSettled: (data, error, variables, context) => {
      if (alive.current) onSettled?.(data, error, variables, context);
    },
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
