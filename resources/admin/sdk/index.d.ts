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
