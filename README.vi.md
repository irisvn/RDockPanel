# RDockPanel

**Panel quản trị server tự host, miễn phí, theo tinh thần [EasyPanel](https://easypanel.io), viết bằng Rust.**
Theo dõi server, quản lý Docker, và sắp tới là deploy app lên Docker Swarm kèm domain và SSL. Mọi tính
năng đều miễn phí: không giới hạn số project, không có gói trả phí.

[English](README.md) · [Tài liệu (tiếng Anh)](docs/) · [Lộ trình](docs/roadmap.md)

> **Trạng thái: đang phát triển.**
> - Phase 1 (giám sát server, quản lý Docker) đã chạy được.
> - Phase 2 (deploy app) đang làm.
> - Chưa có installer: hiện chạy từ mã nguồn như hướng dẫn bên dưới.
> - Chưa dùng được cho production.

![Bảng điều khiển server](docs/images/server.png)

## Tính năng

**Đã có (Phase 1)**

- **Server:**
  - biểu đồ CPU, RAM (kèm số liệu swap), network vào/ra, ổ đĩa, cập nhật trực tiếp (lấy mẫu mỗi 2 giây, giữ 5 phút gần nhất);
  - thông tin máy: OS, kernel, số nhân, uptime.
- **Container:**
  - danh sách có tìm kiếm;
  - chạy, dừng, khởi động lại, tạm dừng, kill, xoá;
  - log trực tiếp (stderr tô đỏ), tài nguyên trực tiếp;
  - xem chi tiết (che các biến môi trường giống mật khẩu);
  - **terminal** ngay trên trình duyệt.
- **Image:** pull (xem tiến độ trực tiếp), xoá theo tag, dọn container đã dừng và image "dangling" (không còn tag).
- **Volume và network:** tạo và xoá (bridge hoặc overlay).
- **An toàn:**
  - container, volume, network của chính panel (label `rdockpanel.system=true`) bị ẩn và không thao tác được nếu không có quyền `system.manage`;
  - lệnh dọn dẹp luôn bỏ qua chúng.
- **Truy vết:** mọi thay đổi lên Docker được ghi vào nhật ký thao tác (Actions) và audit log, kèm người thực hiện.
- **Người dùng và vai trò:** phân quyền chi tiết (`docker.view`, `docker.manage`, `docker.logs`, `docker.exec`, `server.view`, `system.manage`), dựa trên hệ thống role của RustCMS.

**Sắp tới** (xem [lộ trình](docs/roadmap.md))

- **Phase 2:**
  - project và service deploy lên **Docker Swarm** từ image;
  - rolling update có rollback;
  - domain và HTTPS qua **Traefik**;
  - installer cài bằng một lệnh.
- **Phase 3:** build từ Git, database kèm backup, Compose, template, thông báo, nhiều node, xác thực 2 lớp, API và CLI.

| Container | Log trực tiếp | Terminal |
|---|---|---|
| ![Container](docs/images/containers.png) | ![Log](docs/images/logs.png) | ![Terminal](docs/images/terminal.png) |

## Chạy thử (môi trường dev)

Cần có:
- Rust (bản stable);
- Docker (Docker Desktop hoặc OrbStack trên macOS);
- CLI [`rustcms`](https://github.com/irisvn/RustCMS).

```sh
git clone https://github.com/irisvn/rdockpanel && cd rdockpanel
cp .env.example .env && rustcms key:generate    # tạo APP_KEY
docker compose up -d                            # MySQL 8.4 tại 127.0.0.1:3306
rustcms migrate
rustcms permission:sync
rustcms make:user --email admin@example.com --name Admin --super-admin
rustcms serve                                   # http://localhost:8000/admin
```

Panel nói chuyện với Docker daemon trên máy (`/var/run/docker.sock` hoặc `DOCKER_HOST`): **nó quản lý
các container thật trên máy bạn**.

## Test

```sh
export RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql RUSTCMS_TEST_BACKENDS=mysql
cargo test                       # unit và feature test trên MySQL, Docker giả lập
PANEL_TEST_DOCKER=1 cargo test   # chạy thêm với Docker thật (tạo một container alpine)
```

CI kiểm tra format, `clippy -D warnings` và chạy test trên MySQL 8.4 khi push lên `main` và khi có pull request (test với Docker thật không chạy trong CI).

## Kiến trúc

- **[RustCMS](https://github.com/irisvn/RustCMS-Core):** framework giống Laravel/Filament, xây trên axum và SeaORM. Cung cấp đăng nhập, phân quyền, audit log, hàng đợi job, trang admin, cùng các route admin dạng server-sent events và WebSocket.
- **Thư viện và dữ liệu:**
  - **[bollard](https://github.com/fussybeaver/bollard)** cho Docker Engine API;
  - **sysinfo** và `/proc` cho số liệu máy;
  - **MySQL 8.4** lưu dữ liệu của panel.
- **Giao diện:** các trang admin là script do panel RustCMS nạp. Terminal dùng [xterm.js](https://xtermjs.org) (đi kèm trong repo, license MIT).

Chi tiết xem [docs/architecture.md](docs/architecture.md).

## Tài liệu (tiếng Anh)

| | |
|---|---|
| [Kiến trúc](docs/architecture.md) | các module, luồng request, cập nhật trực tiếp, thao tác và job |
| [Phân quyền](docs/permissions.md) | ai được xem và làm gì |
| [Bảo mật](docs/security.md) | Docker socket, object được bảo vệ, terminal, CSP |
| [API](docs/api.md) | các route JSON, event stream và WebSocket của panel |
| [Phát triển](docs/development.md) | môi trường, test, quy ước |
| [Lộ trình](docs/roadmap.md) | các phase và phụ thuộc |

## License

Dùng theo một trong hai license tuỳ chọn: [Apache License 2.0](LICENSE-APACHE) hoặc [MIT](LICENSE-MIT).
xterm.js dùng license MIT ([xem tại đây](resources/admin/js/rdockpanel/vendor/LICENSE-xterm)).
