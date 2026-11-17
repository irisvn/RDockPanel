# Docker

Các trang Docker cho thấy mọi thứ Docker đang chạy trên server, không chỉ các service của panel: container, image, volume và network. Trang **Hoạt động** (Actions) giữ lịch sử những gì đã làm.

## Số liệu server trên Bảng điều khiển

Với quyền `server.view`, Bảng điều khiển (Dashboard) mở đầu bằng các ô số liệu trực tiếp của server: **CPU**, **RAM** (Memory), **Ổ đĩa** (Disk), **Mạng vào** (Network in) và **Mạng ra** (Network out), mỗi ô kèm lịch sử gần đây. Các ô tự cập nhật khi trang đang mở.

## Container

Trang **Container** (Containers) liệt kê mọi container trên server này, kể cả container của chính panel (nếu anh được phép xem, xem [bên dưới](#doi-tuong-cua-chinh-panel)). Danh sách tự làm mới sau vài giây.

- **Tìm** (Search) theo tên hoặc image; tick **Chỉ đang chạy** (Running only) để ẩn các container đã dừng.
- Mỗi lần một service được deploy lại, Docker giữ các container đã dừng của task cũ. Chúng được ẩn đi; tick **Hiện {n} task cũ** (Show {n} old tasks) để xem.
- Mỗi dòng có tên, image, trạng thái và các port được publish. Nút bấm tuỳ trạng thái: **Dừng** (Stop), **Chạy** (Start) hoặc **Chạy tiếp** (Unpause), sau đó là biểu tượng log và terminal.
- Menu **⋯** có **Khởi động lại** (Restart), **Tạm dừng** (Pause), **Inspect**, **Đưa vào panel** (Bring into the panel, cho quản trị viên, xem [Import](/vi/apps/import)) và **Xoá…** (Remove…).

Xoá một container sẽ dừng nó trước nếu nó đang chạy. Nếu đó là task của một Swarm service, Swarm sẽ chạy một task mới: muốn dừng hẳn một service, hãy dùng trang của service đó.

![Container](/shots/containers.png)

### Trang của một container

Bấm vào tên container để mở. Các tab:

| Tab | Nội dung |
|---|---|
| **Log** (Logs) | Các dòng mới nhất (anh chọn bao nhiêu dòng), rồi các dòng mới khi bật **Theo dõi** (Follow). Dòng ghi ra stderr được tô nổi. |
| **Số liệu** (Stats) | CPU, RAM, mạng và ổ đĩa, trực tiếp (chỉ container đang chạy). |
| **Terminal** | Một shell trong container, ngay trên trình duyệt (chỉ container đang chạy). Tự đóng sau 30 phút không hoạt động. |
| **Inspect** | Toàn bộ tài liệu Docker của container. |

**Inspect** ẩn các giá trị bí mật: biến môi trường có tên giống secret (`PASSWORD`, `TOKEN`, `KEY`, `SECRET`, `AUTH`…), mật khẩu trong URL (`mysql://user:******@host`), mật khẩu trên dòng lệnh, trong health check và trong label. Panel chỉ nhận ra các dạng quen thuộc, nên hãy để secret trong biến môi trường.

![Log container](/shots/container-logs.png)

## Image

Trang **Image** (Images) liệt kê các image trên server cùng tag, dung lượng và việc có container nào dùng hay không (**Đang chạy** (Running), **Chỉ còn container đã dừng** (Stopped containers only), **Không dùng** (Unused)).

- **Pull image** (Pull an image): gõ tên, ví dụ `traefik/whoami:latest` (Docker Hub hoặc registry bất kỳ), rồi bấm **Pull**. Tiến trình hiện trực tiếp.
- **Xoá…** (Remove…) gỡ một tag. Image vẫn còn container dùng thì cần tick **Gỡ bắt buộc (cả khi đang dùng)** (Force (also when in use)).
- **Dọn dẹp** (Prune) xoá các container đã dừng và các image lơ lửng (dangling: layer không còn tag nào trỏ tới). Của panel thì được giữ lại.

![Image](/shots/images.png)

## Volume và network

Trang **Volume** (Volumes) liệt kê các volume Docker (tên, driver, mount point). **Volume mới** (New volume) tạo một volume; **Xoá…** (Remove…) xoá volume, và dữ liệu trong đó mất hẳn.

![Volume](/shots/volumes.png)

Trang **Network** (Networks) liệt kê các network Docker. Mỗi project có network riêng, Traefik được gắn vào từng network đó. **Network mới** (New network) tạo network **bridge** (chỉ server này) hoặc **overlay** (cả Swarm). Network có sẵn của Docker (`bridge`, `host`, `none`…) không xoá được.

## Đối tượng của chính panel {#doi-tuong-cua-chinh-panel}

Panel, database của panel, Traefik và Cloudflare Tunnel của panel cũng là các đối tượng Docker, mang label `rdockpanel.system=true`. Không có quyền `system.manage` thì:

- chúng không hiện trong danh sách nào;
- xem log, số liệu, terminal, inspect và mọi thao tác trên chúng đều bị từ chối;
- không xoá được image mà một trong số chúng đang chạy;
- dọn dẹp (prune) luôn bỏ qua chúng, dù ai chạy.

## Hoạt động: lịch sử thao tác

Trang **Hoạt động** (Actions, trong menu Không gian làm việc) liệt kê những gì panel vừa làm, mới nhất ở trên: deploy, backup và khôi phục, các thao tác Docker (chạy, dừng, xoá, pull, dọn dẹp, terminal…). Lọc theo **Loại** (Kind): **Tất cả** (All), **Lần deploy** (Deployments), **Backup**, **Docker**.

- **Mở** (Open) ở một lần deploy hoặc backup sẽ đưa anh tới tab tương ứng của service.
- **Log** ở một thao tác Docker hiện log của nó, kèm lỗi nếu thất bại.
- Trang giữ 100 thao tác gần nhất; deploy và backup cũ hơn vẫn nằm trong lịch sử của từng service.

Mọi thao tác Docker cũng được ghi vào audit log (Hệ thống → Audit log), kèm người thực hiện.

![Hoạt động](/shots/actions.png)

## Ai được làm gì

| Quyền | Cho phép |
|---|---|
| `server.view` | số liệu server trên Bảng điều khiển |
| `docker.view` | các trang Container, Image, Volume và Network; inspect và số liệu trực tiếp |
| `docker.logs` | log container |
| `docker.manage` | chạy, dừng, khởi động lại, tạm dừng, xoá container; pull và xoá image; dọn dẹp; tạo và xoá volume, network |
| `docker.exec` | terminal của container |
| `system.manage` | đối tượng của chính panel; xoá bắt buộc image đang dùng; xem thao tác của mọi người dùng |

Trang Hoạt động mở được với quyền `projects.view_any` hoặc khi là thành viên của một project (`apps.access`). Xem [Người dùng và vai trò](/vi/server/users).

::: warning Lưu ý
Docker điều khiển toàn bộ server. `docker.manage` và `docker.exec` gần như là quyền root (shell trong đa số image chạy bằng root): chỉ cấp cho người anh tin tưởng giao cả server.
:::

## Lưu ý

- Các trang này tác động thẳng lên Docker. Muốn đổi một app, hãy dùng trang service của nó: container sửa ở đây sẽ bị thay ở lần deploy sau.
- Log và terminal của một service cũng có trên trang của service, với quyền `services.logs` và `services.exec`, không cần vào các trang Docker. Xem [Log, terminal, script](/vi/apps/logs-terminal-scripts).
- Cài đặt → Panel có thêm các mục dọn dẹp (container đã dừng, image không dùng, build cache, volume không dùng). Xem [Cài đặt](/vi/server/settings#panel).
