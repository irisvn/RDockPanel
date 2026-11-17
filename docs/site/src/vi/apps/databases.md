# Database và backup

PostgreSQL, MySQL, MariaDB, MongoDB và Redis chỉ trong một cú bấm: đặt tên là đủ. Panel tự sinh thông tin đăng nhập, giữ dữ liệu trên volume, và có thể backup theo lịch.

## Tạo database

1. Trong một project, bấm **Thêm service** (Add service).
2. Chọn **PostgreSQL**, **MySQL**, **MariaDB**, **MongoDB** hoặc **Redis**.
3. Gõ tên, ví dụ `db`. Để trống **Image (tuỳ chọn)** để dùng image chính thức (`postgres:17`, `mysql:9`, `mariadb:11`, `mongo:8`, `redis:7`), hoặc ghi phiên bản khác.
4. Bấm **Tạo**, rồi **Deploy**.

Panel cấu hình sẵn cho bạn:

- thông tin đăng nhập tự sinh (mật khẩu 24 ký tự chữ và số), lưu mã hoá trong biến môi trường của nó;
- một volume `data` ở nơi database lưu dữ liệu;
- healthcheck riêng của từng loại database (`pg_isready`, `mysqladmin ping`…);
- một replica, khi cập nhật thì dừng bản cũ trước rồi mới chạy bản mới (hai bản không bao giờ dùng chung dữ liệu).

Database deploy, rollback và có log như mọi service khác.

![Tab Tổng quan của database](/shots/database-overview.png)

## Thông tin kết nối

Tab **Thông tin kết nối** (Credentials) hiện những gì app cần để kết nối: **Host nội bộ**, **Port**, **User**, **Mật khẩu**, **Database**, **Mật khẩu root** (khi database có) và **URL nội bộ**, mỗi mục có nút copy.

Xem mật khẩu cần quyền `services.secrets`, và mỗi lần xem đều được ghi vào audit log.

::: warning Lưu ý
Thông tin đăng nhập chỉ có tác dụng lúc volume của database được tạo lần đầu. Đổi sau này trong biến môi trường không đổi được mật khẩu bên trong database: hãy đổi cả trong database.
:::

![Tab Thông tin kết nối](/shots/database-credentials.png)

## Kết nối từ service khác {#ket-noi-tu-service-khac}

Các service cùng project gọi database bằng host nội bộ `<project>_<service>`. Copy **URL nội bộ** vào [biến môi trường](/vi/apps/environment) của app, ví dụ:

```sh
DATABASE_URL=postgres://app:<mật-khẩu>@shop_db:5432/app
```

Service của project khác không gọi tới được. Đừng publish port của database nếu không thật cần: nếu có, chỉ cho phép địa chỉ của bạn bằng [firewall](/vi/server/firewall).

## Backup {#backup}

Bản backup được ghi vào một **nơi lưu trữ** (storage provider): thư mục trên server hoặc bucket tương thích S3 (AWS, Cloudflare R2, MinIO…). Quản trị viên thêm nơi lưu trữ trước ở **Cài đặt → Backup** (Settings → Backups).

Sau đó, ở tab **Backup** của database:

1. Bấm **Tạo backup** (Create backup).
2. **Backup** (Back up): **Database** (bản dump) hoặc một **Volume** của service (bản nén).
3. **Tên database**: database cần dump.
4. **Lịch** (Schedule): chọn sẵn (**Hằng ngày lúc 02:00 (UTC)**, **Mỗi 6 giờ**, **Mỗi giờ**, **Hằng tuần, Chủ nhật 03:00 (UTC)**) hoặc **Tuỳ chỉnh** (một dòng cron theo giờ UTC: phút giờ ngày tháng thứ).
5. **Nơi lưu trữ**, **Đường dẫn lưu** (Destination path: một thư mục trong nơi lưu trữ, mỗi database một thư mục) và **Số bản giữ lại** (Retention: số file của database này được giữ ở đó; file cũ hơn bị xoá).
6. Để **Chạy theo lịch** bật, rồi bấm **Tạo**.

**Chạy ngay** (Run now) chạy backup luôn. **Nhật ký backup** (Backups log) liệt kê mọi lần chạy với trạng thái, dung lượng và log.

Bản dump dùng công cụ riêng của database, chạy bên trong container của nó: `pg_dump`, `mysqldump`, `mariadb-dump` hoặc `mongodump`, nén gzip. Redis lưu dữ liệu trong volume (file append-only): không có bản dump database, nhưng volume của nó có thể được nén để backup.

![Tab Backup](/shots/database-backups.png)

## Khôi phục

1. Ở tab **Backup**, bấm **Khôi phục** (Restore).
2. Chọn **Nơi lưu trữ** và **Thư mục**, bấm **Liệt kê file** (List files), rồi chọn một **File**.
3. Chọn khôi phục cái gì (database, hoặc một volume) và, với database, chọn database ở ô **Khôi phục vào database**. Bấm **Khôi phục** và xác nhận.

File được đọc hết và kiểm tra trước: file hỏng không làm thay đổi gì. Sau đó nó **thay dữ liệu hiện có của database**. Hãy dừng các app đang ghi vào database và backup trước.

Khôi phục volume sẽ làm trống thư mục rồi giải nén bản backup vào đó: hãy dừng những gì đang ghi vào đó trước.

::: tip Lưu ý
- Quyền: `backups.run` để chạy backup ngay, `backups.restore` để khôi phục. Quản trị của project chỉ khôi phục được các bản backup của chính service đó.
- Lịch chạy theo giờ UTC. Các lần bị lỡ (lúc panel tắt) không được chạy bù.
- Mỗi lần chạy tối đa 3 giờ; mỗi service chỉ chạy một backup tại một thời điểm.
- Backup dùng được với container trên node của panel (xem [Chọn node chạy](/vi/apps/nodes)).
- Các kênh thông báo có thể nhận tin về backup và khôi phục ([Cài đặt](/vi/server/settings)).
:::
