# Lưu trữ và volume

File bên trong container mất mỗi khi container bị thay (mỗi lần deploy). Hãy giữ dữ liệu ở tab **Lưu trữ** (Storage) của service: một volume, một thư mục host hoặc một file.

## Thêm mount

1. Mở tab **Lưu trữ** của service và bấm **Thêm mount** (Add mount).
2. Chọn **Loại** (Type):
   - **Volume**: volume Docker do panel giữ (lựa chọn thường dùng);
   - **Thư mục host** (Host directory): một thư mục trên server (chỉ quản trị viên, xem bên dưới);
   - **File**: file cấu hình viết ngay trong panel.
3. Điền **Tên volume** (hoặc **Thư mục host**) và **Đường dẫn trong container**, ví dụ `/var/lib/app`.
4. Đánh dấu **Chỉ đọc** (Read-only) nếu app chỉ đọc.
5. Bấm **Thêm**, rồi **Deploy**: mount thay đổi ở lần deploy tới.

![Tab Lưu trữ](/shots/service-storage.png)

## Volume

Volume tên `data` của service `web` trong project `shop` là volume Docker `rdp_shop_web_data`. Nó còn nguyên qua các lần deploy, khởi động lại và **Gỡ khỏi Docker**.

Gỡ mount khỏi service vẫn giữ dữ liệu của volume trong Docker: lần deploy tới chỉ đơn giản là không gắn nó nữa. Muốn xoá hẳn dữ liệu, xoá volume ở trang **Docker → Volume**. Dọn volume không dùng không bao giờ xoá volume của service (`rdp_…`).

## File

Mount loại **File** đặt một file cấu hình vào một đường dẫn trong container, ví dụ `/etc/app/config.ini`. Nội dung được mã hoá trong panel, không bao giờ hiện lại, và được gắn ở chế độ chỉ đọc. Mỗi lần deploy giữ lại các file nó đã áp dụng, nên rollback sẽ khôi phục chúng. Tổng các file của một service tối đa 32 KB.

## Thư mục host

Thư mục host gắn một thư mục của server vào container. Mặc định tính năng này tắt:

- chỉ quản trị viên (`system.manage`) được thêm;
- thư mục phải nằm dưới một trong các thư mục gốc được phép trong cấu hình của panel (`bind_roots`, để trống sau khi cài: xem [Bảo mật](/vi/reference/security));
- `/`, `/proc`, `/sys`, `/dev`, `/boot`, `/var/lib/docker`, Docker socket, `/etc/rdockpanel`, `/run/secrets`, và mọi thư mục phía trên chúng (`/run`, `/etc`, `/var`) luôn bị từ chối.

Đường dẫn được kiểm tra khi lưu và kiểm tra lại trước mỗi lần deploy.

## Thư mục dùng tại chỗ (từ import) {#thu-muc-dung-tai-cho}

Container được [import](/vi/apps/import) có dùng thư mục của server sẽ tiếp tục dùng thư mục đó tại chỗ: tab Lưu trữ hiện nó với loại `link`. Service như vậy chỉ quản trị viên được đụng tới cho tới khi thư mục được chuyển đi.

- **Chuyển toàn bộ** (Move all, ở tab Lưu trữ) dừng service, chép các thư mục vào volume, rồi deploy service chạy trên volume. Trang của nó hiện tiến trình, và việc chuyển có thể hoàn tác.
- Khi đã yên tâm, **Dọn thư mục cũ** (Clean the old folder) trên trang đó (gõ tên thư mục để xác nhận) sẽ xoá thư mục cũ. Từ lúc đó không hoàn tác được nữa.

Chuyển toàn bộ bị từ chối khi có service hay container khác cũng dùng thư mục đó: chép ra sẽ tách đôi dữ liệu.

## Backup volume

Service có volume sẽ có tab **Backup**: một lịch backup có thể nén volume (`.tar.gz`) gửi tới nơi lưu trữ, và khôi phục lại. Xem [Database và backup](/vi/apps/databases#backup).

::: tip Lưu ý
- Backup volume chạy `tar` bên trong container đang chạy của service: image cần có `tar`.
- Volume thuộc về một node. Service chuyển sang node khác sẽ bắt đầu với volume trống, trừ khi lưu trữ được dùng chung (xem [Chọn node chạy](/vi/apps/nodes)).
- Mỗi volume thuộc về một service; hai service không dùng chung được.
:::
