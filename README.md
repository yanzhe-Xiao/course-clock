# 课时钟

Windows 10/11 x64 便携应用。使用 Rust 和原生 Win32，提供课表、待办、托盘提醒和 WebDAV 双机同步。

## 开始使用

1. 双击 `dist\CourseClock.exe`。程序在同一目录下创建 `CourseClock-data`。关闭主窗口后仍会托盘提醒，右键托盘图标可以退出。
2. 在「导入课表」选择文件、拖入文件或粘贴文件路径。支持西工大 CSV、教务系统导出的 HTML 和 ICS。选择教务系统保存的 `首页.html` 时，程序会自动读取旁边的 `首页_files\course-table.html`。CSV/HTML 需要填写第一教学周的周一，并选择长安、友谊夏季或友谊冬季作息。核对预览后点击「确认导入」。
3. 「周课表」的七列共用一条时间轴，可用滚轮或右侧滚动条上下浏览。课块显示课程名、教师和教室，长文字会换行，必要时会拉开对应的时间段。具体时间、完整地点和提醒设置可在课程详情查看。点击「编辑课程」修改，点击「返回」回到原滚动位置。重叠课程在课表中并排显示，也会在「冲突」页面列出。导入的课程默认提前 10 分钟提醒。
4. 新建或编辑待办和课程时，点击日期打开月历，点击时刻选择小时和分钟。待办的截止时间可留空。完成每天、每周或工作日待办后会自动生成下一次。
5. 在「设置」可试播右上角弹窗、启用登录启动、配置 WebDAV，以及导出或恢复本地备份。弹窗跟随鼠标所在的屏幕，点击弹窗正文可收起。弹窗也提供查看、推迟 5 分钟，以及完成待办。

西工大 CSV 支持 `课程名称,星期,开始节数,结束节数,老师,地点,周数` 表头。周次可写成 `1-8`、`1-8(单)`、`1-8(双)` 或逗号分隔的组合。CSV 和 HTML 的上课时间采用北京时间。友谊校区换季、节假日调课和停课请对照教务通知，可以在应用内调整单次课。重新导入会添加或更新匹配的课次，旧课次可在周课表手动删除。ICS 重复事件会展开到未来约 18 个月，导入预览会报告跳过的全天或无法解析事件。

周课表按课程名分配颜色。同名课程在不同周保留同色。36 个浅色取自 Color Hunt 的 [Light Sunset](https://colorhunt.co/palettes/light-sunset)、[Light Green](https://colorhunt.co/palettes/light-green/1000) 和 [Pastel Purple](https://colorhunt.co/palettes/pastel-purple)。

HTML 导入针对西工大课表页面的 `courseTable` 和 `course-name` 结构做了解析。如果教务系统更换页面结构，CSV 导入仍是稳定的回退方式。

## 双机同步

在两台电脑的「设置」填写同一个已创建的 HTTPS WebDAV 目录，并输入账号和应用密码。目录中会创建 `course-clock.json`。服务器需要支持强 `ETag`、`If-Match`、`If-None-Match` 和测试文件删除。课程、待办、完成状态和学期第一周会双向同步，设备各自保留弹窗状态、稍后提醒和启动设置。同步不可用时本地提醒照常工作。

同一条记录的不同字段可以自动合并。双方同时修改同一字段时会保留一个标记为「冲突副本」的版本，可以在「冲突」页面编辑或标记处理。一边编辑另一边删除时会保留编辑内容。WebDAV 应用密码由当前 Windows 账户加密保存，每台电脑分别填写。本地手动备份保存在 `CourseClock-data\backups`，最多保留 12 份。

## 从源代码构建

在安装了 MSVC 构建工具的 Windows Rust 环境中执行 `cargo build --release --locked`。这个工作区使用安装在 D 盘的 LLVM-MinGW 生成 `dist\CourseClock.exe`。使用现有 D 盘工具链时，可在 PowerShell 运行以下命令。

```powershell
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = 'D:\Tools\llvm-mingw\bin\x86_64-w64-mingw32-clang.exe'
cargo test --locked --target x86_64-pc-windows-gnullvm
cargo rustc --locked --release --target x86_64-pc-windows-gnullvm -- -C target-feature=+crt-static
```

默认把本地数据放在可执行文件旁。需要独立数据目录时，可以用 `COURSE_CLOCK_DATA` 指定可写路径。使用 `--tray` 参数可直接启动托盘模式。
