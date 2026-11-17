# Cài trên macOS

Chạy RDockPanel trên máy Mac Apple silicon, cùng installer và cùng image như trên server Linux.

## Trước khi bắt đầu

- Một máy Mac **Apple silicon**.
- **[OrbStack](https://orbstack.dev)** hoặc **Docker Desktop**, đã chạy, và đặt tự chạy khi đăng
  nhập. Panel được thử với OrbStack; Docker Desktop chắc cũng chạy như vậy (bật chế độ Swarm).
- Port **80** và **443** còn trống trên Mac (thêm **3000** khi panel chưa có domain).

::: tip Mẹo: giấy phép
OrbStack và Docker Desktop cần giấy phép trả phí cho một số trường hợp dùng trong công ty: hãy kiểm
tra điều khoản của chúng.
:::

## Cài đặt

1. Trong terminal, **chạy bằng user của bạn, không `sudo`**:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
     | sh -s -- --admin-email you@example.com
   ```

2. Ghi lại mật khẩu quản trị viên được in ra.
3. Mở http://localhost:3000/admin và đăng nhập. Từ máy khác cùng mạng, dùng
   `http://<địa chỉ của Mac>:3000` khi Mac còn giữ địa chỉ đó.

Các tuỳ chọn giống trên Linux (`--version`, `--database`, `--domain`…): xem
[Cài trên Linux](/vi/guide/install#options). Chạy lại đúng lệnh đó để nâng cấp hoặc đổi tuỳ
chọn; **Cài đặt → Phiên bản → Cập nhật** (Settings → Version → Update) chạy như trên server.

## Mọi thứ nằm ở đâu

- `~/.rdockpanel` chứa những gì `/etc/rdockpanel` chứa trên Linux: `install.conf`, `secrets/`, route
  và chứng chỉ của Traefik, thêm `backups/` (backup cục bộ và bản copy database khi cập nhật) và
  `compose/` (file của các service Compose).
- Database của panel nằm trong một Docker volume: `rdp-mysql-data` (MySQL) hoặc `rdp-panel-data`
  (SQLite).

## Khác gì so với server Linux

- **Docker chạy trong một VM Linux.** "Server" mà panel hiện (CPU, RAM, ổ đĩa, mạng) là VM đó, và
  bước kiểm tra RAM trước khi deploy tính theo RAM của VM, không phải của cả máy Mac.
- **Giới hạn tài nguyên do Docker đặt.** OrbStack dùng mọi CPU và lấy RAM theo nhu cầu container,
  tới giới hạn của nó (Settings → System của OrbStack, hoặc `orb config set memory_mib 12288`, rồi
  khởi động lại OrbStack). Của Docker Desktop nằm ở Settings → Resources (CPU, RAM, swap, ổ đĩa).
  Hãy chừa cho macOS vài GB.
- **Không có Firewall và Reboot.** Chúng sẽ tác động lên VM. Dùng firewall của chính Mac (System
  Settings → Network → Firewall), và tự khởi động lại Docker hoặc Mac.
- **Mac phải thức và Docker phải chạy** thì app mới trả lời. Khi Docker chạy lại, Swarm tự chạy lại
  panel và các service.
- **Không có địa chỉ công khai.** Mac nằm sau router ở nhà hay văn phòng thì Let's Encrypt qua HTTP
  không tới được. Với domain thật, dùng [Cloudflare Tunnel](/vi/apps/tunnel) (không cần mở port)
  hoặc Let's Encrypt qua DNS (Cài đặt → Chứng chỉ). Hoặc dùng tên như `app.localhost`, truy cập
  ngay trên Mac.
- **Thư mục host**: thư mục host của service thấy đường dẫn của VM; chỉ thư mục dưới `/Users` là của
  Mac. Thư mục host bị tắt trừ khi quản trị viên cho phép một thư mục (xem
  [Lưu trữ và volume](/vi/apps/storage)).
- **Lấy lại IP** (Refresh IP) tìm địa chỉ của mạng Mac đang dùng, không phải của một server.

## Lưu ý

- Installer dừng khi một port cần dùng đã bị chiếm, và nêu tên chương trình. Chương trình do user
  khác chạy (kể cả root) có thể không thấy được: khi đó Docker không chạy được Traefik.
- Đừng chạy panel đã cài và một bản dev (`rustcms serve`) trên cùng Docker: installer dừng khi thấy
  Traefik của bản dev.

## Gỡ cài đặt

Làm theo [Gỡ cài đặt trên Linux](/vi/guide/install#uninstall) **không `sudo`**, với các thay đổi:

- khi container của panel đã biến mất (vài giây sau `docker service rm`), xoá volume
  `rdp-panel-data` (SQLite) hoặc `rdp-mysql-data` (MySQL);
- xoá `~/.rdockpanel` thay cho `/etc/rdockpanel` và `/var/lib/rdockpanel/…`, nhưng giữ
  `~/.rdockpanel/compose` cho tới khi các service Compose và dữ liệu của chúng đã được gỡ.
