# Người dùng và vai trò

Ai được đăng nhập vào panel, và mỗi người được xem, được làm gì. Quyền truy cập **mặc định là từ chối**: người dùng chỉ thấy những gì vai trò của họ, hoặc project của họ, cho phép.

## Ba cách cấp quyền

- **Super admin**: được làm mọi thứ. Installer tạo super admin đầu tiên (`--admin-email`).
- **Vai trò** (Hệ thống → Vai trò, tức System → Roles): tập hợp quyền áp dụng ở mọi nơi, cho mọi project.
- **Thành viên project**: người dùng được thêm vào một project, với vai trò viewer, developer hoặc admin của riêng project đó. Không cần vai trò hệ thống nào.

Quyền của một người được cộng dồn: quyền từ các vai trò của họ, cộng thêm, bên trong một project, quyền của vai trò trong project đó.

## Tạo người dùng

1. Mở **Hệ thống → Người dùng** (System → Users) và bấm **New user**.
2. Điền tên, email và mật khẩu (ít nhất 8 ký tự).
3. Gán cho họ một vai trò, hoặc để trống rồi thêm họ vào các project (xem [bên dưới](#thanh-vien-project)).
4. Lưu, rồi gửi cho họ địa chỉ panel và mật khẩu.

Trên trang Người dùng anh còn có thể đổi tên, vô hiệu hoá hoặc xoá người dùng, và đổi vai trò của họ. Super admin đang hoạt động cuối cùng thì không vô hiệu hoá hay xoá được.

![Người dùng](/shots/users.png)

## Vai trò

**Hệ thống → Vai trò** liệt kê các vai trò. Trang của một vai trò hiện các quyền theo nhóm (Ứng dụng và deploy, Docker, Server, Hệ thống), mỗi quyền có nhãn và mô tả, kèm ô tìm kiếm.

Để bắt đầu nhanh, áp dụng một **preset**, chỉnh lại rồi lưu:

| Preset | Cấp |
|---|---|
| **Xem ứng dụng** (Apps viewer) | xem mọi project và service |
| **Developer ứng dụng** (Apps developer) | sửa, deploy và chạy service của mọi project |
| **Quản trị ứng dụng** (Apps administrator) | thêm quyền xoá service, khôi phục database, chọn thành viên |
| **Giám sát** (Monitor) | xem server và Docker, không thay đổi gì (`server.view`, `docker.view`, `docker.logs`) |

Preset chỉ điền sẵn lựa chọn: chưa cấp gì cho tới khi anh lưu vai trò. Anh chỉ cấp được những quyền mà chính anh đang có.

![Vai trò](/shots/roles.png)

### Các quyền chính

Ngoài một bộ quyền cho mỗi loại bản ghi (`projects.*`, `services.*`, `service_domains.*`…: `view_any`, `view`, `create`, `update`, `delete`), panel có các quyền riêng:

| Quyền | Cho phép |
|---|---|
| `apps.access` | các trang ứng dụng (Bảng điều khiển, Domain, Hoạt động); thành viên project tự có |
| `services.deploy` | deploy, chạy, dừng, khởi động lại và rollback service |
| `services.destroy` | gỡ service khỏi Swarm (volume được giữ lại) |
| `services.secrets` | xem giá trị biến môi trường dạng rõ (mỗi lần xem đều ghi audit) |
| `services.logs`, `services.exec` | log và terminal của một service, từ trang của service |
| `projects.members` | thêm, sửa và xoá thành viên của project |
| `backups.run`, `backups.restore` | chạy backup ngay; khôi phục backup (thay dữ liệu hiện có) |
| `server.view` | số liệu server trên Bảng điều khiển |
| `docker.view`, `docker.logs` | các trang Docker; log container |
| `docker.manage`, `docker.exec` | thao tác trên container, image, volume, network; terminal container |
| `system.manage` | Cài đặt, Tường lửa, Traefik, chứng chỉ, import, container của chính panel, khởi động lại server |

Danh sách đầy đủ: [docs/permissions.md](https://github.com/irisvn/RDockPanel/blob/main/docs/permissions.md).

### `system.manage` dành cho quản trị viên

`system.manage` cho phép thay đổi chính panel và server. Quyền này cũng bao gồm những gì có thể chạm tới cả máy chủ: thư mục trên host, Compose service, thêm Linux capability, chọn node. Chỉ cấp cho người mà anh cũng sẵn sàng giao quyền root trên server.

::: warning Lưu ý
`docker.manage` và `docker.exec` cũng gần như quyền root: Docker điều khiển toàn bộ server, và shell trong đa số image chạy bằng root.
:::

## Thành viên project {#thanh-vien-project}

Admin của project thêm người vào project từ trang của project, nút **Thành viên** (Members): gõ email của người dùng (người dùng phải có sẵn), chọn vai trò, bấm **Thêm** (Add).

| Vai trò trong project | Được |
|---|---|
| **viewer** | xem project, các service, lần deploy, backup và log của chúng |
| **developer** | như viewer, và được sửa, thêm, deploy và rollback service; domain, port, mount, chuyển hướng, script và lịch backup của chúng; xem biến môi trường; mở terminal; chạy backup |
| **admin** | như developer, và được gỡ và xoá service, khôi phục database (từ backup của chính nó), đổi tên project, chọn thành viên |

Thành viên chỉ thấy project của mình, ở mọi nơi: Bảng điều khiển, Domain, Hoạt động, API. Khi hết là thành viên, log và terminal đang mở của họ sẽ đóng trong vài giây.

Project luôn giữ ít nhất một admin. `system.manage` không bao giờ do project cấp: Compose service và service đặc quyền vẫn thuộc về quản trị viên của panel.

## Lưu ý

- Quyền được kiểm tra trên server ở mọi request. Log trực tiếp và terminal được kiểm tra lại định kỳ (tối đa 30 giây một lần), nên khi bị gỡ quyền, đăng xuất hoặc đổi mật khẩu, chúng dừng lại sau ít giây.
- Mỗi người dùng tự chọn ngôn ngữ (tiếng Anh hoặc tiếng Việt) và giao diện sáng/tối cho mình.
- API token làm được những gì người dùng của nó được phép, không hơn: xem [API](/vi/reference/api).
- Mất mật khẩu của super admin duy nhất? Từ server, tạo thêm một super admin:

  ```sh
  docker exec "$(docker ps -q --filter label=com.docker.swarm.service.name=rdockpanel | head -n1)" \
    /app/rdockpanel make:user --email admin2@example.com --name Admin2 --super-admin
  ```

  Lệnh sẽ in ra mật khẩu mới.
