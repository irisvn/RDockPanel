# Docker Compose

Service Compose chạy cả một file `docker-compose.yml` (nhiều container) thành một service của project. Dùng nó cho các app được phát hành dưới dạng file Compose; những trường hợp khác nên dùng service app, chạy trên Swarm với rolling update và rollback.

## Ai dùng được

Chỉ quản trị viên của panel (`system.manage`) được tạo và sửa service Compose: một file Compose có thể chạy bất cứ thứ gì trên server (thư mục host, container privileged). Thành viên project xem được, nhưng không sửa, không deploy và không mở terminal được.

## Tạo service Compose

1. Trong một project, bấm **Thêm service** (Add service) và chọn **Compose**.
2. Gõ tên và viết **File Compose** (Compose file) trong ô soạn thảo:

   ```yaml
   services:
     web:
       image: ghcr.io/traefik/whoami:latest
   ```

3. Bấm **Tạo**, rồi **Deploy**.

**Lấy từ Git**: ở tab **Nguồn** (Source), chọn **Repo Git**, điền **URL repo**, **Nhánh, tag hoặc commit**, **Thư mục build** (thư mục của project Compose) và **File Compose** (mặc định `docker-compose.yml`). Repo được clone một lần, rồi fetch lại mỗi lần Deploy; các file repo không theo dõi (ví dụ `./data`) vẫn được giữ.

## Cách chạy

- Panel chạy nó bằng `docker compose` trên chính server của panel, không phải dưới dạng service Swarm.
- Mỗi container tham gia network của project, nên các service khác trong project gọi nó bằng `<project>_<service>_<container>`.
- [Biến môi trường](/vi/apps/environment) của service trở thành file `.env` của project Compose: dùng `${VAR}` trong file Compose.

| Nút | Chạy lệnh |
|---|---|
| **Deploy** | `docker compose up -d --build --remove-orphans` |
| **Chạy** (Start) | `docker compose up -d` |
| **Dừng** (Stop) | `docker compose stop` |
| **Khởi động lại** (Restart) | `docker compose restart` |
| **Gỡ khỏi Docker** (Remove from Docker) | `docker compose down --remove-orphans` (named volume được giữ lại) |

## Domain

Thêm domain ở tab **Domain** như mọi service, và điền **Service trong Compose** (Compose service): service trong file Compose mà domain trỏ tới (ví dụ `web`). Khác với các service khác, domain của service Compose **áp dụng ở lần Deploy tới**.

## Giới hạn

- Không có **Rollback**: hãy deploy lại với file bạn muốn.
- Không có tab **Script**; không chọn được node (luôn chạy trên server của panel).
- `container_name`, `ports` và `network_mode` trong file sẽ bị cảnh báo: chúng có thể đụng với service khác. Nên dùng domain thay vì publish port.
- Bước kiểm tra RAM trước khi deploy tính theo các service trong file; file nằm trong repo thì không biết trước khi chạy.

::: tip Lưu ý
Template dùng Compose sẽ tạo service Compose, nên cũng cần quyền `system.manage`.
:::
