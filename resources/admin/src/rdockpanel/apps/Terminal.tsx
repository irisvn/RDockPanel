// A service's terminal (`/apps/services/{id}/exec/{container}`) and its
// logs (`/apps/services/{id}/logs/{tail}`), for whoever may use them in its
// project — no access to Docker's pages needed. Drawn by the shared viewers
// (lib/viewers.tsx), as the Docker pages are.
import type { PageProps } from "@rustcms/admin-sdk";
import { Dialog } from "@rustcms/admin-sdk/ui";
import { LogViewer, TerminalView } from "../lib/viewers";

export function ServiceLogs(props: PageProps<unknown> & { serviceId: number; onClose: () => void }) {
  return (
    <Dialog open onClose={props.onClose} title={props.t("Logs")} className="rdp2-wide">
      <LogViewer {...props} url={(n) => `/apps/services/${props.serviceId}/logs/${n}`} />
    </Dialog>
  );
}

export function ServiceTerminal(props: PageProps<unknown> & { serviceId: number; container: string; onClose: () => void }) {
  return (
    <Dialog open onClose={props.onClose} title={props.t("Shell")} className="rdp2-wide">
      <TerminalView {...props} path={`/apps/services/${props.serviceId}/exec/${encodeURIComponent(props.container)}`} />
    </Dialog>
  );
}
