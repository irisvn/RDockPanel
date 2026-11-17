# Xử lý sự cố

Các sự cố hay gặp, vì sao xảy ra, và cách xử lý. Mỗi mục gồm: dấu hiệu, nguyên nhân, cách sửa.

## Cài xong không mở được panel {#panel-not-reachable}

**Dấu hiệu**: trình duyệt không vào được địa chỉ của panel.

**Nguyên nhân**: sai địa chỉ hoặc port, port bị đóng ở nhà cung cấp, hoặc DNS chưa trỏ về server.

**Cách sửa**:
- Chưa có domain thì panel ở `http://<ip-server>:3000/admin`. Có domain rồi thì port 3000 bị đóng:
  dùng `https://panel.example.com/admin`.
- Mở port 80 và 443 (thêm 3000 khi chưa có domain) ở firewall hoặc security group của nhà cung cấp.
- Kiểm tra bản ghi A của domain đã trỏ về server, rồi chạy lại installer với đúng các tuỳ chọn cũ.

## Domain không trả lời {#domain-not-reachable}

**Dấu hiệu**: domain của app bị timeout, hoặc trả về "404 page not found".

**Nguyên nhân**: DNS chưa trỏ về server này, service không chạy, hoặc domain trỏ sai port của
container.

**Cách sửa**:
1. Kiểm tra bản ghi DNS (A, hoặc wildcard cho domain tự sinh) đã trỏ về server.
2. Mở trang **Domain** (Domains): cột **Trạng thái** (State) ghi **Đang chạy** (Live), **Không
   chạy** (Not running: hãy chạy hoặc deploy service), hoặc **Chờ Deploy** (Waiting for deploy:
   domain của service Compose, hãy deploy nó).
3. Kiểm tra **Port của container** (Container port) của domain đúng là port app lắng nghe *bên
   trong* container (dialog mặc định là 80).
4. Thay đổi domain áp dụng ngay khi lưu, nhưng chỉ khi service đang chạy.

## HTTPS: chứng chỉ không được cấp {#certificate}

**Dấu hiệu**: trình duyệt cảnh báo chứng chỉ, hoặc HTTPS lỗi trong khi HTTP vẫn chạy.

**Nguyên nhân**: Let's Encrypt không kiểm tra được tên miền. Qua HTTP, nó phải tới được server ở
port 80 bằng đúng tên đó. Domain wildcard (`*.example.com`) cần resolver DNS. Tên miền đi qua
Cloudflare thì bị kiểm tra ở Cloudflare, không phải ở đây.

**Cách sửa**:
- Đảm bảo DNS trỏ về đây và port 80 đang mở, rồi chờ một phút: chứng chỉ đầu tiên mất vài giây mới
  có.
- Với wildcard, thêm một resolver **qua DNS** trong **Cài đặt → Chứng chỉ** (Settings →
  Certificates; dùng token của Cloudflare, DigitalOcean, Hetzner, Vultr hoặc Linode), hoặc tải lên
  một chứng chỉ wildcard.
- Đi qua [Cloudflare Tunnel](/vi/apps/tunnel) thì Cloudflare lo HTTPS cho người truy cập: tắt HTTPS
  trên domain (dialog có nút **Tắt HTTPS**). Nếu là proxy của Cloudflare về server này, dùng resolver
  DNS hoặc chứng chỉ tải lên.
- Đọc log trực tiếp của Traefik ở **Cài đặt → Traefik**: lỗi chứng chỉ nằm ở đó.

## Deploy bị lỗi {#deployment-failed}

**Dấu hiệu**: lần deploy kết thúc với trạng thái **Lỗi** (Failed), hoặc service khởi động lại liên
tục.

**Nguyên nhân**: không pull được image (gõ sai, registry private), build lỗi, app bị crash khi chạy,
hoặc health check không qua.

**Cách sửa**:
1. Mở tab **Lần deploy** (Deployments) và đọc log của lần đó: mọi bước (pull, build, chạy) đều nằm
   ở đây.
2. Tab **Tổng quan** (Overview) có **Lỗi gần đây** (Recent errors): các task lỗi kèm exit code.
3. Với image private, điền **User registry** và **Mật khẩu hoặc token registry** ở tab **Nguồn**
   (Source).
4. Nếu một lần cập nhật bị lỗi, Swarm đã tự quay về bản trước. Muốn chủ động deploy lại cấu hình
   cũ, dùng **Rollback về lần này** (Roll back to this) trên một lần deploy đã thành công.

Nếu bấm Deploy mà được báo đang có lần deploy khác chạy: mỗi service chỉ chạy một lần deploy mỗi
lúc. Hãy chờ nó xong.

