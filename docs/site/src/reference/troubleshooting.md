# Troubleshooting

Common problems, why they happen, and what to do. Each section reads: symptom, cause, fix.

## The panel does not open after install {#panel-not-reachable}

**Symptom**: the browser cannot reach the panel's address.

**Cause**: the wrong address or port, a closed port at your provider, or DNS not pointing to the
server yet.

**Fix**:
- Without a domain the panel is at `http://<server-ip>:3000/admin`. With a domain, port 3000 is
  closed: use `https://panel.example.com/admin`.
- Open ports 80 and 443 (and 3000 without a domain) in your provider's firewall or security group.
- Check that the domain's A record points to the server, then run the installer again with the
  same options.

## A domain does not answer {#domain-not-reachable}

**Symptom**: the app's domain times out, or answers "404 page not found".

**Cause**: DNS does not point to this server, the service is not running, or the domain leads to
the wrong container port.

**Fix**:
1. Check the DNS record (A, or a wildcard for generated domains) points to the server.
2. Open the **Domains** page: the **State** column says **Live**, **Not running** (start or
   deploy the service), or **Waiting for deploy** (a Compose service's domain: deploy it).
3. Check the domain's **Container port** is the port the app listens on *inside* its container
   (80 by default in the dialog).
4. A domain change applies when saved, but only while the service runs.

## HTTPS: the certificate is not issued {#certificate}

**Symptom**: the browser warns about the certificate, or HTTPS fails while HTTP worked.

**Cause**: Let's Encrypt could not check the name. Over HTTP it must reach the server on port 80
under that name. A wildcard (`*.example.com`) needs a DNS resolver. A name behind Cloudflare is
checked at Cloudflare, not here.

**Fix**:
- Make sure DNS points here and port 80 is open, then wait a minute: the first certificate takes
  a few seconds to arrive.
- For a wildcard, add a resolver **over DNS** in **Settings → Certificates** (with a Cloudflare,
  DigitalOcean, Hetzner, Vultr or Linode token), or upload a wildcard certificate.
- Through [Cloudflare Tunnel](/apps/tunnel), Cloudflare gives visitors HTTPS: turn HTTPS off on the
  domain (the dialog offers **Turn HTTPS off**). Proxied by Cloudflare to this server, use a DNS
  resolver or an uploaded certificate.
- Read Traefik's live logs in **Settings → Traefik**: certificate errors are there.

## A deployment failed {#deployment-failed}

**Symptom**: a deployment ends in **Failed**, or the service keeps restarting.

**Cause**: the image could not be pulled (a typo, a private registry), the build failed, the app
crashed at start, or its health check failed.

**Fix**:
1. Open the **Deployments** tab and read that deployment's log: every step (pull, build, start) is
   there.
2. The **Overview** shows **Recent errors**: failed tasks with their exit code.
3. For a private image, set **Registry user** and **Registry password or token** on the
   **Source** tab.
4. If an update failed, Swarm has already gone back to the previous version. To deploy an older
   configuration on purpose, use **Roll back to this** on a deployment that worked.

If Deploy answers that a deployment is already under way: one deployment runs at a time per
service. Wait for it to end.

## "Is already published" or "is reserved" on a port {#port-in-use}

**Symptom**: saving a published port is refused.

**Cause**: another service already publishes that port, or it is one the panel keeps for itself
and Traefik (80, 443, 3000, 8000, 8080, 8443 by default).

**Fix**: choose another published port. For a website, prefer a [domain](/apps/domains): it needs
no published port at all.

## "Not enough memory" before a deploy {#not-enough-memory}

**Symptom**: creating or deploying a service shows **Not enough memory for this**, **Memory will be
low**, or **This service cannot run**.

**Cause**: the panel estimates what the service needs (a build takes about 1 GB while it runs) and
compares it with what the server has free.

**Fix**:
- **Memory will be low**: it can still be deployed; keep an eye on it.
- **Not enough memory for this**: services may be stopped for lack of memory. Free memory (stop
  unused services) or confirm with **I understand, deploy anyway**.
- **This service cannot run**: its memory reservations exceed the server; Swarm would leave it
  pending. Lower its **Memory reservation (MB)** on the **Advanced** tab, or add memory.

On a Mac, the memory is the Docker VM's: raise it in OrbStack's or Docker Desktop's settings.

## Nothing answers after a reboot {#after-reboot}

**Symptom**: after the server restarts, the panel and the apps are down, but SSH works.

**Cause**: with the [Firewall](/server/firewall) on, Docker starts only once the firewall's rules
are loaded. If the saved rules do not load, a fallback (SSH and the web only) loads instead and the
Firewall page says so; if neither loads, Docker does not start, so nothing is exposed unfiltered.

**Fix**: sign in over SSH and check `systemctl status docker`. To recover by hand when the
firewall's files are gone:

```sh
rm /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf && systemctl daemon-reload
systemctl start docker
```

On a Mac, the Mac must be awake and Docker running; Swarm starts everything again with Docker.

## Locked out by a firewall change {#firewall-lockout}

**Symptom**: after **Apply** on the Firewall page, you cannot reach the panel or SSH.

**Fix**: wait. Each change is undone by the server itself after 120 seconds unless you press
**Keep these rules**. Before pressing it, test from a **new** connection (a private window, a new SSH
session).

## An import is blocked or refused {#import-blocked}

**Symptom**: the import's plan marks a container as blocked, or the start is refused.

**Cause**: the plan lists what blocks: for example a privileged container, the host network, a
folder the panel never gives a service (`/`, the Docker socket, `/proc`…), a volume shared by two
containers, or a password on the command line it cannot move. A start is refused when the data and
a margin (10%, at least 2 GiB) do not fit on the disk.

**Fix**: read the reason next to each item, change the container or leave it out, free disk space,
then read it again. While an import runs, its services refuse deployments and backups: finish or
undo it first. See [Import existing containers](/apps/import).

## Still stuck?

Search or open an issue on [GitHub](https://github.com/irisvn/RDockPanel/issues), with the panel's
version (Settings → Version) and the deployment's or Traefik's log, without secrets.
