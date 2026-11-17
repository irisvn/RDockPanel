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

## Ai đọc được giá trị

- Giá trị được mã hoá bằng key của panel (`APP_KEY`) và không bao giờ được ghi vào audit log hay log deploy.
- Xem giá trị ở tab Biến môi trường cần quyền `services.secrets` (developer và quản trị của project có quyền này). **Mỗi lần đọc đều được ghi vào audit log.**
- Không có quyền đó, tab chỉ hiện tên biến. Bạn vẫn ghi được biến môi trường, nhưng **lưu sẽ thay toàn bộ biến**: phải gõ lại tất cả.
- API token chỉ đọc được giá trị khi có scope "Đọc giá trị biến môi trường".

## Placeholder

Các placeholder sau được thay trong giá trị mỗi lần deploy:

| Placeholder | Thành |
|---|---|
| `$(PROJECT_NAME)` | tên project |
| `$(SERVICE_NAME)` | tên service |
| `$(PRIMARY_DOMAIN)` | domain đầu tiên của service (nếu không có thì domain tự sinh) |
| `$(EASYPANEL_DOMAIN)` | domain tự sinh của service |
| `$(EASYPANEL_HOST)` | domain cho app đặt lúc cài |
| `$(EASYPANEL_HOST_IP)` | địa chỉ public của server |

Placeholder không biết hoặc chưa có giá trị thì giữ nguyên như đã viết. Template dùng chúng; bạn cũng dùng được.

## Rollback

Mỗi lần deploy giữ lại biến môi trường nó đã áp dụng (được mã hoá). **Rollback về lần này** đưa lại biến môi trường của lúc đó, không phải của hiện tại (xem [Deploy](/vi/apps/deploy)).

::: tip Lưu ý
- Toàn bộ biến môi trường tối đa 32 KB; nhiều hơn sẽ bị từ chối kèm thông báo rõ ràng.
- Bản thân Docker giữ giá trị ở dạng rõ trong service đang chạy và các container: ai có terminal trong container, hoặc quyền root trên server, đều đọc được.
- Thông tin đăng nhập của database nằm trong biến môi trường của nó; đổi ở đây không đổi được mật khẩu của database đã tạo (xem [Database](/vi/apps/databases)).
- Với service Compose, biến môi trường là file `.env` của project Compose, đọc bằng `${VAR}` (xem [Compose](/vi/apps/compose)).
- Mật khẩu registry (tab Nguồn) cũng được mã hoá và không bao giờ hiện ra.
:::
