# Log, terminal, script

Xem service in ra gì, mở shell trong container của nó, và lưu những lệnh hay chạy. Cả ba đều nằm trên trang service; không cần quyền vào các trang Docker.

## Log trực tiếp

1. Mở tab **Tổng quan** (Overview) của service.
2. Trong **Giám sát** (Monitor), bấm **Log**.
3. Chọn **Số dòng** (Lines) cần tải (100, 500 hoặc 2000), và để **Theo dõi** (Follow) bật để thấy dòng mới khi chúng xuất hiện.

Các dòng ghi ra stderr được tô nổi bật. Phần **Giám sát** cũng vẽ biểu đồ CPU và RAM của service theo thời gian thực.

Log riêng của mỗi lần deploy (pull, build, khởi động) nằm ở tab **Lần deploy** (Deployments) (xem [Deploy](/vi/apps/deploy)).

![Log](/shots/service-logs.png)

## Terminal

Bấm **Shell** trong **Giám sát** để mở shell trong một container đang chạy của service, ngay trên trình duyệt.

- **Phiên mới** (New session) mở một shell mới.
- Terminal tự đóng sau 30 phút không hoạt động, và trong vài giây khi bạn mất quyền hoặc đăng xuất.
- Mỗi phiên được ghi vào nhật ký hoạt động và audit log, kèm người mở và thời lượng. Những gì bạn gõ và output không bao giờ được ghi lại.

::: warning Lưu ý
Với đa số image, shell chạy bằng root bên trong container. Chỉ cấp quyền terminal cho người bạn tin tưởng với dữ liệu và secret của app.
:::

![Terminal](/shots/service-terminal.png)

## Script

Tab **Script** (Scripts) lưu các lệnh có tên mà bạn hay chạy, ví dụ một migration hay lệnh xoá cache.

1. Bấm **Thêm script** (Add script), đặt **Tên** và **Command** (ví dụ `php artisan migrate --force`), rồi lưu.
2. Bấm **Chạy** (Run) trên dòng của nó. Output và **mã thoát** (exit code) hiện ra khi chạy xong.

Script chạy bằng `sh -c` trong một container đang chạy của service:

- tối đa **25 giây**, quá thời gian sẽ bị dừng;
- output hiển thị là 64 KB cuối cùng;
- mỗi lần chạy được ghi audit theo tên script (không bao giờ ghi output).

Database và service Compose không có tab Script. Việc chạy lâu hơn thì dùng terminal.

![Tab Script](/shots/service-scripts.png)

## Ai được làm gì

| Để | Cần |
|---|---|
| Đọc log của service | `services.logs` (trong project của nó), hoặc `docker.logs` |
| Mở terminal, chạy script | `services.exec` (trong project của nó), hoặc `docker.exec` |
| Thêm, sửa, xoá script | `service_scripts.*` |

Developer và quản trị của project có `services.exec` trong project của họ (xem [Project](/vi/apps/projects-services#thanh-vien-project)). Terminal của service Compose hoặc service có quyền đặc biệt (privileged) chỉ dành cho quản trị viên (`system.manage`).

::: tip Lưu ý
- Terminal, script và số liệu trực tiếp chỉ dùng được với container trên node của panel (xem [Chọn node chạy](/vi/apps/nodes)).
- Docker → Container có cùng log và terminal cho mọi container trên server (xem [Docker](/vi/server/docker)).
:::
