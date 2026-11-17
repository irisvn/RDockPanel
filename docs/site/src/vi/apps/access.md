# Chuyển hướng, bảo mật, bảo trì

Ba tab của service quyết định khách vào domain của nó sẽ gặp gì: **Chuyển hướng** (Redirects), **Bảo mật** (Security: xác thực cơ bản, giới hạn request và WAF) và **Bảo trì** (Maintenance). Mọi thứ ở đây áp dụng **ngay khi lưu**: route của Traefik được ghi lại, container vẫn chạy, không cần Deploy.

Database không có các tab này. Chúng chỉ tác động lên domain của service, không lên port đã publish.

## Chuyển hướng

Chuyển địa chỉ cũ sang địa chỉ mới, hoặc đưa mọi khách sang `www`.

1. Mở **Chuyển hướng** và bấm **Thêm chuyển hướng** (Add redirect).
2. **Biểu thức chính quy** (Regular expression): so khớp với toàn bộ URL của request.
3. **Thay bằng** (Replacement): URL mới; `${1}` là nhóm đầu tiên của biểu thức.
4. **Vĩnh viễn (301)** (Permanent): bật cho chuyển hướng vĩnh viễn, tắt cho chuyển hướng tạm thời.
5. Bấm **Thêm chuyển hướng**.

Mỗi chuyển hướng áp dụng cho mọi domain của service và có thể **Bật** (On) hoặc tắt ngay trong danh sách.

```text
Biểu thức chính quy   ^https?://example\.com/(.*)
Thay bằng             https://www.example.com/${1}
```

Biểu thức dùng cú pháp của Go (cú pháp Traefik đọc), chỉ nhận các cờ inline `i`, `m`, `s` và `U`.

![Tab Chuyển hướng](/shots/service-redirects.png)

## Bảo mật

### Xác thực cơ bản

Bắt khách nhập tên và mật khẩu trước khi vào service.

1. Mở **Bảo mật** và bấm **Thêm user** (Add user).
2. Gõ **User** và **Mật khẩu** (từ 8 ký tự). Mật khẩu được lưu dạng băm, không bao giờ hiện ra.

Không có user nào thì service **Ai cũng vào được** (Open to everyone). User vừa thêm, vừa gỡ hay vừa đổi mật khẩu có hiệu lực ngay.

### Giới hạn request {#rate-limit}

Giới hạn số request mỗi khách (theo địa chỉ IP) gửi tới các domain của service. Vượt giới hạn, khách nhận `429 Too Many Requests`.

- **Mức giới hạn** (Limit): **Tắt**, **Thường (website)** (trung bình 50 request/giây, dồn tối đa 200), **Chặt (API)** (10 request/giây, dồn tối đa 30) hoặc **Tuỳ chỉnh** (Custom).
- Với **Tuỳ chỉnh**: **Số request trung bình** mỗi **giây**, **phút** hoặc **giờ**, và **Dồn tối đa** (Burst: số request một khách được gửi dồn cùng lúc; trang có nhiều ảnh, script cần nhiều hơn).
- **Địa chỉ được miễn** (Exempt addresses): IP hoặc dải mạng không bị giới hạn, mỗi dòng một mục (VPN, máy monitoring).

Khách được đếm theo từng domain. Khách đi qua Cloudflare (bản ghi bật proxy hoặc Tunnel) được đếm theo IP thật của họ; khách IPv6 đếm theo dải `/64`. Địa chỉ được miễn chỉ khớp với khách kết nối thẳng tới server, không qua Cloudflare. Bộ đếm bắt đầu lại khi Traefik khởi động lại.

### WAF

**WAF** theo [tường lửa ứng dụng web](/vi/server/waf) của panel (**Theo panel**), hoặc đặt **Bật** hay **Tắt** riêng cho service này. **Tắt** là bỏ bảo vệ của service: chỉ quản trị viên panel được làm. Áp dụng ngay khi lưu.

![Tab Bảo mật](/shots/service-security.png)

## Bảo trì

Khi bật, domain của service hiện trang bảo trì (HTTP 503) thay cho service.

1. Mở **Bảo trì** và bật **Chế độ bảo trì** (Maintenance mode).
2. Chọn **Trang** (Page):
   - **Mặc định** (Default): trang bảo trì chung của panel (**Cài đặt → Trang bảo trì**) nếu có, nếu không thì trang có sẵn, theo ngôn ngữ của khách, sáng hoặc tối;
   - **HTML riêng** (Its own HTML): tự viết trang bằng HTML và CSS (script bị chặn; tối đa 256 KB). **Bắt đầu từ trang mặc định** điền sẵn nội dung vào ô soạn thảo.
3. Điền tuỳ ý: **Tiêu đề**, **Mô tả**, **Dự kiến trở lại** (Back at: lúc bạn dự kiến quay lại, theo giờ của bạn), **Liên hệ** (email hoặc link https), và **Logo** (PNG, JPEG hoặc WebP, tối đa 256 KB; không có thì dùng logo của panel).
4. **IP vẫn vào được site thật** (Addresses that still reach the service): IP hoặc dải mạng, mỗi dòng một mục (bạn, team, VPN). Những địa chỉ này vẫn thấy service như bình thường.
5. Xem **Xem trước** (Preview), rồi bấm **Lưu**. Panel hỏi xác nhận trước khi bật bảo trì.

Khi tắt, domain dẫn lại vào service ngay.

![Tab Bảo trì](/shots/service-maintenance.png)

::: tip Lưu ý
- Thứ tự trên mỗi request: giới hạn request, rồi chuyển hướng, rồi xác thực cơ bản.
- Đi qua Cloudflare, danh sách IP vẫn vào được chỉ khớp IP lẻ, không khớp dải mạng.
- HTML riêng có thể chèn giá trị như `{{title}}`, `{{eta}}` hay `{{contact}}`; ô soạn thảo liệt kê chúng.
- Với các tính năng khác của Traefik (headers, danh sách IP được phép), xem [Middleware Traefik](/vi/apps/domains#middleware-traefik).
:::
