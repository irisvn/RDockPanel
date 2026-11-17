# Nguồn và template

Image của app lấy từ đâu, đặt ở tab **Nguồn** (Source): một Docker image, một repo Git được build trên server, hoặc một Dockerfile viết ngay trong panel. Template tạo sẵn cả app chỉ trong một lần.

## Tạo service

Trong một project, bấm **Thêm service** (Add service), chọn **App (Docker image)**, gõ tên và image (ví dụ `ghcr.io/org/app:v2`), rồi bấm **Tạo** (Create). Sau này có thể đổi nguồn ở tab **Nguồn**.

![Service mới](/shots/new-service.png)

## Docker image

Image được pull mỗi lần deploy và ghim theo digest chính xác, nên rollback sẽ chạy lại đúng image đó.

Với **image private**, điền **User registry** (Registry user) và **Mật khẩu hoặc token registry** (Registry password or token):

- mật khẩu được mã hoá và không bao giờ hiện lại (ô nhập báo là đã đặt; để trống để giữ nguyên, hoặc bấm **Xoá mật khẩu**);
- thông tin đăng nhập gắn với registry ghi trong tên image (Docker Hub nếu image không ghi registry).

![Tab Nguồn](/shots/service-source.png)

## Repo Git

Panel clone repo và build bằng Dockerfile của repo ngay trên server này. Output của quá trình build chính là log của lần deploy.

1. Chọn **Repo Git** (Git repository) ở tab **Nguồn**.
2. Điền **URL repo** (Repository URL): `https://…` cho repo public, hoặc `git@github.com:org/app.git` kèm deploy key.
3. Đặt **Nhánh, tag hoặc commit**, **Thư mục build** (Build path: thư mục trong repo để build, mặc định `/`) và **Đường dẫn Dockerfile** (mặc định `Dockerfile`).
4. Bấm **Lưu**, rồi **Deploy**.

**Repo private**, có hai cách:

- **Deploy key**: bấm **Tạo key** (Generate a key), thêm public key hiện ra vào repo dưới dạng deploy key chỉ đọc, và dùng URL ssh. **Key mới** (New key) thay key cũ (key cũ ngừng hoạt động).
- **GitHub token**: quản trị viên thêm token ở **Cài đặt → Git** (Settings → Git) và chọn các project được dùng. Trong service, ô **Clone bằng** (Clone with) chọn token, rồi **Chọn repo** (Choose a repository) liệt kê các repo và branch của token đó.

GitHub token có thể là loại fine-grained (quyền đọc Contents và Metadata) hoặc classic (quyền `repo`). Developer của các project được giao token xem được và clone được mọi repo mà token truy cập được.

### Deploy khi push

- **Tự deploy (GitHub)** (Auto deploy): bấm **Bật** (Enable). Copy URL và secret hiện ra (chỉ hiện một lần) vào repo: Settings → Webhooks → Add webhook, content type `application/json`, chọn sự kiện `push`. Mỗi lần push lên branch của service sẽ deploy nó; các lần push tới trong lúc đang deploy sẽ được deploy khi lần đó xong.
- **URL kích hoạt deploy** (Deployment trigger): **Tạo URL** (Make a URL) cho một URL, gửi `POST` tới đó (ví dụ một bước CI) là service được deploy. URL chỉ hiện một lần; **URL mới** (New URL) thay URL cũ.

Cả hai hiện ở service build từ Git (URL kích hoạt có cả ở service dùng Dockerfile).

## Dockerfile viết trong panel

Chọn **Dockerfile** và viết trong ô soạn thảo. Mỗi lần **Deploy**, nó được build trên server này, giống như repo.

## Template {#template}

Template là một app làm sẵn, kèm database và cấu hình. Danh mục có hơn 800 template, được tải từ một danh mục mã nguồn mở công khai khi bạn mở nó.

1. Trong một project, bấm **Template** (Templates).
2. **Tìm template**, rồi chọn một cái.
3. Điền form của nó (giá trị mặc định đã có sẵn).
4. Để **Deploy ngay** (Deploy now) bật nếu muốn deploy luôn (mặc định bật nếu bạn có quyền deploy), hoặc tắt để xem trước. Bấm **Tạo** (Create).

Các service được tạo cùng lúc (database trước): hoặc tạo hết, hoặc không tạo gì. Phần nào của template không làm được ở đây sẽ được báo trong danh sách cảnh báo, không bị bỏ đi âm thầm.

**Tạo từ Schema** (Create from schema) nhận kết quả của một template dán vào dưới dạng JSON (`{"services": [...]}`), dùng cho template bạn đã chạy ở nơi khác.

![Template](/shots/templates.png)

::: tip Lưu ý
- Việc build chạy trên server của panel; repo gửi cho Docker tối đa 1 GB (không tính `.git` và những gì `.dockerignore` liệt kê).
- Chạy, Dừng và Khởi động lại dùng lại image đã build. **Deploy** sẽ build lại; **Build lại từ đầu** (Force rebuild) build không dùng cache.
- Template dùng Compose chỉ dành cho quản trị viên (`system.manage`).
- Panel kiểm tra RAM trước khi template được deploy (xem [Deploy](/vi/apps/deploy#canh-bao-ram-va-o-dia)).
:::
