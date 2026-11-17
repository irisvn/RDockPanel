---
layout: home

hero:
  name: RDockPanel
  text: A free, self-hosted panel for Docker Swarm
  tagline: Deploy apps, domains, HTTPS, databases and backups on your own server, from one page. Every feature is free.
  actions:
    - theme: brand
      text: Get started
      link: /guide/introduction
    - theme: alt
      text: Install
      link: /guide/install
    - theme: alt
      text: GitHub
      link: https://github.com/irisvn/RDockPanel

features:
  - title: Apps on Docker Swarm
    details: Projects and services with rolling deploys, rollback and live logs. From an image, a Git repository, a Dockerfile or a Compose file.
    link: /apps/projects-services
  - title: Domains and HTTPS
    details: Domains through Traefik with Let's Encrypt, your own certificates or Cloudflare Tunnel. Applied when saved.
    link: /apps/domains
  - title: Databases and backups
    details: PostgreSQL, MySQL, MariaDB, MongoDB and Redis in one click, with scheduled backups to a folder or an S3 bucket.
    link: /apps/databases
  - title: 800+ app templates
    details: Search a catalog of ready-made apps, fill in a short form and deploy them with their databases.
    link: /apps/sources-templates
  - title: Bring in what already runs
    details: Containers started with docker run or docker compose become services, with a plan, a copy of their data and an undo.
    link: /apps/import
  - title: Server, Docker and firewall
    details: Live CPU, memory, disk and network; containers, images, volumes and networks; a default-deny firewall.
    link: /server/docker
---

## One page for your server

![Dashboard](/shots/dashboard.png)

The Dashboard shows the server's live figures and every project with its services. From there you
add a service, open a template or bring in containers that already run.

## Why RDockPanel

RDockPanel is one Rust program with a web interface that runs on your own server. It turns Docker
Swarm into something you can use from a browser: you pick an image or a repository, give it a
domain, press Deploy, and the panel takes care of the Swarm service, the Traefik route and the
HTTPS certificate. Nothing is locked behind a paid tier, there is no limit on projects, and your
apps and data stay on your server. It speaks English and Vietnamese, in light and dark.

::: warning Early development
RDockPanel is still young. Try it on a test server first, and keep backups.
:::
