# Your first app

Deploy a small web app from an image, give it a domain, open it and read its logs. It takes about
five minutes.

We use `ghcr.io/traefik/whoami`, a tiny web server that answers with details of each request on
port 80.

## 1. Sign in

Open the panel at `/admin` (see [Install](/guide/install#first-sign-in)) and sign in. You land on
the **Dashboard**: the server's live figures at the top, then your projects.

![Dashboard](/shots/dashboard.png)

## 2. Create a project

A project groups the services of one app (its web, its database…), and has its own network.

1. Press **New project**.
2. Type a name: lowercase letters, digits and dashes, for example `demo`.
3. Press **Create**. The project appears on the Dashboard.

## 3. Add a service

1. On the project's card, press **Add service**.
2. In **Create service in demo**:
   - **Type**: keep **App (Docker image)**;
   - **Service name**: `web`;
   - **Docker image**: `ghcr.io/traefik/whoami`.
3. Press **Create**. The service's page opens.

![New service](/shots/new-service.png)

Before you create it, the dialog estimates the memory the service needs and warns you when the
server is short (see [Troubleshooting](/reference/troubleshooting#not-enough-memory)).

## 4. Deploy it

The new service is saved but not running yet: its Overview says "Not deployed yet: press Deploy."

1. Press **Deploy** in the service's header.
2. The deployment is queued, then runs: the panel pulls the image, pins it to its digest and
   creates the Swarm service. Its live log is on the **Deployments** tab.
3. When it ends, the service's status turns to running.

![Service overview](/shots/service-overview.png)

## 5. Add a domain

1. Point a DNS record (A) for your name, say `whoami.example.com`, to the server.
2. Open the **Domains** tab and press **Add domain**.
3. Fill in:
   - **Host**: `whoami.example.com`;
   - **Path**: `/` (the default);
   - **Container port**: `80` (the port the app listens on inside its container);
   - **HTTPS**: on. HTTP then redirects to HTTPS, and the certificate comes from Let's Encrypt.
4. Press **Add**. A domain applies when saved: no new deployment is needed.

![Add domain](/shots/add-domain.png)

If the panel was installed with `--apps-domain`, **Add generated domain** gives the service a
name such as `demo-web.apps.example.com` without any DNS work (the wildcard record does it).

::: tip On a Mac or without a public address
Use a name such as `whoami.localhost`, reached on the machine itself, with HTTPS off. For real
domains, see [Cloudflare Tunnel](/apps/tunnel).
:::

## 6. Open it

Press **Open** in the service's header. whoami answers with your request's details. The first
HTTPS request can take a few seconds while the certificate is issued.

## 7. Read its logs

On the **Overview** tab, the **Monitor** section shows the service's CPU and memory and has
**Logs** (the live logs of every container of the service) and **Shell** (a terminal inside it).
Reload your site and watch the new lines arrive.

## What next

- Change the image or the environment, then press **Deploy** again: see
  [Deploy and roll back](/apps/deploy) and [Environment and secrets](/apps/environment).
- Add a database in the same project: [Databases and backups](/apps/databases).
- Start from one of 800+ apps instead: [Sources and templates](/apps/sources-templates).
- To remove the demo: on the **Advanced** tab, under **Danger zone**, press **Remove from Docker**
  (volumes are kept), then **Delete service**; then delete the empty project from the Dashboard.
