# API

Deploy, restart and check your services from scripts and CI with the panel's public API,
`/api/v1`, and a token.

The full list of routes is in
[docs/api.md](https://github.com/irisvn/RDockPanel/blob/main/docs/api.md).

## Make a token

1. Open **Settings → API tokens** and press **New token**. Each user has their own tokens.
2. Give it a **Name** (say `CI`), tick its **Scopes**, and choose **Expires in (days)** (at most
   365).
3. Press **Create** and copy the token: **it is not shown again**.

A token does what its user may (their roles, and their roles in projects) and **never more than
its scopes**: both must allow. To stop a token, use **Revoke…** in its row.

| Scope | Allows |
|---|---|
| `read` | see projects, services, deployments and domains |
| `deploy` | deploy and restart services |
| `secrets` | read environment values (each read is audited) |
| `write` | change and add services and domains (for routes to come) |
| `logs` | read services' logs (for routes to come) |

## Call the API

Send the token in the `Authorization` header:

```sh
export RDP_TOKEN=rcms_…
curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/me
```

A service is named by its id or by `project/service`, URL-encoded: `shop/web` is `shop%2Fweb`.

| Method | Path | Scope | What it does |
|---|---|---|---|
| GET | `/me` | any | the token's user and scopes |
| GET | `/projects` | read | the projects you see, with their services |
| GET | `/services/{id}` | read | image, status, replicas, domains, latest deployment |
| POST | `/services/{id}/deploy` | deploy | queues a deployment: 202 `{"deployment": …}` |
| POST | `/services/{id}/restart` | deploy | queues a restart: 202 |
| GET | `/deployments/{id}` | read | a deployment's status, error and log |
| GET | `/services/{id}/env` | secrets | the environment, `{"env": {NAME: value}}` |
| GET | `/services/{id}/domains` | read | the service's domains |

## Examples

List your projects:

```sh
curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/projects
```

Deploy `shop/web`, then follow the deployment by its id:

```sh
curl -X POST -H "Authorization: Bearer $RDP_TOKEN" \
  https://panel.example.com/api/v1/services/shop%2Fweb/deploy

curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/deployments/42
```

## Deploy without a token

Two URLs deploy one service without any token of a user:

- **Deployment trigger** (the service's **Source** tab): `POST` to the URL deploys the service.
  It is shown once; make a new one if it leaks (the old one stops working). Anyone with the URL
  can deploy the service, not change it.

  ```sh
  curl -X POST "https://panel.example.com/api/v1/deploy/12?token=…"
  ```

- **Auto deploy (GitHub)** (the same tab, for a Git source): a signed GitHub webhook that deploys on
  each push to the service's branch.

## Answers and limits

- Errors are JSON `{"code", "message"}`: **401** without a valid token, **403** outside its scopes
  or its user's projects, **404** for what does not exist or is in a project you are not a member
  of (the same answer, on purpose), **409** while another deployment of the service is under way.
- Requests are limited per user and route: 120 a minute, deploy and restart 10. Beyond: **429**.
- A Compose service, or one with added capabilities, sysctls or groups, cannot be deployed through
  the API: deploy it in the panel.
- `GET /api/v1/ping` (no token) answers `{"pong": true}`: handy for a health check.

## The `rdp` command line

`rdp` wraps this API: `rdp login`, `rdp projects`, `rdp status shop/web`,
`rdp deploy shop/web --wait`, `rdp logs 42`… It is built with the panel from source; see
[docs/api.md](https://github.com/irisvn/RDockPanel/blob/main/docs/api.md#the-rdp-command-line).
