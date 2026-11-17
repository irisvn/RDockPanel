# Bảo mật

RDockPanel làm gì để giữ server của bạn an toàn, và phần nào bạn phải tự lo.

::: warning Lưu ý: panel điều khiển Docker, mà Docker là root
Panel nói chuyện với Docker qua socket của nó, và ai điều khiển được Docker là điều khiển được
server. Hãy coi tài khoản được quản lý container (`docker.manage`) hoặc mở terminal (`docker.exec`)
như quyền root.
:::

## Panel làm gì

**Mặc định là từ chối.** Người dùng chỉ làm được những gì vai trò của họ cho phép, thành viên
project chỉ làm được những gì vai trò trong project cho phép, trong project của họ. Mọi trang và
route kiểm tra quyền trước khi đọc request; log trực tiếp và terminal kiểm tra lại sau mỗi vài giây
và dừng ngay khi mất quyền hoặc người dùng đăng xuất. Xem [Người dùng và vai trò](/vi/server/users).

**Đối tượng của panel được bảo vệ.** Panel, database của nó và Traefik được gắn nhãn
`rdockpanel.system=true`. Không có quyền `system.manage` thì chúng bị ẩn khỏi mọi danh sách và không
xem chi tiết, sửa hay prune được.

**Thao tác chỉ dành cho quản trị viên** cần `system.manage`: Tường lửa (Firewall); các tab Panel,
Phiên bản, Traefik, Chứng chỉ, Cloudflare Tunnel, Thương hiệu, Trang bảo trì và Git của Cài đặt;
service Compose; thư mục host; thêm capability, sysctl và group; domain wildcard và chọn chứng chỉ;
import container; chọn node chạy service.

**Secret được mã hoá.** Biến môi trường, mật khẩu registry, nội dung file mount, nửa private của
deploy key, secret của webhook và token Cloudflare Tunnel được mã hoá bằng key của panel
(`APP_KEY`). Giá trị chỉ hiện cho người có `services.secrets`, và mỗi lần đọc đều được ghi vào audit
log trước. Mật khẩu registry không bao giờ hiện lại. Phần chi tiết của container ẩn các giá trị có
tên giống secret, mật khẩu trong URL và trên dòng lệnh.

**Mọi thứ đều được ghi lại.** Thay đổi dữ liệu của panel vào audit log (**System → Audit log**),
thao tác Docker vào **Hoạt động** (Actions), kèm người thực hiện; mỗi lần deploy có lịch sử và log
riêng.

**Các project tách biệt.** Mỗi project có network riêng: service của project này không gọi được
service của project khác, cũng không gọi được database của panel. Những gì service công khai (domain,
port publish) thì ai cũng truy cập được.

**[Firewall](/vi/server/firewall) chặn mặc định** (server Linux) đóng mọi port trừ SSH, 80, 443 và
các port bạn mở; mỗi thay đổi tự hoàn tác sau hai phút nếu bạn không giữ lại.

**Token và URL** chỉ hiện một lần và chỉ lưu dạng hash khi chỉ cần kiểm tra (URL kích hoạt deploy);
chữ ký của webhook GitHub được kiểm tra mỗi lần gửi tới. API token hết hạn trong vòng một năm và
không bao giờ vượt quá scope của nó.

## Bạn cần tự làm

- **Gán domain cho panel** để panel chỉ chạy HTTPS (port 3000 bị đóng).
- **Dùng mật khẩu mạnh, không trùng.** Xác thực hai lớp chưa có.
- **Cấp quyền tối thiểu** mỗi người cần. Ưu tiên thành viên project hơn vai trò toàn cục.
- **Cập nhật** panel khi có bản mới ([Cập nhật](/vi/guide/update)), và cả các gói của server.
- **Backup** `/etc/rdockpanel` (chứa `APP_KEY`) và database của panel, cất ở ngoài server. Mất
  `APP_KEY` là mất mọi giá trị đã mã hoá.
- **Để secret trong biến môi trường**, không để trong command hay health check của service (những
  chỗ đó được lưu đúng như gõ).
- **Đọc kỹ template và repo trước khi deploy**: image và bản build của chúng chỉ đáng tin bằng tác
  giả của chúng.
- **Xem lại** audit log và trang Hoạt động thỉnh thoảng.

Chi tiết cho nhà phát triển nằm trong
[docs/security.md](https://github.com/irisvn/RDockPanel/blob/main/docs/security.md).
