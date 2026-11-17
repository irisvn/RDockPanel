# Import container có sẵn

Đưa các container đã chạy bằng `docker run` hoặc `docker compose` vào một project. Mỗi container thành một service bình thường của panel (deploy, rollback, biến môi trường, domain, backup, log, terminal, thành viên). Panel **tạo lại** chúng: đọc chúng, đưa ra kế hoạch, dừng bản gốc, chép dữ liệu và deploy các service của panel. Bản gốc được giữ tới khi bạn kết thúc, nên có thể hoàn tác toàn bộ lần import.

Chỉ quản trị viên (`system.manage`, kèm `projects.create` và `services.create`) mới import được.

## 1. Chọn thứ cần đưa vào

Mở import từ một trong các chỗ sau:

- **Nhập container** (Import containers) trên Bảng điều khiển hoặc trong một project;
- **Docker → Container**, menu **⋯** của một dòng, **Đưa vào panel** (Bring into the panel).

Hộp thoại liệt kê các container trên server theo project Compose (container chạy lẻ hiện là **Một container**). **Tìm** theo tên, rồi bấm **Đọc** (Read) ở project hoặc container cần đưa vào.

Panel chỉ đọc Docker, không thay đổi gì. Panel cũng lấy luôn các container phải đi cùng: những container dùng chung volume hay thư mục ghi được, được link với nhau, hoặc gọi nhau bằng tên (tối đa 50).

![Chọn container](/shots/import-source.png)

## 2. Xem kế hoạch

Trang của lần import hiện kế hoạch:

- **Tổng quan** (Overview): bao nhiêu container sẽ vào (**Sẽ đưa vào**), bao nhiêu **Bị chặn**, **Dữ liệu sẽ copy**, **Dung lượng trống** và **Thời gian dừng (ước tính)**. Đặt tên **Project** mới ở đây.
- **Thứ tự và quan hệ**: các tầng khởi động service (một tầng chỉ bắt đầu khi tầng trước đã chạy).
- **Container**: từng container với trạng thái. Phần **Chi tiết** (Details) hiện tên biến môi trường (không bao giờ hiện giá trị), port, domain, volume và các phát hiện, kèm nút **Để lại** (Leave it behind) hoặc **Đưa vào** (Bring it).
- **Cái gì mất hoặc đổi** (What is lost or changes): cái gì chặn, cái gì cần bạn xác nhận, cái gì bị mất.

Mỗi phát hiện có một mức:

| Mức | Nghĩa là |
|---|---|
| **Chặn** (Blocks) | Container chưa vào được cho tới khi hết nguyên nhân. Sửa xong thì bấm **Đọc lại**. |
| **Xác nhận** (Confirm) | Vào được, nhưng có thứ thay đổi; bắt đầu nghĩa là bạn chấp nhận. Container hiện **Cần xác nhận**. |
| **Mất** (Lost) | Thiết lập panel không mang sang. |
| **Note** | Thông tin cần biết, không phải làm gì. |

![Kế hoạch import](/shots/import.png)

### Các lựa chọn trong chi tiết container

- **Cách chạy** (How it runs): container PostgreSQL, MySQL, MariaDB hoặc MongoDB chính thức có thể vào thành **service database** (thông tin kết nối, dump, backup; image, cấu hình và dữ liệu giữ nguyên), hoặc thành app. Container `cloudflared` có thể thành **một tunnel của panel** (Cài đặt → Cloudflare Tunnel) thay vì một service.
- **Domain**: label Traefik và `VIRTUAL_HOST` thành domain của service, được chọn sẵn. Bỏ chọn để không lấy domain đó. Domain có basic auth của Traefik mang theo các user; domain có danh sách IP được phép thì giữ nguyên (thành một middleware hiện ở Cài đặt → Traefik). Domain có lớp bảo vệ khác bị bỏ ra, để trang được bảo vệ không bao giờ bị mở công khai: hãy bảo vệ service trong panel, rồi thêm domain.
- **Port**: publish như cũ. Port panel đang giữ (80, 443…) hoặc port chỉ mở trên loopback có thể mở ra qua cổng khác: **Mở {port} qua cổng** (Publish {port} on).
- **Health check**: mật khẩu viết trong đó được chuyển sang biến môi trường mà health check đọc. Khi panel không tách ra được, bấm **Bỏ health check**, hoặc nhập lệnh đọc secret từ biến môi trường (`"$NAME"`) rồi bấm **Dùng lệnh này**.

