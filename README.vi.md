# RDockPanel

**Panel quản trị server tự host, miễn phí, viết bằng Rust.**
Theo dõi server, quản lý Docker, và deploy app lên Docker Swarm kèm domain và HTTPS. Mọi tính năng
đều miễn phí: không giới hạn số project, không có gói trả phí.

[English](README.md) · [Tài liệu (tiếng Anh)](docs/) · [Lộ trình](docs/roadmap.md)

> **Trạng thái: giai đoạn đầu phát triển.** Đã chạy được: giám sát server, quản lý Docker, app và
> database trên Docker Swarm kèm domain và HTTPS, build từ Git hoặc Dockerfile, hơn 800 template
> EasyPanel, service Compose, backup database và volume, thông báo, và installer cài bằng một lệnh.
> Hãy thử trên một server test trước.

![Dashboard](docs/images/dashboard.png)

## Tính năng

**Giao diện**

- Vào `/admin` là thấy ngay **Dashboard**: số liệu server, các project với thẻ service, nút **+ Service** và **Templates**. Sidebar gồm Dashboard, Domains, Firewall, Actions, Settings và các trang Docker.
- **Trang service** có header (trạng thái, Deploy, Start/Stop, Restart, Open, Logs, Shell) và các tab: Overview, Credentials (với database), Backups (với database và service có volume), Deployments, Environment, Domains, Redirects, Security, Maintenance, Scripts, Source, Storage, Advanced.
- **Settings → Phiên bản**: panel đang chạy bản nào (cùng Core, Traefik, MySQL, Docker, hệ điều hành), có bản phát hành mới chưa (kiểm tra 6 giờ một lần, báo cả trên Bảng điều khiển), có gì mới, nút **Cập nhật** (admin bấm: panel không bao giờ tự cập nhật; database được backup trước, bản mới không chạy thì Swarm tự quay về), lịch sử, và dọn các bản cũ.
- **Settings → Panel:** log và khởi động lại panel; Refresh IP và Reboot server; log và khởi động lại Traefik; dọn Docker mà không đụng tới dữ liệu của service.
- **Tường lửa** (menu Workspace, chỉ quản trị viên): đóng mọi port với internet trừ SSH, 80 và 443, cùng các port anh mở cho địa chỉ chỉ định (ví dụ IP của VPN), cho máy chủ hoặc cho port service publish; chặn địa chỉ; xem các port đang lắng nghe và của ai. Mỗi thay đổi được thử trước, server tự quay về nếu không bấm Giữ; nạp lúc khởi động trước Docker ([docs/firewall.md](docs/firewall.md)).
- **Trang chủ** ở địa chỉ của panel (`/`): giới thiệu RDockPanel, ảnh giao diện, lệnh cài đặt và link GitHub, bằng tiếng Anh hoặc tiếng Việt, không có thông tin nào của máy chủ. Tắt được trong Settings → Panel (khi đó `/` mở trang đăng nhập).
- **Trang Domains** liệt kê domain của mọi service, nơi chúng trỏ tới và domain nào đang được phục vụ.
- Domain, redirect, giới hạn request, basic auth và maintenance **áp dụng ngay khi lưu** (không cần Deploy, container của khách không bị khởi động lại); image, env, port, mount, tài nguyên… mới cần Deploy.
- **Actions** liệt kê các lần deploy, backup và thao tác Docker; **Settings** có các tab Panel, Version, Traefik, Certificates, Cloudflare Tunnel, Branding, Trang bảo trì, Git, Backups, Notifications và API tokens; **⌘K** tìm nhanh project hoặc service. Mặc định giao diện sáng, mỗi người tự chọn Sáng / Tối / Theo hệ thống; có tiếng Anh và tiếng Việt.
- Mọi trang theo chung một bộ quy tắc thiết kế ([docs/ui-guidelines.md](docs/ui-guidelines.md)): nút cùng cỡ, thao tác phụ của từng dòng gom vào menu ⋯, trạng thái cùng chữ và màu, hộp thoại của panel thay cho của trình duyệt, bảng dùng được trên điện thoại.

**Server và Docker**


- **Số liệu server** ngay trên Dashboard: CPU, RAM, ổ đĩa, network vào/ra trực tiếp, kèm lịch sử gần đây.
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

**App trên Docker Swarm**

