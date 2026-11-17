# Cloudflare Tunnel

Vào các domain của bạn qua Cloudflare mà không cần mở cổng 80 và 443, cũng không cần IP public. Panel chạy `cloudflared` của Cloudflare cạnh Traefik.

## Khi nào nên dùng

- Server nằm sau router ở nhà hay văn phòng, hoặc nhà cung cấp không cho địa chỉ public (ví dụ một máy Mac ở nhà).
- Bạn muốn giữ cổng 80 và 443 đóng với internet.
- Khi đó Cloudflare lo HTTPS cho khách truy cập.

Các domain phải nằm trên Cloudflare (DNS do Cloudflare quản lý).

## Cài đặt

Chỉ quản trị viên (`system.manage`) thấy tab này.

1. Trong Cloudflare: **Zero Trust → Networks → Tunnels → Create a tunnel** (cloudflared). Copy token của nó.
2. Trong panel: **Cài đặt → Cloudflare Tunnel → Thêm tunnel** (Settings → Cloudflare Tunnel → Add tunnel). Đặt tên (chữ thường, số và dấu gạch ngang, bắt đầu bằng chữ cái) và dán **Token của tunnel** (Tunnel token).
3. Chờ trạng thái chuyển sang **Đã kết nối** (Connected).
4. Trong menu **⋯** của tunnel, chọn **Hostname…** (Hostnames…). Ở đó có **Service (origin)** mà mọi hostname phải trỏ tới (ví dụ `http://rdp-traefik:8081`).
5. Trong Cloudflare, với mỗi domain, thêm một **public hostname** cho tunnel với service đó.
6. Trong panel, thêm đúng domain đó cho service ở [tab Domain](/vi/apps/domains), **tắt HTTPS**.

![Cài đặt Cloudflare Tunnel](/shots/settings-tunnel.png)

## Nhiều tunnel

Mỗi tunnel chạy một cloudflared riêng với token riêng: mỗi token một tunnel, ví dụ khi bạn có nhiều tài khoản Cloudflare. Trên mỗi dòng:

- **Tắt** / **Bật** (Turn off / Turn on): khi tắt, các domain đi qua tunnel đó chỉ còn vào được qua cổng 80 và 443;
- menu **⋯**: **Hostname…**, **Thay token…** (Replace token…) và **Xoá…** (Remove…: cloudflared của nó dừng và token bị xoá).

Trạng thái của từng tunnel là **Đang khởi động** (Starting), **Đã kết nối** (Connected), **Chưa kết nối** (Not connected) hoặc **Tắt** (Off).

## HTTPS và địa chỉ của khách

- Đi qua tunnel thì Cloudflare lo HTTPS: chứng chỉ riêng của domain không được dùng. Khách vào bằng HTTP thường được Cloudflare chuyển sang HTTPS.
- Form domain đánh dấu các tên trỏ về Cloudflare (**qua Cloudflare**) và có nút **Tắt HTTPS**.
- App nhận địa chỉ của khách qua header `Cf-Connecting-Ip` của Cloudflare. [Giới hạn request](/vi/apps/access#rate-limit) đếm từng khách theo địa chỉ thật của họ.

::: tip Lưu ý
- Token được mã hoá trong panel và đưa cho cloudflared dưới dạng Docker secret, không bao giờ nằm trên dòng lệnh.
- Địa chỉ của chính panel cũng đi qua tunnel được, khi panel đã có domain.
- Container `cloudflared` bạn tự chạy có thể thành một tunnel của panel khi bạn [import](/vi/apps/import) nó.
:::
