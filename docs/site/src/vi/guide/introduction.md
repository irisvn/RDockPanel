# Giới thiệu

RDockPanel là panel miễn phí, tự host, chạy app của bạn trên Docker Swarm, kèm domain, HTTPS,
database và backup, từ một trang web ngay trên server của bạn.

![Bảng điều khiển](/shots/dashboard.png)

## Dành cho ai

- Bạn thuê một VPS (hoặc có server ở nhà) và muốn chạy vài app, website hay database trên đó mà
  không phải tự viết file Swarm và rule Traefik.
- Bạn chạy app cho một nhóm hoặc cho khách hàng và muốn mỗi người chỉ thấy project của mình.
- Bạn đang chạy container bằng `docker run` hoặc `docker compose` và muốn quản lý chúng từ trình
  duyệt, có lịch sử, rollback và backup.

Bạn cần quen dùng terminal để cài đặt, và nắm những thứ cơ bản của Docker (image, port, volume).
Phần còn lại panel lo.

## Panel làm được gì

**Ứng dụng**
- [Project và service](/vi/apps/projects-services): mỗi service chạy một image trên Docker Swarm
  với replicas, giới hạn CPU và RAM, health check, volume và port. Mỗi project có network riêng.
- [Nguồn và template](/vi/apps/sources-templates): image từ registry, repo Git hoặc Dockerfile build
  ngay trên server, hoặc một trong hơn 800 template app dựng sẵn.
- [Deploy và rollback](/vi/apps/deploy): rolling update, log trực tiếp cho mỗi lần deploy, và
  "Rollback về lần này" trên một lần deploy cũ. URL kích hoạt deploy hoặc webhook GitHub sẽ deploy
  mỗi khi push.
- [Biến môi trường và secret](/vi/apps/environment): biến môi trường được mã hoá bằng key của
  panel, chỉ hiện cho người có quyền xem.
- [Domain và HTTPS](/vi/apps/domains) và [Cloudflare Tunnel](/vi/apps/tunnel): route Traefik, chứng
  chỉ Let's Encrypt (qua HTTP hoặc DNS) hoặc chứng chỉ của bạn, áp dụng ngay khi lưu.
- [Chuyển hướng, bảo mật, bảo trì](/vi/apps/access): chuyển hướng, giới hạn request, basic
  authentication và trang bảo trì.
- [Database và backup](/vi/apps/databases): PostgreSQL, MySQL, MariaDB, MongoDB và Redis chỉ một cú
  bấm; backup theo lịch vào thư mục trên server hoặc bucket tương thích S3, và khôi phục.
- [Log, terminal, script](/vi/apps/logs-terminal-scripts), [Docker Compose](/vi/apps/compose),
  [Import container có sẵn](/vi/apps/import) và [Chọn node chạy](/vi/apps/nodes).

**Server**
- [Docker](/vi/server/docker): CPU, RAM, ổ đĩa và mạng trực tiếp; container (log, thống kê,
  terminal trong trình duyệt), image, volume và network.
- [Firewall](/vi/server/firewall): đóng mọi port trừ SSH, 80, 443 và những port bạn mở.
- [Cài đặt](/vi/server/settings): domain của panel, phiên bản và nút cập nhật, Traefik, chứng chỉ,
  thương hiệu, Git, backup, thông báo và API token.
- [Người dùng và vai trò](/vi/server/users): phân quyền chi tiết và thành viên project.

**Ngoài ra**: [API công khai](/vi/reference/api) với token giới hạn scope, thông báo (Discord,
Slack, Telegram, email, webhook), tiếng Anh và tiếng Việt, giao diện sáng và tối.

## Panel không phải là

- **Không phải dịch vụ hosting.** Panel chạy trên server của bạn; bạn tự lo cập nhật server và
  backup.
- **Không phải công cụ quản lý cluster.** Installer dựng một Swarm một node, và panel được làm quanh
  node đó. Có thể cho một service chạy trên node khác của Swarm, nhưng một số công cụ (backup,
  script, terminal, thống kê trực tiếp, firewall) chỉ dùng được trên node của panel.
- **Chưa hoàn thiện.** Panel đang trong giai đoạn đầu: hãy thử trên server test trước. Xác thực hai
  lớp và trình quản lý database trên web đã có kế hoạch, chưa có.

## Yêu cầu

| | Server Linux | Mac |
|---|---|---|
| Hệ điều hành | Ubuntu hoặc Debian, amd64 hoặc arm64 | macOS trên Apple silicon |
| Docker | installer tự cài nếu chưa có | OrbStack hoặc Docker Desktop, đang chạy |
| Quyền | root (`sudo`) | user của bạn, không `sudo` |
| Port | 80 và 443 (thêm 3000 khi panel chưa có domain) | như vậy, còn trống trên Mac |
| Hướng dẫn | [Cài trên Linux](/vi/guide/install) | [Cài trên macOS](/vi/guide/install-macos) |

Panel lưu dữ liệu của chính nó trong MySQL 8.4 hoặc SQLite, chọn lúc cài. SQLite tốn ít RAM hơn,
hợp với server nhỏ.

Tiếp theo: [cài panel](/vi/guide/install), rồi [deploy app đầu tiên](/vi/guide/first-app).
