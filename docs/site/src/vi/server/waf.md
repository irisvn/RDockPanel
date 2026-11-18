# Tường lửa ứng dụng web (WAF)

**Cài đặt → Bảo mật (Settings → Security)** bật một tường lửa đọc mọi request tới domain của các
service và chặn các kiểu tấn công đã biết trước khi chúng tới app của bạn.

## Hoạt động thế nào

Panel chạy WAF của [CrowdSec](https://www.crowdsec.net/) (bộ máy *AppSec*) thành một service riêng,
`rdp-crowdsec`, và gắn cho Traefik một plugin hỏi nó về từng request:

```text
 khách ──► Traefik ──► plugin WAF ──hỏi──► rdp-crowdsec (bộ luật)
                          │  ◄── cho qua / chặn ──┘
                          ├── cho qua ──► app của bạn
                          └── chặn ─────► 403 Forbidden
```

- **Chỉ domain của các service** đi qua WAF. Địa chỉ của chính panel và việc xác minh chứng chỉ HTTPS
  thì không, nên bạn luôn vào được panel để đổi thiết lập.
- Body của request được đọc trước (kiểm tra 1 MB đầu; upload lớn hơn vẫn qua nguyên vẹn, phần dư không
  được kiểm tra), rồi request đi tiếp, không bị thay đổi. Mỗi request chiếm tối đa khoảng 1 MB bộ nhớ
  của Traefik: phần còn lại của upload lớn nằm tạm trong một file.
- Bộ luật của CrowdSec đi kèm panel (tải về lúc build bản phát hành): CrowdSec khởi động từ đó, không cần
  internet, và mọi server cùng một bản có cùng bộ luật. Cập nhật panel sẽ mang bộ luật mới hơn.
- WAF dùng IP thật của khách, kể cả khi qua Cloudflare và Cloudflare Tunnel.
- **Nếu WAF lỗi** (CrowdSec dừng, chậm, hoặc plugin không nạp được), request vẫn đi qua **mà không được
  kiểm tra**: WAF không bao giờ làm sập trang. Trang Cài đặt sẽ báo điều đó.

![Cài đặt → Bảo mật](/shots/settings-security.png)

## Chế độ và mức độ

| Chế độ | Điều gì xảy ra |
|---|---|
| **Tắt (Off)** | Không kiểm tra gì. CrowdSec được dừng. |
| **Chỉ ghi nhận (Detect only)** | Tấn công được **ghi lại, không bị chặn**. Bắt đầu ở đây khoảng một tuần và đọc cảnh báo. |
| **Chặn (Block)** | Tấn công bị trả **403 Forbidden** trước khi tới app. |

| Mức | Chặn được |
|---|---|
| **Cơ bản (Basic)** | Lỗ hổng đã biết của các phần mềm phổ biến (vá ảo cho các CVE đã công bố) và việc dò file bí mật (`/.env`, file backup…). Mức này **không** chặn SQL injection hay XSS chung chung. |
| **Nghiêm ngặt (Strict)** | Cơ bản, thêm bộ luật **OWASP Core Rule Set**: SQL injection, XSS, path traversal và nhiều kiểu khác. Có thể chặn nhầm request hợp lệ: hãy thử ở chế độ Chỉ ghi nhận trước. |

## Bật WAF

1. Mở **Cài đặt → Bảo mật**.
2. Chọn **Chỉ ghi nhận** và một mức, bấm **Lưu**. Trạng thái chuyển từ *Đang khởi động* sang **Bật**
   trong khoảng một phút (lần đầu CrowdSec tải bộ luật về).
3. Xem **Cảnh báo gần đây** vài ngày. Thấy một request hợp lệ trong danh sách (chặn nhầm)? Thêm địa chỉ
   đó vào **Địa chỉ tin cậy**, hoặc tắt WAF cho service đó.
4. Chuyển sang **Chặn**.

Lần khởi động đầu, và mỗi lần tắt rồi bật lại, Traefik khởi động lại một lần: mọi domain tạm ngừng vài
giây.

## Theo từng service

Mỗi service theo thiết lập của panel. Ở tab **Bảo mật (Security)** của service, bạn có thể đặt **Bật**
hoặc **Tắt**:

- **Tắt** là bỏ bảo vệ của service đó: chỉ quản trị viên panel được làm.
- Ai sửa được service cũng có thể đặt lại về *theo panel* hoặc **Bật**.

Áp dụng ngay khi lưu, không cần deploy lại.

![Service → Bảo mật](/shots/service-security.png)

## Trạng thái

| Trạng thái | Ý nghĩa | Nên làm gì |
|---|---|---|
| **Bật (On)** | Panel vừa kiểm tra: một request tấn công thử đã bị chặn. | Không cần gì. |
| **Đang khởi động** | CrowdSec hoặc Traefik đang khởi động. | Chờ một phút. |
| **Không hoạt động** | Request đi qua mà không được kiểm tra; lý do hiện kèm. | Bấm **Sửa chữa (Repair)**. Vẫn không được? Xem lý do. |
| **Tắt** | WAF đang tắt. | — |

Mỗi phút panel gửi một request bình thường và vài request tấn công thử qua các route ẩn của riêng nó:
request dò file phải bị chặn, ở mức Nghiêm ngặt thì cả một SQL injection, và ở chế độ Chỉ ghi nhận thì
CrowdSec phải đếm được tấn công vừa gửi. Nếu Traefik không nạp được plugin, panel chuyển các trang sang
*không kiểm tra* và khởi động lại Traefik để nạp lại plugin (tối đa mười phút một lần). Các request kiểm
tra này không bao giờ hiện trong cảnh báo.

Khi WAF chuyển sang **Không hoạt động**, và khi **Bật** lại, các kênh thông báo nhận sự kiện của Traefik
sẽ được báo (Cài đặt → Thông báo).

## Key giữa Traefik và CrowdSec

Traefik gọi CrowdSec bằng một key do panel tạo (một Docker secret: không lưu trong database, không bao giờ
hiện ra). **Key mới (New key)** (Cài đặt → Bảo mật) tạo key khác: CrowdSec rồi Traefik nhận key mới
(Traefik khởi động lại: tạm ngừng vài giây), key cũ bị xoá khi key mới đã chạy. Bị ngắt giữa chừng thì
key cũ vẫn dùng được.

## Gỡ bỏ

Đặt **Tắt**: CrowdSec dừng, các route không còn đi qua WAF. Muốn gỡ hết những gì WAF tạo ra (CrowdSec,
volume và key của nó, plugin trong Traefik), chạy installer với `--remove-waf` sau khi đã tắt.

## Ai thấy gì

| Người dùng | Thấy |
|---|---|
| Quản trị viên panel (`system.manage`) | Cài đặt → Bảo mật: chế độ, mức, địa chỉ, cảnh báo, Key mới, Sửa chữa; tắt WAF của một service. |
| Sửa được một service | Thiết lập WAF của service đó; bật lại được, không tắt được. |
| Người khác | Không thấy gì. |

Cảnh báo (100 cái gần nhất; CrowdSec giữ 30 ngày, tối đa 5000) hiện thời gian, địa chỉ của khách, domain
và đường dẫn, và luật bị khớp. **Body của request không bao giờ được lưu** (body trên 1 MB nằm tạm trong
một file trong lúc request đi qua).

## Địa chỉ tin cậy

Địa chỉ trong danh sách này **hoàn toàn không bị kiểm tra** (văn phòng, dịch vụ giám sát). Giữ danh
sách ngắn: một địa chỉ tin cậy là một lối đi vòng qua WAF.

## Chia sẻ với cộng đồng CrowdSec

Tắt mặc định: không gửi gì cho CrowdSec, và CrowdSec không cần internet. Khi bật, CrowdSec gửi địa chỉ kẻ tấn công, luật và thời gian
cho cộng đồng (không bao giờ gửi request của bạn), và nhận lại tri thức chung.

## Khi dùng Cloudflare

- **Domain proxy qua Cloudflare** (đám mây cam): WAF của Cloudflare kiểm tra trước; WAF của panel là lớp
  thứ hai và thấy IP thật của khách.
- Rủi ro là ai đó **gọi thẳng IP server**, đi vòng qua Cloudflare. Với [Cloudflare
  Tunnel](/vi/apps/tunnel) không cần mở port nào: đóng 80 và 443 ở firewall của nhà cung cấp khi mọi
  domain đều đi qua Tunnel.
- **Domain chỉ dùng DNS** (đám mây xám): WAF của panel là lớp bảo vệ duy nhất.

## Giới hạn

- Port do service publish (Nâng cao → Port) không đi qua WAF: chỉ domain mới đi qua.
- Bộ luật là của CrowdSec, tại thời điểm phát hành panel: một kiểu tấn công hoàn toàn mới có thể lọt cho
  tới khi bản cập nhật panel mang bộ luật mới hơn.
- Chỉ 1 MB đầu của body được kiểm tra.
- Khi Traefik được khởi động lại không phải bởi panel (cập nhật server, chạy installer) và không nạp
  được plugin, các domain có WAF trả 404 cho tới lần kiểm tra kế tiếp của panel (tối đa một phút);
  sau đó panel phục vụ chúng mà không kiểm tra và khởi động lại Traefik để nạp plugin.
- Không dùng được `rdp-waf-canary.invalid` (và `*.invalid`) làm domain của service: route kiểm tra ẩn
  của panel dùng tên này.
