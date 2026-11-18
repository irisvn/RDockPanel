# Cài trên Linux

Cài RDockPanel lên một server Ubuntu hoặc Debian mới bằng một lệnh.

## Trước khi bắt đầu

- Một server **Ubuntu hoặc Debian** mới, **amd64 hoặc arm64**, có quyền root (`sudo`).
- Port **80** và **443** được mở ở firewall của nhà cung cấp (thêm **3000** khi panel chưa có
  domain).
- Muốn dùng HTTPS: một domain cho panel (ví dụ `panel.example.com`) có bản ghi A trỏ về server, và
  nếu cần thì bản ghi wildcard `*.apps.example.com` cho domain tự sinh của app. Trỏ DNS **trước**
  khi chạy installer.

Không cần cài Docker trước: installer tự cài khi chưa có.

## Cài đặt

Cách cài đơn giản nhất, truy cập bằng địa chỉ IP của server:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --admin-email you@example.com
```

Có domain và HTTPS cho panel (Let's Encrypt), kèm domain tự sinh cho app:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- \
    --domain panel.example.com --email you@example.com \
    --apps-domain apps.example.com --admin-email you@example.com
```

Cuối cùng installer in ra **mật khẩu quản trị viên** và **địa chỉ của panel**. Hãy cất mật khẩu
ở nơi an toàn.

::: tip Mẹo: chọn bản phát hành
Thêm `--version X.Y.Z` (bản mới nhất có ở
[trang phát hành](https://github.com/irisvn/RDockPanel/releases)). Không có tuỳ chọn này, bạn nhận
image `main`, một bản phát triển mà **Cài đặt → Phiên bản** (Settings → Version) không cập nhật bằng
nút bấm được.
:::

## Database của panel: MySQL hay SQLite

Panel lưu dữ liệu của chính nó (project, service, người dùng, audit log…) trong một trong hai loại
database, chọn ở lần cài đầu và giữ nguyên về sau:

| | MySQL 8.4 (mặc định) | SQLite |
|---|---|---|
| Chạy gì | một service `rdp-mysql` cạnh panel | một file, không có database server |
| RAM | nhiều hơn | ít hơn |
| Hợp với | phần lớn server | server nhỏ; mỗi lúc chỉ một thao tác ghi, nhiều người cùng lưu sẽ phải chờ lượt |

Chạy trong terminal, installer sẽ hỏi (Enter là MySQL). `--database mysql` hoặc `--database sqlite`
trả lời trước; chạy từ script thì là MySQL. Lựa chọn này không ảnh hưởng tới database của các app.
Không đổi được về sau: chạy lại với `--database` khác thì installer dừng, không thay đổi gì.

## Các tuỳ chọn {#options}

| Tuỳ chọn | Tác dụng |
|---|---|
| `--admin-email EMAIL` | tạo quản trị viên đầu tiên và in mật khẩu |
| `--domain HOST` | panel chỉ chạy ở `https://HOST` (cần `--email`; trỏ DNS về server trước) |
| `--email ADDRESS` | tài khoản Let's Encrypt, cho chứng chỉ của panel và các app |
| `--apps-domain HOST` | domain tự sinh cho app `<project>-<service>.HOST` |
| `--no-domain` | quay về `http://<ip-server>:3000` |
| `--database mysql\|sqlite` | database của panel, chỉ ở lần cài đầu |
| `--version X.Y.Z` | bản phát hành đó của panel |
| `--image IMAGE` | image khác cho panel (mặc định: bản phát triển `main`) |
| `--upgrade-system` | pull lại MySQL và Traefik theo tag của chúng (bản vá mới hơn) |
| `--remove-waf` | gỡ [WAF](/vi/server/waf) và mọi thứ nó tạo ra rồi dừng (tắt WAF trước) |

Các tuỳ chọn được lưu trong `/etc/rdockpanel/install.conf`. Lần chạy sau chỉ đổi những gì bạn đưa vào.

## Installer dựng những gì

- Docker (khi chưa có) và một **Swarm** một node.
- Ba service Swarm, ẩn với người không có quyền `system.manage`:
  - `rdockpanel`: chính panel;
  - `rdp-traefik`: Traefik v3.7 ở port 80 và 443, route mọi domain;
  - `rdp-mysql`: MySQL của panel (không có khi dùng SQLite).
- `/etc/rdockpanel`: secret của panel (gồm `APP_KEY`), route và chứng chỉ của Traefik.
- `/var/lib/rdockpanel`: backup, file Compose, và với SQLite là database (`data/`).

## Đăng nhập lần đầu {#first-sign-in}

1. Mở địa chỉ installer đã in ra:
   - không có domain: `http://203.0.113.10:3000/admin` (địa chỉ server của bạn);
   - có domain: `https://panel.example.com/admin`.
2. Đăng nhập bằng email đã đưa vào `--admin-email` và mật khẩu được in ra.
3. Bạn sẽ vào **Bảng điều khiển** (Dashboard). Tiếp theo: [app đầu tiên](/vi/guide/first-app).

Bản thân địa chỉ của panel (`/`) hiện một trang chủ công khai giới thiệu RDockPanel, không có thông
tin gì về server của bạn. **Cài đặt → Panel** (Settings → Panel) tắt được trang này; panel luôn ở
`/admin`.

::: warning Lưu ý: hãy gán domain
Chưa có domain, panel chạy HTTP thường ở port 3000. Hãy gán domain trước khi dùng qua internet: có
domain rồi thì port 3000 bị đóng và panel chỉ truy cập được qua HTTPS. **Cài đặt → Panel** kiểm tra
domain mới (bản ghi DNS, tên đã dùng) và đưa ra đúng lệnh installer cần chạy.
:::

## Đổi tuỳ chọn về sau

Chạy lại installer với các tuỳ chọn cần đổi. Secret, database và chứng chỉ được giữ nguyên. Xem
[Cập nhật và sao lưu panel](/vi/guide/update).

## Gỡ cài đặt {#uninstall}

1. Nếu [Firewall](/vi/server/firewall) đang bật, tắt nó trước (**Tường lửa → Tắt**).
2. Gỡ các service của panel (`rdp-mysql` khi dùng MySQL; `rdp-cloudflared` và mọi
   `rdp-cloudflared-*` nếu đã dùng Cloudflare Tunnel):

   ```sh
   docker service rm rdockpanel rdp-traefik rdp-mysql rdp-cloudflared
   ```

3. Xoá dữ liệu. **Lệnh này xoá database của panel và cả các backup của nó:**

   ```sh
   docker volume rm rdp-mysql-data                                         # MySQL
   docker secret rm rdp_app_key rdp_database_url
   docker secret rm rdp_mysql_root rdp_mysql_password                      # MySQL
   docker secret ls -q -f name=rdp_tunnel_ -f name=rdp-dns- | xargs -r docker secret rm
   docker network rm rdp-internal rdp-tunnel     # rdp-tunnel chỉ khi đã dùng tunnel
   rm -r /etc/rdockpanel /var/lib/rdockpanel/backups
   rm -r /var/lib/rdockpanel/data                                          # SQLite
   ```

Hãy xoá cả database: lần cài mới tạo `APP_KEY` mới, không đọc được database cũ. Service của các app
vẫn chạy cho tới khi bạn gỡ chúng. Giữ `/var/lib/rdockpanel/compose` cho tới khi các service Compose
và dữ liệu của chúng đã được gỡ: thư mục `./data` của chúng nằm ở đó.
