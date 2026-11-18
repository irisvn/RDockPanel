# Firewall

Trang **Tường lửa** (Firewall) đóng mọi port của server với internet, trừ SSH, web (80 và 443) và các port anh mở cho những địa chỉ anh chọn. Trang cũng cho thấy server đang lắng nghe những port nào.

Trang nằm trong menu Không gian làm việc (Workspace) và cần quyền `system.manage`.

::: warning Chỉ trên Linux
Panel cài trên macOS, hoặc chạy từ mã nguồn, sẽ báo điều đó trên trang và không bao giờ đụng tới máy chủ. Trên Mac, hãy dùng firewall của macOS (System Settings → Network → Firewall).
:::

![Tường lửa](/shots/firewall.png)

## Những gì vẫn mở

Khi firewall bật, mọi kết nối mới đi vào đều bị chặn, trừ:

- **SSH**: mọi port mà SSH daemon của server lắng nghe (TCP). SSH luôn mở, nên anh không thể tự khoá mình khỏi SSH.
- **Web**: TCP 80 và 443. Traefik phục vụ panel và mọi domain ở đây.
- **Các port anh mở**: một port hoặc một dải, TCP hoặc UDP, chỉ cho các nguồn đã khai báo.

Kết nối đang mở vẫn chạy tiếp tới khi đóng; quy tắc mới áp dụng cho kết nối mới.

## Trước khi bật

Trang sẽ từ chối bật firewall, kèm lý do, khi server không phù hợp:

