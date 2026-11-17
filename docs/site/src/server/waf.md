# Web application firewall (WAF)

**Settings → Security** turns on a firewall that reads every request to your services' domains and
stops known attacks before they reach your app.

## How it works

The panel runs [CrowdSec](https://www.crowdsec.net/)'s WAF (its *AppSec* engine) as a service of its
own, `rdp-crowdsec`, and gives Traefik a plugin that asks it about each request:

```text
 visitor ──► Traefik ──► WAF plugin ──ask──► rdp-crowdsec (rules)
                            │  ◄── allow / block ──┘
                            ├── allowed ──► your app
                            └── blocked ──► 403 Forbidden
```

- **Only your services' domains** go through it. The panel's own address and HTTPS certificate
  checks never do, so you can always reach the panel to change a setting.
- The request's body is read first (up to 10 MB is checked; larger uploads pass whole, the rest
  unchecked), then the request goes on unchanged.
- The visitor's real address is used, also behind Cloudflare and the Cloudflare Tunnel.
- **If the WAF fails** (CrowdSec stopped, slow, or its plugin not loaded), requests go through
  **unchecked**: a site is never taken down by the WAF. The Settings page says so.

![Settings → Security](/shots/settings-security.png)

## Modes and levels

| Mode | What happens |
|---|---|
| **Off** | Nothing is checked. CrowdSec is stopped. |
| **Detect only** | Attacks are **recorded, never stopped**. Start here for a week and read the alerts. |
| **Block** | Attacks get **403 Forbidden** before reaching the app. |

| Level | Stops |
|---|---|
| **Basic** | Known exploits of popular software (virtual patches for published CVEs) and probes of secret files (`/.env`, backups…). It does **not** stop generic SQL injection or XSS. |
| **Strict** | Basic, plus the **OWASP Core Rule Set**: SQL injection, XSS, path traversal and more. It can block legitimate requests: try it in Detect first. |

## Turn it on

1. Open **Settings → Security**.
2. Choose **Detect only** and a level, press **Save**. The state goes from *Starting* to **On**
   within about a minute (the first start downloads CrowdSec's rules).
3. Look at **Latest alerts** for a few days. A legitimate request in the list (a false positive)?
   Add its address to **Trusted addresses**, or turn the WAF off for that service.
4. Switch to **Block**.

The first start, and turning it off and on, restart Traefik once: every domain pauses for a few
seconds.

## Per service

A service follows the panel's setting. On its **Security** tab you can set it to **On** or **Off**:

- **Off** removes that service's protection: only the panel's administrators may.
- Anyone who may change the service may set it back to the panel's setting or **On**.

It applies when saved, without redeploying.

![Service → Security](/shots/service-security.png)

## States

| State | Meaning | What to do |
|---|---|---|
| **On** | The panel checked a moment ago: a test attack was blocked. | Nothing. |
| **Starting** | CrowdSec or Traefik is starting. | Wait a minute. |
| **Unavailable** | Requests go through unchecked; the reason is shown. | Press **Repair**. Still unavailable? See the reason. |
| **Off** | The WAF is off. | — |

Every minute the panel sends a normal request and a test attack through a hidden route of its own.
If Traefik could not load the plugin, the panel switches the sites to *unchecked* and starts Traefik
again to load it (at most once every ten minutes). These test requests never appear in the alerts.

## Who sees what

| User | Sees |
|---|---|
| Panel administrator (`system.manage`) | Settings → Security: mode, level, addresses, alerts; turns a service's WAF off. |
| May change a service | Its WAF setting; can set it back on, not off. |
| Others | Nothing of it. |

Alerts (the latest 100) show the time, the visitor's address, the domain and path, and the rule.
**Request bodies are never kept** (a body over 10 MB waits in a temporary file while it is checked).

## Trusted addresses

Addresses listed there are **never checked at all** (an office, a monitoring service). Keep the list
short: a trusted address is a way around the WAF.

## Sharing with CrowdSec's community

Off by default: nothing is sent to CrowdSec. Turned on, CrowdSec sends the attacker's address, the rule
and the time to its community (never your requests), and receives its shared knowledge.

## Behind Cloudflare

- **Proxied domains** (orange cloud): Cloudflare's own WAF checks first; the panel's WAF is a second
  layer and sees the visitor's real address.
- The risk is someone **reaching your server's IP directly**, bypassing Cloudflare. With the
  [Cloudflare Tunnel](/apps/tunnel) no port needs to be open: close 80 and 443 in your provider's
  firewall when every domain uses the Tunnel.
- **DNS-only domains** (grey cloud): the panel's WAF is their only layer.

## Limits

- Ports published by a service (Advanced → Ports) do not go through the WAF: only domains do.
- The rules are CrowdSec's; a brand-new attack may pass until they are updated.
- When Traefik is started again by something else than the panel (an update of the server, the
  installer) and cannot load the plugin, the sites with the WAF answer 404 until the panel's next
  check (a minute at most); the panel then serves them unchecked and starts Traefik again to load it.
- `rdp-waf-canary.invalid` (and `*.invalid`) cannot be a service's domain: the panel's hidden test
  route uses it.