- **Project và service:**
  - service chạy một Docker image trên **Docker Swarm**;
  - có replicas, giới hạn CPU và RAM, healthcheck, volume, port và command;
  - mỗi project có network riêng: service của project này không gọi thẳng được service của project khác (những gì đã public qua domain hoặc port thì vẫn public).
- **Deploy:**
  - Deploy, Start, Stop, Restart và Destroy ngay trên trang service, có log trực tiếp; mỗi service chỉ chạy một lần deploy tại một thời điểm;
  - Rollback từ một lần deploy trước ("Roll back to this" trong danh sách Deployments);
  - trước khi tạo hoặc deploy service, panel ước tính RAM cần thêm và cảnh báo khi máy sắp hết (không đủ thì phải xác nhận mới deploy; deploy qua API hoặc webhook không bị chặn, chỉ ghi log và gửi thông báo);
  - Build lại từ đầu (Force rebuild: không dùng build cache, pull lại image, thay mọi task); Overview hiện lỗi gần đây;
  - image được ghim theo digest của registry (image chỉ có trên máy thì ghim theo id, kèm cảnh báo);
  - rolling update: task mới chạy trước khi bật zero downtime, có healthcheck và không publish port kiểu host (nếu không thì task cũ dừng trước); Swarm tự rollback khi update lỗi;
  - mỗi lần deploy lưu lại cấu hình để xem lịch sử và rollback;
  - trang service báo khi có thay đổi chưa deploy.
