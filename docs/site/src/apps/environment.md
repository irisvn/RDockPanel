# Environment and secrets

A service's environment variables are edited as `.env` text on its **Environment** tab. Their values are encrypted in the panel and shown only to people allowed to read them.

## Edit the environment

1. Open the service's **Environment** tab.
2. Write one `NAME=value` per line. A line starting with `#` is a comment. Quoted and multi-line values work as in a `.env` file.
3. Press **Save**.
4. Press **Deploy**: changes apply on the next deployment.

```sh
# Database
DATABASE_URL=postgres://app:secret@shop_db:5432/app
APP_ENV=production
```

![Environment tab](/shots/service-environment.png)

## Who can read the values

- Values are encrypted with the panel's key (`APP_KEY`) and never written to the audit log or to deployment logs.
- Reading them in clear on the Environment tab needs the `services.secrets` permission (project developers and admins have it). **Every read is recorded in the audit log.**
- Without it, the tab shows only the variable names. You can still write the environment, but **saving replaces every variable**: type them all again.
- An API token reads values only with its "Read environment values" scope.

## Placeholders

These are replaced in the values at each deployment:

| Placeholder | Becomes |
|---|---|
| `$(PROJECT_NAME)` | the project's name |
| `$(SERVICE_NAME)` | the service's name |
| `$(PRIMARY_DOMAIN)` | the service's first domain (else its generated one) |
| `$(EASYPANEL_DOMAIN)` | the service's generated domain |
| `$(EASYPANEL_HOST)` | the apps' domain set at install |
| `$(EASYPANEL_HOST_IP)` | the server's public address |

An unknown or unset placeholder is left as written. Templates use them; you can too.

## Rollback

Each deployment keeps the environment it applied (encrypted). **Roll back to this** brings back the environment of then, not today's (see [Deploy](/apps/deploy#deployment-history-and-rollback)).

::: tip Good to know
- The whole environment holds at most 32 KB; more is refused with a clear message.
- Docker itself holds the values in clear in the running service and its containers: anyone with a terminal in the container, or root on the server, can read them.
- A database's credentials live in its environment; changing them there does not change a database already created (see [Databases](/apps/databases)).
- In a Compose service, the environment is the Compose project's `.env`, read with `${VAR}` (see [Compose](/apps/compose)).
- Registry passwords (Source tab) are encrypted too, and never shown.
:::
