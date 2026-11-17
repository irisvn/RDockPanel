---
layout: home

hero:
  name: RDockPanel
  text: Panel Docker Swarm miễn phí, tự host
  tagline: Deploy app, domain, HTTPS, database và backup trên server của bạn, từ một trang. Mọi tính năng đều miễn phí.
  actions:
    - theme: brand
      text: Bắt đầu
      link: /vi/guide/introduction
    - theme: alt
      text: Cài đặt
      link: /vi/guide/install
    - theme: alt
      text: GitHub
      link: https://github.com/irisvn/RDockPanel

features:
  - title: App trên Docker Swarm
    details: Project và service với rolling deploy, rollback và log trực tiếp. Từ image, repo Git, Dockerfile hoặc file Compose.
    link: /vi/apps/projects-services
  - title: Domain và HTTPS
    details: Domain qua Traefik với Let's Encrypt, chứng chỉ của riêng bạn hoặc Cloudflare Tunnel. Áp dụng ngay khi lưu.
    link: /vi/apps/domains
  - title: Database và backup
    details: PostgreSQL, MySQL, MariaDB, MongoDB và Redis chỉ một cú bấm, kèm backup theo lịch vào thư mục hoặc bucket S3.
    link: /vi/apps/databases
  - title: Hơn 800 template app
    details: Tìm trong kho app dựng sẵn, điền một form ngắn rồi deploy cùng database của app.
    link: /vi/apps/sources-templates
  - title: Đưa vào những gì đang chạy
    details: Container chạy bằng docker run hay docker compose thành service, có kế hoạch, bản copy dữ liệu và hoàn tác.
    link: /vi/apps/import
  - title: Server, Docker và firewall
    details: CPU, RAM, ổ đĩa, mạng trực tiếp; container, image, volume, network; firewall chặn mặc định.
    link: /vi/server/docker
---

## Một trang cho cả server

![Bảng điều khiển](/shots/dashboard.png)

Bảng điều khiển (Dashboard) hiện số liệu trực tiếp của server và mọi project cùng các service của
nó. Từ đây bạn thêm service, mở template hoặc đưa các container đang chạy vào panel.

## Vì sao chọn RDockPanel

RDockPanel là một chương trình Rust có giao diện web, chạy trên chính server của bạn. Nó biến Docker
Swarm thành thứ dùng được từ trình duyệt: bạn chọn một image hoặc một repo, gán domain, bấm Deploy,
panel lo phần service Swarm, route Traefik và chứng chỉ HTTPS. Không có gói trả phí, không giới hạn
số project, app và dữ liệu nằm trên server của bạn. Giao diện có tiếng Anh và tiếng Việt, sáng và tối.

::: warning Đang phát triển
RDockPanel còn mới. Hãy thử trên một server test trước, và luôn giữ backup.
:::