- **Domain và HTTPS** qua **Traefik:** mỗi service có thể gắn nhiều domain và path; domain bật HTTPS thì HTTP tự chuyển sang HTTPS; domain wildcard (`*.example.com`, chỉ quản trị panel) và middleware Traefik tuỳ chỉnh; chuyển hướng (biểu thức chính quy), giới hạn request theo từng khách (qua Cloudflare vẫn đếm đúng IP thật, có danh sách IP được miễn), xác thực cơ bản (basic auth) và trang bảo trì (trang có sẵn hoặc HTML của riêng bạn, có giờ dự kiến trở lại và danh sách IP vẫn vào được site thật). Mỗi domain chọn được certificate resolver riêng (Settings → Chứng chỉ: Let's Encrypt qua HTTP, hoặc qua DNS bằng token Cloudflare, DigitalOcean, Hetzner, Vultr, Linode), áp dụng vào Traefik ngay trong panel, không cần SSH; hoặc tải chứng chỉ của riêng bạn lên (được kiểm tra, khoá được mã hoá, báo trước khi hết hạn).
- **Database:** PostgreSQL, MySQL, MariaDB, MongoDB và Redis chỉ với một cú bấm (chỉ cần đặt tên: mật khẩu tự sinh, dữ liệu trên volume, có healthcheck), kèm URL kết nối để copy.
- **Hơn 800 template EasyPanel**, tạo bằng một cú bấm (tìm kiếm, điền form của chính template, deploy kèm database), tải về khi mở danh sách; hoặc dán kết quả của template ("Create from Schema").
- **Build:** app từ repo Git (https, ssh với deploy key sinh sẵn, hoặc GitHub token ở Settings → Git được cấp cho project, chọn repo từ danh sách; branch, tag hoặc commit; thư mục con), build bằng Dockerfile của repo, hoặc từ Dockerfile viết ngay trong panel; log build nằm trong log deploy. URL kích hoạt deploy cho CI, và webhook GitHub có chữ ký để tự deploy mỗi lần push.
- **Compose:** file Compose (viết trong panel hoặc lấy từ Git) chạy bằng `docker compose` trong network của project, domain trỏ tới một service trong file.
- **Backup:** dump PostgreSQL, MySQL, MariaDB và MongoDB, và nén volume, theo lịch (cron) vào thư mục trên server hoặc bucket tương thích S3, giữ lại N bản, có log từng lần chạy, nút "Run now" và Khôi phục.
- **Thành viên project** (Xem, Developer, Quản trị theo từng project) và **API công khai** với token có scope (Settings → API token), kèm lệnh `rdp`.
- **Thông báo:** Discord, Slack, Telegram, email (SMTP) và webhook; mỗi kênh tự chọn sự kiện: deploy, backup, ổ đĩa sắp đầy, service lỗi và chạy lại, Traefik ngừng và chạy lại, certificate sắp hết hạn, có bản panel mới, dọn Docker, đăng nhập từ IP mới; có nút "Gửi thử".
- **Panel trong Settings:** domain panel, email Let's Encrypt và domain cho app được kiểm tra (DNS, tên đang dùng), kèm sẵn lệnh installer để áp dụng.
- **Traefik trong Settings:** xem trạng thái, khởi động lại, log trực tiếp, và middleware tuỳ chỉnh (`custom-<tên>`: headers, giới hạn tốc độ, danh sách IP…) cho domain, không cần SSH.
- **Cloudflare Tunnel** (Settings): chạy cloudflared bằng token tunnel của bạn (mã hoá, đưa vào dưới dạng Swarm secret), để vào domain qua Cloudflare mà không cần mở cổng 80 và 443. Domain trỏ về Cloudflare được đánh dấu "qua Cloudflare" (đi qua tunnel thì HTTPS do Cloudflare lo).
- **Thương hiệu** (Settings): tên, màu chính, logo (sáng và tối) và favicon của panel, trên mọi trang và trang đăng nhập, không cần khởi động lại.
- **Trang Roles:** quyền chia theo nhóm (Ứng dụng, Docker, Server, Hệ thống) có nhãn và mô tả, ô tìm kiếm, và preset (xem, developer, quản trị ứng dụng; giám sát) để tạo role nhanh.
- **Từng service:** biểu đồ CPU và RAM, ghi chú cho nhóm, script (lệnh chạy một lần trong container đang chạy, hiện output); quản trị panel còn đặt được capability, sysctl và group bổ sung.
- **Bí mật:** biến môi trường (sửa như file `.env`) và tài khoản registry riêng được mã hoá bằng app key; chỉ người có quyền `services.secrets` mới xem được giá trị, mỗi lần xem đều ghi audit. Rollback trả lại đúng biến môi trường của lần deploy đó.

**Sắp tới** (xem [lộ trình](docs/roadmap.md))

- Nguồn Upload; web client cho database.
- **Sau đó:** nhiều node, xác thực 2 lớp.

| Trang service | Template | Tạo service |
|---|---|---|
| ![Trang service](docs/images/service.png) | ![Template](docs/images/templates.png) | ![Tạo service](docs/images/new-service.png) |
| Lịch sử deploy | Domain | Database |
| ![Lịch sử deploy](docs/images/deployments.png) | ![Domain](docs/images/domains.png) | ![Database](docs/images/database.png) |
| Actions | Settings | Container |
| ![Actions](docs/images/actions.png) | ![Settings](docs/images/settings.png) | ![Container](docs/images/containers.png) |
| Log trực tiếp | Terminal |   |
| ![Log trực tiếp](docs/images/logs.png) | ![Terminal](docs/images/terminal.png) |   |

## Cài đặt

Trên một server Ubuntu hoặc Debian mới (amd64 hoặc arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --admin-email ban@example.com
```

Thêm `--domain panel.example.com --email ban@example.com` để panel chạy HTTPS (Let's Encrypt), và `--apps-domain apps.example.com` để có domain tự sinh cho app. Xem [docs/install.md](docs/install.md).

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
- **Giao diện:** các trang admin viết bằng React, chạy trên extension host của RustCMS (file build được commit sẵn, chỉ cần Node khi sửa giao diện). Terminal dùng [xterm.js](https://xtermjs.org) (đi kèm trong repo, license MIT).

Chi tiết xem [docs/architecture.md](docs/architecture.md).

## Tài liệu (tiếng Anh)

| | |
|---|---|
| [Kiến trúc](docs/architecture.md) | các module, luồng request, cập nhật trực tiếp, thao tác và job |
| [Phân quyền](docs/permissions.md) | ai được xem và làm gì |
| [Bảo mật](docs/security.md) | Docker socket, object được bảo vệ, terminal, CSP |
| [API](docs/api.md) | các route JSON, event stream và WebSocket của panel |
| [Phát triển](docs/development.md) | môi trường, test, quy ước |
| [Quy tắc giao diện](docs/ui-guidelines.md) | bộ quy tắc thiết kế mọi trang phải theo |
| [Lộ trình](docs/roadmap.md) | các phase và phụ thuộc |

## License

Dùng theo một trong hai license tuỳ chọn: [Apache License 2.0](LICENSE-APACHE) hoặc [MIT](LICENSE-MIT).
xterm.js dùng license MIT ([xem tại đây](resources/admin/js/rdockpanel/vendor/LICENSE-xterm)).
