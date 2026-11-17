# App đầu tiên

Deploy một web app nhỏ từ một image, gán domain, mở nó và đọc log. Mất khoảng năm phút.

Ví dụ dùng `ghcr.io/traefik/whoami`, một web server rất nhỏ trả về thông tin của từng request ở
port 80.

## 1. Đăng nhập

Mở panel ở `/admin` (xem [Cài đặt](/vi/guide/install#first-sign-in)) và đăng nhập. Bạn vào
**Bảng điều khiển** (Dashboard): số liệu trực tiếp của server ở trên, các project ở dưới.

![Bảng điều khiển](/shots/dashboard.png)

## 2. Tạo project

Mỗi project gom các service của một app (web, database…), và có network riêng.

1. Bấm **Project mới** (New project).
2. Gõ tên: chữ thường, số và dấu gạch ngang, ví dụ `demo`.
3. Bấm **Tạo** (Create). Project hiện trên Bảng điều khiển.

## 3. Thêm service

1. Trên thẻ của project, bấm **Thêm service** (Add service).
2. Trong **Tạo service trong demo** (Create service in demo):
   - **Loại** (Type): giữ **App (Docker image)**;
   - **Tên service** (Service name): `web`;
   - **Docker image**: `ghcr.io/traefik/whoami`.
3. Bấm **Tạo** (Create). Trang của service sẽ mở ra.

![Service mới](/shots/new-service.png)

Trước khi tạo, dialog ước tính RAM service cần và cảnh báo khi server không đủ (xem
[Xử lý sự cố](/vi/reference/troubleshooting#not-enough-memory)).

## 4. Deploy

Service mới đã được lưu nhưng chưa chạy: tab Tổng quan ghi "Chưa deploy: bấm Deploy."

1. Bấm **Deploy** ở phần đầu trang service.
2. Lần deploy được xếp lịch rồi chạy: panel pull image, ghim theo digest và tạo service Swarm. Log
   trực tiếp nằm ở tab **Lần deploy** (Deployments).
3. Khi xong, trạng thái service chuyển sang đang chạy.

![Tổng quan service](/shots/service-overview.png)

## 5. Gán domain

1. Trỏ một bản ghi DNS (A) cho tên của bạn, ví dụ `whoami.example.com`, về server.
2. Mở tab **Domain** (Domains) và bấm **Thêm domain** (Add domain).
3. Điền:
   - **Host**: `whoami.example.com`;
   - **Path**: `/` (mặc định);
   - **Port của container** (Container port): `80` (port app lắng nghe bên trong container);
   - **HTTPS**: bật. HTTP sẽ tự chuyển sang HTTPS, chứng chỉ lấy từ Let's Encrypt.
4. Bấm **Thêm** (Add). Domain áp dụng ngay khi lưu: không cần deploy lại.

![Thêm domain](/shots/add-domain.png)

Nếu panel được cài với `--apps-domain`, nút **Thêm domain tự sinh** (Add generated domain) gán cho
service một tên như `demo-web.apps.example.com` mà không cần trỏ DNS (bản ghi wildcard đã lo).

::: tip Mẹo: trên Mac hoặc khi không có địa chỉ công khai
Dùng tên như `whoami.localhost`, truy cập ngay trên máy đó, và tắt HTTPS. Với domain thật, xem
[Cloudflare Tunnel](/vi/apps/tunnel).
:::

## 6. Mở app

Bấm **Mở** (Open) ở phần đầu trang service. whoami trả về thông tin request của bạn. Request HTTPS
đầu tiên có thể mất vài giây trong lúc chứng chỉ được cấp.

## 7. Xem log

Ở tab **Tổng quan** (Overview), phần **Giám sát** (Monitor) hiện CPU và RAM của service, có nút
**Log** (Logs: log trực tiếp của mọi container của service) và **Shell** (terminal bên trong
container). Tải lại trang web và xem các dòng mới hiện ra.

## Tiếp theo

- Đổi image hoặc biến môi trường, rồi bấm **Deploy** lần nữa: xem
  [Deploy và rollback](/vi/apps/deploy) và [Biến môi trường và secret](/vi/apps/environment).
- Thêm database vào cùng project: [Database và backup](/vi/apps/databases).
- Hoặc bắt đầu từ một trong hơn 800 app: [Nguồn và template](/vi/apps/sources-templates).
- Muốn xoá bản demo: ở tab **Nâng cao** (Advanced), phần **Vùng nguy hiểm** (Danger zone), bấm
  **Gỡ khỏi Docker** (Remove from Docker; volume được giữ), rồi **Xoá service** (Delete service);
  sau đó xoá project trống từ Bảng điều khiển.
