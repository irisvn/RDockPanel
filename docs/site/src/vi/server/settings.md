# Cài đặt

Trang **Cài đặt** (Settings, trong menu Không gian làm việc) chứa phần cấu hình của chính panel, proxy và chứng chỉ, nơi lưu backup, thông báo và API token của anh. Mỗi tab chỉ hiện với người có quyền tương ứng:

| Tab | Quyền |
|---|---|
| Panel, Phiên bản, Traefik, Chứng chỉ, Cloudflare Tunnel, Thương hiệu, Trang bảo trì, Git | `system.manage` |
| Backup | `storage_providers.view_any` |
| Thông báo | `notification_channels.view_any` |
| API token | không cần: mỗi người dùng có token riêng |

## Panel {#panel}

**Địa chỉ panel** (Panel address) cho thấy domain của panel, email Let's Encrypt và domain cho app:

- **Domain panel** (Panel domain): nơi panel trả lời qua HTTPS. Để trống: panel trả lời tại `http://<server-ip>:3000`. Hãy trỏ DNS của domain (bản ghi A) về server trước.
- **Email Let's Encrypt**: tài khoản dùng cho chứng chỉ của panel và của các app.
- **Domain cho app** (Apps' domain): domain tự sinh có dạng `<project>-<service>.<domain này>`; cần một bản ghi DNS wildcard `*.<domain này>` trỏ về server. Các domain đã có vẫn giữ nguyên.

Các giá trị này do installer thay đổi. Điền vào, bấm **Kiểm tra** (Check: DNS, tên đã bị dùng chưa), rồi copy **Lệnh installer** (Installer command) và chạy trên server bằng root. Panel khởi động lại ở địa chỉ mới; installer giữ nguyên mọi thứ khác.

**Trang chủ** (Homepage): **Hiện trang chủ** (Show the homepage) đặt một trang giới thiệu RDockPanel tại địa chỉ của panel (`/`), không có thông tin gì của server này. Tắt: địa chỉ đó mở trang đăng nhập.

Mục **Hệ thống** (System) bên dưới liệt kê những gì panel đang chạy trên đó, kèm thao tác:

- **RDockPanel**: phiên bản, log, **Khởi động lại…** (Restart…).
- **Server**: hostname và địa chỉ public, **Lấy lại IP** (Refresh IP), **Khởi động lại server…** (Reboot…; gõ hostname của server để xác nhận; mọi service dừng cho tới khi server chạy lại).
- **Traefik**: log và **Khởi động lại…** (domain ngừng trả lời vài giây).
- **Docker**: dọn những gì không dùng tới, không bao giờ đụng tới của panel: **Dọn container đã dừng**, **Dọn image không dùng**, **Dọn build cache**, **Dọn volume không dùng** (dữ liệu của service do panel quản lý và của Compose project luôn được giữ).

![Cài đặt, tab Panel](/shots/settings-panel.png)

## Phiên bản

Panel đang chạy bản nào (phiên bản, image, và các thành phần đi kèm: Core, Traefik, MySQL hoặc SQLite, Docker, hệ điều hành), và đã có bản mới chưa. Panel kiểm tra 6 tiếng một lần (hoặc bấm **Kiểm tra ngay** (Check now)) và báo ở đây cũng như trên Bảng điều khiển, kèm ghi chú phát hành.

Panel **không bao giờ tự cập nhật**: quản trị viên bấm **Cập nhật** (Update). Deploy và backup mới phải chờ, database của panel được backup, rồi Swarm thay panel (panel tạm không truy cập được khoảng một phút; các service của anh vẫn chạy). Nếu bản mới không khởi động được, Swarm quay về bản cũ. **History** liệt kê các lần đổi phiên bản và khởi động lại panel, còn **Xoá bản cũ…** (Remove old versions…) giải phóng ổ đĩa. Xem [Cập nhật và sao lưu panel](/vi/guide/update).

## Traefik

Traefik là proxy đứng trước mọi domain. Tab này cho thấy trạng thái (service, image, task, port), nút **Khởi động lại** (Restart) và **Log** trực tiếp.

**Middleware tuỳ chỉnh (YAML)** (Custom middlewares) để thêm middleware Traefik của riêng anh: header, giới hạn tốc độ, chuyển hướng, prefix, nén… Tên phải bắt đầu bằng `custom-`, và domain dùng nó dưới dạng `custom-<tên>@file`. Không cho phép `forwardAuth`, `errors` và plugin.

**Danh sách IP được phép từ import** (Imported IP allow lists) là các allow list mà việc [import](/vi/apps/import) container tạo ra cho domain của nó; mỗi danh sách tự bị gỡ khi không còn domain nào dùng.

![Cài đặt, tab Traefik](/shots/settings-traefik.png)

## Chứng chỉ

- **Certificate resolver**: cách Traefik lấy chứng chỉ. Thêm Let's Encrypt qua **HTTP** (domain phải tới được server này ở port 80) hoặc qua **DNS** (dùng được cho wildcard) với token của nhà cung cấp DNS: Cloudflare, DigitalOcean, Hetzner, Vultr hoặc Linode. Token được niêm phong (mã hoá). Bấm **Áp dụng** (Apply) để Traefik dùng chúng (Traefik khởi động lại, vài giây không định tuyến). Mỗi domain chọn resolver trong hộp thoại của nó, không chọn thì dùng resolver mặc định của panel.
- **Chứng chỉ tải lên** (Uploaded certificates): chứng chỉ của riêng anh (của một CA, hoặc wildcard): chứng chỉ kèm các chứng chỉ trung gian (`fullchain.pem`) và khoá riêng (`privkey.pem`, không mã hoá). Panel kiểm tra nó, niêm phong khoá, và dùng cho các tên mà chứng chỉ bao phủ. Các kênh thông báo được báo trước 14, 7 và 1 ngày khi chứng chỉ sắp hết hạn.

Xem [Domain và HTTPS](/vi/apps/domains).

## Cloudflare Tunnel

Cho domain đi qua Cloudflare mà không cần mở port 80 và 443: thêm tunnel với token của nó, bật hoặc tắt, xem các hostname. Xem [Cloudflare Tunnel](/vi/apps/tunnel).

## Thương hiệu

Tên và màu chính của panel, logo (sáng và tối) và favicon (PNG, WebP hoặc ICO, tối đa 512 KB mỗi file, không nhận SVG). Chúng hiện trên mọi trang và trang đăng nhập, không cần khởi động lại (tải lại trang để thấy). Để trống: dùng của RDockPanel.

## Trang bảo trì

Trang hiện cho mọi service đang bảo trì mà dùng trang mặc định: để trống thì là trang có sẵn, hoặc HTML của riêng anh với nút **Xem trước** (Preview). **Liên hệ mặc định** (Default contact: một email hoặc link https) hiện cho các service không ghi liên hệ riêng. Mỗi service cũng có thể có trang riêng: xem [Chuyển hướng, bảo mật, bảo trì](/vi/apps/access).

## Git

GitHub token để clone repository private. **Thêm GitHub token** (Add GitHub token): một fine-grained token có quyền đọc Contents và Metadata (hoặc classic token có `repo`), cùng các **project được dùng token này**. GitHub kiểm tra token trước; sau khi lưu, token không bao giờ hiện lại. Developer của các project đó có thể liệt kê và clone mọi repository mà token tới được. Xem [Nguồn và template](/vi/apps/sources-templates).

## Backup

**Nơi lưu trữ backup** (Storage providers): nơi ghi backup của database và volume.

- **Thư mục trên server** (Local directory): một thư mục tuyệt đối trên server. Với panel đã cài, hãy để nó trong `/var/lib/rdockpanel`.
- **Tương thích S3** (AWS, Cloudflare R2, MinIO…): endpoint, region, bucket, access key và secret key. Tick **URL kiểu path** (Path-style URLs) với MinIO và đa số dịch vụ tương thích S3.

**Thử** (Test) ghi, liệt kê rồi xoá một file nhỏ ở đó. Nơi lưu trữ đang có backup dùng thì không xoá được; xoá một nơi lưu trữ thì các file của nó vẫn nằm nguyên chỗ cũ. Xem [Database và backup](/vi/apps/databases).

![Cài đặt, tab Backup](/shots/settings-backups.png)

## Thông báo

Các kênh nhận tin khi có chuyện xảy ra: **Discord**, **Slack**, **Telegram**, **Email (SMTP)** hoặc **Webhook**. Bấm **Thêm kênh** (Add channel), rồi tick các sự kiện:

- deploy (và mọi lần thất bại), backup và khôi phục, ổ đĩa dùng vượt một ngưỡng;
- service đang chạy bị lỗi và khi nó chạy lại, Traefik ngừng và khi chạy lại;
- chứng chỉ tải lên sắp hết hạn, có bản RDockPanel mới;
- mặc định tắt: Docker dọn dẹp (prune), đăng nhập từ một địa chỉ mới.

**Gửi thử** (Send test) để kiểm tra một kênh.

## API token

Token riêng của anh cho [API](/vi/reference/api) và dòng lệnh `rdp`. **Token mới** (New token): một tên, các **scope** (`read`, `deploy`, `secrets`…) và thời hạn tính bằng ngày (tối đa 365). Copy ngay: token không hiện lại. Token làm được những gì anh được phép, không bao giờ vượt ngoài scope của nó. **Thu hồi…** (Revoke…) để vô hiệu hoá.

## Lưu ý

- Thay đổi Traefik (resolver, khởi động lại) và khởi động lại panel sẽ làm domain hoặc panel gián đoạn vài giây: nên làm lúc ít người dùng.
- Các giá trị bí mật nhập ở những tab này (secret key, token, mật khẩu) được lưu mã hoá và không bao giờ hiện lại. Khi sửa, để trống ô đó để giữ giá trị đã lưu.
