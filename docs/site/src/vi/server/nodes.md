# Node

**Docker → Node (Nodes)** liệt kê các server trong Swarm, và cho quản trị viên panel thêm, tạm
dừng, drain hoặc gỡ một server.

## Hoạt động thế nào

RDockPanel chạy trên **Docker Swarm**. Mỗi server trong Swarm là một **node**:

- Server cài panel là node đầu tiên, một **manager**. Panel, database của panel và Traefik luôn
  chạy ở đây.
- Các server thêm vào là **worker**. Chúng chạy service của bạn, không chạy panel.
- **Swarm quyết định** container (task) của mỗi service chạy ở đâu, theo luật bạn đặt: node đã chọn
  ở [Chọn node chạy](/vi/apps/nodes), hoặc node bất kỳ còn chỗ.
- Request vẫn đi vào qua Traefik trên node của panel, rồi tới container ở node nào cũng được, qua
  network riêng của Swarm.

```text
            internet
               │ 80 / 443
     ┌─────────▼──────────┐        network Swarm        ┌──────────────────┐
     │ node của panel     │◄───────────────────────────►│ worker-01        │
     │ manager            │   2377 · 7946 · 4789        │ chạy service     │
     │ panel, MySQL,      │                             └──────────────────┘
     │ Traefik, service   │◄───────────────────────────►┌──────────────────┐
     └────────────────────┘                             │ worker-02        │
                                                        └──────────────────┘
```

![Node](/shots/nodes.png)

Bảng hiện với mỗi node: vai trò, trạng thái (**Sẵn sàng/Ready** hoặc **Ngừng/Down**), nhận việc
(**Kích hoạt/Active**, **Tạm dừng/Paused**, **Đã drain/Drained**), địa chỉ, CPU và RAM, và số task
đang chạy ở đó. Node của panel ghi *node của panel*; manager đang điều khiển ghi *leader*.

## Thêm node

1. Cài Docker trên server mới.
2. Giữa các server, mở **2377/tcp**, **7946/tcp và udp** và **4789/udp** (ở firewall của nhà cung
   cấp, chỉ giữa các server này).
3. Nếu [Firewall](/vi/server/firewall) của panel đang bật, tắt nó: nó chỉ hoạt động với một node.
4. Trong panel, bấm **Thêm node (Add a node)** rồi copy lệnh hiện ra:
   ```sh
   docker swarm join --token SWMTKN-1-… 203.0.113.10:2377
   ```
5. Chạy lệnh đó trên server mới. Vài giây sau nó hiện trong danh sách, **Sẵn sàng**.

![Thêm node](/shots/nodes-add.png)

Từ đó, Swarm có thể chạy service trên node mới (xem [những gì thay đổi](#nhieu-node)).

::: warning Token là chìa khoá
Ai có lệnh này cũng cho được một server vào Swarm của bạn. Mỗi lần đọc lệnh được ghi vào audit
log. Nếu lộ, bấm **Đổi token (Rotate token)**: server đã trong Swarm vẫn giữ nguyên, lệnh cũ hết dùng được.
:::

::: tip Trên máy Mac
Docker trên Mac chỉ nghe ở 127.0.0.1: server khác không join được, và hộp thoại sẽ nói rõ.
:::

## Tạm dừng, drain, kích hoạt

Từ menu **⋯** của một node:

| Thao tác | Swarm làm gì |
|---|---|
| **Tạm dừng (Pause)** | Giữ những gì đang chạy; không đặt thêm gì mới. |
| **Drain** | Dừng những gì đang chạy ở đó và chạy lại ở node khác khi có thể. |
| **Kích hoạt (Activate)** | Nhận task mới trở lại. Những gì đã chuyển đi **không** tự quay về (deploy lại service để chia lại). |

Trước khi drain, panel liệt kê những gì đang chạy trên node và chuyện gì xảy ra với từng service:

- **gắn với node này**: được đặt trên node này (Nâng cao → Chạy trên), hoặc giữ ở đó vì chỉ máy này
  có image hay thư mục của nó. Nó không chuyển được: **sẽ chờ mãi** cho tới khi node được kích hoạt lại;
- **volume của nó sẽ trống trên node khác**: volume nằm trên node đã tạo ra nó;
- **không thuộc panel**: Swarm service không do panel tạo.

Dùng **Drain** trước khi bảo trì một server (cập nhật, reboot), xong thì **Kích hoạt** lại.

## Gỡ node

1. **Drain** node đó để service chuyển đi trước.
2. Trên server đó, chạy `docker swarm leave`. Trong vòng một phút nó hiện **Ngừng**.
3. Trong panel, chọn **Gỡ… (Remove…)**.

Service được đặt trên node đó (Nâng cao → Chạy trên) sẽ deploy lỗi ở lần tới cho tới khi bạn chọn
node khác.

## Giới hạn

- Node của panel không bao giờ bị tạm dừng, drain hay gỡ.
- Không gỡ **manager** ở đây, và không nâng node lên manager (nhiều manager cần đa số còn sống mới
  chạy được: hãy tự quyết bằng Docker).
- Backup, khôi phục, script, terminal và số liệu trực tiếp chỉ tới được container trên node của panel.

## Những gì thay đổi khi có nhiều node {#nhieu-node}

- Service có **volume** giữ dữ liệu trên node nó đã chạy. Nếu chuyển node, nó bắt đầu trống ở đó
  trừ khi lưu trữ dùng chung. Hãy cố định node của service ở mục [Chạy trên](/vi/apps/nodes).
- Image do panel **build**, hoặc image chỉ máy này có, chỉ chạy trên node của panel.
- **Firewall** của panel giữ ở trạng thái tắt (nó từ chối áp dụng khi có nhiều hơn một node).

## Ai làm được gì

| Quyền | Được |
|---|---|
| `docker.view` | Xem danh sách (không đếm task của chính panel). |
| `system.manage` | Thêm node (đọc lệnh join), đổi token, tạm dừng, drain, kích hoạt, gỡ. Mỗi thao tác được ghi ở trang [Actions](/vi/server/docker) và audit log. |
