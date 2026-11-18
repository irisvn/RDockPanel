# Tường lửa ứng dụng web (WAF)

Trang **WAF** (trên menu, cạnh Firewall) bật một tường lửa đọc mọi request tới domain của các service
và chặn các kiểu tấn công đã biết trước khi chúng tới app của bạn. Các tab: **Tổng quan**, **Cảnh
báo**, **Bộ luật**, **Danh sách IP**, **Cloudflare** và **Cài đặt**.

## Hoạt động thế nào

Panel chạy WAF của [CrowdSec](https://www.crowdsec.net/) (bộ máy *AppSec*) thành một service riêng,
`rdp-crowdsec`, và gắn cho Traefik một plugin hỏi nó về từng request:

```text
 khách ──► Traefik ──► danh sách chặn ──► plugin WAF ──hỏi──► rdp-crowdsec (bộ luật)
                          │                   │  ◄── cho qua / chặn ──┘
                          │                   ├── cho qua ──► app của bạn
                          └───────────────────┴── chặn ─────► 403 Forbidden
```

- **Chỉ domain của các service** đi qua WAF. Địa chỉ của chính panel và việc xác minh chứng chỉ HTTPS
  thì không, nên bạn luôn vào được panel để đổi thiết lập.
- Body của request được đọc trước (kiểm tra 1 MB đầu; upload lớn hơn vẫn qua nguyên vẹn, phần dư không
  được kiểm tra), rồi request đi tiếp, không bị thay đổi. Mỗi request chiếm tối đa khoảng 1 MB bộ nhớ
  của Traefik: phần còn lại của upload lớn nằm tạm trong một file.
- WAF dùng IP thật của khách, kể cả khi qua Cloudflare và Cloudflare Tunnel.
- **Nếu WAF lỗi** (CrowdSec dừng, chậm, hoặc plugin không nạp được), request vẫn đi qua **mà không được
  kiểm tra**: WAF không bao giờ làm sập trang. Tab Tổng quan sẽ báo điều đó.

## Chế độ và mức độ

Ở tab **Cài đặt**:

| Chế độ | Điều gì xảy ra |
|---|---|
| **Tắt (Off)** | Không kiểm tra gì. CrowdSec được dừng. |
| **Chỉ ghi nhận (Detect only)** | Tấn công được **ghi lại, không bị chặn**. Bắt đầu ở đây khoảng một tuần và đọc cảnh báo. |
| **Chặn (Block)** | Tấn công bị trả **403 Forbidden** trước khi tới app. |

| Mức | Chặn được |
|---|---|
| **Cơ bản (Basic)** | Các lỗ hổng đã biết của phần mềm phổ biến (vá ảo cho các CVE đã công bố) và việc dò file bí mật (`/.env`, file backup…). **Không** chặn SQL injection hay XSS chung chung. |
| **Nghiêm ngặt (Strict)** | Cơ bản, thêm **OWASP Core Rule Set**: SQL injection, XSS, path traversal và hơn nữa. Có thể chặn nhầm request hợp lệ: hãy thử ở chế độ Chỉ ghi nhận trước. |

![WAF → Cài đặt](/shots/waf-settings.png)

## Bật WAF

1. Mở **WAF → Cài đặt**.
2. Chọn **Chỉ ghi nhận** và một mức, bấm **Lưu**. Tab Tổng quan hiện từng bước khởi động: chép plugin
   và bộ luật, khởi động CrowdSec, nạp plugin cho Traefik, rồi kiểm tra (CrowdSec nạp bộ luật: khoảng
   nửa phút). Đồng hồ cho biết đã chạy bao lâu, và **Xem log** mở cùng các bước đó ở trang Actions.
   Tối đa ba phút, kết quả là **Bật** hoặc **Không khả dụng** kèm lý do.
3. Xem **Cảnh báo** trong vài ngày. Có request hợp lệ trong danh sách (chặn nhầm)? Bỏ qua luật đó cho
   domain của nó (xem [Ngoại lệ](#ngoai-le)), tin cậy địa chỉ đó, hoặc tắt WAF cho service đó.
4. Chuyển sang **Chặn**.

Lần khởi động đầu, và mỗi lần tắt rồi bật lại, Traefik khởi động lại một lần: mọi domain ngừng phản
hồi vài giây.

Cài đặt → Panel → **Hệ thống** có dòng WAF cạnh Traefik và Docker: trạng thái, chế độ, log của
CrowdSec, **Mở** (trang WAF), **Sửa chữa** và **Khởi động lại CrowdSec**.

## Tổng quan: request và thống kê

- **Request đã kiểm tra** và **Request bị chặn** trong 24 giờ hoặc 7 ngày, theo từng giờ. Panel đọc bộ
  đếm của CrowdSec mỗi phút và giữ mỗi giờ một dòng (90 ngày). Request thử của chính panel không được
  tính. Giờ nào panel dừng thì hiện là khoảng trống, không bao giờ là số 0.
- **Ghi lại (Chỉ ghi nhận)**: các tấn công được ghi lại mà không bị chặn.
- Các luật, địa chỉ và domain xuất hiện nhiều nhất trong cảnh báo.

![WAF → Tổng quan](/shots/waf-overview.png)

## Cảnh báo

Mọi request khớp một luật: bị chặn (chế độ Chặn) hoặc được ghi lại (chế độ Chỉ ghi nhận). Mỗi phút
panel chép chúng từ CrowdSec vào database của mình và giữ **30 ngày**, 25 dòng một trang, lọc theo
domain, địa chỉ, luật, kết quả và khoảng thời gian. Mỗi dòng có thời điểm, địa chỉ (quốc gia và nhà
mạng nếu biết), domain và đường dẫn, và luật (`OWASP CRS: sql_injection` với Core Rule Set).

- **Không bao giờ lưu nội dung request.** Giá trị trong query trông như bí mật (`token`, `password`,
  `key`…) được che thành `***`.
- Ở chế độ Chỉ ghi nhận, cùng địa chỉ và đường dẫn được gộp mỗi phút một cảnh báo.
- Mỗi phút chép tối đa 1.000 cảnh báo (mới nhất); khi bị tấn công dồn dập hơn thế, thống kê vẫn đếm
  đủ mọi request.
- Menu của mỗi dòng: **Chi tiết**, **Bỏ qua luật này cho domain**, **Chặn địa chỉ**, **Tin cậy địa
  chỉ**.

![WAF → Cảnh báo](/shots/waf-alerts.png)

## Bộ luật

Bộ luật của CrowdSec gồm ba bộ: **Vá ảo**, **Luật chung** và **OWASP Core Rule Set**. Tab Bộ luật cho
biết phiên bản đang dùng, phiên bản kèm theo bản phát hành này và phiên bản mới nhất ở CrowdSec.

- **Kèm theo** (mặc định): bộ luật tải về lúc build bản phát hành của panel. CrowdSec khởi động từ đó,
  không cần internet, và mọi server cùng một bản có cùng bộ luật.
- **Kiểm tra cập nhật** hỏi hub của CrowdSec (khi WAF bật, panel cũng tự kiểm tra mỗi ngày một lần và
  báo qua kênh thông báo khi có bộ luật mới hơn). Bộ nào mới hơn sẽ hiện *mới hơn*.
- **Cập nhật bộ luật** tải bản mới nhất và khởi động lại CrowdSec với chúng (khoảng một phút, trong lúc
  đó request đi qua mà không được kiểm tra); log mở ra trong khi chạy. Nếu CrowdSec không khởi động
  được với chúng, bộ luật kèm theo tự được dùng lại. Sau đó WAF giữ bộ *mới nhất*, kể cả khi CrowdSec
  khởi động lại. Panel không bao giờ tự cài bộ luật mới: đó là quyết định của quản trị viên.
- **Quay về bộ kèm theo** dùng lại bộ luật của bản phát hành.

![WAF → Bộ luật](/shots/waf-rules.png)

## Ngoại lệ

Một lần chặn nhầm thường là một luật trên một domain. **Ngoại lệ** bỏ qua đúng luật đó cho domain đó:
mọi luật khác vẫn kiểm tra domain này, và luật đó vẫn kiểm tra mọi domain khác.

- Từ một cảnh báo: menu, **Bỏ qua luật này cho domain**. Panel lấy luật đã bắt request đó (mã của Core
  Rule Set như `942100`, hoặc tên như `crowdsecurity/vpatch-env-access`).
- Thêm tay: Bộ luật → **Thêm ngoại lệ**, nhập luật và domain (không dùng wildcard).
- Mỗi thay đổi khởi động lại CrowdSec (khoảng một phút). Tối đa 100 ngoại lệ.

## Danh sách IP: danh sách chặn

Mặc định tắt. Khi bật trong Cài đặt (**Chặn các địa chỉ cộng đồng CrowdSec báo cáo và các địa chỉ bạn
chặn**), nó chỉ hoạt động ở chế độ **Chặn**. Một plugin thứ hai đứng trước WAF giữ bản sao danh sách
địa chỉ bị cấm của CrowdSec (làm mới mỗi mười giây) và trả **403 trên mọi domain** cho các địa chỉ đó,
trước khi chạy bất kỳ luật nào.

- **Địa chỉ bạn chặn**: từ tab Danh sách IP, trang Firewall hoặc menu của một cảnh báo (xem
  [Chặn một địa chỉ](#chan-mot-dia-chi-tren-website-hay-tren-may-chu)).
- **Của cộng đồng**: khi bật **chia sẻ**, CrowdSec nhận thêm các địa chỉ cộng đồng báo cáo. Không chia
  sẻ thì chỉ các địa chỉ bạn chặn mới bị chặn.
- Các phát hiện của chính CrowdSec không bao giờ biến thành lệnh cấm: cảnh báo chỉ là cảnh báo, lệnh
  cấm là quyết định của bạn hoặc của cộng đồng.
- **Rủi ro:** trên mạng di động và một số nhà mạng, nhiều người dùng chung một địa chỉ (CGNAT). Khi đó
  một địa chỉ bị báo cáo có thể chặn cả khách thật. Tab này cho biết những gì đang bị chặn; bỏ chặn
  một địa chỉ ở đó, hoặc tắt danh sách chặn.
- Nếu CrowdSec ngừng phản hồi, các địa chỉ đã bị chặn vẫn bị chặn tới khi lệnh chặn hết hạn (plugin giữ
  bản sao danh sách), và không khách nào khác bị chặn.

![WAF → Danh sách IP](/shots/waf-lists.png)

## Chặn một địa chỉ: trên website hay trên máy chủ

Cùng một hộp thoại, ở trang WAF (Danh sách IP, menu của cảnh báo) và ở trang Firewall, chặn một địa
chỉ ở hai lớp. Chọn một hoặc cả hai:

| | Trên website (WAF) | Trên toàn máy chủ (Firewall) |
|---|---|---|
| Chặn gì | Mọi domain của các service (403) | Mọi thứ trừ SSH: mọi site và cả panel |
| Khách đi qua proxy Cloudflare hoặc Tunnel | Bị chặn (WAF đọc địa chỉ thật của họ) | **Không bị chặn**: máy chủ chỉ thấy Cloudflare |
| Nhận | Một địa chỉ | Một địa chỉ hoặc một dải mạng (`203.0.113.0/24`) |
| Thời hạn | Một ngày, một tuần hoặc ba mươi ngày | Như vậy, hoặc tới khi gỡ |
| Có hiệu lực | Trong vòng mười giây (danh sách chặn bật, chế độ Chặn) | Ngay (Firewall bật, panel cài trên Linux) |

![Chặn địa chỉ](/shots/waf-block.png)

- Hai trang hiện **cùng một danh sách**: mỗi địa chỉ bị chặn ở đâu và tới khi nào. Menu của nó bỏ chặn
  ở mọi nơi, chỉ trên website hoặc chỉ trên máy chủ.
- **Không bao giờ chặn**, ở cả hai lớp: dải IP của Cloudflare (mọi khách đi qua đó sẽ bị chặn), toàn
  bộ địa chỉ (`/0`), và **địa chỉ bạn đang dùng** (bạn sẽ mất panel hoặc các site). Hộp thoại báo trước
  khi bạn bấm Chặn.
- Trên máy chủ, lệnh chặn được áp dụng ngay, không qua bước "thử rồi giữ" của Firewall: chặn hay bỏ chặn
  một địa chỉ không thể khoá bạn ở ngoài (địa chỉ bị chặn vẫn vào được SSH, và địa chỉ của bạn bị từ
  chối). Các port bạn đã lưu nhưng chưa áp dụng vẫn là bản nháp. Nếu đang có một thay đổi Firewall chờ
  giữ, lệnh chặn được lưu và áp dụng ngay khi thay đổi đó được giữ hoặc hoàn tác (trong lúc chờ, danh sách
  hiện "chưa áp dụng trên máy chủ"; panel kiểm tra mỗi phút).
- Lệnh chặn có thời hạn sẽ được gỡ khi hết hạn (kiểm tra mỗi năm phút), rồi Firewall được áp dụng lại.
  Lệnh chặn của WAF thì CrowdSec tự hết hạn.

Chọn lớp nào? Khách tấn công **các site**: chặn trên website (chạy được sau Cloudflare). Một máy dò
**các port bạn đã mở**, hoặc cả một dải mạng bạn không bao giờ muốn tới máy chủ: chặn trên máy chủ.
SSH cố ý vẫn mở cho địa chỉ bị chặn (để không ai tự khoá mình): hãy bảo vệ SSH bằng key và firewall của
nhà cung cấp.

## Địa chỉ của Cloudflare

Request đi qua Cloudflare mang địa chỉ của khách trong một header. Traefik, giới hạn tần suất và WAF
**chỉ tin header đó khi nó đến từ các dải IP của Cloudflare**, nên cần danh sách hiện hành của
Cloudflare.

- Panel kiểm tra danh sách Cloudflare công bố mỗi ngày một lần (và khi bấm **Kiểm tra ngay**).
- Danh sách chỉ **bớt** dải thì được áp dụng ngay. Dải **mới** thì được đề xuất: bấm **Áp dụng**
  (Traefik khởi động lại, vài giây), và các kênh thông báo nhận sự kiện của Traefik sẽ được báo. Trước
  khi áp dụng, khách đi qua các dải mới bị nhìn thấy là địa chỉ của Cloudflare.
- Danh sách trông bất thường (dải nội bộ hoặc quá lớn, quá ít dải, đổi tới một nửa) bị từ chối và các
  dải đang dùng được giữ nguyên: tab hiện lỗi.
- Installer dùng cùng danh sách này khi chạy lại.

![WAF → Cloudflare](/shots/waf-cloudflare.png)

## Theo từng service

Mỗi service theo thiết lập của panel. Ở tab **Bảo mật** của service, bạn có thể đặt **Bật** hoặc **Tắt**:

- **Tắt** bỏ bảo vệ cho service đó: chỉ quản trị viên của panel được làm.
- Ai được sửa service thì được đặt lại theo panel hoặc **Bật**.

Áp dụng ngay khi lưu, không cần deploy lại.

![Service → Bảo mật](/shots/service-security.png)

## Trạng thái

| Trạng thái | Ý nghĩa | Cần làm gì |
|---|---|---|
| **Bật** | Panel vừa kiểm tra: một đòn tấn công thử đã bị chặn. | Không cần gì. |
| **Đang khởi động** | Panel đang khởi động WAF: Tổng quan hiện bước đang chạy. | Chờ (tối đa ba phút). |
| **Đang dừng** | Vừa tắt: panel đang gỡ WAF khỏi các route và dừng CrowdSec. | Chờ vài giây. |
| **Không khả dụng** | Request đi qua mà không được kiểm tra; có ghi lý do. | Bấm **Sửa chữa**. Vẫn không được? Xem lý do và log. |
| **Tắt** | WAF đang tắt. | — |

Mỗi phút panel gửi một request bình thường và vài đòn tấn công thử qua các route ẩn của riêng nó: đòn
dò file phải bị chặn, ở mức Nghiêm ngặt thì cả SQL injection, và ở chế độ Chỉ ghi nhận thì CrowdSec
phải đếm được đòn tấn công vừa thấy. Nếu Traefik không nạp được plugin, panel chuyển các trang sang
*không kiểm tra* và khởi động lại Traefik để nạp (tối đa mười phút một lần). Các request thử này không
bao giờ xuất hiện trong cảnh báo hay thống kê.

Khi WAF chuyển sang **Không khả dụng**, và khi **Bật** trở lại, các kênh thông báo nhận sự kiện của
Traefik sẽ được báo (Cài đặt → Thông báo).

## Key giữa Traefik và CrowdSec

Traefik hỏi CrowdSec bằng một key do panel tạo (một Docker secret: không bao giờ nằm trong database,
không bao giờ hiện ra). **Key mới** (trên đầu trang WAF) tạo key khác: CrowdSec rồi Traefik nhận nó
(Traefik khởi động lại: dừng vài giây), và key cũ được xoá khi key mới chạy được. Nếu bị gián đoạn, key
cũ vẫn dùng được.

## Gỡ bỏ

Đặt **Tắt**: CrowdSec dừng, các route chạy không qua WAF. Để xoá hết những gì WAF tạo ra (CrowdSec,
volume và key của nó, plugin trong Traefik), chạy installer với `--remove-waf` sau khi đã tắt.

## Ai thấy gì

| Người dùng | Thấy |
|---|---|
| Quản trị viên panel (`system.manage`) | Trang WAF và dòng của nó trong Hệ thống: trạng thái, thống kê, cảnh báo, bộ luật, ngoại lệ, danh sách IP, dải IP của Cloudflare, cài đặt, Key mới, Sửa chữa; tắt WAF của một service. |
| Được sửa một service | Thiết lập WAF của service đó; được bật lại, không được tắt. |
| Người khác | Không thấy gì. |

Mọi thay đổi (cài đặt, ngoại lệ, cập nhật bộ luật, chặn địa chỉ, dải IP của Cloudflare) đều có trong
nhật ký audit; các lần khởi động, cập nhật bộ luật và khởi động lại còn có trong trang Actions kèm log.

## Địa chỉ tin cậy

Các địa chỉ trong Cài đặt **không bao giờ bị kiểm tra** (văn phòng, dịch vụ giám sát). Giữ danh sách
ngắn: một địa chỉ tin cậy là một lối đi vòng qua WAF. Dải IP của Cloudflare và `0.0.0.0/0` bị từ chối:
khách đi qua Cloudflare đã được nhận diện bằng địa chỉ thật của họ.

## Chia sẻ với cộng đồng CrowdSec

Mặc định tắt: không gửi gì cho CrowdSec, và CrowdSec không cần internet trừ khi cập nhật bộ luật. Khi
bật, CrowdSec gửi IP kẻ tấn công, luật và thời gian cho cộng đồng (không bao giờ gửi request của bạn),
và nhận danh sách của cộng đồng (danh sách chặn dùng nó khi được bật).

## Sau Cloudflare

- **Domain proxy** (đám mây cam): WAF của Cloudflare kiểm tra trước; WAF của panel là lớp thứ hai và
  thấy IP thật của khách.
- Rủi ro là có người **truy cập thẳng IP server**, đi vòng qua Cloudflare. Với
  [Cloudflare Tunnel](/vi/apps/tunnel) không cần mở cổng nào: đóng 80 và 443 ở firewall của nhà cung cấp
  khi mọi domain đều dùng Tunnel.
- **Domain chỉ DNS** (đám mây xám): WAF của panel là lớp bảo vệ duy nhất.

## Giới hạn

- Cổng do service mở ra (Nâng cao → Cổng) không đi qua WAF: chỉ domain mới qua.
- Một kiểu tấn công hoàn toàn mới có thể lọt qua cho tới khi bộ luật được cập nhật (Bộ luật → Cập nhật,
  hoặc cập nhật panel).
- Chỉ 1 MB đầu của body được kiểm tra.
- Khi Traefik được khởi động lại bởi thứ khác ngoài panel (cập nhật server, installer) và không nạp được
  plugin, các trang có WAF trả 404 cho tới lần kiểm tra kế tiếp của panel (tối đa một phút); khi đó panel
  cho chúng chạy không kiểm tra và khởi động lại Traefik để nạp plugin.
- `rdp-waf-canary.invalid` (và `*.invalid`) không dùng được làm domain của service: route thử ẩn của panel
  dùng nó.
