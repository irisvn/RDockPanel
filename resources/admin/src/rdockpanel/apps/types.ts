// What `/admin/api/x/apps/...` answers (apps_controller.rs).

export type ServiceCard = {
  id: number;
  name: string;
  kind: string;
  image: string;
  status: string;
  desired_state: string;
  busy: boolean;
  domain: string | null;
};

export type Project = {
  id: number;
  name: string;
  description: string | null;
  services: ServiceCard[];
  /** The caller's role in it (none: they see it through a role of theirs). */
  role: string | null;
  can: { create_service: boolean; deploy: boolean; update: boolean; delete: boolean; members: boolean };
};
export type Overview = { projects: Project[]; services_visible: boolean; can: { create_project: boolean } };

export type Domain = { id: number; service_id: number; host: string; path: string; port: number; https: boolean; compose_service: string | null; middlewares: string | null; cert_resolver_id: number | null; certificate_id: number | null };
export type Port = { id: number; published: number; target: number; protocol: string; mode: string };
export type Mount = { id: number; kind: string; name: string | null; source: string | null; target: string; read_only: boolean };

export type Deployment = {
  id: number;
  trigger: string;
  /** Asked as "Force rebuild" (no build cache, new tasks). */
  rebuild?: boolean;
  status: string;
  error: string | null;
  image: string | null;
  published: boolean;
  finished: boolean;
  created_at?: string;
  finished_at?: string | null;
};

export type Abilities = { view: boolean; create: boolean; delete: boolean };

export type ServiceDetail = {
  service: {
    id: number;
    name: string;
    kind: string;
    image: string;
    command: string | null;
    replicas: number;
    cpu_limit_millis: number | null;
    mem_limit_mb: number | null;
    cpu_reservation_millis: number | null;
    mem_reservation_mb: number | null;
    zero_downtime: boolean;
    healthcheck_cmd: string | null;
    healthcheck_interval_s: number | null;
    healthcheck_retries: number | null;
    registry_username: string | null;
    source: "image" | "git" | "dockerfile";
    git_url: string | null;
    git_branch: string | null;
    git_path: string | null;
    /** The GitHub token it is cloned with (Settings → Git). */
    git_provider_id: number | null;
    builder: "dockerfile";
    dockerfile_path: string | null;
    dockerfile: string | null;
    compose: string | null;
    notes: string | null;
    cap_add: string | null;
    cap_drop: string | null;
    sysctls: string | null;
    groups: string | null;
    maintenance: boolean;
    maintenance_title: string | null;
    maintenance_subtitle: string | null;
    /** Its rate limit: average requests per period by visitor, burst at once (average null: off). */
    rate_limit_average: number | null;
    rate_limit_period: "1s" | "1m" | "1h" | null;
    rate_limit_burst: number | null;
    /** Addresses not limited (IPs or networks, one a line). */
    rate_limit_exempt: string | null;
    webhook_set: boolean;
    compose_file: string | null;
    deploy_key_public: string | null;
    trigger_set: boolean;
    registry_password_set: boolean;
    status: string;
    desired_state: string;
    deployed_at: string | null;
    busy: boolean;
  };
  project: { id: number; name: string };
  swarm_name: string;
  internal_host: string;
  auto_domain: string | null;
  /** The certificate resolvers a domain may choose (the panel's administrators only; else none). */
  cert_resolvers: Array<{ id: number; name: string; kind: string }>;
  /** The uploaded certificates a domain may choose (the panel's administrators only). */
  certificates: Array<{ id: number; name: string; names: string[] }>;
  /** The installer's resolver for HTTPS (none: Traefik's own certificate). */
  default_resolver: string | null;
  /** A database's connection facts (password and URL with services.secrets). */
  database: {
    engine: string;
    host: string;
    port: number;
    user: string;
    database: string;
  } | null;
  pending: boolean;
  zero_downtime_possible: boolean;
  env_names: string[];
  env_readable: boolean;
  domains: Domain[];
  redirects: Array<{ id: number; regex: string; replacement: string; permanent: boolean; enabled: boolean }>;
  basic_auth: Array<{ id: number; username: string }>;
  scripts: Array<{ id: number; name: string; command: string }>;
  ports: Port[];
  mounts: Mount[];
  latest: Deployment | null;
  deployments: Deployment[];
  /** Its project's permissions the caller holds (a role's, or their role's there). */
  permissions: string[];
  can: {
    update: boolean;
    delete: boolean;
    deploy: boolean;
    destroy: boolean;
    secrets: boolean;
    deployments: boolean;
    /** The service's logs and terminal (its own permissions, or Docker's). */
    logs: boolean;
    exec: boolean;
    members: boolean;
    /** `system.manage`: wildcards, a domain's own certificate. */
    system: boolean;
    domains: Abilities & { update: boolean };
    ports: Abilities;
    mounts: Abilities;
  };
};
