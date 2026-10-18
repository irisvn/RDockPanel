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

export type Project = { id: number; name: string; description: string | null; services: ServiceCard[] };
export type Overview = { projects: Project[]; services_visible: boolean };

export type Domain = { id: number; service_id: number; host: string; path: string; port: number; https: boolean };
export type Port = { id: number; published: number; target: number; protocol: string; mode: string };
export type Mount = { id: number; kind: string; name: string | null; source: string | null; target: string; read_only: boolean };

export type Deployment = {
  id: number;
  trigger: string;
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
  ports: Port[];
  mounts: Mount[];
  latest: Deployment | null;
  deployments: Deployment[];
  can: {
    update: boolean;
    delete: boolean;
    deploy: boolean;
    destroy: boolean;
    secrets: boolean;
    deployments: boolean;
    domains: Abilities;
    ports: Abilities;
    mounts: Abilities;
  };
};
