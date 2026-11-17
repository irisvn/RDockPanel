# Domain và HTTPS

Gắn domain cho service, Traefik (proxy đứng trước mọi service) sẽ đưa khách truy cập vào service, kèm chứng chỉ HTTPS. Thay đổi domain có hiệu lực ngay khi bạn lưu.

## Trước tiên: DNS

Ở nhà cung cấp DNS, trỏ tên miền về server:

- **bản ghi A** `app.example.com` → IPv4 public của server (ví dụ `203.0.113.10`), và **bản ghi AAAA** nếu server có IPv6;
- với **domain tự sinh** (`<project>-<service>.apps.example.com`), cần một bản ghi wildcard `*.apps.example.com` → server, và domain cho app đã đặt lúc cài (Cài đặt → Panel).

Nếu server nằm sau router ở nhà, không có địa chỉ public, hãy dùng [Cloudflare Tunnel](/vi/apps/tunnel).

## Thêm domain

1. Mở tab **Domain** của service và bấm **Thêm domain** (Add domain).
2. Điền:
   - **Host**: `app.example.com`, hoặc `*.example.com` cho mọi tên bên dưới (chỉ quản trị viên);
   - **Path**: `/` cho cả site, hoặc một tiền tố như `/api`;
   - **Port của container** (Container port): port app lắng nghe bên trong container (mặc định 80; không phải port đã publish);
   - **HTTPS**: mặc định bật; khi đó HTTP tự chuyển sang HTTPS;
   - **Chứng chỉ** (Certificate, khi bật HTTPS, cho quản trị viên): xem bên dưới;
   - **Middleware Traefik** (Traefik middlewares): không bắt buộc, xem bên dưới.
3. Bấm **Thêm**. Domain được áp dụng ngay, khi service đang chạy.

**Thêm domain tự sinh** (Add generated domain) thêm `<project>-<service>.<domain cho app>` chỉ bằng một cú bấm, khi đã đặt domain cho app.

Domain đầu tiên là domain **chính** (Primary): nút **Mở** (Open) và `$(PRIMARY_DOMAIN)` dùng nó. Không có domain thì service chỉ truy cập được bên trong project.

![Tab Domain](/shots/service-domains.png)

![Thêm domain](/shots/add-domain.png)

## Chứng chỉ

Khi bật HTTPS, chứng chỉ lấy từ:

- **Mặc định của panel** (The panel's default): Let's Encrypt qua HTTP, do installer đặt. Tên miền phải trỏ về server này qua cổng 80.
- **Một certificate resolver** ở **Cài đặt → Chứng chỉ** (Settings → Certificates): Let's Encrypt qua HTTP, hoặc qua **DNS** bằng token của Cloudflare, DigitalOcean, Hetzner, Vultr hoặc Linode. Sau khi thêm hay sửa resolver, bấm **Áp dụng** (Apply) ở đó (Traefik khởi động lại, vài giây không định tuyến). Resolver DNS là cách duy nhất để có chứng chỉ cho wildcard (`*.example.com`), và dùng được cho server không mở cổng 80 ra ngoài.
- **Một chứng chỉ tải lên** (Cài đặt → Chứng chỉ → **Tải chứng chỉ lên**): chứng chỉ và khoá riêng của bạn dạng PEM. Panel kiểm tra chúng, mã hoá khoá, và báo các kênh thông báo trước 14, 7 và 1 ngày khi chứng chỉ sắp hết hạn.

Chọn resolver hay chứng chỉ tải lên cho domain cần quyền `system.manage`, domain wildcard cũng vậy. Mỗi host chỉ có một chứng chỉ: mọi domain HTTPS cùng host dùng cùng một lựa chọn.

::: warning Đi qua Cloudflare
Khi tên miền trỏ về Cloudflare, panel đánh dấu **qua Cloudflare**. Nếu đi qua Cloudflare Tunnel, Cloudflare đã lo HTTPS cho khách: tắt HTTPS cho domain đó (form có nút **Tắt HTTPS**). Nếu là proxy của Cloudflare về server này (chế độ Full), giữ HTTPS và dùng resolver DNS hoặc chứng chỉ tải lên.
:::

## Middleware Traefik

Ô **Middleware Traefik** nhận danh sách middleware cách nhau bằng dấu phẩy, định nghĩa trong file của Traefik, ví dụ `custom-headers@file`. Quản trị viên tự viết middleware ở **Cài đặt → Traefik** (tên bắt đầu bằng `custom-`; headers, giới hạn, danh sách IP được phép, nén…). Chúng chạy sau các middleware của panel (chuyển hướng, xác thực). Với giới hạn request và mật khẩu, nên dùng [tab Bảo mật](/vi/apps/access) của service.

## Trang Domain

Mục **Domain** ở thanh bên liệt kê domain của mọi service trong một bảng: host, nơi nó trỏ tới (project, service và port) và trạng thái, ví dụ **Đang chạy** (Live) hay **Trang bảo trì**. Bạn cũng thêm, sửa, xoá domain ở đây (**Thêm domain** sẽ hỏi chọn service), và lọc theo project. Domain của chính panel hiện là **RDockPanel (Settings → Panel)**.

![Trang Domain](/shots/domains.png)

::: tip Lưu ý
- Domain, HTTPS và chứng chỉ áp dụng ngay khi lưu: không cần Deploy, không khởi động lại container. Gỡ domain thì nó ngừng phục vụ ngay.
- Domain của service Compose chỉ tới một service trong file (**Service trong Compose**) và áp dụng ở lần Deploy tới.
- Host của chính panel không bao giờ làm domain cho service được.
- Mỗi cặp host và path thuộc về một domain: hai service không dùng chung được.
:::
