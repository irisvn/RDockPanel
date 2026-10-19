// @rustcms/admin-sdk: host API v2 types and hooks (ADR 0032).
import type * as React from "react";
import type { UseMutationOptions, UseMutationResult, UseQueryOptions, UseQueryResult } from "@tanstack/react-query";

export type QueryValue = string | number | boolean | Array<string | number | boolean>;
export type Query = Record<string, QueryValue | undefined>;

/** The panel's API under `{admin}/api/x` (your `AdminRoutes`), bound to the view. */
export interface AdminApi {
  get<T>(path: string, options?: { query?: Query }): Promise<T>;
  post<T>(path: string, body?: unknown, options?: { query?: Query }): Promise<T>;
  put<T>(path: string, body?: unknown, options?: { query?: Query }): Promise<T>;
  patch<T>(path: string, body?: unknown, options?: { query?: Query }): Promise<T>;
  delete<T>(path: string, options?: { query?: Query }): Promise<T>;
}

export type EventsState =
  | { state: "open" }
  | { state: "reconnecting"; status?: number; reason?: string }
  | { state: "closed"; status?: number; reason?: string };
export interface EventsHandlers {
  onEvent: (name: string, data: unknown) => void;
  onState?: (state: EventsState) => void;
}
export type SocketState =
  | { state: "open" }
  | { state: "reconnecting"; code?: number; reason?: string }
  | { state: "closed"; code?: number; reason?: string };
export interface SocketHandlers {
  onMessage: (data: string | ArrayBuffer) => void;
  onOpen?: () => void;
  onState?: (state: SocketState) => void;
}
export interface Realtime {
  events(path: string, handlers: EventsHandlers): { close(): void };
  socket(path: string, handlers: SocketHandlers, options?: { reconnect?: boolean }): {
    send(data: string | ArrayBuffer | ArrayBufferView): boolean;
    close(): void;
  };
}

/**
 * The resource API (`ui` 2), as the panel's own pages call it: policy,
 * `scope()`, validation, audit and encrypted fields apply. Values use the
 * panel's wire format (i64 and decimals as strings). Updates send the
 * changed fields; the last write wins (encrypted maps: `409 secret_stale`).
 */
export interface Resources {
  /** Marks `slug` as used by the view (its paused-write banner shows there; the hooks do it). */
  track(slug: string): void;
  /** One page of records; `query` as the panel's lists (`page`, `per_page`, `sort`, `search`, `filter[field][op]`). */
  list<T = Record<string, unknown>>(slug: string, query?: Query): Promise<RecordPage<T>>;
  get<T = Record<string, unknown>>(slug: string, key: string): Promise<Envelope<T>>;
  create<T = unknown>(slug: string, data: Record<string, unknown>): Promise<Envelope<T>>;
  update<T = unknown>(slug: string, key: string, data: Record<string, unknown>): Promise<Envelope<T>>;
  delete(slug: string, key: string): Promise<void>;
  restore<T = unknown>(slug: string, key: string): Promise<Envelope<T>>;
  forceDelete(slug: string, key: string): Promise<void>;
  action<T = unknown>(slug: string, name: string, keys: string[], data?: Record<string, unknown>): Promise<T>;
}

/** One page of records. */
export interface RecordPage<T = Record<string, unknown>> {
  data: Envelope<T>[];
  meta: { page: number; perPage: number; total: number; lastPage: number };
}

/** A record as the panel sends it (readable fields only). */
export interface Envelope<T = Record<string, unknown>> {
  key: string;
  title?: string;
  /** What the user may do with it (`update`, `delete`, …). */
  abilities: Record<string, boolean>;
  data: T;
  trashed?: boolean;
  /** Titles of related records, by field. */
  labels?: Record<string, string>;
}

/** Messages shown by the panel (`ui` 2): kept within the session, dropped when it changes. */
export interface Toasts {
  success(text: string): void;
  error(text: string): void;
}

/** The view's cache scope (issued by the panel per mount). */
export interface Scope {
  readonly namespace: string;
  readonly epoch: number;
}

interface Common {
  locale: string;
  /** `text` in the panel's language (`registerTranslations` catalogs). */
  t(text: string): string;
  scope: Scope;
  /** Whether the user holds this admin permission (UX only: the server checks). */
  can(permission: string): boolean;
}

