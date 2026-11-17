# API

Deploy, khởi động lại và kiểm tra service từ script và CI bằng API công khai của panel, `/api/v1`,
kèm một token.

Danh sách đầy đủ các route nằm trong
[docs/api.md](https://github.com/irisvn/RDockPanel/blob/main/docs/api.md).

## Tạo token

1. Mở **Cài đặt → API token** (Settings → API tokens) và bấm **Token mới** (New token). Mỗi người
   dùng có token riêng.
2. Đặt **Tên** (Name, ví dụ `CI`), tick các **Scope**, và chọn **Hết hạn sau (ngày)** (Expires in
   (days), tối đa 365).
3. Bấm **Tạo** (Create) và copy token: **token sẽ không hiện lại**.

Token làm được những gì người dùng của nó được làm (vai trò của họ, và vai trò trong các project) và
**không bao giờ vượt quá scope của nó**: cả hai phải cho phép. Muốn ngừng một token, dùng
**Thu hồi…** (Revoke…) ở dòng của nó.

| Scope | Cho phép |
|---|---|
| `read` | xem project, service, lần deploy và domain |
| `deploy` | deploy và khởi động lại service |
| `secrets` | đọc giá trị biến môi trường (mỗi lần đọc đều được ghi audit) |
| `write` | sửa và thêm service, domain (cho các route sắp có) |
| `logs` | đọc log của service (cho các route sắp có) |

## Gọi API

Gửi token trong header `Authorization`:

```sh
export RDP_TOKEN=rcms_…
curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/me
```

Service được gọi bằng id hoặc bằng `project/service`, mã hoá URL: `shop/web` thành `shop%2Fweb`.

| Method | Path | Scope | Tác dụng |
|---|---|---|---|
| GET | `/me` | bất kỳ | người dùng và scope của token |
| GET | `/projects` | read | các project bạn thấy, kèm service |
| GET | `/services/{id}` | read | image, trạng thái, replicas, domain, lần deploy gần nhất |
| POST | `/services/{id}/deploy` | deploy | xếp lịch một lần deploy: 202 `{"deployment": …}` |
| POST | `/services/{id}/restart` | deploy | xếp lịch khởi động lại: 202 |
| GET | `/deployments/{id}` | read | trạng thái, lỗi và log của một lần deploy |
| GET | `/services/{id}/env` | secrets | biến môi trường, `{"env": {NAME: value}}` |
| GET | `/services/{id}/domains` | read | các domain của service |

## Ví dụ

Liệt kê các project:

```sh
curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/projects
```

Deploy `shop/web`, rồi theo dõi lần deploy bằng id của nó:

```sh
curl -X POST -H "Authorization: Bearer $RDP_TOKEN" \
  https://panel.example.com/api/v1/services/shop%2Fweb/deploy

curl -H "Authorization: Bearer $RDP_TOKEN" https://panel.example.com/api/v1/deployments/42
```

## Deploy không cần token

Hai loại URL deploy một service mà không cần token của người dùng:

- **URL kích hoạt deploy** (Deployment trigger, ở tab **Nguồn** của service): gửi `POST` tới URL
  là deploy service. URL chỉ hiện một lần; nếu lộ thì tạo URL mới (URL cũ sẽ không dùng được nữa).
  Ai có URL đều deploy được service, nhưng không sửa được nó.

  ```sh
  curl -X POST "https://panel.example.com/api/v1/deploy/12?token=…"
  ```

- **Tự deploy (GitHub)** (Auto deploy (GitHub), cùng tab, cho nguồn Git): một webhook GitHub có chữ
  ký, deploy mỗi khi push lên nhánh của service.

## Phản hồi và giới hạn

- Lỗi trả về JSON `{"code", "message"}`: **401** khi không có token hợp lệ, **403** khi ngoài scope
  hoặc ngoài project của người dùng, **404** cho thứ không tồn tại hoặc nằm trong project bạn không
  phải thành viên (cố ý trả lời giống nhau), **409** khi service đang có lần deploy khác chạy.
- Request bị giới hạn theo người dùng và route: 120 lần một phút, deploy và restart 10 lần. Vượt
  quá: **429**.
- Service Compose, hoặc service có thêm capability, sysctl hay group, không deploy được qua API:
  hãy deploy trong panel.
- `GET /api/v1/ping` (không cần token) trả về `{"pong": true}`: tiện để kiểm tra panel còn sống.

## Dòng lệnh `rdp`

`rdp` gói API này lại: `rdp login`, `rdp projects`, `rdp status shop/web`,
`rdp deploy shop/web --wait`, `rdp logs 42`… Nó được build cùng panel từ mã nguồn; xem
[docs/api.md](https://github.com/irisvn/RDockPanel/blob/main/docs/api.md#the-rdp-command-line).
