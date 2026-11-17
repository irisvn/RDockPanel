// RDockPanel's user guide, in English (/) and Vietnamese (/vi/).
//   DOCS_BASE  `/RDockPanel/` on GitHub Pages; `/` by default
import { readFileSync } from 'node:fs'
import { defineConfig, type DefaultTheme } from 'vitepress'

const env = process.env
const cargo = readFileSync(new URL('../../../Cargo.toml', import.meta.url), 'utf8')
const version = /^version = "([^"]+)"/m.exec(cargo)?.[1] ?? 'unknown'
const repo = 'https://github.com/irisvn/RDockPanel'

type Words = Record<string, string>
const en: Words = {
  start: 'Getting started', intro: 'Introduction', install: 'Install on Linux', mac: 'Install on macOS',
  first: 'Your first app', update: 'Update and back up the panel',
  apps: 'Apps', projects: 'Projects and services', sources: 'Sources and templates', deploy: 'Deploy and roll back',
  env: 'Environment and secrets', domains: 'Domains and HTTPS', tunnel: 'Cloudflare Tunnel',
  access: 'Redirects, security, maintenance', storage: 'Storage and volumes', databases: 'Databases and backups',
  logs: 'Logs, terminal, scripts', compose: 'Docker Compose', import: 'Import existing containers', nodes: 'Where a service runs',
  server: 'Server', docker: 'Docker', swarmNodes: 'Nodes', firewall: 'Firewall', settings: 'Settings', users: 'Users and roles',
  ref: 'Reference', api: 'API', security: 'Security', trouble: 'Troubleshooting',
}
const vi: Words = {
  start: 'Bắt đầu', intro: 'Giới thiệu', install: 'Cài trên Linux', mac: 'Cài trên macOS',
  first: 'App đầu tiên', update: 'Cập nhật và sao lưu panel',
  apps: 'Ứng dụng', projects: 'Project và service', sources: 'Nguồn và template', deploy: 'Deploy và rollback',
  env: 'Biến môi trường và secret', domains: 'Domain và HTTPS', tunnel: 'Cloudflare Tunnel',
  access: 'Chuyển hướng, bảo mật, bảo trì', storage: 'Lưu trữ và volume', databases: 'Database và backup',
  logs: 'Log, terminal, script', compose: 'Docker Compose', import: 'Import container có sẵn', nodes: 'Chọn node chạy',
  server: 'Server', docker: 'Docker', swarmNodes: 'Node', firewall: 'Firewall', settings: 'Cài đặt', users: 'Người dùng và vai trò',
  ref: 'Tham khảo', api: 'API', security: 'Bảo mật', trouble: 'Xử lý sự cố',
}

function sidebar(w: Words, p: string): DefaultTheme.SidebarItem[] {
  return [
    { text: w.start, items: [
      { text: w.intro, link: `${p}/guide/introduction` },
      { text: w.install, link: `${p}/guide/install` },
      { text: w.mac, link: `${p}/guide/install-macos` },
      { text: w.first, link: `${p}/guide/first-app` },
      { text: w.update, link: `${p}/guide/update` },
    ] },
    { text: w.apps, items: [
      { text: w.projects, link: `${p}/apps/projects-services` },
      { text: w.sources, link: `${p}/apps/sources-templates` },
      { text: w.deploy, link: `${p}/apps/deploy` },
      { text: w.env, link: `${p}/apps/environment` },
      { text: w.domains, link: `${p}/apps/domains` },
      { text: w.tunnel, link: `${p}/apps/tunnel` },
      { text: w.access, link: `${p}/apps/access` },
      { text: w.storage, link: `${p}/apps/storage` },
      { text: w.databases, link: `${p}/apps/databases` },
      { text: w.logs, link: `${p}/apps/logs-terminal-scripts` },
      { text: w.compose, link: `${p}/apps/compose` },
      { text: w.import, link: `${p}/apps/import` },
      { text: w.nodes, link: `${p}/apps/nodes` },
    ] },
    { text: w.server, items: [
      { text: w.docker, link: `${p}/server/docker` },
      { text: w.swarmNodes, link: `${p}/server/nodes` },
      { text: w.firewall, link: `${p}/server/firewall` },
      { text: w.settings, link: `${p}/server/settings` },
      { text: w.users, link: `${p}/server/users` },
    ] },
    { text: w.ref, items: [
      { text: w.api, link: `${p}/reference/api` },
      { text: w.security, link: `${p}/reference/security` },
      { text: w.trouble, link: `${p}/reference/troubleshooting` },
    ] },
  ]
}

export default defineConfig({
  title: 'RDockPanel',
  description: 'A free, self-hosted Docker Swarm panel: deploy apps, domains, HTTPS, databases and backups from one page.',
  srcDir: 'src',
  base: env.DOCS_BASE || '/',
  cleanUrls: false,
  lastUpdated: true,
  ignoreDeadLinks: 'localhostLinks',
  markdown: { theme: { light: 'github-light', dark: 'github-dark' } },
  locales: {
    root: {
      label: 'English',
      lang: 'en',
      themeConfig: {
        nav: [
          { text: 'Guide', link: '/guide/introduction' },
          { text: `v${version}`, items: [{ text: 'Changelog', link: `${repo}/blob/main/CHANGELOG.md` }, { text: 'Releases', link: `${repo}/releases` }] },
        ],
        sidebar: sidebar(en, ''),
        editLink: { pattern: `${repo}/edit/main/docs/site/src/:path`, text: 'Edit this page on GitHub' },
      },
    },
    vi: {
      label: 'Tiếng Việt',
      lang: 'vi',
      link: '/vi/',
      description: 'Panel Docker Swarm miễn phí, tự host: deploy app, domain, HTTPS, database và backup từ một trang.',
      themeConfig: {
        nav: [
          { text: 'Hướng dẫn', link: '/vi/guide/introduction' },
          { text: `v${version}`, items: [{ text: 'Nhật ký thay đổi', link: `${repo}/blob/main/CHANGELOG.md` }, { text: 'Bản phát hành', link: `${repo}/releases` }] },
        ],
        sidebar: sidebar(vi, '/vi'),
        editLink: { pattern: `${repo}/edit/main/docs/site/src/:path`, text: 'Sửa trang này trên GitHub' },
        outline: { label: 'Trên trang này', level: [2, 3] },
        docFooter: { prev: 'Trang trước', next: 'Trang sau' },
        lastUpdated: { text: 'Cập nhật lần cuối' },
        returnToTopLabel: 'Về đầu trang',
        sidebarMenuLabel: 'Menu',
        darkModeSwitchLabel: 'Giao diện',
        langMenuLabel: 'Ngôn ngữ',
      },
    },
  },
  themeConfig: {
    siteTitle: 'RDockPanel',
    outline: { level: [2, 3] },
    search: {
      provider: 'local',
      options: {
        locales: {
          vi: { translations: { button: { buttonText: 'Tìm kiếm', buttonAriaLabel: 'Tìm kiếm' }, modal: { noResultsText: 'Không có kết quả cho', resetButtonTitle: 'Xoá', footer: { selectText: 'chọn', navigateText: 'di chuyển', closeText: 'đóng' } } } },
        },
      },
    },
    socialLinks: [{ icon: 'github', link: repo }],
    footer: { message: `RDockPanel ${version} · MIT OR Apache-2.0` },
  },
})
