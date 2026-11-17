# Deploy và rollback

Cách cho một service chạy, cập nhật nó mà không gián đoạn, và quay về một lần deploy trước.

## Các thao tác

| Nút | Làm gì |
|---|---|
| **Deploy** | Pull (hoặc build) image và áp dụng cấu hình hiện tại của service. |
| **Chạy** (Start) | Chạy lại service đã dừng với image nó đang có (lần Chạy đầu tiên sẽ build nếu cần). |
| **Dừng** (Stop, menu ⋯) | Gỡ các task; service và dữ liệu vẫn còn. |
| **Khởi động lại** (Restart) | Thay các task bằng task mới, cùng image và cấu hình. |
| **Build lại từ đầu** (Force rebuild, menu ⋯) | Build lại không dùng build cache (service dùng image thì pull lại), rồi thay mọi task. |
| **Gỡ khỏi Docker** (Remove from Docker, ở Nâng cao → Vùng nguy hiểm) | Gỡ service khỏi Swarm. Volume được giữ lại; bấm **Deploy** để tạo lại. |

Mỗi service chỉ chạy một thao tác tại một thời điểm: khi có thao tác đang chạy, các nút phải chờ. Mỗi thao tác là một lần deploy, có log trực tiếp riêng ở tab **Lần deploy** (Deployments).

## Cái gì cần Deploy

- **Cần Deploy**: image hoặc nguồn, biến môi trường, port, mount, replicas, command, healthcheck, tài nguyên, nơi chạy. Trang service có thông báo khi cấu hình đã đổi so với lần deploy trước.
- **Áp dụng ngay khi lưu** (không cần Deploy, container không bị đụng tới): domain, chuyển hướng, xác thực cơ bản, giới hạn request và bảo trì.

## Zero downtime và rolling update

Mỗi lần cập nhật thay từng task một. Khi bật **Zero downtime** (tab Nâng cao, chỉ có ở app), task mới khởi động và ổn định rồi task cũ mới dừng. Điều này cần:

- một **healthcheck**, để Swarm biết khi nào task mới sẵn sàng;
- không có port publish ở chế độ host;
- không có volume hay thư mục ghi được (hai task sẽ ghi cùng dữ liệu một lúc).

Nếu không, task cũ dừng trước, và service gián đoạn một lúc.

Nếu một lần cập nhật bị lỗi, Swarm tự quay lại: lần deploy đó hiện **Đã rollback** (Rolled back) và bản trước vẫn chạy.

## Healthcheck

Ở tab **Nâng cao** (Advanced):

- **Healthcheck** (Health check): lệnh chạy trong container, ví dụ `wget -qO- http://127.0.0.1/health`;
- **Mỗi (giây)** (Every, để trống là 10), **Số lần thử** (Retries, 3), **Thời gian chờ (giây)** (Timeout, 5), **Thời gian khởi động (giây)** (Start period, 10: lỗi trong khoảng này không bị tính);
- **Không health check, kể cả health check sẵn có của image**: tắt mọi kiểm tra.

Khi có healthcheck, task chỉ được tính là đang chạy khi đã ổn định.

## Replicas và tài nguyên

Ở tab **Nâng cao**: **Replicas** (số task chạy), và **Tài nguyên** (Resources):

- **Giới hạn CPU** (1000 = một nhân) và **Giới hạn RAM (MB)**: mức tối đa service được dùng;
- **CPU dành riêng** và **RAM dành riêng (MB)** (reservation): phần Swarm giữ sẵn cho service.

Để trống nghĩa là không giới hạn. Thay đổi áp dụng ở lần Deploy tới.

![Tab Nâng cao](/shots/service-advanced.png)

## Cảnh báo RAM và ổ đĩa {#canh-bao-ram-va-o-dia}

Trước khi bạn tạo service, dùng template, Deploy, Chạy, Khởi động lại hoặc Build lại từ đầu, panel ước tính service sẽ cần bao nhiêu và so với server:

- **RAM sẽ còn rất ít** (Memory will be low): vẫn deploy được, nhưng nên theo dõi.
- **Không đủ RAM** (hoặc ổ đĩa): panel hỏi xác nhận bằng nút **Tôi hiểu, vẫn deploy**.
- **Service này không thể chạy**: RAM dành riêng lớn hơn phần server còn cho được, Swarm sẽ để nó chờ mãi. Panel từ chối.

Ước tính lấy từ RAM service đang dùng thật, nếu không có thì từ giới hạn RAM, nếu không nữa thì từ mức thường gặp của loại service đó. Đặt giới hạn RAM để có con số sát hơn. Deploy từ API, webhook và URL kích hoạt không bao giờ bị chặn: log của chúng ghi lại, và [các kênh thông báo](/vi/server/settings) được báo.

## Lịch sử deploy và rollback

Tab **Lần deploy** (Deployments) liệt kê mọi lần deploy, mới nhất trước: số thứ tự, thao tác, trạng thái, lúc bắt đầu và image. Log của từng dòng cho biết nó đã làm gì.

Để quay về:

1. Mở menu **⋯** của dòng đó và chọn **Rollback về lần này** (Roll back to this).
2. Xác nhận. Cấu hình và image của lần deploy đó được deploy lại, kèm cả biến môi trường và file của lúc đó.

Chỉ rollback được về lần deploy đã thành công và có image được ghi lại (không phải lần Dừng hay Gỡ). Cần quyền `services.deploy` và `deployments.view`.

![Tab Lần deploy](/shots/service-deployments.png)

## Lỗi gần đây

Tab **Tổng quan** liệt kê các lỗi gần đây của service. Lỗi mà Swarm đã tự khắc phục được đánh dấu **Đã tự động phục hồi** (Recovered automatically).

::: tip Lưu ý
- Lần deploy chờ quá lâu hoặc mất worker sẽ chuyển sang **Bị ngắt** (Interrupted) và trả service về trạng thái rảnh.
- Nếu không ghi được route của Traefik sau một lần deploy, trang service hiện nút **Ghi lại route** (Republish routes).
- Image được ghim theo digest của registry; image chỉ có trên server này được ghim theo id, kèm cảnh báo.
- Service Compose không rollback được (xem [Compose](/vi/apps/compose)).
:::