## 3. Bắt đầu

Bấm **Bắt đầu nhập…** (Start importing…) và gõ tên project để xác nhận. Sau đó panel:

1. dừng bản gốc (tắt restart policy của chúng trước);
2. kiểm tra còn đủ chỗ, rồi chép từng named volume sang volume của panel;
3. nối từng thư mục của server để service **dùng tại chỗ**;
4. đổi tên file Compose (thành `<tên>.rdockpanel-imported`) khi mọi container của project đó đều được đưa vào;
5. deploy các service theo từng tầng, và theo dõi chúng trong 60 giây.

Phần **Tiến trình** (Progress) hiện từng bước. Sau đó lần import ở trạng thái **Đang chạy** (Active): các service của bạn đã chạy trong panel.

![Một lần import đang chạy](/shots/import-detail.png)

## 4. Kết thúc, hoặc hoàn tác

- **Hoàn tác** (Undo, trước khi kết thúc): gỡ các service mới, trả lại file Compose, và chạy lại bản gốc. Dữ liệu service mới đã ghi vẫn nằm trong volume của chúng.
- **Kết thúc** (Finish): xoá các container gốc (không xoá volume). Từ lúc này không hoàn tác được nữa. Nếu Docker từ chối xoá, bấm **Kết thúc lại**.

Khi một bước lỗi sau khi khách đã có thể vào service mới, lần import chuyển sang **Cần bạn xử lý** (Needs you), với **Thử lại bước**, **Tiếp tục**, **Chấp nhận hiện trạng** (khi mọi service đều chạy) và **Hoàn tác**. Lỗi xảy ra trước đó thì tự hoàn tác.

## Sau khi import

- **Volume gốc** được giữ tới khi bạn gỡ: bấm **Xoá volume gốc** (Remove the original volume) trên trang import (bản chép vẫn còn).
- **Thư mục dùng tại chỗ**: **Chuyển toàn bộ** (Move all) ở tab Lưu trữ của service chép chúng vào volume; sau đó **Dọn thư mục cũ** xoá chúng đi (xem [Lưu trữ](/vi/apps/storage#thu-muc-dung-tai-cho)).
- Khi một lần import đang chạy hoặc đang chờ bạn, các service của nó không deploy, backup hay khôi phục được, và panel không cập nhật.
- Nếu bản gốc chạy lại (`docker compose up`, hoặc ai đó), panel sẽ dừng nó lại. Menu ⋯ có **Dừng các bản gốc chạy lại** và **Ngừng canh bản gốc**.

::: warning Lưu ý: những gì bị từ chối
Container đang tạm dừng hoặc đã chết, `--rm`, container privileged, mạng host, chế độ mạng dùng chung với container khác, entrypoint khác với của image, volume có driver hoặc option, hai container dùng chung một volume Docker, mount Docker socket, `/proc`, `/sys`, `/dev`, `/var/lib/docker` hoặc thư mục của panel, và các container của chính panel.
:::

::: tip Mẹo
- Được mang sang: image (theo digest), biến môi trường, named volume, thư mục, health check, command, user, capability, sysctl, group, giới hạn CPU, mount chỉ đọc, port, domain.
- Không mang sang (kế hoạch có ghi): shared memory, ulimit, tmpfs, device, extra host, DNS, thư mục làm việc, log driver, hostname, network phụ, restart policy (panel luôn khởi động lại).
- Mật khẩu trên dòng lệnh được chuyển sang biến môi trường (khi image chạy đúng lệnh được truyền vào).
- Service có thư mục dùng tại chỗ, hoặc dùng image chỉ có trên server này, vẫn chỉ quản trị viên được đụng tới và chạy trên chính server này.
:::
