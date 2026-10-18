# RDockPanel

**Panel quản trị server tự host, miễn phí, theo tinh thần [EasyPanel](https://easypanel.io), viết bằng Rust.**
Theo dõi server, quản lý Docker, và deploy app lên Docker Swarm kèm domain và HTTPS. Mọi tính năng
đều miễn phí: không giới hạn số project, không có gói trả phí.

[English](README.md) · [Tài liệu (tiếng Anh)](docs/) · [Lộ trình](docs/roadmap.md)

> **Trạng thái: đang phát triển.**
> - Phase 1 (giám sát server, quản lý Docker) đã chạy được.
> - Phase 2 (deploy app) đã chạy được từ mã nguồn: project, service, deploy, domain, biến môi trường mã hoá, registry riêng.
> - Phase 2 còn lại: installer (hiện chạy từ mã nguồn như hướng dẫn bên dưới).
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
- **Truy vết:** mọi thao tác quản lý Docker (container, image, volume, network, terminal) được ghi vào nhật ký thao tác (Actions) và audit log, kèm người thực hiện; các lần deploy có lịch sử riêng (ai bấm, lúc nào, log).
- **Người dùng và vai trò:** phân quyền chi tiết (`docker.view`, `docker.manage`, `docker.logs`, `docker.exec`, `server.view`, `system.manage`, và mỗi resource một bộ quyền như `services.deploy`), dựa trên hệ thống role của RustCMS.

**Đã có (Phase 2, chạy từ mã nguồn)**

- **Project và service:**
  - service chạy một Docker image trên **Docker Swarm**;
  - có replicas, giới hạn CPU và RAM, healthcheck, volume, port và command;
  - mỗi project có network riêng: service của project này không gọi thẳng được service của project khác (những gì đã public qua domain hoặc port thì vẫn public).
- **Deploy:**
  - Deploy, Start, Stop, Restart và Destroy ngay trên trang service, có log trực tiếp; mỗi service chỉ chạy một lần deploy tại một thời điểm;
  - Rollback từ một lần deploy trước ("Roll back to this" trong danh sách Deployments);
  - image được ghim theo digest của registry (image chỉ có trên máy thì ghim theo id, kèm cảnh báo);
  - rolling update: task mới chạy trước khi bật zero downtime, có healthcheck và không publish port kiểu host (nếu không thì task cũ dừng trước); Swarm tự rollback khi update lỗi;
  - mỗi lần deploy lưu lại cấu hình để xem lịch sử và rollback;
  - trang service báo khi có thay đổi chưa deploy.
- **Domain và HTTPS** qua **Traefik:** mỗi service có thể gắn nhiều domain và path; domain bật HTTPS thì HTTP tự chuyển sang HTTPS.
- **Bí mật:** biến môi trường và tài khoản registry riêng được mã hoá bằng app key; panel chỉ hiện tên biến, không bao giờ hiện giá trị. Rollback trả lại đúng biến môi trường của lần deploy đó.

**Sắp tới** (xem [lộ trình](docs/roadmap.md))

- **Phase 2:** Let's Encrypt khi chạy thật, installer cài bằng một lệnh.
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
sh scripts/dev-swarm.sh                         # Swarm một node, Traefik ở :8080 và :8443
rustcms serve                                   # http://localhost:8000/admin
```

Mở panel bằng `localhost` (đúng `APP_URL` trong `.env`), không dùng `127.0.0.1`: đăng nhập có kiểm tra origin.
Service có domain `app.localhost` sẽ mở được ở http://app.localhost:8080, và ở https://app.localhost:8443 nếu domain bật HTTPS (chứng chỉ tự ký của Traefik).

Panel nói chuyện với Docker daemon trên máy (`/var/run/docker.sock` hoặc `DOCKER_HOST`): **nó quản lý
các container thật trên máy bạn**. `dev-swarm.sh` biến Docker trên máy thành Swarm manager (gỡ bằng `docker swarm leave --force`).

## Test

```sh
export RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql RUSTCMS_TEST_BACKENDS=mysql
cargo test                       # unit và feature test trên MySQL, Docker giả lập
PANEL_TEST_DOCKER=1 cargo test   # chạy thêm với Docker thật (tạo một container alpine)
PANEL_TEST_SWARM=1 cargo test    # chạy thêm deploy trên Swarm thật và route qua Traefik
                                 # (sau khi chạy scripts/dev-swarm.sh)
```

CI kiểm tra format, `clippy -D warnings` và chạy test trên MySQL 8.4 khi push lên `main` và khi có pull request. Một job thứ hai chạy với Docker và Swarm thật, có Traefik: deploy, rolling update khi đang có tải, kiểm tra HTTPS và việc cô lập giữa các project.

## Kiến trúc

- **[RustCMS](https://github.com/irisvn/RustCMS-Core):** framework giống Laravel/Filament, xây trên axum và SeaORM. Cung cấp đăng nhập, phân quyền, audit log, hàng đợi job, trang admin, cùng các route admin dạng server-sent events và WebSocket.
- **Thư viện và dữ liệu:**
  - **[bollard](https://github.com/fussybeaver/bollard)** cho Docker Engine API (gồm service, task, network của Swarm);
  - **Docker Swarm** chạy các app; **[Traefik](https://traefik.io)** v3 route domain cho chúng, từ các file do panel ghi;
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
