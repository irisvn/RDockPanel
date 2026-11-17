# Project và service

App của bạn nằm trong các **project**, mỗi project gồm nhiều **service**: web app, database, worker. Trang này giải thích cách chúng được tổ chức và những gì bạn thấy trên trang của chúng.

## Project

Một project gom các service của một ứng dụng. Mỗi project có network Docker riêng:

- các service cùng project gọi nhau bằng tên (xem [Database](/vi/apps/databases#ket-noi-tu-service-khac));
- service của project khác không gọi trực tiếp được;
- những gì bạn đã public (một [domain](/vi/apps/domains) hay một port được publish) thì ai cũng vào được.

Để tạo project, bấm **Project mới** (New project) trên Bảng điều khiển và gõ tên (chữ thường, số và dấu gạch ngang).

Chỉ xoá được project khi không còn service nào (menu ⋯ của project, **Xoá project…**). Không đổi tên được project khi có service của nó đang được deploy.

## Service

Service là một thứ project chạy trên Docker Swarm. Bấm **Thêm service** (Add service) trong project rồi chọn loại:

| Loại | Là gì |
|---|---|
| **App (Docker image)** | App của bạn, từ image, repo Git hoặc Dockerfile (xem [Nguồn](/vi/apps/sources-templates)). |
| **PostgreSQL**, **MySQL**, **MariaDB**, **MongoDB**, **Redis** | Database: chỉ cần đặt tên, thông tin đăng nhập được tự sinh (xem [Database](/vi/apps/databases)). |
| **Compose** | File Compose chạy bằng `docker compose`, chỉ dành cho quản trị viên (xem [Compose](/vi/apps/compose)). |

Bạn cũng có thể bắt đầu từ hơn 800 [template](/vi/apps/sources-templates#template), hoặc đưa các container đang chạy trên server vào panel bằng [Import](/vi/apps/import).

Service mới chỉ được lưu lại: chưa có gì chạy cho tới khi bạn bấm **Deploy** (xem [Deploy](/vi/apps/deploy)).

## Bảng điều khiển và trang project

**Bảng điều khiển** (Dashboard) hiện các ô số liệu của server (CPU, RAM, ổ đĩa, mạng) và mọi project bạn được xem, mỗi project có các thẻ service: trạng thái, loại và domain (hoặc image). Bấm vào thẻ để mở service.

Trang của một project có các thẻ service và các nút:

- **Thêm service** (Add service), **Template** (Templates);
- **Nhập container** (Import containers, cho quản trị viên);
- **Thành viên** (Members), để cấp quyền chỉ trong project này (xem [bên dưới](#thanh-vien-project)).

Bấm **⌘K** ở bất kỳ đâu để nhảy tới một project hoặc service.

![Một project](/shots/project.png)

## Trang service

Phần đầu trang hiện tên, trạng thái của service và các nút chính:

- **Mở** (Open) domain đầu tiên của service (khi service đang chạy và có domain);
- **Chạy** (Start, khi service đã dừng) hoặc **Khởi động lại** (Restart);
- menu **⋯**: **Dừng** (Stop) và **Build lại từ đầu** (Force rebuild);
- **Deploy**.

Khi cấu hình đã thay đổi so với lần deploy trước, một thông báo hiện ra: bấm Deploy để áp dụng.

Các tab:

| Tab | Nội dung |
|---|---|
| **Tổng quan** (Overview) | Image, replicas, host nội bộ, domain; **Giám sát** (Monitor: biểu đồ CPU và RAM, **Log**, **Shell**); lỗi gần đây; ghi chú cho team. |
| **Thông tin kết nối** (Credentials) | Chỉ có ở database: user, mật khẩu, URL nội bộ. |
| **Backup** | Database, và service có volume. |
| **Lần deploy** (Deployments) | Mọi lần deploy kèm log, và **Rollback về lần này**. |
| **Biến môi trường** (Environment) | Biến môi trường dạng text `.env`. |
| **Domain** | Domain và HTTPS. |
| **Chuyển hướng**, **Bảo mật**, **Bảo trì** (Redirects, Security, Maintenance) | Các quy tắc áp dụng ngay khi lưu (không có ở database). |
| **Script** | Lệnh đã lưu, chạy trong container (không có ở database và Compose). |
| **Nguồn** (Source) | Image lấy từ đâu. |
| **Lưu trữ** (Storage) | Volume, thư mục host và file. |
| **Nâng cao** (Advanced) | Replicas, command, healthcheck, tài nguyên, nơi chạy, port và **Vùng nguy hiểm** (Danger zone). |

![Tab Tổng quan của một service](/shots/service-overview.png)

## Các từ chỉ trạng thái

Mọi trang dùng chung một bộ từ:

| Service | Nghĩa là |
|---|---|
| **Đang chạy** (Running) | Các task đang chạy (và ổn định, nếu service có healthcheck). |
| **Đang deploy** (Deploying) | Có một lần deploy đang chờ hoặc đang chạy. |
| **Đang khởi động** (Starting) | Các task đang khởi động. |
| **Lỗi** (Failed) | Lần deploy gần nhất bị lỗi. |
| **Đã dừng** (Stopped) | Được dừng có chủ đích: không task nào chạy. |
| **Chưa deploy** (Not deployed) | Đã lưu, chưa deploy lần nào. |

Một lần deploy có trạng thái **Đang chờ** (Queued), **Đang chạy** (Running), **Thành công** (Succeeded), **Lỗi** (Failed), **Bị ngắt** (Interrupted: tiến trình chạy nó đã chết), **Đã rollback** (Rolled back: Swarm tự quay lại sau một lần update lỗi) hoặc **Đã huỷ** (Cancelled).

## Gỡ một service

Trong **Nâng cao → Vùng nguy hiểm** (Advanced → Danger zone):

1. **Gỡ khỏi Docker** (Remove from Docker) dừng service và gỡ nó khỏi Swarm. Volume được giữ lại; bấm **Deploy** để tạo lại.
2. Sau đó **Xoá service** (Delete service) xoá cấu hình, domain và lịch sử của nó.

Service đã deploy giữ nguyên tên và project cho tới khi được gỡ khỏi Docker.

## Thành viên project {#thanh-vien-project}

Người dùng không cần vai trò nào để làm việc trên một project: quản trị của project thêm họ làm thành viên trong **Thành viên** (Members), bằng email, với một trong ba vai trò:

| Vai trò | Được làm |
|---|---|
| **Xem** (viewer) | Xem project, service, các lần deploy, backup và log. |
| **Developer** | Như Xem, thêm: sửa, thêm, deploy và rollback service; đọc biến môi trường; mở terminal; chạy backup. |
| **Quản trị** (admin) | Như Developer, thêm: gỡ và xoá service, khôi phục database, đổi tên project, chọn thành viên. |

Thành viên chỉ thấy các project của mình. Mỗi project luôn giữ ít nhất một quản trị. Vai trò cho toàn panel nằm ở [Người dùng và vai trò](/vi/server/users).

::: tip Mẹo
- Service Compose và service có quyền đặc biệt (privileged) vẫn chỉ quản trị viên của panel (`system.manage`) mới được đụng tới, bất kể vai trò thành viên.
- Service không publish được các port panel đang dùng (80, 443 và port của panel).
:::