## "Is already published" hoặc "is reserved" khi publish port {#port-in-use}

**Dấu hiệu**: lưu một port publish bị từ chối.

**Nguyên nhân**: một service khác đã publish port đó, hoặc đó là port panel giữ cho chính nó và
Traefik (mặc định 80, 443, 3000, 8000, 8080, 8443).

**Cách sửa**: chọn port publish khác. Với website, nên dùng [domain](/vi/apps/domains): không cần
publish port nào.

## Báo "không đủ RAM" trước khi deploy {#not-enough-memory}

**Dấu hiệu**: khi tạo hoặc deploy service, panel hiện **Không đủ RAM cho service này** (Not enough
memory for this), **RAM sẽ còn rất ít** (Memory will be low), hoặc **Service này không thể chạy**
(This service cannot run).

**Nguyên nhân**: panel ước tính lượng RAM service cần (một lần build tốn khoảng 1 GB trong lúc chạy)
và so với lượng RAM còn trống của server.

**Cách sửa**:
- **RAM sẽ còn rất ít**: vẫn deploy được; hãy để mắt tới nó.
- **Không đủ RAM cho service này**: các service có thể bị dừng vì thiếu RAM. Giải phóng RAM (dừng
  service không dùng) hoặc xác nhận bằng **Tôi hiểu, vẫn deploy** (I understand, deploy anyway).
- **Service này không thể chạy**: RAM dành riêng vượt quá khả năng của server; Swarm sẽ để nó ở
  trạng thái chờ. Giảm **RAM dành riêng (MB)** (Memory reservation) ở tab **Nâng cao** (Advanced),
  hoặc thêm RAM.

Trên Mac, RAM là của VM Docker: tăng nó trong cài đặt của OrbStack hoặc Docker Desktop.

## Khởi động lại server xong không gì chạy {#after-reboot}

**Dấu hiệu**: sau khi server khởi động lại, panel và các app đều không chạy, nhưng SSH vẫn vào được.

**Nguyên nhân**: khi [Firewall](/vi/server/firewall) đang bật, Docker chỉ chạy sau khi rule của
firewall được nạp. Nếu bộ rule đã lưu không nạp được, một bộ dự phòng (chỉ SSH và web) được nạp thay
và trang Tường lửa sẽ báo; nếu cả hai đều không nạp được, Docker không chạy, để không service nào bị
lộ ra mà không có lọc.

**Cách sửa**: vào SSH và kiểm tra `systemctl status docker`. Muốn tự khôi phục khi các file của
firewall đã mất:

```sh
rm /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf && systemctl daemon-reload
systemctl start docker
```

Trên Mac, Mac phải thức và Docker phải chạy; Swarm tự chạy lại mọi thứ cùng Docker.

## Bị khoá ngoài vì đổi firewall {#firewall-lockout}

**Dấu hiệu**: sau khi bấm **Áp dụng** (Apply) ở trang Tường lửa, bạn không vào được panel hay SSH.

**Cách sửa**: chờ. Mỗi thay đổi được chính server hoàn tác sau 120 giây nếu bạn không bấm **Giữ bộ
rule này** (Keep these rules). Trước khi bấm, hãy thử từ một kết nối **mới** (cửa sổ ẩn danh, phiên
SSH mới).

## Import bị chặn hoặc bị từ chối {#import-blocked}

**Dấu hiệu**: kế hoạch import đánh dấu một container là bị chặn, hoặc bị từ chối khi bắt đầu.

**Nguyên nhân**: kế hoạch liệt kê lý do chặn: ví dụ container privileged, dùng network của host, thư
mục mà panel không bao giờ cho service dùng (`/`, socket Docker, `/proc`…), một volume dùng chung
giữa hai container, hoặc mật khẩu trên dòng lệnh không tách ra được. Bị từ chối khi bắt đầu nếu dữ
liệu cộng phần dư (10%, ít nhất 2 GiB) không vừa ổ đĩa.

**Cách sửa**: đọc lý do cạnh từng mục, sửa container hoặc bỏ nó ra, giải phóng ổ đĩa, rồi cho panel
đọc lại. Khi một lần import đang chạy, các service của nó từ chối deploy và backup: hãy hoàn tất hoặc
hoàn tác nó trước. Xem [Import container có sẵn](/vi/apps/import).

## Vẫn chưa xong?

Tìm hoặc mở một issue trên [GitHub](https://github.com/irisvn/RDockPanel/issues), kèm phiên bản
panel (Cài đặt → Phiên bản) và log của lần deploy hoặc của Traefik, nhớ bỏ secret ra.
