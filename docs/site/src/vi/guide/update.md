# Cập nhật và sao lưu panel

Giữ RDockPanel luôn mới, backup dữ liệu của chính panel, và quay về bản cũ khi cần.

## Xem bản đang chạy

**Cài đặt → Phiên bản** (Settings → Version) hiện phiên bản và **Kênh** (Channel) của panel, các
thành phần đi kèm (RustCMS Core, Traefik, MySQL hoặc SQLite, Docker, hệ điều hành), bản phát hành mới
nhất và lần kiểm tra cuối.

![Cài đặt → Phiên bản](/shots/settings-version.png)

Kênh tuỳ theo cách cài panel:

| Kênh | Cài bằng | Nút Cập nhật |
|---|---|---|
| Phát hành (Release) | `--version X.Y.Z` | có |
| Bản phát triển (main) | không có `--version` (image `main`) | không: không so với các bản phát hành |

Panel bản phát hành kiểm tra bản mới 6 giờ một lần (**Kiểm tra ngay** để kiểm tra liền). Khi có bản
mới, Cài đặt → Phiên bản và Bảng điều khiển sẽ báo cho quản trị viên, kèm **Có gì mới** (What
changed).

## Cập nhật từ panel

Panel không bao giờ tự cập nhật: quản trị viên phải bấm nút.

1. Mở **Cài đặt → Phiên bản** và bấm **Cập nhật lên X.Y.Z** (Update to X.Y.Z).
2. Đọc hộp xác nhận rồi xác nhận. Panel sẽ:
   - chờ các deploy, backup và khôi phục đang chạy kết thúc, các việc mới phải chờ;
   - copy database của nó vào `/var/lib/rdockpanel/backups/panel/` (không copy được thì không cập
     nhật);
   - nhờ Swarm thay panel. Panel tạm không truy cập được khoảng một phút; **các service của bạn vẫn
     chạy bình thường**.
3. Khi bản mới đã chạy, trang báo "RDockPanel X.Y.Z đang chạy." Bấm **Tải lại trang**.

Nếu bản mới không khởi động được, Swarm tự quay về bản trước. Phần **History** của Cài đặt → Phiên
bản ghi lại mỗi lần đổi phiên bản và khởi động lại.

::: tip Mẹo: chạy từ server
Cùng trang đó có **Lệnh installer** (Installer command) cho bản phát hành đó. Chạy nó bằng quyền root trên
server cũng cập nhật y như vậy và giữ nguyên cấu hình:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --version X.Y.Z
```
:::

**Bản cũ** (Old versions, cùng trang) xoá các container đã dừng của task cũ và image của chúng để
giải phóng ổ đĩa. Bản đang chạy và bản liền trước luôn được giữ.

## Sao lưu panel

Database của các app được backup từ tab **Backup** của chính nó (xem
[Database và backup](/vi/apps/databases)). Dữ liệu của panel cần hai thứ:

1. **`/etc/rdockpanel`**: secret (gồm `APP_KEY`) và chứng chỉ. Trên Mac là `~/.rdockpanel`.
2. **Database của panel**:
   - **MySQL**: chạy `mysqldump panel` trong container `rdp-mysql`:

     ```sh
     c=$(docker ps -q -f name=rdp-mysql)
     docker exec "$c" sh -c 'MYSQL_PWD="$(cat /run/secrets/rdp_mysql_root)" mysqldump -uroot panel' \
       > /root/rdockpanel-panel.sql
     ```

   - **SQLite**: dừng panel, copy **toàn bộ** thư mục `/var/lib/rdockpanel/data`, rồi chạy lại:

     ```sh
     docker service scale rdockpanel=0
     # chờ tới khi `docker ps` không còn container rdockpanel
     cp -a /var/lib/rdockpanel/data /root/rdockpanel-data-backup
     docker service scale rdockpanel=1
     ```

   Hoặc giữ bản copy mà mỗi lần **Cập nhật** tạo ra trong `/var/lib/rdockpanel/backups/panel/` (một
   file đầy đủ).

::: danger Cẩn thận: chỉ có panel.sqlite thì chưa phải backup
Những thay đổi mới nhất có thể còn nằm trong `panel.sqlite-wal`. Hãy copy cả thư mục, khi panel đã
dừng.
:::

::: warning Lưu ý: giữ kỹ APP_KEY
Mất `APP_KEY` là mất các biến môi trường đã mã hoá, mật khẩu registry và file mount. Hãy cất bản
backup của `/etc/rdockpanel` ở ngoài server.
:::

## Quay về bản cũ

Cài lại bản phát hành cũ bằng installer:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --version 0.13.0
```

Từ 0.5.0, bản cũ chạy được trên database như bản mới để lại (Cài đặt → Phiên bản sẽ liệt kê các
migration mới hơn). Muốn hoàn tác những gì một bản phát hành đã đổi trong database, hãy khôi phục bản
copy do lần cập nhật tạo ra, đúng bản đã tạo ra nó:

- **MySQL** (file `rdockpanel-<version>-<time>-<id>.sql`):

  ```sh
  c=$(docker ps -q -f name=rdp-mysql)
  docker exec -i "$c" sh -c 'MYSQL_PWD="$(cat /run/secrets/rdp_mysql_root)" mysql -uroot' \
    < /var/lib/rdockpanel/backups/panel/rdockpanel-0.13.0-….sql
  ```

- **SQLite** (file `rdockpanel-<version>-<time>-<id>.sqlite`): cài bản đó, dừng panel và chờ container
  của nó biến mất, xoá các file `-wal` và `-shm` cũ, đặt bản copy vào, rồi chạy lại:

  ```sh
  docker service scale rdockpanel=0
  cd /var/lib/rdockpanel/data
  rm -f panel.sqlite-wal panel.sqlite-shm
  cp /var/lib/rdockpanel/backups/panel/rdockpanel-0.13.0-….sqlite panel.sqlite
  docker service scale rdockpanel=1
  ```

## Lưu ý

- Chạy installer không có `--version` hay `--image` sẽ giữ bản phát hành mới hơn mà panel đã tự cập
  nhật lên.
- Panel không cập nhật khi đang có một lần [import](/vi/apps/import) chạy.
- Trước khi quay về bản cũ hơn 0.12.0, hãy để tunnel Cloudflare `default` bật (hoặc gỡ các tunnel
  khác) và hoàn tất hoặc hoàn tác các lần import đang dở.
- `--upgrade-system` cũng pull bản vá mới hơn của MySQL 8.4 và Traefik v3.7.
