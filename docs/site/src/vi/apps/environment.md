# Biến môi trường và secret

Biến môi trường của service được sửa dưới dạng text `.env` ở tab **Biến môi trường** (Environment). Giá trị được mã hoá trong panel và chỉ hiện cho người có quyền đọc.

## Sửa biến môi trường

1. Mở tab **Biến môi trường** của service.
2. Mỗi dòng một `NAME=value`. Dòng bắt đầu bằng `#` là chú thích. Giá trị có dấu nháy và giá trị nhiều dòng dùng được như trong file `.env`.
3. Bấm **Lưu**.
4. Bấm **Deploy**: thay đổi áp dụng ở lần deploy tới.

```sh
# Database
DATABASE_URL=postgres://app:secret@shop_db:5432/app
APP_ENV=production
```

![Tab Biến môi trường](/shots/service-environment.png)

## Giá trị được bảo vệ thế nào

Theo dõi một biến, `DB_PASSWORD=abc123`, từ lúc bấm **Lưu** tới khi app chạy:

1. **Lưu**: panel mã hoá toàn bộ biến môi trường bằng **AES-256-GCM** với khoá của panel,
   `APP_KEY`. Khoá được tạo lúc cài, giữ trong Docker secret (`rdp_app_key`) và trong
   `/etc/rdockpanel/secrets`, không bao giờ nằm trong database.
2. **Trong database**: dòng của service chỉ chứa chuỗi đã mã hoá. Có bản dump hay backup database mà
   không có `APP_KEY` thì không đọc được. Chuỗi mã hoá còn gắn với đúng bảng và cột của nó: chép sang
   chỗ khác cũng không giải mã được.
3. **Mỗi lần deploy**: lần deploy giữ một bản sao đã mã hoá của riêng nó (để **Rollback về lần này**).
   Log deploy và audit log chỉ ghi **tên** biến, không bao giờ ghi giá trị.
4. **Giao cho Docker**: panel giải mã và đưa giá trị cho Swarm service. Từ đây **Docker giữ giá trị ở
   dạng rõ**, vì app của bạn phải đọc được: trong cấu hình service và trong biến môi trường của từng
   container.

```text
 Lưu ──► mã hoá trong database của panel ──► Deploy ──► giải mã ──► Docker (dạng rõ) ──► app của bạn
         (APP_KEY, AES-256-GCM)               (bản sao mã hoá giữ lại để rollback)
```

::: warning Giữ APP_KEY
Sao lưu `/etc/rdockpanel` cùng với database (xem [Cập nhật và sao lưu](/vi/guide/update)). Database
khôi phục mà không có `APP_KEY` thì biến môi trường, mật khẩu registry và file mount đều không đọc
được nữa: phải nhập lại.
:::

## Ai thấy gì

| Người dùng | Ở tab Biến môi trường |
|---|---|
| Có quyền `services.secrets` (developer, quản trị của project, quản trị viên panel) | Toàn bộ `.env` kèm giá trị; sửa được. **Mở tab là một lần đọc bí mật: được ghi vào audit log trước** (không ghi được audit thì không trả giá trị). |
| Sửa được service nhưng không có `services.secrets` | Ô trống và một cảnh báo liệt kê **tên** các biến. Vẫn lưu được, nhưng lưu sẽ **thay toàn bộ biến**: phải gõ lại tất cả, kể cả biến không định sửa. |
| Chỉ xem được service | Không thấy giá trị, không sửa được. |
| API token | Chỉ đọc giá trị khi có scope "Đọc giá trị biến môi trường". |

Ở những chỗ khác:

- **Docker → Containers → Inspect** che các biến có tên giống bí mật (`PASSWORD`, `TOKEN`, `KEY`,
  `SECRET`, `AUTH`…) và mật khẩu trong URL. Bí mật đặt tên lạ vẫn có thể lộ ở đó: hãy đặt tên rõ ràng.
- **Root trên server**, hoặc **terminal trong container**, thấy mọi giá trị: đó là nơi app đọc chúng.

## Placeholder

Các placeholder sau được thay trong giá trị mỗi lần deploy:

| Placeholder | Thành |
|---|---|
| `$(PROJECT_NAME)` | tên project |
| `$(SERVICE_NAME)` | tên service |
| `$(PRIMARY_DOMAIN)` | domain đầu tiên của service (nếu không có thì domain tự sinh) |
| `$(RDP_DOMAIN)` | domain tự sinh của service |
| `$(RDP_HOST)` | domain cho app đặt lúc cài |
| `$(RDP_HOST_IP)` | địa chỉ public của server |

Placeholder không biết hoặc chưa có giá trị thì giữ nguyên như đã viết. Template dùng chúng; bạn cũng dùng được.

## Rollback

Mỗi lần deploy giữ lại biến môi trường nó đã áp dụng (được mã hoá). **Rollback về lần này** đưa lại biến môi trường của lúc đó, không phải của hiện tại (xem [Deploy](/vi/apps/deploy)).

::: tip Lưu ý
- Toàn bộ biến môi trường tối đa 32 KB; nhiều hơn sẽ bị từ chối kèm thông báo rõ ràng.
- Thông tin đăng nhập của database nằm trong biến môi trường của nó; đổi ở đây không đổi được mật khẩu của database đã tạo (xem [Database](/vi/apps/databases)).
- Với service Compose, biến môi trường là file `.env` của project Compose, đọc bằng `${VAR}` (xem [Compose](/vi/apps/compose)).
- Mật khẩu registry (tab Nguồn) cũng được mã hoá và không bao giờ hiện ra.
:::
