# Rules cho AI Agent

## 2. Style giao diện
- Khi sửa giao diện hoặc tạo giao diện mới, phải giữ style chung của dự án.
- Nếu style khác với style chung, cần người dùng confirm trước khi làm.

## 3. Tái sử dụng
- Tái sử dụng component chung / function chung trước khi tạo mới.

## 4. Database
- Tạo / chỉnh sửa cấu trúc database cần tuân thủ pattern chung của dự án.

## 5. Change-log
- Mỗi khi hoàn thành 1 nhiệm vụ: tóm tắt thay đổi đã làm vào thư mục `change-logs` (chưa có thì tạo mới).
- Mỗi ngày sẽ có 1 file riêng (chưa có thì tạo mới), ví dụ: `change-logs/30-10-2026.md`.

## 6. Nhiệm vụ test không cần ghi change-log
- Khi thực hiện các nhiệm vụ **test** (test API / test giao diện...), **không cần** cập nhật file change-log theo rule 5.

## 7. Nhiệm vụ review merge / compare nhánh không cần ghi change-log
- Khi thực hiện các nhiệm vụ **review merge** hoặc **so sánh (compare) các nhánh** (vd: `git diff`, so sánh code giữa 2 branch...), **không cần** cập nhật file change-log theo rule 5.

## 8. Cập nhật test khi thay đổi logic / nghiệp vụ
- Khi thêm / sửa code mà thay đổi **logic hoặc nghiệp vụ**, phải cập nhật (thêm / sửa) test tương ứng.
- Trước khi hoàn thành nhiệm vụ, phải chạy toàn bộ test và đảm bảo **tất cả test pass**.