export interface PageProps<D = unknown> extends Common {
  slug: string;
  /** The sub-path after the slug, canonical (`containers/web`), `""` for none. */
  path: string;
  search: URLSearchParams;
  data: Readonly<D>;
  navigate(path: string): void;
  reload(): void;
  api: AdminApi;
  realtime: Realtime;
  /** `ui` 2: given to scripts whose `Requires` declares `ui: 2` or more. */
  resources: Resources;
  toast: Toasts;
  /**
   * `ui` 2: the panel's heading and breadcrumb for the current sub-path
   * (`null`: no visible heading, the page draws its own). Another sub-path
   * shows the page's title again until it is set there too.
   */
  title(text: string | null): void;
}

export interface SlotProps<D = unknown, R = unknown> extends Common {
  slot: string;
  resource: string;
  recordKey: string;
  data: Readonly<D>;
  record: Readonly<R>;
  navigate(path: string): void;
  reload(): void;
  api: AdminApi;
  realtime: Realtime;
  resources: Resources;
  toast: Toasts;
}

export interface CellProps<V = unknown, R = unknown, P = unknown> extends Common {
  value: Readonly<V>;
  record: Readonly<R>;
  props: Readonly<P>;
}

type WithApi = Pick<PageProps, "api" | "scope" | "locale">;
type WithRealtime = Pick<PageProps, "realtime">;

export declare function registerPage<D>(name: string, component: React.ComponentType<PageProps<D>>): void;
export declare function registerSlot<D, R>(name: string, component: React.ComponentType<SlotProps<D, R>>): void;
export declare function registerCell<V, R, P>(name: string, component: React.ComponentType<CellProps<V, R, P>>): void;

export declare function queryPairs(query?: Query): Array<[string, string]>;
export declare function adminKey(props: WithApi, path: string, query?: Query): unknown[];

export declare function useAdminQuery<T>(
  props: WithApi,
  path: string,
  options?: { query?: Query } & Omit<UseQueryOptions<T>, "queryKey" | "queryFn">,
): UseQueryResult<T>;

export declare function useAdminMutation<T, B = unknown>(
  props: WithApi,
  path: string,
  method?: "post" | "put" | "patch" | "delete",
  options?: Omit<UseMutationOptions<T, Error, B>, "mutationFn">,
): UseMutationResult<T, Error, B>;

export declare function useEvents(props: WithRealtime, path: string, handlers: EventsHandlers): void;
export declare function useSocket(
  props: WithRealtime,
  path: string,
  handlers: SocketHandlers,
  options?: { reconnect?: boolean },
): { send(data: string | ArrayBuffer | ArrayBufferView): boolean };

/** The panel level this SDK is built for: declare `Requires { ui: 2, … }`. */
export declare const SDK_UI: 2;

/** Field errors of a refused write (422): `{ field: [messages] }`, `{}` otherwise. */
export declare function fieldErrors(error: unknown): Record<string, string[]>;

type WithResources = Pick<PageProps, "resources" | "scope" | "locale">;

export declare function resourceKey(props: WithResources, slug: string, ...rest: unknown[]): unknown[];

export declare function useResource<T = Record<string, unknown>>(
  props: WithResources,
  slug: string,
  options?: { query?: Query } & Omit<UseQueryOptions<RecordPage<T>>, "queryKey" | "queryFn">,
): UseQueryResult<RecordPage<T>>;

export declare function useRecord<T = Record<string, unknown>>(
  props: WithResources,
  slug: string,
  key: string,
  options?: Omit<UseQueryOptions<Envelope<T>>, "queryKey" | "queryFn">,
): UseQueryResult<Envelope<T>>;

export type ResourceOperation = "create" | "update" | "delete" | "restore" | "forceDelete" | "action";
export type ResourceVariables = {
  create: { data: Record<string, unknown> };
  update: { key: string; data: Record<string, unknown> };
  delete: { key: string };
  restore: { key: string };
  forceDelete: { key: string };
  action: { name: string; keys: string[]; data?: Record<string, unknown> };
};

export declare function useResourceMutation<O extends ResourceOperation, T = unknown>(
  props: WithResources,
  slug: string,
  op: O,
  options?: Omit<UseMutationOptions<T, Error, ResourceVariables[O]>, "mutationFn">,
): UseMutationResult<T, Error, ResourceVariables[O]>;

/** The panel's color scheme, re-rendering when it changes (canvas colors, charts). */
export declare function useColorScheme(): "light" | "dark";