- thiếu `nft` hoặc systemd;
- Swarm có nhiều hơn một node;
- có Docker network kiểu macvlan hoặc ipvlan;
- `nftables.service` đang bật (cấu hình mặc định của nó xoá mọi bảng khi khởi động);
- một Docker bridge đang giữ default route;
- panel chưa có domain và chưa có Cloudflare Tunnel: port 3000, nơi panel trả lời khi chưa có domain, sẽ bị đóng. Hãy gán domain cho panel trước (xem [Cài đặt → Panel](/vi/server/settings#panel)).

Xem mục **Port đang lắng nghe** (Listening ports) trước: nó liệt kê những gì server đang lắng nghe, chương trình sở hữu, và port đó có còn mở không. Thứ gì anh vẫn cần truy cập từ ngoài (VPN, port database cho văn phòng…) phải được mở trước khi bật firewall.

## Mở một port

1. Bấm **Mở port** (Open a port), hoặc **Mở cho…** (Open for…) trên một dòng của Port đang lắng nghe.
2. **Port hoặc dải** (Port or range): `5432`, hoặc `8000-8100`. Chọn **Giao thức** (Protocol): TCP hoặc UDP.
3. **Áp cho** (Applies to):
   - **Máy chủ này (chương trình ngoài Docker)**, ví dụ VPN hay một chương trình cài thẳng trên máy;
   - **Port do một service publish**, cho port mà một service của anh publish.
4. **Mở cho** (Open for): **Chỉ các địa chỉ dưới đây** (thường chọn cái này), rồi điền **Nguồn IPv4** và **Nguồn IPv6**, ví dụ `203.0.113.0/24`. Để trống một loại thì không địa chỉ nào thuộc loại đó vào được. **Mọi nơi (mọi người trên internet)** là lựa chọn phải chọn rõ ràng, hiện màu đỏ.
5. Thêm **Ghi chú** (Note), ví dụ "Qua VPN", rồi lưu.
6. Bấm **Áp dụng…** (Apply…) để dùng bộ rule mới (xem bên dưới).

SSH, 80 và 443 luôn mở: không cần thêm gì cho chúng.

### Port publish và Docker

Bình thường, port mà một service publish sẽ đi vòng qua các quy tắc firewall của máy: Docker mở nó cho tất cả mọi người. Khi firewall của RDockPanel bật, port publish cũng bị đóng như mọi port khác cho tới khi anh mở nó ở đây với **Port do một service publish**. Domain thì không cần: chúng đi qua Traefik ở port 80 và 443.

## Chặn một địa chỉ

**Chặn địa chỉ** (Block an address) khiến một địa chỉ hoặc một dải mạng (`198.51.100.7`, `2001:db8::/32`) không tới được gì ngoài SSH. Kể cả mọi website ở port 80 và 443, panel cũng vậy, vì server không phân biệt được domain này với domain khác. Người truy cập qua proxy Cloudflare hoặc Tunnel không bị ảnh hưởng: server chỉ thấy Cloudflare. Hãy chặn họ trên website (WAF).

- Cùng hộp thoại và cùng danh sách với trang WAF: chọn **Trên toàn máy chủ (Firewall)**, **Trên các website (WAF)** hoặc cả hai. Xem [Chặn một địa chỉ](/vi/server/waf#chan-mot-dia-chi-tren-website-hay-tren-may-chu).
- Lệnh chặn được áp dụng **ngay**, không qua bước "thử rồi giữ" bên dưới: nó không thể khoá anh ở ngoài (địa chỉ bị chặn vẫn vào được SSH, và địa chỉ anh đang dùng bị từ chối). Các port đã lưu nhưng chưa áp dụng vẫn là bản nháp.
- Trong một ngày, một tuần, ba mươi ngày, hoặc tới khi gỡ; lệnh chặn hết hạn được gỡ trong vòng năm phút rồi Firewall được áp dụng lại.
- Dải IP của Cloudflare bị từ chối: chặn chúng sẽ cắt mọi khách đi qua Cloudflare.

## Áp dụng an toàn: thử trước, giữ sau

Mỗi thay đổi đều được thử trước, rồi anh mới giữ:

1. Bấm **Bật…** (Turn on…, lần đầu) hoặc **Áp dụng…** (Apply…). Hộp thoại liệt kê các port sẽ đóng với internet. Xác nhận.
2. Trang hiện **Đang thử bộ rule mới.** kèm đồng hồ đếm ngược. Kiểm tra, **từ một kết nối mới**, rằng anh vẫn vào được panel (cửa sổ ẩn danh của trình duyệt) và SSH (một phiên SSH mới).
3. Ổn cả: bấm **Giữ bộ rule này** (Keep these rules).
4. Có gì sai: bấm **Quay về ngay** (Go back now), hoặc không làm gì. Sau hai phút server **tự quay về** bộ rule trước, kể cả khi panel không liên lạc được với nó.

Bộ rule cũng được nạp khi khởi động, trước cả Docker, nên không port publish nào bị lộ khi server khởi động lại. Nếu ai đó sửa bên ngoài panel (ví dụ `nft flush ruleset`), panel nạp lại bộ rule trong vòng hai phút và ghi lại thành một thao tác.

## Tắt firewall

**Tắt** (Turn off) gỡ hẳn bộ rule của panel: mọi port lại mở như Docker publish, và Docker không còn chờ firewall khi khởi động.

## Nếu bị khoá ngoài

- Ngay sau khi **Áp dụng**: chờ hai phút, server tự quay về bộ rule trước.
- SSH luôn mở. Từ một phiên SSH, với quyền root, tắt firewall mà không cần panel:

  ```sh
  sh /var/lib/rdockpanel/firewall/fw.sh disable
  ```

- Nếu các file của firewall bị xoá và Docker không khởi động được nữa, gỡ liên kết giữa Docker và firewall:

  ```sh
  rm /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf && systemctl daemon-reload
  ```

## Lưu ý

- Firewall này không thay cho firewall hay security group của nhà cung cấp. **Port đang lắng nghe** cho biết server lắng nghe gì, không phải internet tới được gì.
- Firewall dùng bảng nftables riêng (`inet rdockpanel`). Quy tắc của ufw, firewalld và Docker vẫn áp dụng song song: trang liệt kê các firewall khác mà nó tìm thấy.
- Tắt firewall trước khi gỡ cài đặt panel (xem [Cài đặt](/vi/guide/install)).
- Chi tiết kỹ thuật: [docs/firewall.md](https://github.com/irisvn/RDockPanel/blob/main/docs/firewall.md).
