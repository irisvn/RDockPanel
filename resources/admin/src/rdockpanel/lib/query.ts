// The SDK's `useAdminQuery`, for the panel's pages. The panel remounts a
// view on its sub-paths (a tab, an action going back to the overview): a
// read the old view had on its way is refused ("your session changed") and
// that answer lands in the cache the new view reads, a whole-page error
// until the next refresh. It is no answer for the new view: asked again.
import { useEffect } from "react";
import { useAdminQuery as useSdkQuery } from "@rustcms/admin-sdk";

/** The error of a call made, or answered, for a view that is gone. */
export function sessionChanged(error: unknown): boolean {
  return (error as { code?: unknown } | null)?.code === "session_changed";
}

export function useAdminQuery<T>(...args: Parameters<typeof useSdkQuery<T>>): ReturnType<typeof useSdkQuery<T>> {
  const query = useSdkQuery<T>(...args);
  const stale = sessionChanged(query.error);
  const { refetch, errorUpdatedAt } = query;
  // Once for each such answer: a live view's read gives the real one (a
  // sign-out is then a 401, as for any read).
  useEffect(() => {
    // Shared with another view's recovery of the same read (never cancels it).
    if (stale) void refetch({ cancelRefetch: false });
  }, [stale, refetch, errorUpdatedAt]);
  if (!stale) return query;
  const waiting = query.data === undefined;
  return {
    ...query,
    error: null,
    isError: false,
    isLoadingError: false,
    isRefetchError: false,
    // Asked again: fetching until the new answer comes.
    isFetching: true,
    fetchStatus: "fetching",
    isLoading: waiting,
    isInitialLoading: waiting,
    isPending: waiting,
    isSuccess: !waiting,
    status: waiting ? "pending" : "success",
  } as ReturnType<typeof useSdkQuery<T>>;
}
