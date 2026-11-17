# Chọn node chạy

Khi Swarm có nhiều server (node), bạn chọn được node nào chạy một service. Với một server duy nhất thì không có gì để chọn: mọi thứ chạy ở đó.

Các node là server trong Swarm của bạn: thêm node ở [Docker → Node](/vi/server/nodes) (chạy một lệnh trên server mới). Sau đó chúng tự hiện trong danh sách này.

## Chọn node

1. Mở tab **Nâng cao** (Advanced) của service.
2. Trong **Chạy trên** (Runs on), chọn **Node**:
   - **Tự động** (Automatic, mặc định): chạy trên node của panel khi chỉ máy đó có thứ service cần (image build hoặc import tại đây, thư mục dùng tại chỗ, volume của service đã import); còn lại thì node bất kỳ.
   - **Một node** của Swarm, theo tên (node của panel được ghi **(node của panel)**).
   - **Node bất kỳ (lưu trữ dùng chung giữa các node)** (Any node): không ràng buộc, cho service có dữ liệu nằm trên lưu trữ mà mọi node dùng chung.
3. Bấm **Lưu**, rồi **Deploy**.

![Tab Nâng cao](/shots/service-advanced.png)

## Ai được đặt

Chỉ quản trị viên của panel (`system.manage`): đặt lựa chọn này, và sửa service đã có lựa chọn này. Người khác chỉ thấy khi đã có lựa chọn, và không đổi được.

Bị từ chối với:

- service **Compose** (luôn chạy trên server của panel);
- service **do panel build** (từ Git hoặc Dockerfile) và image **chỉ có trên máy này**: các node khác không lấy được image.

Lần deploy sẽ lỗi, kèm lý do trong log, khi node đã chọn không còn trong Swarm.

## Volume nằm theo node

Volume Docker nằm trên node đã tạo ra nó. Khi service có volume chuyển sang node khác, **volume ở node đó sẽ trống**, trừ khi lưu trữ được dùng chung giữa các node. Tab Nâng cao có cảnh báo việc này.

Hãy tự chuyển dữ liệu trước khi đổi node của service có dữ liệu, hoặc dùng lưu trữ dùng chung với **Node bất kỳ**.

## Những gì chỉ chạy trên node của panel

Panel làm việc với container qua Docker của chính nó. Với service có container chạy trên node khác:

- [backup và khôi phục](/vi/apps/databases#backup), [script](/vi/apps/logs-terminal-scripts#script), [terminal](/vi/apps/logs-terminal-scripts#terminal) và số liệu trực tiếp không dùng được;
- trang service ghi rõ lý do: "Service này chạy trên node khác".

::: tip Lưu ý
- Tính năng chuyển dữ liệu của service sang node khác sẽ có sau; hiện tại bạn tự làm.
- Giống mọi thiết lập ở tab Nâng cao, lựa chọn này áp dụng ở lần Deploy tới.
:::
