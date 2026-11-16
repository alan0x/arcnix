# 0.1 验证记录

## 主题并入常规设置（2026-11-13）

移除 Appearance 分类，General 同时展示 Language 与 Theme，已有主题设置不变。结构检查 5 组与导航回归 6 组通过，验证入口移除、主题即时切换和全新进程恢复；同步修正主题相关脚本的导航入口。运行中的用户窗口保留，新版重新打开后生效。

## 设置内容结构（2026-11-13）

设置按用途重新分组，版本／更新独立放入 About；页内标题使用具体用途，不重复 General／Appearance 等分类名。日志显示和导出与写文件分开，未启用写文件时收起其选项，空错误节点不再渲染。

`tools/settings_structure_smoke.py` 新增 5 组真实 WebView 检查，覆盖八个分类的精确可见分组、About 的版本及更新归属、隐藏无用下载／安装操作、导出按钮宽度、关闭文件日志时的折叠、真实非目录路径引起的 I/O 错误及恢复、中英文、主题、窄窗口和新进程恢复。截图在 `artifacts/settings-structure-ui`，未修改真实服务器数据。

原有站点／编辑／偏好 5 组、更新安装及回滚 5 组、导航 6 组回归通过；其中更新通知跳转 About 经过实际倒计时验证。工作区常规 Rust 测试 92 项通过、47 项需要 fixture 的测试保持忽略；严格 Clippy、Rustfmt、typos 和 Windows release 构建通过。本次为界面结构变更，真实 SSH 场景由上述 WebView 回环测试覆盖。验证后重新打开正式版。

## 站点设置、远程编辑与更新（2026-11-13）

完整工作区 138 项 Rust 测试（含显式启用的回环 SSH/SFTP）通过，另增 1 项 Windows 原生 COM 检查，实际创建文件／目录选择器并验证选项被系统接受。严格 Clippy、Rustfmt、typos、核心文档和 Windows release 构建通过；静态 OpenSSL 缺少 PDB 仅产生调试信息链接警告，不影响构建或运行。

- `tools/workspace_preferences_smoke.py`：5 组实际 WebView 检查通过，验证每服务器默认／记忆路径、断开重连与全新进程恢复、显示格式、连接覆盖、文件日志落盘、远程 UTF-8 BOM／CRLF 保存、同大小同时间修改的摘要冲突保护、放弃草稿、二进制拒绝、内容不进入日志，以及全局 Skip／服务器 Overwrite 的真实上传结果。
- `tools/update_smoke.py`：5 组隔离更新检查通过，涵盖 404／当前版／缺附件、摘要错误拒绝、服务器停止发送时快速取消、程序版本不匹配拒绝、SSH 连接期间仅下载、断开后倒计时与稍后、手动替换和启动确认、失败恢复原程序并重启，以及完整的定时检查到自动安装。辅助进程通过就绪握手后主程序才退出。测试程序和替换目标仅位于 `artifacts/update-ui` 的本次运行目录，原安装与真实配置未被替换。
- `tools/release_desktop_smoke.py`：2 组正式构建检查通过，证明 release 忽略开发专用更新源覆盖并成功访问固定的正式接口；真实回环 SSH 编辑、Ctrl+S 保存 CRLF、深色背景和窄窗口边界通过。正式接口本次返回暂无已发布版本；未创建 tag 或发布任何附件。
- 原有文件右键 8 组、删除／权限／显示列 6 组、导航 6 组、传输进度 5 组回归全部通过。加上上述新检查，共 37 组 WebView 场景。
- `tools/package_release.py` 的 Windows 打包函数验证 ZIP 与独立更新 EXE 的字节 SHA-256 一致，发布 tag 校验通过；仅检查本地产物，未上传。

汇总日志为 `artifacts/workspace-final-tests.log`、`native-picker-test.log`、`workspace-preferences-smoke.log`、`update-smoke.log`、`release-desktop-smoke.log`；截图与结构化结果分别在对应测试目录。自动安装限 Windows x64，验证基于固定 HTTPS API 与 SHA-256 摘要，不等同于独立发布者签名。

## 文件任务、权限和显示列（2026-11-13）

完整 Rust 测试（含回环 SSH/SFTP）129 项、严格 Clippy 及桌面构建通过。新增覆盖扫描中取消保留全部文件、删除中取消保留剩余文件并报告精确部分数量、无连接／过期连接请求返回最终事件，以及 POSIX 权限解析、特殊位、多选目标预检、目录权限不影响子项、拒绝路径穿越、取消及权限拒绝。列偏好按服务器及本地／远程隔离，配置保存重载与旧配置兼容通过。

`python tools/file_options_smoke.py` 的 6 组真实 WebView 检查通过：慢速嵌套目录扫描及执行阶段的即时关闭／取消、实际 SFTP 权限读取写入与内容保留、无效八进制禁用保存、Sapid 读写执行复选框保留特殊位、多选混合权限明确输入、目录子项权限不变、本地 ACL 功能边界、两栏显示列同步、切换服务器与全新进程恢复、重置隔离及窄窗口。截图与结果位于 `artifacts/file-options-ui`。原有 8 组文件右键和 6 组导航回归通过，测试全部使用独立 fixture、配置和测试文件。

## 远程删除目录项兼容（2026-11-13）

修正删除扫描在跳过 `.`／`..` 前调用 `Path::file_name` 的问题。回环 SFTP 服务现在与真实服务器一样返回这两项；新增回归在修复前复现 `Invalid remote child`，修复后验证空目录、嵌套目录及 Unicode 文件递归删除，且相邻目录内容不变。完整回环测试 121 项通过，桌面构建通过；记录在 `artifacts/delete-dots-before.log` 和 `artifacts/delete-dots-tests.log`。此错误发生在全部树预扫描期间、实际删除开始之前。

## 文件删除（2026-11-13）

120 项完整 Rust 测试（含回环 SSH/SFTP）通过，严格 Clippy 通过。新增覆盖递归与父子去重、选择变化时整批预检拒绝、根目录／相对路径／路径穿越拒绝、新到达子项保留、部分失败计数，以及 Windows 目录联接删除后链接目标文件保留。真实 SFTP 覆盖递归、去重、过期选择、权限拒绝及连接变更检查。

`python tools/file_context_smoke.py` 扩展至 8 组真实 WebView 检查，验证本地与远程删除确认中的完整路径、取消保持内容、Delete 快捷键、多选与递归删除、展开子项删除，以及权限失败提示和目录刷新。文件仅位于独立测试沙箱。确认弹窗截图为 `artifacts/file-context-ui/delete-confirm.png`。

## 文件右键菜单（2026-11-13）

112 项完整 Rust 测试（含回环 SSH/SFTP）通过。`python tools/file_context_smoke.py` 的 6 组真实 WebView 检查覆盖多选保持／替换／空白清除、Unicode 路径复制、目录打开与新建、本地及展开远程子项重命名、真实上传下载和文件内容核对、属性与权限显示、键盘菜单／快捷键／焦点恢复、图标模式与工具栏菜单一致性、外部点击关闭和窄窗口定位，以及 SFTP 不可用时的禁用状态和实时中英文切换。截图与结果在 `artifacts/file-context-ui`，全部文件仅位于隔离测试目录，未对真实服务器执行文件操作。

## 服务管理与常用命令（2026-11-13）

工作区完整测试（包含回环 SSH/SFTP）112 项通过。新增配置兼容／保存重载、服务名参数保护、列表解析、root／sudo 模式、无 SFTP 查询与服务操作、无 systemd 的命令执行、审批单次使用与连接隔离，以及凭据跨输出分块过滤和 UTF-8 输出上限检查。

`python tools/operations_smoke.py` 的 7 组实际 WebView 检查覆盖服务列表与筛选、状态和日志跳转、确认前取消、sudo 输入过滤及操作后刷新、命令新增校验／重复执行／编辑／删除、全新进程恢复、非零退出码、窄窗口和实时语言切换，以及换服务器后的数据清理和不支持 systemd 时的提示。另有 6 组原有导航／主题／终端／SFTP 回归通过，主题测试显式设置初始深色，避免跟随系统时依赖宿主外观。截图与结果位于 `artifacts/operations-ui`、`artifacts/navigation-redesign`。测试仅使用本机隔离 SSH fixture 的固定响应，不执行本机 shell，也不控制真实服务器服务。

## Arcnix 更名验证（2026-11-13）

实际工作区为 `D:\Works\arcnix`，`cargo metadata --no-deps --format-version 1` 返回的产物目录为 `D:\Works\arcnix\target`。两个包为 `arcnix-core`、`arcnix`，桌面程序为 `arcnix.exe`，Windows 产品名称和命令行帮助均为 Arcnix。

工作区 103 项 Rust 测试全部通过，包含显式启用的回环 SSH/SFTP、主机指纹、密钥认证、终端、资源、日志及包管理测试；格式检查、构建和严格 Clippy 通过。导航脚本的 6 组实际 WebView 验证及服务器树回归通过，覆盖改名后的资源加载、SSH 连接、终端、SFTP、双主题及拖动排序。图标透明度检查改为检查四角透明及图案非空，避免把图案内部的固定像素误认为背景。

源码、资源和文档已检查旧名称残留；日志和汇总位于 `artifacts/rename-validation`，改名前的文件备份与历史验证产物保留在忽略的 `artifacts` 目录。

## 上传下载进度与速度（2026-11-13）

完整工作区测试（含显式启用的真实回环 SSH/SFTP 测试）103 项通过，格式、桌面构建及严格 Clippy 通过。新增覆盖混合目录上传／下载中跳过项推进逻辑进度但不增加实际字节、零字节文件、冲突等待不参与计速、停滞速度归零、超大计数、完成／取消边界。

`python tools/transfer_progress_smoke.py` 的 5 组真实 WebView 验证通过。延迟固定的回环 SFTP 服务完成 6 MiB 上传和下载，观察中间进度、可访问进度值、速度及有限 ETA，结束后检查 100%／平均速度与实际字节；下载 SHA256 一致。冲突等待时隐藏速度／ETA，跳过后为零字节和零平均速度。取消后统计冻结，重试完整重新传输；1000×700 窗口无横向溢出。截图和结果位于 `artifacts/transfer-progress-ui`。

全部文件及认证材料均为隔离 fixture 数据，未连接真实服务器。速度表示 SSH/SFTP 传输的文件内容速率，不是网卡线速；现有协议、落盘和网络条件会影响真实服务器表现。

## 文件名焦点框修正

移除文件名与目录展开按钮的输入框式焦点轮廓；鼠标选择保留整行高亮，键盘导航将细焦点线放在整行。构建、格式检查与 18 组无需 SSH 的实际桌面回归通过，检查 Ctrl 多选后文件名无 outline／shadow、鼠标行无 outline，以及 Tab 后整行有约 1 CSS px 焦点线。WebView 的 DPI 缩放使计算值约为 0.98px，断言使用合理容差。截图为 `artifacts/ui-redesign/file-focus-selection.png`，记录为 `artifacts/file-focus-ui.log`。

## 分段路径、资源管理器选择、目录树与文件信息

2026-11-13：完整 93 项 Rust 测试通过（核心 34、密钥 7、日志 1、资源 6、包管理 2、SFTP 12、命令 3、桌面 28），构建、格式及严格 Clippy 通过。新增驱动器／UNC 根目录与 POSIX 分段路径测试，以及嵌套选择去重、同名前缀目录不误判的传输测试。

`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 27 组实际 WebView 检查通过，新增验证：普通单击替换选择，Ctrl 增减、Shift 范围、Ctrl+A／Ctrl+空格／Shift+方向键；异步展开两级目录与收起后的不可见选择清理；Ctrl+L 聚焦、Escape 不提交、Enter 跳转；点击上级路径段跳转；Windows 剪贴板实测中文完整路径与远程 POSIX 路径。

两栏详细列表／平铺图标独立切换，模式跨当前会话内目录跳转保留。检查选中项类型、精确字节数、完整时间、完整路径和远程权限；展开的 SFTP 子项重命名仍位于原父目录，并能实际下载到本地。原有传输冲突、软件、日志、资源与设置回归通过。系统命令仍为隔离 fixture 响应，未操作真实服务器。

截图位于 `artifacts/ui-redesign/files-icon-grid.png`、`file-details-info.png`、`file-tree-expanded.png`、`explorer-files.png`；日志为 `artifacts/browser-*.log`。并发 Rust 集成测试曾暴露 fixture 枚举期间临时文件消失的竞态，修复为忽略该条已消失项目后全套通过；不是忽略真实目录或权限错误。

## Files 双击进入文件夹

桌面构建、格式检查和严格 Clippy 通过。`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 21 组实际 WebView 回归通过，覆盖本地／远程文件夹前的 `+`、单击只选中、双击进入、Enter 进入、本地行空白处双击进入、多选控件与双击文件保留选中。原有上传下载、冲突处理、应用设置及系统模块回归通过。

Glory 0.7.0 的 Wry Activate 不包含点击次数，因此在应用 bootstrap 中将浏览器原生单击／双击转发至现有 Rust 选择／激活动作，使用系统双击时序；键盘激活保持原生。路径和文件业务状态由 Rust 管理。截图为 `artifacts/ui-redesign/folder-double-click.png`，记录为 `artifacts/file-navigation-ui.log`。没有新增远程命令，测试使用隔离回环服务器。

## 主机密钥算法切换与已有信任记录

通过对配置中主机进行不带认证的握手核对，确认新默认 Ed25519 指纹与报警一致，而 RSA SHA-2 协商得到的指纹仍与已有信任记录完全相同。没有读取用户私钥、发送密码或改写其主机信任。

`PendingConnection::open_for_known_host` 在共享 15 秒期限内尝试受支持的现代主机密钥类型，只返回与原 SHA256 指纹匹配的连接，再进入认证。RSA 使用 SHA-2 签名；不为匹配旧记录额外启用 SHA-1／DSA。所有密钥都不匹配时，拒绝连接且保留原记录。

90 项完整 Rust 测试、格式、桌面构建和严格 Clippy 通过。新增测试覆盖默认算法变化后保留 RSA 指纹并用 Ed25519 客户端密钥认证、默认指纹本就匹配的连接，以及服务器密钥不匹配时服务端认证审计没有收到任何凭据。

`python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 的 19 组实际桌面回归通过，复现“保存 RSA、默认协商 Ed25519”的升级场景，连接、文件传输和系统模块均可使用。最终独立配置保留原 RSA 信任记录。记录为 `artifacts/host-key-*.log`。同时修复操作日志初始消息与下一条日志缺少换行。

## Windows Ed25519 与内置 OpenSSL

旧 WinCNG 测试程序使用临时生成的 OpenSSH Ed25519 密钥连接本机 fixture，复现 `[Session(-1)] unknown error`。启用 `ssh2` 的 `openssl-on-win32` 与 `vendored-openssl` 后，同类认证与文件往返通过。OpenSSL 源码版本由 Cargo.lock 固定为 3.6.3，静态库在实际 workspace 的 target 下构建。

全部 87 项 Rust 测试通过，包含新增的 4 项密钥专项：普通 Ed25519 的命令与 SFTP 文件往返、口令加密 Ed25519、空／错误口令拒绝且不回显秘密、未授权密钥的服务器拒绝提示。原有密码与 RSA PEM 认证、SSH 主机指纹、SFTP、系统操作与后台任务回归通过。

专项测试程序在 PATH 仅含 Windows System32 的环境下通过，导入表没有 libssl／libcrypto DLL。所有认证使用 fixture 临时生成的密钥，没有读取真实私钥或连接真实服务器。

复现命令（先启动 `python tools/sftp_fixture.py --ready artifacts/fixture.json`）：

```powershell
$env:ARCNIX_TEST_FIXTURE = (Resolve-Path artifacts/fixture.json).Path
cargo test -p arcnix-core --test key_auth_integration --locked -- --ignored
```

MSVC 源码构建需要原生 Windows Perl；本轮将经官方 SHA256 校验的便携 Strawberry Perl 放在忽略的 `artifacts/tooling` 中，通过进程级 `OPENSSL_SRC_PERL` 使用，没有安装到系统或修改全局环境。运行程序不需要这些构建工具。上游 OpenSSL 静态库未携带其 C 调试 PDB，MSVC 链接产生 LNK4099 提示，不影响认证功能或静态链接结果。

桌面构建、格式检查和严格 Clippy 通过。`python tools/ui_smoke.py --packages --isolated-crypto-runtime` 的 19 组实际 WebView 检查通过：桌面子进程 PATH 仅含 Windows System32／Windows，使用生成的 OpenSSH Ed25519 私钥完成连接、SFTP 双向冲突处理、资源、日志和软件模块回归。系统操作仍为 fixture 固定响应，没有运行实际 APT。最终 `arcnix.exe` 导入表没有 OpenSSL／libssh2 DLL；记录位于 `artifacts/ed25519-runtime-dependencies.txt`，测试与构建日志为 `artifacts/ed25519-*.log`。

2026-11-12，Windows x64，Rust 1.99.1。实际工作区产物目录：`D:\Works\arcnix\target`。

## 自动化验证

- 工作区构建通过。
- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- 15 个常规测试通过：配置持久化、错误配置保护、分组、主机信任、本地文件、传输取消、路径约束、旧会话响应和取消状态。
- 6 个真实回环集成测试通过：5 个 SSH/SFTP 后端测试及 1 个桌面后台任务通道测试。验证密码和未加密 RSA PEM 私钥认证、指纹不符拒绝、中文路径、空文件、SHA256 往返一致、拒绝覆盖、取消与临时文件清理、命令输出与退出码、跨服务器传输拒绝、旧 epoch 拒绝和断开。

集成测试服务仅监听本机，使用临时目录与独立生成的密钥。远程 shell 返回固定测试结果，未据此宣称验证了真实 Linux 主机的信息采集输出。

## 实际桌面验证

使用独立配置 `artifacts/ui-smoke/config.json` 启动编译后的 `arcnix.exe`，不使用用户服务器或凭据：

- 系统 WebView2 窗口启动、Glory/Sapid 组件渲染与可访问性树正常。
- 双栏文件列表、分组/站点侧栏、传输队列、日志和状态栏显示正常。
- 通过实际控件填写名称、主机和用户名，保存服务器后确认 JSON 文件落盘。
- 通过“打开”进入本地子目录，实际目录列表和配置中的上次本地目录同步更新。
- 验证后修正搜索框下的多余留白，并将工作区和侧栏滚动限制在各自区域，状态栏保持可见。

SSH 认证和传输通过真实后端与任务通道集成测试验证；未在桌面 UI 中输入认证秘密。非 Windows 平台、其他私钥格式、真实远程主机兼容性和网络中断恢复仍需后续环境验收。功能边界详见 README。

## 同日后续改进验收

自动回归共 42 项通过（含显式启用的回环集成测试）：核心 21 项、真实 SSH/SFTP 7 项、桌面 14 项。其中桌面包括 1 个真实后台任务通道测试，另有 2 个不依赖 SSH 服务的并发／结束事件测试。

新增验证覆盖：

- 中文嵌套目录与空目录递归上传下载，逐文件 SHA256 一致、累计进度单调、已有目录不合并、取消后保留已完成文件并清理文件级临时文件。
- OpenSSH 配置顺序与模式匹配、代理拒绝、编码与大小限制、不同密钥身份保留、导入事务性更新。
- 筛选排序、目录置前、重试标识隔离、目录请求序列拒绝过期结果。
- 阻塞 SSH 握手期间本地浏览仍可完成；失效会话传输必定结束；通过测试服务主动断开连接，验证 ConnectionLost 事件和后续传输失败。

最终工作区构建、格式检查和严格 Clippy 通过。已重新启动新版默认配置窗口，进程响应正常；通过实际窗口可访问性树确认 SSH 导入、双栏筛选排序／全选、队列暂停和批量重试控件存在。

本轮桌面自动化点击遇到系统错误 `DwmFlush ... desktop composition is disabled`，恢复后重试返回 `coordinate input geometry is unavailable`。因此未完成新版控件的点击／截图验收；交互行为目前以 Rust 测试和真实任务通道集成为证据，不将可访问性树检查等同于完整视觉验收。旧窗口确认未连接且队列为空后停止并替换程序；新版已保持运行。

## 界面原语重构验收

系统窗口自动化仍报告坐标不可用，因此改用独立测试进程的本地 WebView2 CDP 接口进行真实渲染和交互验收。`tools/ui_smoke.py` 为此创建了独立配置、本地测试目录与虚构服务器，并在结束后关闭测试进程；未连接真实服务器或输入凭据。

8 组界面回归全部通过：常驻表单／全局编辑删除移除；悬停菜单与具体站点绑定；模态初始焦点、背景 inert 和 Escape；服务器认证选项、内联验证与保存；文件操作失败保留对话框、成功后关闭；底部单一标签面板；分组模态；SSH 导入错误处理、长列表菜单无裁剪、删除确认的目标名称；1000×700 下对话框提交区保持可见。

截图和详细检查记录在 `artifacts/ui-redesign`。已检查工作台、站点菜单、新建服务器、校验错误与窄窗口截图。本轮常规 Rust 测试 34 项通过，另 8 个需 SSH fixture 的测试按默认设置忽略；后端未在本轮修改。严格 Clippy、格式检查和工作区构建通过。

## 图标按钮统一验收

界面回归扩展为 10 组，全部通过。新增检查：导航/刷新/筛选/队列/折叠按钮文本为空，实际 SVG mask 存在；活动面板和队列状态切换时图标及可访问名称同步切换；关闭使用 Sapid CloseButton，无可见文字，悬停显示“关闭对话框”，点击正常关闭。已目视检查 `artifacts/ui-redesign/close-icon-tooltip.png`。常规 Rust 测试 34 项、严格 Clippy、格式检查及工作区构建通过；后端需独立 SSH fixture 的 8 项测试未在本轮重跑。

## 系统软件模块验收

完整启用 fixture 的工作区测试共 58 项通过：核心单元测试 28，包管理 SSH 集成 2，SFTP 集成 7，流式命令集成 3，桌面测试 18。覆盖精确包名与注入拒绝、模拟计划、root/sudo 命令生成、无 SFTP 连接、单次会话绑定计划、实时双流与退出码、大量输出尾部限制、私密 stdin、超时后会话恢复及跨字节分块凭据隐藏。

`python tools/ui_smoke.py --packages` 的 11 组界面回归通过。新增通过生成的隔离私钥连接本机 SSH fixture，验证软件页面识别和已安装列表、搜索 nginx、确认前显示模拟结果与命令、确认后流式结果及退出码、刷新为已安装、执行时断开/其他服务器选择禁用、更新索引预览及取消。配置与本地目录均为独立测试数据，测试服务不运行 shell/apt。

目视检查 `software.png`、`package-confirm.png`、`package-completed.png` 的布局。CPU/内存使用率监控未实现；软件包管理本轮未在真实 Debian/Ubuntu 系统执行 apt。构建、格式和严格 Clippy 通过。

## 资源数值与系统日志验收

完整启用回环 SSH fixture 的工作区测试共 71 项通过：核心单元测试 32，日志集成 1，资源解析／采样 6，包管理集成 2，SFTP 集成 7，流式命令集成 3，桌面测试 20。新增覆盖 CPU guest 去重、内存单位和回退估算、远程时间间隔、重启／计数器重置、缺失与截断数据、journal JSON／二进制消息、UTC 时间、命令参数拒绝及无 SFTP 的资源与日志后台通道。

14 组实际 WebView 检查通过；新增资源首个／第二个快照、CPU 30% 与网速 1 KiB/s 的差值计算、磁盘挂载路径、窄窗口两列卡片，以及 journal 服务／时间／级别／文本筛选、多行消息、权限错误、传统文件日志、自动刷新和断开后旧数据清空。界面统一检查输入框焦点无外层粗圈，仅保留 1px 边框变色。

截图在 `artifacts/ui-redesign/resources.png`、`resources-narrow.png`、`system-logs.png`、`system-logs-narrow.png` 和 `input-focus.png`。测试为真实 SSH 传输上的固定 Linux 响应，未读取真实服务器日志或运行真实 apt；不同 Linux 发行版和权限策略仍需实际环境验收。资源曲线尚未实现。

## 紧凑顶栏、分栏与文件冲突验收

工作区测试共 79 项通过：核心单元测试 33，日志 1，资源 6，包管理 2，SFTP 12，流式命令 3，桌面 22。SFTP 新增真实回环传输验证双向覆盖／跳过／取消、递归目录合并、目标类型拒绝、复制期间取消、晚出现的目标重新询问、特殊路径引用、发布失败保留原文件，以及保留远程 0644／0755 权限和设置权限失败保护。后台桥接测试验证等待用户选择时可以取消或切换会话，任务能收到结束事件，过期选择不会继续执行。

17 组实际 WebView 界面回归通过，验证顶栏标签与品牌同排、无常驻连接按钮；拖动分隔条实时改变双栏宽度，Home 恢复均分，两个服务器各自保存不同的比例，切换恢复并启动全新进程再核验本地配置。文件冲突模态实际完成跳过、覆盖、保存上传规则、后续连接沿用、下载方向仍询问、重置规则、关闭后取消；通过测试目录逐字节检查最终内容。

滚动条检查其真实计算样式为 6px 且没有上下箭头。目视检查 `split-panes.png` 和 `file-conflict.png`，本地路径不展示 Windows 内部前缀。所有远程文件操作仅作用于隔离 fixture 沙箱；APT 与日志仍为固定测试响应。

## 统一 SSH 连接状态验收

右上角显示当前服务器、SSH 状态和连接／取消／断开按钮；无站点时提供添加入口，已有站点但未选择时明确提示选择左侧服务器。文件区仅显示 SFTP 能力，不引入第二次连接操作。

79 项 Rust 测试与 18 组真实 WebView 检查通过，格式、构建、严格 Clippy 通过。新增通过只开放 SSH、拒绝 SFTP 子系统的隔离账号核验：全局显示 SSH 已连接，文件区显示 SFTP 不可用且下载禁用，资源数值、软件查询和系统日志仍能读取。截图为 `artifacts/ui-redesign/ssh-without-sftp.png`。

## 默认英语、应用设置、站点分组与原生路径选择验收

完整工作区 83 项 Rust 测试通过：核心 34，日志 1，资源 6，包管理 2，SFTP 12，流式命令 3，桌面 25。新增本机设置的默认英语／保存重载／非法间隔保护，以及词条模板保留用户数据、语言和源绑定同时更新的测试。格式、构建及严格 Clippy 通过。

19 组主界面回归通过，覆盖英语菜单和提示、真实回环 SSH 上切换中英文后连接与软件状态保留、资源／软件／日志及文件冲突全流程。默认语言使用英语；用户定义的中文站点名、路径和远程消息保持原文。

分组专项实际复现旧问题：JSON 已保存 Group B，但侧栏仍筛选 Group A。新版检查指定分组、继承分组、未分组、编辑移动和取消，JSON 与侧栏显示一致。Settings 专项验证深浅主题、语言、点号文件显示、采样间隔的即时生效和全新进程重载。

Windows 原生文件窗口完成真实选择和取消：仅使用项目隔离的假文件，文件名包含中文、空格且没有扩展名；选中后回填路径，取消保留原值。界面自动化坐标点击报告 geometry unavailable，采用 UI Automation 设置系统窗口的 File name 后以 Enter 确认，再以 Escape 取消；已通过界面回填结果验证，未读取真实私钥。

截图位于 `artifacts/settings-ui/settings-dark.png`、`settings-light.png`、`settings-chinese.png`；分组截图与独立脚本位于 `artifacts/group-assignment`、`tools/group_assignment_smoke.py`。系统日志和 APT 仍使用固定响应，SFTP 使用真实隔离文件传输。

## 交互式 SSH Terminal 验收（2026-11-13）

完整启用本机 SSH fixture 的工作区测试共 97 项通过：核心单元 34、认证 7、日志 1、资源 6、包管理 2、SFTP 12、流式命令 3、PTY 3、桌面 29。新增真实 PTY 验证输入、UTF-8 分块、ANSI、窗口尺寸、Ctrl+C、退出码、大量输出以及服务器拒绝 PTY；后台通道验证 SFTP 阻塞时终端仍响应、断开后旧会话输入被拒绝。格式、严格 Clippy 和桌面构建通过。

`python tools/terminal_smoke.py` 的 9 组实际 WebView 检查通过：使用隔离 Ed25519 私钥及原 RSA 主机指纹打开终端；中文复制粘贴及阻止远程 OSC 52 修改剪贴板；快速输入校验 SHA256；实际 SFTP 上传并保留终端；切换中英文后保留屏幕并继续输入；缩小窗口；超过 256 KiB 输出完整到达；shell 退出／重启保留全局 SSH；断开后销毁终端显示。图标工具栏、窄窗口布局和输出已检查截图 `artifacts/terminal-ui/terminal.png` 与 `terminal-narrow.png`。

测试仅连接本机回环服务，服务使用固定交互协议，不执行本机系统 shell；未读取真实服务器凭据、访问真实服务器或运行 apt。真实 Linux 发行版、全屏终端程序及网络断线恢复仍需实际环境验证。终端渲染使用程序内打包的 xterm.js，Rust 负责 SSH、PTY、状态及输入输出桥接；不依赖系统 SSH、Node 或 CDN。


## 全局脚本与服务器任务（2026-11-14）

- `cargo check --workspace --locked`、严格 all-targets Clippy、Windows debug 构建通过。
- 核心与桌面单元测试 91 项通过，7 项现有回环测试未在此命令启用。新增参数单次字面量展开、引用／拼接拒绝、官方模板校验、历史恢复状态、损坏历史保留及 UTF-8 日志边界测试。
- `python tools/scripts_smoke.py` 的 12 组实际 WebView 检查通过：Servers 为首入口，Scripts／Tasks 无需单机选择；官方和自定义脚本；两台独立 SSH peer 的 APT 预览；服务器任务提交；关闭客户端后重启查询；不支持 systemd 的预检查失败；参数字面量；取消待执行目标；全局脚本创建；窄窗口；中文切换。SSH peer 仅模拟固定命令，不执行 apt 或任意本机 shell。
- `python tools/navigation_smoke.py` 的 6 组原有导航、SSH／PTY、SFTP、主题及窄窗口检查通过。
- 显式设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu` 后，`cargo test -p arcnix-core --test hosted_tasks_integration -- --ignored` 通过：在本机 WSL Ubuntu 的真实 systemd 上运行独立临时 service，提交进程退出后任务继续完成，保留非零退出码和日志；另验证以原 SSH 用户 UID 执行时没有提升工作负载权限。测试只运行 sleep、printf、id，不安装软件、不访问外部服务器。

完整安装流程仍需在实际目标发行版验证包安装行为；本次未运行真实 apt 安装。持续调度及依赖客户端常驻的功能不在本次范围内。


## 备份与恢复验收（2026-11-14）

- Rust 核心与桌面单元测试 94 项通过；7 项现有回环测试未在该单元测试命令启用。新增策略校验、凭据地址拒绝、仓库重叠、快照来源绑定、保留范围和 JSON 进度解析测试。构建、格式检查、严格 all-targets Clippy 通过。
- `python tools/backups_smoke.py` 的 12 组实际 WebView 检查通过：全局入口、策略校验和保存、两台独立服务器备份、进度条、带标签的快照列表及 UTF-8 分块、恢复目录与完整快照计划、保留 dry-run、取消未提交目标、编辑重启、中文窄窗口、仅删除本地策略。SSH fixture 不执行真实备份工具。
- `python tools/scripts_smoke.py` 的 12 组脚本检查和 `python tools/navigation_smoke.py` 的 6 组原导航检查再次通过，覆盖 Servers 首入口、全局页面、独立 SSH 任务、重启查询、终端及 SFTP。
- 从 restic 官方 GitHub Release 下载测试用 Linux restic 0.19.1 到项目 artifacts，核对发布 SHA-256；未安装到服务器系统目录。显式设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu` 和 `ARCNIX_TEST_RESTIC_BIN_DIRECTORY=/mnt/d/Works/arcnix/artifacts/backup-tools`，运行 `cargo test -p arcnix-core --test backups_integration --test hosted_tasks_integration -- --ignored`，2 项真实集成检查通过。
- 真实 restic 检查使用 WSL 内独立测试目录：初始化加密仓库、systemd 提交后客户端进程退出、备份中文内容、修改源文件、不同来源服务器标签、全仓库数据检查、原快照恢复并验证字节、拒绝已有目标、拒绝符号链接绕过、拒绝 644 密码文件、保留预览不删除数据、正式保留只影响选定来源。测试结束后验证路径并清理该测试目录。
- 真实测试发现并修复两处执行问题：systemd root 服务显式设置 UID 以获得 HOME；查询服务状态时再次读取已落盘结果，避免任务完成瞬间被误报为未知。

本次真实数据测试使用本地 restic 仓库，未验收 SFTP／S3 后端连接与凭据链，也未运行数据库导出或恢复。普通目录备份不能替代数据库一致性备份。本次不提供依赖客户端常驻的定时备份、持续监控或自动维护。

参考：[restic backup](https://restic.readthedocs.io/en/stable/040_backup.html)、[restore](https://restic.readthedocs.io/en/stable/050_restore.html)、[scripting and JSON](https://restic.readthedocs.io/en/stable/075_scripting.html)。

## 数据库备份、恢复与独立确认记录（2026-11-14）

- 最终格式检查、严格 all-targets Clippy 和 Windows debug 构建通过。核心 65 项、桌面 31 项单元测试通过，共 96 项；7 项需回环 fixture 的桌面测试在该命令未启用。
- 实际 WebView 回归共 48 组通过：`database_smoke.py` 17 组、`scripts_smoke.py` 13 组、`backups_smoke.py` 12 组、`navigation_smoke.py` 6 组。新增检查覆盖 PostgreSQL／MySQL 类型化表单、归档计划、独立的备份／检查／数据库恢复确认、拒绝源库名称、新库恢复角色与认证文件、失败保留旧成功、旧配置标识、重启持久化、中文窄窗口、数据库客户端安装入口、中文官方模板搜索，以及单机命令中的参数脚本转入全局参数表单。固定响应 SSH fixture 不运行真实导出或安装。
- `python tools/database_fixture.py` 的 1 项真实集成测试通过：使用项目 artifacts 中解包的 PostgreSQL 16.2、MariaDB 10.11.7 及已核对发布摘要的 restic 0.19.1，未安装到系统目录。测试服务器只监听回环地址，使用专用合成账户、凭据及独立数据目录。
- 真实 PostgreSQL 检查包括 custom 导出、归档与 SHA-256 校验、systemd 托管、新库恢复原始中文数据、保持已修改的源库不变、拒绝已有目标；另强制 pg_dump 失败并核对没有新增快照、导出工作目录已清理。
- 真实 MariaDB 检查包括 InnoDB 事务导出、restic 恢复 SQL 文件、通过原生客户端受控导入独立测试库并核对中文数据；拒绝仅有部分表 SELECT 权限的账户及包含 MyISAM 表的数据库，拒绝后没有新增快照。界面仍只提供 MySQL SQL 文件恢复，不提供自动 SQL 导入。最后执行完整仓库数据检查。
- 临时数据库数据位于隔离的 `/dev/shm/arcnix-databases.*`，测试关闭同步落盘以避免共享宿主 I/O 干扰；不据此宣称通过断电、崩溃持久性测试。结束后停止测试数据库，核验精确路径并清理其数据。Cargo 使用 workspace 返回的正常 `target_directory`，未重定向缓存或 TEMP/TMP。
- 检查了最终中文窄窗口截图 `artifacts/database-ui/desktop-1794648592215642900/database-chinese.png`。测试记录另见 `artifacts/database-final-*.log`、`artifacts/database-integration.log` 和各 UI 目录的 `results.json`。

本次验证使用本地 restic 仓库和 MariaDB 客户端；Oracle MySQL、角色／通配授权、SFTP／S3 后端、集群恢复及 PITR 尚未验收。导出、上传和恢复继续由服务器 systemd 服务持有，关闭或升级客户端不依赖本地后台进程；定时调度、持续监控和自动修复不在本次范围内。

## 跨服务器按需诊断（2026-11-14）

- 格式、严格 all-targets Clippy 和 Windows debug 构建通过；核心 68 项、桌面 31 项单元测试通过，共 99 项，7 项需额外回环 fixture 的桌面测试未在该命令启用。新增检查覆盖参数边界、字面量路径、完整／重复／无效协议、检查范围绑定，以及短数字／状态词密码的结构化脱敏。
- 实际 WebView 回归共 65 组通过：`diagnostics_smoke.py` 17、`scripts_smoke.py` 13、`backups_smoke.py` 12、`database_smoke.py` 17、`navigation_smoke.py` 6。诊断专项使用两台隔离 SSH peer，覆盖无需单机选择的全局入口、安装入口、阈值和端口输入拒绝、六项检查计划、多服务器、执行期间关闭应用、重启后仅查询原结果、按服务器汇总严重／警告／无法判断、中文明细、目录统计与工作区入口、不完整结果拒绝、检查范围持久化和中文窄窗口。SSH peer 返回固定响应，不执行真实系统检查或 apt。
- 设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu`、`ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY=/mnt/d/Works/arcnix/artifacts/diagnostic-tools/extracted/usr/sbin` 后，`cargo test --locked -p arcnix-core --test diagnostics_integration -- --ignored` 的 1 项真实集成测试通过。该测试提交 5 个独立 systemd 任务，提交进程退出后再读取结果；实际执行 GNU df、systemctl、ss、OpenSSL 和 Nginx 1.24.0。
- 真实数据检查包括隔离回环 TCP 监听的存在及停止后缺失、临时一日证书到期提醒、包含中文／引号／双花括号的文件路径、缺失证书标记无法判断，以及隔离 Nginx 配置成功和无效指令失败。核对配置内容未变、测试模式没有写入运行中进程 PID，并在原始结果上应用短密码脱敏后解析、核对检查范围。原生校验发现 Nginx `-t` 会创建空 PID 文件；这属于工具行为，已在开发文档说明并清理该测试文件。
- Nginx 仅通过 APT 下载包并把测试二进制解包至项目 artifacts，未安装、启动或重载系统 Nginx；临时监听、证书和任务使用独立 UUID 目录／unit，结束后只停止自己的 unit 并清理已知文件。未读取真实站点配置、密钥或服务器凭据。
- 日志查询改为更新服务器返回的有界快照，避免分段变化或日志截断时追加出重复结构记录；诊断协议保留结构字段，对解码后的明细脱敏，再编码保存。该变更后的原有脚本、备份、数据库和导航界面回归全部通过。
- 最终截图已检查：`artifacts/diagnostics-ui/desktop-1794651682243282200/comparison.png`、`findings.png`、`diagnostics-chinese.png`。日志位于 `artifacts/diagnostics-tests.log`、`diagnostics-clippy.log`、`diagnostics-build.log` 及 `diagnostics-final-*.log`。

本次真实环境为 WSL Ubuntu／GNU 工具和 Nginx 1.24.0；OpenSSH 配置校验、其他发行版／工具版本、证书链与外部网络可达性未据此宣称验收。诊断仅为按需采样，不提供持续监控、自动处置或依赖桌面常驻的调度。

## 跨服务器配置发布与恢复（2026-11-14）

- 核心 69 项、桌面 31 项单元测试通过，共 100 项；7 项需要额外回环 fixture 的桌面测试未在该单元测试命令启用。严格 all-targets Clippy 和 Windows debug 构建通过。MSVC 仍有上游 OpenSSL 静态库缺少调试 PDB 的 LNK4099 提示，未影响链接结果。
- 实际 WebView 回归共 86 组通过：`configurations_smoke.py` 21、`scripts_smoke.py` 13、`backups_smoke.py` 12、`database_smoke.py` 17、`diagnostics_smoke.py` 17、`navigation_smoke.py` 6。全局功能无需选择单台服务器；服务器工作区仍只有六个原单机模块。
- 配置界面专项通过真实隔离 SFTP 上传超过旧 stdin 限制的候选；检查逐服务器差异、本地配置和历史不含正文／差异、缺少 SFTP 时拒绝、恢复时预选原目标、自定义校验必填、提交失败停止待提交目标，以及中文 900×700 布局。SSH 执行响应由 fixture 模拟，不执行真实发布命令或 apt。
- 默认只有一个提交名额时，两台服务器在第一台完成前均进入运行状态；关闭客户端，再启动并查询原任务，两台任务均完成，未重新提交。会话内差异在重启后消失，持久元数据及原服务器身份保留；配置页面优先显示重启后的未知结果，避免旧阶段日志被误当作当前执行状态。
- 设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu`、`ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY=/mnt/d/Works/arcnix/artifacts/diagnostic-tools/extracted/usr/sbin` 后，`cargo test --locked -p arcnix-core --test configurations_integration -- --ignored` 的 1 项真实集成测试通过。测试使用 WSL root、实际 Python、Nginx 1.24.0 和私有 systemd unit；所有配置、PID、日志、监听端口和恢复记录属于本次 UUID fixture。
- 真实发布覆盖中文／空格／引号路径、预检查不替换生效文件、差异、校验、原子替换、实际 Nginx reload 和 HTTP 响应核验。检查 0640 模式及用户扩展属性保留，并设置父目录默认 POSIX ACL，确认不会给候选增加原文件没有的继承访问权限。旧版本恢复还覆盖恢复前模式已改变、目标后来被手工修改及目标已删除。
- 故障覆盖无效配置、预览后文件修改、健康检查期间文件被修改、重载失败、预期 HTTP 文本不符，以及持续慢速 HTTP 响应。分别检查发布拒绝、保留并发编辑、文件已恢复但服务未确认、服务恢复已确认；慢速响应不会无限延长检查。另在目标缺失的原子创建处注入 EEXIST，核对操作没有删除或改写此时到达的外部文件及其模式／扩展属性。
- 服务器保存私有原始版本、候选、执行器、manifest 和有界工具日志；真实测试核对失败时仍保有恢复依据。测试结束只停止自己的 unit，并在校验精确目录后清理本次临时数据和恢复记录，未读取真实配置、凭据或修改系统 Nginx 服务。
- 已目视检查 `artifacts/configurations-ui/desktop-1794657349955258800/` 中的 `editor.png`、`private-diffs.png`、`published.png`、`configurations-chinese.png`。检查结果见 `artifacts/configurations-ui/results.json`；测试与构建日志见 `artifacts/configurations-tests.log`、`configurations-clippy.log`、`configurations-build.log`、`configurations-integration.log` 及各 `configurations-*-ui.log`。

一次较早的配置 UI 运行在点击执行后意外显示 Diagnostics 页面；原配置任务仍按预期失败并报告恢复，没有观察到错误操作被提交。原因尚未定位，证据保存在 `artifacts/configurations-ui/desktop-1794655394469270800/failure.txt` 与同目录截图。此后为点击增加目标审计，并在最新专项中重复三轮执行路由，均通过；这只能说明未再次复现，不能视为已证明修复，继续跟踪。

真实后端测试直接使用 WSL root；实际远程 SSH＋SFTP＋sudo 的完整发布链路、非 root 属主与安全标签矩阵、其他发行版和工具版本尚待环境验收。当前发布范围为一个文件；任意自定义命令副作用、引用配置树、SIGKILL、服务器重启／断电及多服务器事务不在该测试保证之内。客户端独立性验证不能替代服务器崩溃恢复验证。

## 全局任务的服务器冲突保护（2026-11-14）

- `cargo test --workspace --lib --bins --locked` 的 100 项单元测试通过：核心 69、桌面 31，7 项需额外回环 fixture 的桌面测试未启用。工作区检查、严格 all-targets Clippy 和 Windows debug 构建通过；日志为 `artifacts/task-coordination-check.log`、`task-coordination-tests.log`、`task-coordination-clippy.log` 和 `task-coordination-build.log`。
- `hosted_tasks_integration` 的 1 项真实 WSL systemd 集成通过。原有提交进程退出后完成、保留非零结果及 UID 1001 工作负载检查保持通过；新增真实普通用户服务持锁、root／另一普通用户工作负载被拒绝且没有生成副作用文件、只读服务并行且不释放原服务的锁、正常结束释放、SIGKILL 后没有伪造退出结果但锁已释放，以及后续新任务可执行并保留非零结果。
- 新锁测试的 6 个服务使用独立 UUID 和 `/tmp/arcnix-lock-test-<id>`；结束时只停止自己的 unit、移除预先列出的测试文件，并删除空测试目录。不删除服务器共享锁文件，避免另一个服务已打开的 inode 与新 inode 形成两把锁。记录见 `artifacts/task-coordination-integration.log`。
- 真实 `backups_integration`、`diagnostics_integration` 和 `database_backups_integration` 各 1 项通过，覆盖新排他执行器下的 restic 备份／恢复／保留、原生数据库导出与恢复，以及诊断的只读执行类别。数据库 fixture 仍只操作隔离的回环服务与临时数据，不接触已有数据库。日志见 `artifacts/task-coordination-tools.log`、`task-coordination-database.log` 和 `database-integration.log`。
- 实际 WebView 回归共 95 组通过：新 `task_coordination_smoke.py` 9 组，以及 `scripts_smoke.py` 13、`backups_smoke.py` 12、`database_smoke.py` 17、`configurations_smoke.py` 21、`diagnostics_smoke.py` 17、`navigation_smoke.py` 6。新专项使用两个独立配置／进程共享同一隔离 SSH peer，验证重启后原任务仍运行时拒绝新预检查；另一客户端已通过预检查的旧计划在执行时得到冲突结果 75；诊断仍完成；原任务完成后新计划可执行；中文窄窗口显示锁语义且不保存凭据。
- UI SSH fixture 以固定协议模拟锁和结果，不执行真实 shell／flock；与真实 systemd 测试合并证明界面路由和服务器执行器行为，不将模拟服务当作 Linux 权限验证。双客户端测试共享一个 Playwright runtime，各自连接独立 WebView；语言切换后等待中文页面完成再继续操作。检查截图 `artifacts/task-coordination-ui/first-1794659563662737800/restart-conflict.png` 与 `coordination-chinese.png`，结果见 `artifacts/task-coordination-ui/results.json` 和 `artifacts/task-coordination-*-ui.log`。

本阶段的锁保护新提交的全局任务，未迁移旧单机软件／命令路径，也不能约束旧执行器已提交任务、手工命令或其他工具。锁空闲只证明当前没有持锁操作，不证明旧任务成功、从未运行或可以直接重试；未知结果仍需核验。服务器重启恢复、不同发行版／flock 版本和实际远程 SSH＋sudo 仍待相应环境验收。

## 软件与运维入口统一服务器任务（2026-11-14）

- 当前单元测试 97 项通过：核心 70、桌面 27；7 项需额外回环 fixture 的桌面测试在该命令未启用。旧流式执行器及其 4 项专属测试随执行路径删除，新增类型化请求重建命令、忽略伪造显示文本及拒绝注入目标的测试，不沿用旧阶段的测试数量。严格 all-targets Clippy、Windows debug 构建通过，日志见 `artifacts/workspace-tasks-tests.log`、`workspace-tasks-clippy.log`、`workspace-tasks-build.log`。
- 使用独立回环 SSH fixture 启用 2 项桌面 worker 测试，验证服务列表／详情及非法单元拒绝、软件检测／搜索／已安装查询和旧连接请求拒绝；工作区 worker 已没有直接变更请求通道。记录见 `artifacts/workspace-tasks-read-workers.log`。
- 当前布局界面检查共 118 组通过：新版 `operations_smoke.py` 23，以及脚本 13、备份 12、数据库 17、配置发布 21、诊断 17、任务协调 9、导航 6。原入口专项检查从软件页预选当前服务器、扩展双服务器安装、精确 APT 计划、执行时断开工作区和切换服务器、关闭／重开后查询相同任务、已安装状态、类型化索引更新、服务筛选／详情／日志／取消／启停重启、命令创建编辑删除、非零结果、参数字面量、只读查询可用、三个原入口均无法绕过存量运行任务的锁，以及不支持 systemd 时拒绝提交。
- 运行旧任务首次查询可能返回 Running；测试继续对同一任务 ID 查询，直到取得结果，未重提任务。SSH fixture 使用固定协议模拟操作和锁，未运行真实 apt 或现有系统服务；不同 SSH 会话共享该隔离 peer 的模拟软件／服务状态。
- 真实 `hosted_tasks_integration` 的 1 项 WSL systemd 测试通过，保留客户端提交进程退出、原 UID、跨账户锁和强制终止检查，并新增实际类型化服务命令：为有 45 秒时限的私有 transient service 提交三个独立服务器任务，核对 start 已运行服务不替换 PID、restart 更换 PID、stop 后不再 active。测试只停止自己的 unit，并按已列举的 UUID 文件清理；未操作系统已有服务。记录见 `artifacts/workspace-tasks-systemd.log`。
- 目视检查最终 `artifacts/workspace-tasks-ui/desktop-1794661431890421600/` 的服务状态与中文截图，确认刷新／切换后不残留旧详情，命令页不再显示已迁走的空输出区；包计划另见同目录 `package-plan.png`。结果见 `artifacts/workspace-tasks-ui/results.json` 和 `artifacts/workspace-tasks-*-ui.log`。

入口统一阶段的 `ui_smoke.py` 当时仍引用旧顶栏、未选服务器文件页和旧确认对话框，该阶段仅使用上述专项验收，未宣称综合脚本通过；随后完成的迁移见下节，原历史记录的日期和范围不变。真实 APT 写操作、实际远程 SSH＋sudo、其他发行版与服务器崩溃恢复仍待专门环境验收；旧执行器已提交的任务不会因客户端升级自动获得新锁。

## 综合桌面回归迁移（2026-11-14）

`tools/ui_smoke.py` 已完成当前导航和服务器任务流程适配。基础模式 18 组、扩展模式 27 组通过；扩展包含基础范围，不将两轮相加为 45 组独立覆盖。本阶段修改测试与说明文档，未修改 Rust 功能代码。

- 基础范围保留文件单击／双击／Enter、Ctrl／Shift 选择、列表与图标网格、嵌套展开、路径编辑与面包屑、原生 Unicode 剪贴板、无外圈的键盘焦点、图标状态、对话框焦点／inert／Escape／错误处理、站点与分组创建、SSH 导入、长列表菜单、窄窗口和分栏跨进程恢复。
- 导航检查采用当前约束：未选服务器时没有单机模块，连接后六个模块位于页头下方；Servers 保持首功能入口。新增其他服务器时保留原 SSH 会话。分组和新增站点会改变行序／列表顺序，目标与分栏记录按固定服务器身份核对。
- 扩展范围通过实际回环 SSH 验证资源采样差值、日志过滤／多行／权限错误／自动刷新、语言切换保留连接、软件安装进入任务中心并在完成后重新查询已安装状态，以及索引更新计划。执行期间工作区连接和切换控件可用；未使用已移除的旧确认框与客户端流式执行器。
- 实际 SFTP 文件检查包括展开子项重命名／下载、中文 POSIX 路径与剪贴板、元数据、双向覆盖／跳过、按服务器保存与重置冲突策略、重连后策略保留。取消上传前特意写入不同的本地内容，取消后逐字节核对远端原文件未被替换；未只凭“Canceled”标签认定数据安全。
- `python tools/ui_smoke.py --packages --legacy-host-key --isolated-crypto-runtime` 通过 27 组，最终目录为 `artifacts/ui-smoke/run-1794662955446780600/`。使用生成的 Ed25519 私钥，保留原 RSA 主机指纹，桌面子进程 PATH 仅含 Windows 系统目录，且未自动新增其他信任记录。SSH 可用但 SFTP 被拒绝的账户仍可读取软件、资源和日志；重启再次核对两个测试配置的独立分栏比例。
- `python tools/ui_smoke.py` 通过 18 组，最终目录为 `artifacts/ui-smoke/run-1794662987219193900/`。两种模式都自行创建 loopback peer、配置、测试文件和密钥，不再读取外部 ready 文件或默认配置；结束时关闭自己的 peer 和桌面进程。每次运行独立留档，失败证据在 WebView 退出前捕获；最近通过记录分别由 `artifacts/ui-smoke/latest-packages.json` 与 `latest-files.json` 指向。
- 目视检查扩展轮的 `package-confirm.png`、`file-conflict.png`、`resources-narrow.png`、`ssh-without-sftp.png`。测试记录分别为各轮 `results.json`、`artifacts/ui-migration-smoke.log` 与 `ui-migration-files.log`。Git 差异可忽略空白后检查语义变更；脚本移出旧的 Playwright 上下文以便先保存失败证据、再关闭运行时，造成主体缩进变化。

软件、资源和日志命令由隔离 SSH fixture 返回固定结果，未执行真实 apt、读取真实系统日志或连接生产服务器。真实 SSH 协议、SFTP 文件往返、Windows 原生剪贴板和桌面状态恢复均在上述范围实际运行；不据此扩大发行版、数据库后端或崩溃恢复的验证矩阵。

## 服务器任务记录浏览（2026-11-14）

本阶段增加按 SSH 账户读取的服务器任务记录，不增加桌面后台执行器或轮询调度。功能通过以下检查：

- 工作区 Clippy（`--all-targets --locked -- -D warnings`）、Windows 桌面构建、99 项 Rust 单元测试及 5 项默认资源解析集成检查通过。`cargo doc -p arcnix-core --no-deps --locked` 在 `RUSTDOCFLAGS=-D warnings` 下通过；crate 包文件清单包含 `src/task_records.rs` 与 `src/task_records.sh`。
- 使用项目指定的 `nightly-2026-11-04` 执行 Rustfmt 并通过检查，另通过 typos。稳定版 Rustfmt 不应用项目的导入分组规则，因此最终以指定 nightly 为准；此次统一了此前新增运维文件的导入格式，未改变执行行为。
- `python tools/task_records_smoke.py` 通过 25 组真实 WebView／回环 SSH 检查。Alpha 使用密码认证且拒绝 SFTP，Beta 使用生成的私钥；不提供 sudo。先实际提交一项桌面任务，再关闭客户端并删除测试自己的本地历史，重启后通过服务器记录找回任务名称、类型和版本。Alpha 共 111 条，验证 100 条首页及 11 条旧记录；Beta 独立显示其记录。
- 覆盖凭据脱敏、活动服务优先于旧退出码、停止中、管理器不可用、失败服务与成功结果冲突、损坏元数据、不可读日志、无元数据且结果未知的旧日志、日志正文内协议标记、刷新及前后翻页。截断响应和根目录变化均拒绝更新，并保留旧快照；读取不重建本地历史，日志和密码不进入配置。忘记凭据、重新打开表单及离开 Tasks 的凭据清理均检查实际控件状态。
- 目视检查最终轮 `artifacts/task-records-ui/desktop-1794667185590656100/` 的 `records.png`、`legacy-log.png` 与 `records-chinese.png`。修复窄窗口下布局被 row 默认样式覆盖的问题；900×700 中文窗口中状态与字节数不拆行，无页面横向溢出。最终测试记录为 `artifacts/task-records-ui/results.json` 和 `artifacts/task-records-ui.log`。
- 设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu` 后运行 `cargo test --locked -p arcnix-core --test task_records_integration -- --ignored --nocapture`，真实 WSL／systemd 测试通过。专属 `/tmp/arcnix-records-*` 夹具创建 110 项任务记录；UID 1001 读取 100＋10 两页、元数据与日志，验证中文及注入形状标题仍为字面量，不创建标题中的文件。直接链接任务目录、符号链接／硬链接日志、坏元数据和变化根目录均拒绝信任；缺少 systemctl 时仍读得到旧日志而结果未知。
- 原生验证使用有时限的私有 systemd service：提交器退出后，旧退出码 0 不能隐藏活动进程；释放屏障后读取保存的退出码 9 和完成日志。测试只清理自身 fixture 和 unit，不删除共享操作锁。首次原生测试发现链接目录判断的 shell 短路问题，后续降权验证又发现 GNU find 依赖不可访问初始目录的问题，均已修复后重新通过。日志为 `artifacts/task-records-integration.log`。
- 原有 operations 23、scripts 13、backups 12、database 17、configurations 21、diagnostics 17、task coordination 9、navigation 6，共 118 组界面检查全部通过，日志位于 `artifacts/task-records-*_smoke.log`。新增记录检查后，这九套共计 143 组；计数为各脚本检查组，不表示完全互斥的覆盖范围。
- 综合桌面扩展回归再次通过 27 组，包含 RSA 信任兼容与 Windows 系统 PATH 隔离。最终目录为 `artifacts/ui-smoke/run-1794667096712075900/`，日志为 `artifacts/task-records-comprehensive.log`；不与上述套件相加宣称独立覆盖。最后的记录页布局调整另通过其 25 组专项复验，不涉及原模块执行逻辑。

回环 SSH 返回的服务状态与任务文件协议由 fixture 模拟，WSL 测试则实际执行服务器 shell 和 systemd，并用降权用户读取；这两类证据分开，不宣称已经验证真实远程 SSH＋Linux sudo 的完整配置发布链路。当前浏览仅覆盖登录账户的任务目录，不包含 root 私有配置版本、任务终止、记录清理或总磁盘占用。服务器重启续跑、跨发行版矩阵及所有既有 opt-in 集成测试未由本阶段重复证明。

## 服务器配置恢复记录（2026-11-14）

本阶段补齐 Configurations 的 root 私有记录浏览与恢复入口，并完善两类记录查询的快照保留。未增加客户端调度器、服务器常驻 Agent 或自动清理。

- 严格 Clippy、Windows 桌面构建、项目指定 nightly Rustfmt、typos、Python 语法检查及 `git diff --check` 通过。工作区单元测试为桌面 27 项、core 74 项，共 101 项；另通过 5 项默认资源解析集成检查。core 文档在 `RUSTDOCFLAGS=-D warnings` 下构建成功，crate 文件清单包含 `configuration_records.rs`／`.py` 及其原生集成测试，无 Python 缓存文件。
- `tools/configuration_records_smoke.py` 的 30 组真实 WebView／回环 SSH 检查通过。Alpha 使用密码认证、sudo stdin 且拒绝 SFTP，Beta 使用生成的私钥。先从桌面实际提交配置，再关闭客户端、删除测试自己的本地批次，重启后仍可从 Beta 的服务器记录找回原发布；Alpha 105 项记录验证 100＋5 翻页、容量统计及不完整提示。读取未创建本地任务历史，也未把配置正文、差异、日志或凭据写入配置。
- 覆盖活动／状态未知源任务、恢复记录、无效元数据、原始备份不可用及当前文件缺失；拒绝不具备恢复依据的记录。详情显示原服务器、保存阶段、检查设置、差异及脱敏日志。列表截断、根路径变化和详情编号不匹配均拒绝更新；详情刷新失败保留旧内容，但禁用恢复，重新核验成功后才恢复操作入口。
- 从服务器记录选择恢复仅打开新的任务表单，预选原 Beta 服务器；重新认证、生成差异并执行后，历史只包含新的恢复任务及原服务器记录编号，不依赖被删除的原本地批次。凭据在提交、主动忘记及离开 Configurations 后均清除；读取成功后再导航，验证返回时不可直接刷新。重新打开认证表单为空，重复读取不改写已有任务结果。
- 目视检查 `artifacts/configuration-records-ui/desktop-1794669862844834500/` 的 `private-version.png` 和 `records-chinese.png`，覆盖原服务器标识、只读与发布所需工具的区别、历史阶段提示，以及 900×700 中文页面。专项结果为 `artifacts/configuration-records-ui/results.json`，最终日志为 `artifacts/configuration-records-configuration_records_smoke.log`。
- `tools/task_records_smoke.py` 扩展为 27 组并通过。新增日志详情刷新失败后仍可读原日志、重连认证失败保留绑定身份的列表和采样时间。测试修复了只凭旧行数就认定新读取完成的等待条件，改为等待查询完成状态；实际实现也保留相同地址、账户、配置编号和主机指纹的旧列表，更换身份则不继承。普通日志详情只在同一页面及认证上下文内保留，不跨重连或离开页面复用。末次详情缓存清理调整后再次通过这 27 组，并通过 Clippy 和构建。
- 设置 `ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu` 后，`configuration_records_integration` 的真实 Linux 测试通过：创建 103 个私有有效记录及一个链接条目，逐页核对全部自有编号；核对当前与原始文件摘要和差异、后续手工修改、当前文件删除、实际活动 systemd 源任务、符号链接／硬链接原始文件及私有日志、坏清单和普通 UID 无法读取。只读查询不改写 manifest，无法计入的条目标为容量不完整；只清理自身随机编号、临时目录、unit 和专属配置路径锁。
- 同一原生测试直接调用发布执行器，并仅在测试进程内注入停止中及不可用的 systemd 查询，确认服务器预检查在创建恢复记录前拒绝这两类源状态；不依赖客户端按钮禁用来阻止恢复。上述记录在 `artifacts/configuration-records-native.log`，其中现有 Nginx 发布恢复测试也通过。
- `configurations_integration` 随后增强并再次通过：用真实 Nginx 发布产生的原始备份、清单及请求，经新读取协议重新构造恢复任务，核对原始／当前摘要与差异后完成真实恢复。恢复构造不使用原内存中的发布设置；其后继续验证元数据、误删目标、健康失败恢复、并发修改和限时 HTTP 检查。该轮日志为 `artifacts/configuration-records-producer-consumer.log`。
- 原有 operations 23、scripts 13、backups 12、database 17、configurations 21、diagnostics 17、task coordination 9、navigation 6，共 118 组均通过；加上普通记录 27 和配置记录 30，十套功能界面检查合计 175 组。日志为 `artifacts/configuration-records-*_smoke.log`，计数表示脚本检查组，不代表互不重叠的覆盖范围。
- 综合桌面扩展回归另通过 27 组，保留 RSA 主机信任兼容、隔离 Windows 系统 PATH、真实 SFTP 数据、原生剪贴板和跨进程状态恢复。目录为 `artifacts/ui-smoke/run-1794670035220121900/`，日志为 `artifacts/configuration-records-comprehensive.log`；不与上述功能套件相加宣称独立覆盖。

真实 Linux 证据覆盖本地 WSL root 下的生产者、记录读取和恢复流程；回环 SSH 证据覆盖桌面认证、sudo stdin、无 SFTP 读取与页面行为，其 sudo／服务响应由 fixture 模拟。真实远程 SSH＋Linux sudo 的完整发布链路及发行版矩阵仍待验收。容量仅为配置记录目录的已计量块占用，不包含账户任务目录、共享锁或全服务器空间；读取不是健康复检，也不执行记录终止、清理预览或删除。

## 记录存储与精确清理预览（2026-11-15）

本阶段为 Tasks 增加 Record storage，并从配置恢复记录提供入口。覆盖账户任务及 root 私有配置记录的占用、保留策略、保护原因和精确对象预览；没有删除执行器，不截断日志、停止服务或新增客户端常驻进程。

- 工作区严格 Clippy、Windows debug 构建及 103 项单元测试通过（桌面 27、core 76），另通过 5 项默认资源解析集成检查。最终首屏帮助折叠调整后，再次通过严格 Clippy、构建和记录存储专项 27 组。项目指定 nightly Rustfmt、typos、Python 语法检查、严格 core 文档及 `git diff --check` 通过；crate 文件清单包含两类记录读取程序、新维护模块及其原生测试，没有 Python 缓存。日志为 `artifacts/record-storage-unit.log`、`record-storage-clippy.log`、`record-storage-build.log`、`record-storage-doc.log`，清单为 `record-storage-package.txt`。
- `tools/record_storage_smoke.py` 的 27 组真实 WebView／回环 SSH 检查通过：Alpha 以密码和 sudo stdin 认证且拒绝 SFTP，Beta 使用生成的私钥；两台服务器独立读取，覆盖参数校验、默认保留策略、分配块与逻辑大小、100＋5 分页、最多 100 条跨页选择以及受保护项与候选混合预览。100 条选择包含 94 个候选，未因 SSH 包限制减少选中范围。
- 活动、状态未知、配置引用、遗留元数据、链接结构、配置任务证据和最新原始备份均显示保护原因；配置登记不可完整核验时所有候选受保护。引用不完整不再被当成空登记库，单个服务器失败不覆盖其他服务器的结果。
- 客户端拒绝越界对象、预览编号不符、截断 JSON、账户变化及根身份变化；读取失败保留旧采样结果，选择变化和成功刷新列表丢弃旧预览。任务标题脱敏，预览不写本地任务历史；忘记、重连、切换视图与导航检查确认凭据／预览清除及旧事件隔离。设置切换中文后，900×700 页面无横向溢出。
- 目视检查最终目录 `artifacts/record-storage-ui/desktop-1794675456777825000/` 中 `exact-preview.png` 与 `storage-chinese.png`，核对精确路径、候选和保护原因、断开状态、只读提示及默认折叠的帮助。结果为 `artifacts/record-storage-ui/results.json`，日志为 `artifacts/record-storage-record_storage_smoke.log`。
- 联调曾发现固定读取程序超过 libssh2 的 channel request 发送上限。当前将程序及参数分别 zlib／base64 压缩，在服务器内存解码；最终 SSH 命令上限 32 KiB，仍保留原有检查和全部记录。单元测试覆盖 100 个各 80 字符的不同编号，以及含中文、引号和美元符号的 HOME；专项界面检查实际通过两台回环 SSH 的压缩传输，不要求 SFTP 或临时脚本落盘。
- `record_maintenance_integration` 在显式指定的 WSL Ubuntu 真实 systemd 环境通过。root 查询 UID 1001 的专属账户任务树，创建 121 个任务及 4 个配置记录，验证多页可达、精确对象、稀疏文件块数、非 root 原始文件所有权、符号／硬链接、嵌套目录、缺失证据、坏元数据、引用、保留版本和实际活动的私有 unit。比较查询前后 inode、大小、mtime 与 ctime，确认记录没有被修改；保存的自定义命令没有执行。
- 同一原生测试仅在测试进程内注入不可用／矛盾的服务状态、配置登记权限错误及超时延迟，确认保护而非误判空库，并验证 1 秒全局超时能穿透逐条容错、在 3 秒延迟完成前结束。普通配置读取也改用独立超时异常并通过相同检查；服务状态故障注入不据此宣称覆盖所有真实 systemd 故障。
- `configuration_records_integration` 和真实 Nginx 的 `configurations_integration` 同时再次通过，共 3 项原生集成检查；后者保留实际发布 → 记录读取 → 新任务恢复链路。原生日志为 `artifacts/record-storage-native.log`，执行时使用项目 artifacts 内的诊断工具和显式 WSL 发行版，没有 Docker 构建或 Cargo 路径重定向。
- 原有 operations 23、scripts 13、backups 12、database 17、configurations 21、diagnostics 17、task coordination 9、navigation 6、普通记录 27、配置记录 30，加上本阶段存储 27，合计 202 组功能界面检查均通过。日志为 `artifacts/record-storage-*_smoke.log`；计数表示脚本检查组，并非互不重叠的场景。此轮全套检查在最终帮助折叠调整前完成，调整后重跑存储专项。
- 综合扩展回归另通过 27 组，包含 RSA 信任兼容、隔离 Windows 系统 PATH、真实 SFTP、原生剪贴板和跨进程恢复。目录为 `artifacts/ui-smoke/run-1794674878833499100/`，日志为 `artifacts/record-storage-comprehensive.log`；不与上述功能检查相加宣称独立覆盖。

容量是有时间标记的只读快照，统计合规记录目录及可计量直接普通文件，不包含共享锁或应用数据；已分配块数不保证等于未来可释放空间。读取有 10,000 条记录、每目录 64 个直接条目、2 MiB 响应及时间限制，链接、嵌套或不可读部分明确标为不完整。真实 Linux 检查运行于本机 WSL root；回环 SSH 的 sudo／服务响应为 fixture，两者不等于真实远程 SSH＋Linux sudo 的完整链路验收。后续实际清理仍须设计服务器持有的任务，执行时重新核验对象、引用和操作／路径锁；本阶段预览不具备删除授权或执行语义。

## 服务器记录清理（2026-11-15）

本阶段在精确预览之上接入独立审核的服务器清理任务，不改变预览协议的只读语义。候选重新认证、冻结与审核后才由 systemd 执行；没有客户端后台调度器，没有生产服务器部署或清理操作。

- 严格工作区 Clippy、Windows debug 构建、105 项单元测试（桌面 27、core 78）、5 项默认资源解析检查均通过。新增单元检查拒绝越界对象、活动状态、引用不完整、任务编号／冻结摘要／候选数不符，以及端点、账户或指纹变化；自由标题不进入冻结计划，类型化清理不能改用普通脚本参数。日志为 `artifacts/record-cleanup-unit.log`、`record-cleanup-clippy.log` 和 `record-cleanup-build.log`。
- 项目指定 nightly Rustfmt、typos、Python 语法及 `git diff --check` 通过。严格 core 文档和 crate 文件清单通过，新增 `.rs`／`.py`／原生集成测试均被收录，没有 Python 缓存。记录为 `artifacts/record-cleanup-doc.log` 与 `record-cleanup-package.txt`。使用正常 workspace target 和项目内 Perl，没有全局 Cargo／临时目录重定向或 Docker 构建。
- `record_cleanup_integration` 使用显式 WSL Ubuntu、root 和 UID 1001 的专属临时 HOME、5 个任务记录及 3 个私有配置记录。实际运行 Linux 文件系统、`renameat2`、flock、systemd 和 Python；任务树包含中文、引号与美元符号，链接条目指向测试自己的外部 sentinel。
- 真实生产者／消费者链路覆盖：原账户通过 stdin 上传 JSON，root 固定引导程序核验摘要与精确预览，冻结可信执行器和请求，随后通过摘要校验后的执行入口清理。预检查后修改输出文件，实际执行拒绝且记录保留；对失败任务再次执行也拒绝。预览后启动真实同名 unit，同样拒绝删除活动任务。
- 分别持有实际操作锁、配置登记锁和目标路径锁，确认清理拒绝、原始备份仍在。另在冻结计划后增加有效恢复引用，确认即使选中原始文件未变，也不能删除新的恢复来源；恢复测试元数据后继续后续检查。配置发布程序在登记独占锁期间不能获取预检查共享锁。
- 将实际清理命令延迟一秒提交到 systemd，提交 shell 已退出时核对 unit 仍 active，再读取服务器结果确认成功；移除三个选中任务，保留链接条目和 sentinel。私有逐项报告记录三个完成条目。另成功清理一个旧配置记录，保留未选中的记录、最新原始备份及共享锁。
- 中断检查仅在测试冻结程序中注入“实际 unlink 后暂停”，并更新测试摘要，以便确定地发送 SIGTERM。确认已移除一项后的剩余内容留在 root 私有暂存区，原路径不暴露部分记录，报告标记 interrupted／quarantined，再次调用原任务被拒绝。该故障注入不代表生产执行器允许上传任意代码，也不验证 SIGKILL、断电、服务器重启恢复或任意外部进程并发写入。
- `configuration_records_integration`、真实 Nginx 的 `configurations_integration` 和只读 `record_maintenance_integration` 一并重跑通过，合计 4 项原生检查；原发布 → 记录读取 → 恢复及只读指纹检查继续通过。总日志为 `artifacts/record-cleanup-native.log`。最终增加暂存区映射／空目录核验后，再次单独通过清理原生检查，日志为 `record-cleanup-native-final.log`。
- `tools/record_cleanup_smoke.py` 的 16 组真实 WebView／回环 SSH 检查通过：多服务器生成候选预览、仅绑定原端点、排除活动记录、重新填写凭据、取消不生成任务、预检查不移除记录、展示完整文件计划，再单独提交。Alpha 使用密码和 sudo stdin 且拒绝 SFTP，Beta 使用私钥，未预览的第三个站点不出现在目标表单。
- 两个已提交任务显示运行后关闭桌面，重新启动显示结果未知；查询原任务后均成功，没有重复提交。mock 存储只保留原活动记录；本地历史不含 SSH／sudo 口令或冻结请求与程序摘要。导航清除预览，中文 900×700 无横向溢出。目视检查 `artifacts/record-cleanup-ui/desktop-1794676921943727600/` 的 `cleanup-review.png` 与 `cleanup-chinese.png`；结果为 `artifacts/record-cleanup-ui/results.json`。
- 原 202 组功能界面检查全部重跑，加上清理 16 组合计 218 组通过。日志为 `artifacts/record-cleanup-*_smoke.log`，计数为脚本检查组，不宣称互不重叠。综合扩展回归单独通过 27 组，包含 RSA 主机信任兼容、隔离 Windows 系统 PATH、真实 SFTP、剪贴板与跨进程状态恢复；目录为 `artifacts/ui-smoke/run-1794677283525100200/`，日志为 `record-cleanup-comprehensive.log`。

上述真实 Linux 与回环 SSH 证据分开成立：前者为本机 WSL root 的实际删除和故障检查，后者模拟 sudo／服务／存储，只验证桌面与 SSH 协议编排；未据此宣称真实远程 SSH＋sudo 的完整 Linux 清理链路或发行版矩阵已验收。清理按服务器独立执行，删除无法撤销；部分失败保留暂存与 intent，不能自动续跑。清理审计记录自身的保留策略、私有报告和暂存区的图形化浏览／人工处置仍待完善，目前不计入普通任务／配置占用与清理范围。

## 清理报告与残留核查（2026-11-15）

本阶段新增 Tasks → Cleanup reports，按需浏览服务器私有审计记录及原位置／暂存区元数据，不新增常驻进程或人工处置执行器。保存阶段、服务状态、已记录移除、pending intent 和当前文件状态分别展示，不把旧阶段或文件缺失推断为成功。

- 严格 Clippy、Windows debug 构建及 107 项单元测试（桌面 27、core 80）通过，另有 5 项默认资源解析检查。新增协议检查覆盖原任务编号、固定私有根、分页边界、未知／遗漏对象、越界暂存路径、文件观察字段和缺失目录证明；active 与 completed／failed 矛盾保留各自含义。日志为 `artifacts/cleanup-records-unit.log`、`cleanup-records-clippy.log`、`cleanup-records-build.log`。
- 指定 nightly Rustfmt、typos、Python 语法和 `git diff --check` 通过。core 文档在 `RUSTDOCFLAGS=-D warnings` 下构建通过；crate 文件清单包含 `cleanup_records.rs`／`.py` 及扩展的实际清理集成测试，无 Python 缓存。记录为 `artifacts/cleanup-records-doc.log` 与 `cleanup-records-package.txt`。保持正常 workspace target，没有 Docker 构建或全局缓存重定向。
- `record_cleanup_integration` 在 WSL Ubuntu 的实际生产者／消费者链路上扩展：由真实冻结／清理程序生成 prepared、completed 和 interrupted 报告，经新的压缩 SSH 命令形式调用固定读取器。准备态保留原文件，完成态显示已保存移除标记与两个位置均缺失，中断态同时显示已落盘 pending、实际已缺失的一项和仍匹配审核的残留文件。
- 新任务在 rename 前写入目录 dev／inode 等完整身份数组，读取时核对其摘要与冻结 token，再证明暂存目录为同一 inode。删除测试报告中的数组后仍能读取安全元数据，但证明和完整性保持未知；更改暂存路径到 `/etc` 或给冻结请求创建硬链接时拒绝读取。UID 1001 无法直接读取 root 私有报告。
- 同一原生检查额外创建 103 个旧格式私有报告目录及一个目录链接，逐页核对全部自建编号、存量清理任务和计量不完整提示；保存的故障 runner 不执行。比较报告目录及实际暂存区读取前后的 inode、大小、mtime 和 ctime 指纹，确认查询没有改写证据。测试清理显式处理自建链接，复核没有残留审计夹具链接。
- 独立超时检查只在测试命名空间将全局期限缩为 1 秒、服务查询延迟为 3 秒，确认超时不会被逐条容错吞掉。原有实际清理的锁冲突、新引用、活动任务、SIGTERM 后私有残留、拒绝重放和提交进程退出后的 systemd 独立运行继续通过。
- 同轮真实配置记录、Nginx 发布 → 记录读取 → 恢复和只读存储检查全部通过，合计 4 项 Linux 集成检查。日志为 `artifacts/cleanup-records-native.log`，其中审计夹具编号为 `18e7fb1a14381894-4334-0`。这些检查运行于本机 WSL root，不将故障注入扩展为 SIGKILL、断电、服务器重启或所有外部并发修改的证明。
- `tools/cleanup_records_smoke.py` 的 29 组真实 WebView／回环 SSH 检查通过：Alpha 密码＋sudo stdin 且拒绝 SFTP，Beta 私钥；100＋5 翻页、有效／无效记录、活动状态优先、失败矛盾、prepared 尚未确认、两类文件位置与 pending 的区别、目录证明、私有错误脱敏均验证。读取不生成本地任务历史或保存报告详情。
- 详情编号、暂存路径、截断 JSON 和审计根变化均以本次具体错误为等待条件，旧快照及采样时间保留。重连认证失败保留原身份列表，私有详情不跨认证；读取在途时忘记凭据，晚到响应不会恢复详情或查询权限。导航、切换子视图、中文 900×700 页面以及返回最近批次的独立性通过。
- 原 `record_cleanup_smoke.py` 增至 18 组：关闭桌面后重新查询完成的两台服务器，直接从新报告入口读取这些实际 mock 提交产生的审计记录并查看文件标记；读取前后 `.tasks.json` 字节不变，不重新提交或补写成功确认。该测试配合原生生产者／消费者检查，分别验证界面编排和真实服务器文件，不将 fixture 的模拟删除冒充真实 Linux 删除。
- 回归中一次旧存储账户变化检查未得到预期错误；发现其前序负例只等待“错误可见”，可能命中旧错误。当前 fixture 为编号不符构造结构完整的异号记录，断言改为等待具体绑定错误；账户变化另等待真实 UID 探针。27 组存储检查随后通过，`artifacts/record-storage-ui/desktop-1794679548950982900/account-context.jsonl` 明确记录 UID 1002，确认测试实际触发账户变化，未据此宣称修复了产品权限绕过。
- 完整功能界面回归合计 249 组通过：原有 202 组、清理 18 组及报告 29 组；日志为 `artifacts/cleanup-records-*_smoke.log`，计数表示检查组而非互不重叠的覆盖。详情增加服务器身份与状态标题后，再次通过 Clippy、构建和报告专项 29 组。目视检查最终 `artifacts/cleanup-records-ui/desktop-1794679958061847200/` 中 `private-audit.png` 与 `reports-chinese.png`，覆盖来源、脱敏、残留标记、私有详情及中文窄页。
- 综合扩展回归单独通过 27 组，包含 RSA 信任兼容、隔离 Windows 系统 PATH、真实 SFTP、剪贴板和跨进程恢复；目录为 `artifacts/ui-smoke/run-1794679965666343200/`，日志为 `artifacts/cleanup-records-comprehensive.log`。不与上述功能组相加宣称独立覆盖。

报告浏览需要 Python 3.8+ 和 root／sudo，无需 SFTP；每次查询有 30 秒服务器总限时、45 秒 SSH 限时及 2 MiB 响应限制。当前只有元数据核查，不运行 runner、读取配置正文、恢复文件、重启服务、续跑任务或删除审计记录。读取期间变化会拒绝更新，旧报告缺少目录依据保持未证明。真实远程 SSH＋Linux sudo 的完整链路、残留人工处置及审计保留策略继续待完善，不能把只读观察和目录证明直接当作后续操作授权。

## 残留导出与核验移除（2026-11-15）

### 已验证

- Windows debug 构建、工作区严格 Clippy 与 109 项单元测试（桌面 27、core 82）通过，另有 5 项默认资源解析检查。新增协议检查覆盖已停止且有目录证明的源、活动／未证明／原位置占用来源拒绝、导出原件成员绑定、伪造摘要、重复编号、导出目标根、任务编号、端点／主机指纹和无参数类型化操作。日志为 `artifacts/record-recovery-build.log`、`record-recovery-clippy.log`、`record-recovery-unit.log`。
- 真实回环 SSH 的 operations／streaming 共 6 项集成检查通过。新增 3 MiB 上传在服务器先写出 6 MiB 输出再读取 stdin 的双向窗口压力下完成，核验接收字节 SHA-256；非 staging 命令、超 4 MiB 上传和输入不完整的提前退出被拒绝，普通命令的 4096 字节限制仍有效，超时／提前关闭后同一 SSH 会话仍可查询。记录为 `artifacts/record-recovery-transport.log`；fixture 只实现固定测试协议，不执行任意本机 Shell。
- WSL Ubuntu 的 `record_cleanup_integration` 实际创建原清理暂存区，由 UID 1002 的另一管理员上传处置计划，原任务账户仍为 UID 1001。来源在预检查后变化时拒绝导出；新审核接受变化后的真实字节。导出覆盖 NUL、0xff、中文及二进制扩展属性，副本为 root／600，原 uid／mode／扩展属性保存于私有元数据，源暂存区保留。
- 真实 systemd 托管导出及暂存移除均在提交进程结束后继续。已导出文件变化阻止移除预检查；另注入仅更新 manifest 文件身份、保留旧字节 SHA 的破坏，运行时完整哈希仍拒绝删除源。真实 unlink 后 SIGTERM 保存中断及余项，同一任务不能重放；新的计划对当前余项复验并校验全部已导出数据后完成，副本保留。
- 同一原生检查还覆盖配置暂存区的原始备份导出及移除：清理在真实 candidate unlink 后中断，original 留在可证明目录；配置登记锁冲突阻止移除，新任务核验后仅移除源暂存区，其他配置版本和导出 original 保留。fixture 为 `18e7ffd39b5f2b38-11714-0`，日志为 `artifacts/record-recovery-native.log`。任务／清理／配置／处置目录只清除本次唯一编号，公共协调锁保留。
- 原配置发布、配置记录和存储预览的 3 项 WSL 原生回归通过，连同扩展清理检查共 4 项；日志为 `artifacts/record-recovery-native-regression.log`。这不等同于已覆盖实际远程 SSH＋Linux sudo 的完整端到端链路。
- 新增 `record_recovery_smoke.py` 的 20 组真实 WebView／回环 SSH 检查通过：Alpha 密码＋sudo 且无 SFTP，Beta 私钥；旧来源缺少证明排除、取消不创建历史、重新认证、多目标预检查独立失败、精确残留清单、显式导出、桌面重启查询、源保留、导出单独选择、独立移除审核、部分失败后只重选余项、私密字段不持久化及中文 900×700 页面均覆盖。fixture 模拟实际复制／删除结果，真实文件字节由上面的 Linux 检查验证，不混为同一种证据。
- 原 249 组功能界面回归全部通过，连同新增共 269 组；覆盖清理 18、清理报告 29、记录存储 27、任务记录 27、配置记录 30、配置发布 21、脚本 13、运维 23、任务协调 9、导航 6、诊断 17、备份 12、数据库 17。每组日志为 `artifacts/record-recovery-regression-<模块>.log`；新页面证据位于 `artifacts/record-recovery-ui/desktop-1794684368541527900/`。
- 独立综合桌面扩展 27 组通过，启用 packages、旧 RSA 信任兼容和隔离加密运行时；包含原生剪贴板、真实 SFTP 字节、取消保护、软件任务和跨进程分栏恢复。日志为 `artifacts/record-recovery-comprehensive.log`，截图／结果位于 `artifacts/ui-smoke/run-1794684599431891600/`，不重复计入上述 269 组。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档与 `git diff --check` 通过；crate 文件清单包含 `record_recovery.rs`／`.py`、清理报告 helper 及扩展原生集成测试，无 Python 缓存。记录为 `artifacts/record-recovery-doc.log` 和 `record-recovery-package.txt`。人工查看两服务器导出审核和中文窄页截图，长路径正常换行。保持正常 workspace target，没有 Docker 构建或全局缓存／临时目录重定向。

### 范围与限制

只有存在的残留可以导出，不恢复已删除文件、业务原路径或原权限。元数据保存 uid／gid、权限、mtime／ctime 及有界扩展属性，不创建可直接接管业务的原记录。移除前全量校验数据及元数据，源删除后副本和审计保留；它们的独立容量、保留策略和直接下载入口继续待完善。新任务可处理已经完成导出的剩余子集，不自动续跑旧任务。没有新增桌面常驻、定时调度或远程 Agent，不承诺服务器重启后自动恢复、任意 root 写入者／硬件故障防护或跨服务器原子性。发行版与文件系统矩阵、实际远程 SSH＋sudo 链路仍需后续验收。

## 快照文件浏览与选定恢复（2026-11-15）

### 已验证

- 工作区严格 Clippy、Windows debug 构建与 111 项单元测试（桌面 27、core 84）通过，另有 5 项默认资源解析检查。新增协议检查覆盖快照／策略／来源绑定、直接子项、完整分页、越界路径、不支持类型的选择限制、字面通配符、重复及非法选择、源项存在证明和创建目标前复核。原确认检查补充精确路径范围及旧记录兼容。日志为 `artifacts/snapshot-files-check.log`、`snapshot-files-build.log`、`snapshot-files-clippy.log`、`snapshot-files-unit.log`。
- 使用项目内已核对发行 SHA-256 的 restic 0.19.1，在 WSL Ubuntu 扩展 `backups_integration`。实际快照浏览支持含中文、单引号、反斜杠和 `[*?]` 的文件名，105 项目录的第二页恰为 5 项，错误来源标签被拒绝。选定文件与子目录通过 systemd 执行，提交进程结束时服务仍 active；恢复后的二进制字节精确一致，未选文件／目录未恢复，缺失选择在预检查及运行时均失败且不创建目标。完整恢复、已有目标拒绝、符号链接重叠、仓库检查和按来源保留继续通过。成功 fixture 为 `/tmp/arcnix-backup-test-18e801141f23a660-115ec-0`，日志为 `artifacts/snapshot-files-native.log`。
- 初次原生负例发现：收到错误来源头后直接 SIGKILL restic，可能留下仓库读锁并阻止后续操作。当前查询失败先 SIGINT、最多等待 3 秒，随后才终止未退出进程；同一原生序列已验证错误来源后仍可恢复、备份和完整检查。不自动执行 restic unlock，也不把退出码未知的锁视作可删除。
- 隔离 PostgreSQL 16／MariaDB、restic 与实际 systemd 的数据库和 hosted-task 两项回归通过，和扩展目录检查共 3 项真实服务器工具检查。数据集只在本次 fixture 的 `/dev/shm/arcnix-databases.*` 内创建并清理，未安装系统数据库服务。记录为 `artifacts/snapshot-files-database-native.log`，本次准备信息为 `snapshot-files-database-fixture.json`。
- `snapshot_files_smoke.py` 的 25 组真实 WebView／回环 SSH 检查通过：密码＋sudo 且无 SFTP、只读浏览不创建历史、100＋5 分页、跨目录选择、目录／Unicode 项、错误编号／越界／截断响应拒绝、错误脱敏、旧页保留但恢复禁用、多服务器独立预检查失败、取消、重新认证、客户端重启查询及精确路径／origin_server 成功确认。还覆盖换服务器清除选择、忘记权限、关闭后晚到结果、中文 900×700 布局与未选目录清单不持久化。fixture 只模拟固定元数据协议及任务状态，实际恢复字节由上面的 restic 检查证明。
- 最终桌面复验暴露原有 Running 发布过早：客户端可能在服务器接收前关闭，重新查询正确显示原任务不存在，而此前界面已显示运行中。现在新增 Submitting，成功启动响应后才发布 Running；故意将 Beta 接收延迟 2 秒，界面先显示 Submitting，确认后才进入 Running，随后关闭客户端并查询两台已接收任务成功。持久历史单元检查也覆盖 Submitting 重启后保持 Unknown，不把未确认提交当成取消或重新执行。
- 原 269 组功能界面回归全部通过，连同新检查共 294 组。原范围为备份 12、数据库 17、脚本 13、运维 23、任务协调 9、导航 6、诊断 17、任务记录 27、配置记录 30、配置发布 21、存储 27、清理 18、清理报告 29、残留处置 20。日志分别为 `artifacts/snapshot-files-regression-<模块>.log`。
- 运维回归有一次在服务启动完成后意外切到 Backups，导致等待 Tasks 状态的断言失败；`artifacts/workspace-tasks-ui/desktop-1794686970785415800/` 保留该失败页面。带点击／导航事件跟踪的完整复验和随后普通复验均通过，跟踪为 `artifacts/snapshot-files-operations-events.jsonl`。没有复现来源，也没有据此宣称修改或修复了导航逻辑；继续保留为一次未定位的界面异常。
- 人工查看多服务器恢复审核和中文 900×700 快照文件窗口，文件名／命令正常换行，底部操作可见；最终专项截图和结果位于 `artifacts/snapshot-files-ui/desktop-1794686911145178400/`，日志为 `artifacts/snapshot-files-ui.log`。
- 独立综合桌面扩展 27 组通过，启用 packages、旧 RSA 主机信任和隔离运行时；覆盖原生剪贴板、真实 SFTP 字节、取消保护、软件服务器任务和跨进程分栏恢复，不重复计入上述 294 组。记录为 `artifacts/snapshot-files-comprehensive.log`，产物位于 `artifacts/ui-smoke/run-1794687392591472800/`。
- 指定 nightly Rustfmt、typos、9 份 Python 源码语法、严格 core 文档及 `git diff --check` 通过。crate 文件清单包含 `snapshot_files.rs`／`.py` 及扩展备份集成测试，无 Python 缓存；记录为 `artifacts/snapshot-files-doc.log`、`snapshot-files-package.txt`。桌面回归期间 Windows 曾拒绝替换正在使用的 exe，等待自有回归退出后重新构建并完成最终专项与全量复验，没有并行替换运行中的程序。保持正常项目 target，未使用 Docker 或全局构建／临时目录重定向。

### 范围与限制

只读浏览和选定恢复需要服务器 Python 3.8+，无需 SFTP；原完整备份／恢复不因此新增 Python 要求。每页 100 项、目录最多 10,000 项，原始元数据最多 16 MiB，响应最多 2 MiB；单个选择最多 100 个路径／4096 字节／32 个父目录，最终命令仍限 8192 字节。目录选择含全部后代，其他特殊节点只可间接随目录由 restic 恢复，不能直接选择。拒绝控制字符及非规范路径，不宣称任意字节文件名兼容。强制终止仍可能遗留 restic 原生锁，需核查原进程，不自动解锁。

成功确认是客户端观察到的明确任务结果，保留选中路径和来源，不等同于数据库或应用接管。恢复只到新目录，不覆盖业务原路径，不增加桌面常驻、调度或定时演练。真实远程 SSH＋Linux sudo 全链路、SFTP／S3 后端及更广发行版／restic 版本矩阵仍待验收；回环 SSH 的 sudo fixture 不冒充实际 sudo 执行。

## 批量目标筛选与复核（2026-11-15）

### 已验证

- Windows debug 构建、工作区严格 Clippy 与 113 项单元测试（桌面 27、core 86）通过，另有 5 项默认资源解析检查。新纯规则检查覆盖多词／大小写／Unicode 分组搜索、未分组、已选过滤、隐藏计数、追加选择、未信任来源排除、操作范围、未知目标、重复端点及恰好 32／超过 32 的边界；失败不返回部分添加集合。日志为 `artifacts/target-picker-check.log`、`target-picker-clippy.log`、`target-picker-unit.log`、`target-picker-build.log`。
- `target_picker_smoke.py` 的 27 组真实 WebView／回环 SSH 检查通过。库存含 38 个配置：三个独立回环服务器、一个未信任地址、一个同端点不同账户，以及 33 个仅供本地选择容量检查的候选。按名称、账户、端口、中文分组及多词搜索均验证；空匹配、禁用未信任项、可见追加、保留隐藏选择、仅清除可见项及查看全部已选均覆盖。
- 隐藏目标存在时预检查按钮禁用；展开已选后才继续。密码字段在筛选卸载／重建后保持与本次内存值一致，参数不丢失；取消该项、清除全部、关闭及下次表单均不复用旧凭据。超限和同端点账户冲突不部分添加，也不清除原有凭据。选择与取消本身不创建任务历史。
- 最终只将审核过的 Alpha／Beta 送入预检查与执行：Alpha 为密码＋sudo 且不支持 SFTP，Beta 为私钥＋sudo。客户端重启后按原编号查询两个已接收任务成功，不重放；任务历史不包含密码或筛选条件。备份策略只允许原 Alpha／Beta，其他分组和候选不能通过筛选进入该操作；取消备份表单不改变原历史。
- 原 294 组功能界面回归全部通过，连同新专项共 321 组。原范围包括脚本 13、诊断 17、备份 12、数据库 17、快照文件 25、运维 23、协调 9、导航 6、任务记录 27、配置记录 30、配置发布 21、存储 27、清理 18、清理报告 29、残留处置 20；日志分别为 `artifacts/target-picker-regression-<模块>.log`。诊断专项仅将共用表单的批量选择按钮名称适配为 Select visible trusted servers；服务器记录自己的只读选择表单未改。
- 独立综合桌面扩展 27 组通过，启用 packages、旧 RSA 信任兼容和隔离运行时，覆盖原生剪贴板、实际 SFTP 数据、取消保护、软件服务器任务及跨进程分栏恢复；不重复计入上述 321 组。日志为 `artifacts/target-picker-comprehensive.log`，产物位于 `artifacts/ui-smoke/run-1794688780901164900/`。
- 人工检查英文已选目标审核和中文 900×700 窗口：分组及动作正常换行，密码遮挡，内部目标列表可滚动，底部预检查／取消可见，无横向溢出。截图和结果位于 `artifacts/target-picker-ui/desktop-1794688332519643000/`，专项日志为 `artifacts/target-picker-ui.log`。
- 指定 nightly Rustfmt、typos、Python 专项语法、严格 core 文档及 `git diff --check` 通过。core 与桌面 crate 文件清单分别收录 `target_selection.rs` 和 `task_target_picker.rs`，无 Python 缓存；记录为 `artifacts/target-picker-doc.log`、`target-picker-package.txt`、`target-picker-desktop-package.txt`。使用正常 workspace target，无 Docker 构建或全局缓存／临时目录重定向。

### 范围与限制

本轮更新统一 ScriptRun 执行表单，没有给所有独立的策略编辑／记录连接表单添加同一过滤器。搜索不查询服务器实时系统或健康状态，不替代服务器预检查；信任仅指已有 SSH 主机指纹。端点去重沿用大小写不敏感的主机字符串和端口，不将 DNS 别名解析为同一机器。隐藏选择必须展开复核，不保存筛选条件，也不根据分组自动信任主机或扩大策略／冻结来源范围。

执行器、服务器端 systemd 生命周期及并发规则不变；此次没有重跑未改动的 Linux 备份／数据库工具矩阵，也没有将回环 fixture 的 sudo 认证当作实际 Linux sudo。没有新增客户端常驻、定时任务或调度器；长期后台能力继续按原边界留给服务器工具。

## 2026-11-15：安装模板检查与服务器报告

- 新增 installers.rs／installers.sh 和 installation_ui.rs。官方安装及软件入口的类型化安装定义与普通 Shell 分开，普通包安装不推断应用专用检查；用户编号的 builtin 前缀不授予官方身份。脚本中心显示实际模板版本和检查范围，复制仍产生普通用户脚本。
- 预检查核对 Debian／Ubuntu APT、OS 版本、原生 dpkg 架构及基础工具，只读取每个请求包的版本和状态，展示可读 APT 模拟／命令及必需检查。执行再次核对冻结环境，分别保存 APT、逐包状态和固定程序探针；包已存在、安装失败和 Nginx 配置／服务检查失败不混为成功。
- 完整报告绑定任务、版本、定义、步骤顺序、数量和退出结果。每项详情最多 512 字节，探针限时 20 秒及额外 2 秒终止宽限，输出有文件上限；超限保留未知及原因。末尾完整报告不超过 32 KiB，可从服务器 48 KiB 日志尾找回。原始协议留在完整服务器日志，界面展示可读检查；未创建任务目录新侧文件，不改变原记录清理白名单。
- 本地安装任务退出 0 仍须有完整匹配的必需检查，缺失／错任务／不一致报告保留未知，加载和重新查询同样核验。独立服务器记录可显示检查；列表保存的进程退出结果与读取日志后的检查结论有不同覆盖范围，旧记录和索引更新不捏造安装证明。认证词恰为 passed、短数字或其他协议常量时，只脱敏解码详情并保持公共结构，脱敏扩张后再次限制长度。
- 工作区严格 Clippy、Windows debug 构建和 117 项单元测试通过（桌面 27、core 90；7 项额外回环测试保持显式忽略），另通过 5 项默认资源解析。新纯规则检查覆盖任务／版本／范围绑定、缺失／乱序／矛盾协议、详情和最大包集合边界、真实安装状态、无效环境、退出 0 缺少证明，以及状态词／Unicode 脱敏。日志为 artifacts/installation-clippy.log、installation-unit.log、installation-build.log、installation-parsers.log。
- tools/installers_smoke.py 的 18 组真实 WebView／SSH 传输检查通过：两服务器安装审核、现有版本、来源伪装、关闭及重启查询、包与应用结果分离、本地历史删除后的独立服务器记录、环境变化、APT／服务失败、缺失／错任务报告、预检查阻止提交，以及中文窄页。另使用状态词口令完成真实 SSH 认证、执行和重启查询，验证详情脱敏后仍能核验。结果为 artifacts/installers-ui/results.json，最新截图位于 artifacts/installers-ui/desktop-1794692664949963700/。人工查看中文 900×700 页，版本已翻译、检查范围换行正常、Servers 保持首位，无横向溢出。
- 原 321 组功能界面回归全部通过，连同新专项共 339 组。范围为脚本 13、诊断 17、备份 12、数据库 17、快照文件 25、运维 23、协调 9、导航 6、任务记录 27、配置记录 30、配置发布 21、存储 27、清理 18、清理报告 29、残留处置 20、目标选择 27。最终日志为 artifacts/installation-regression-<模块>.log；新增专项日志为 installation-regression-installers.log。
- 独立综合桌面扩展 27 组通过，启用 packages、legacy-host-key 和 isolated-crypto-runtime；包含真实 SFTP 数据、原生剪贴板、软件服务器任务、语言切换及跨进程分栏恢复，不重复计入上述 339 组。日志为 artifacts/installation-comprehensive.log，最新产物为 artifacts/ui-smoke/run-1794692944200550400/。
- 显式设置 ARCNIX_TEST_SYSTEMD_DISTRIBUTION=Ubuntu、ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY=/mnt/d/Works/arcnix/artifacts/diagnostic-tools/extracted/usr/sbin，运行 cargo test --locked -p arcnix-core --test installers_integration -- --ignored --nocapture --test-threads=1，2 项真实 Linux 集成通过。第一项在唯一私有 APT／dpkg 根中实际安装本地构建的测试包，APT 配置、状态、缓存和日志均隔离；提交 shell 退出后 systemd 继续，验证真实版本、重复安装、环境改变不执行 APT、APT 失败后仍读取包状态。第二项通过实际提取版 Nginx 和私有服务验证配置错误与 active 服务分离、服务停止及输出超限；此项包获取／状态使用夹具适配，不声称实际从发行版仓库安装 Nginx。日志为 artifacts/installation-native.log。
- Nginx 集成最初因提取版默认临时目录指向 /var/lib/nginx/body 而失败；夹具已将全部临时目录、PID、配置、监听和日志指向唯一私有范围并重新通过，未修改宿主 Nginx 配置或安装系统软件包。失败夹具确认 MainPID=0 后只移除已知文件和空目录，私有失败 unit 已 reset。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、两 crate 文件清单和 git diff --check 通过。清单收录新的固定 Shell、Rust 模块及原生测试，无 Python 缓存；记录为 artifacts/installation-doc.log、installation-package.txt、installation-desktop-package.txt、installation-diff-check.log。新增 Shell 的 LF 属性规则，避免 Windows 检出改变远程执行器换行。使用正常 workspace target，没有 Docker 构建或全局缓存／临时目录重定向。
- 验证边界：界面包操作由回环 SSH 夹具应答，真实 APT 使用私有本地包而非外部仓库；尚未完成多发行版、ARM64、真实 SSH＋sudo 全链路及生产软件升级矩阵。模板指纹不锁定 APT 包或依赖版本；包脚本可能修改配置或启动服务。Nginx 检查不证明 HTTP／外部连通，数据库客户端检查不证明数据库访问或权限，restic 程序检查不证明仓库可用。没有定时计划、自动重试、卸载、安装自动回滚、跨服务器串行编排或通用服务器重启续跑。

## 按需恢复演练（2026-11-15）

- 完整目录快照或 PostgreSQL 导出可恢复到新目标，再运行 1–12 项文件大小／SHA-256 或只读表行数检查。服务器报告绑定任务、版本、执行服务器、原来源、快照和验收范围；退出 0 缺少完整证明时保持未知。演练确认独立保存，后续失败不覆盖此前成功。成功或失败均保留演练数据，不自动接管业务、清理、重试或续跑。
- 严格工作区 Clippy、Windows debug 构建和 122 项单元测试通过（桌面 27、core 95），另通过 5 项默认资源解析检查。记录为 `artifacts/drill-clippy.log`、`drill-unit.log`、`drill-build.log` 和 `drill-parsers.log`；指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、两 crate 文件清单及 diff 检查也通过。
- `tools/recovery_drills_smoke.py` 新增 16 组真实 WebView／回环 SSH 检查，覆盖密码＋sudo 无 SFTP、双目标、策略范围、验收条件编辑与非法输入、关闭重启查询、本地历史删除后独立读取、6 类失败结果、PostgreSQL 表条件、MySQL 不提供数据库演练及中文窄页。完整功能回归为 355 组（原 339 加新增 16）；最终记录为 `artifacts/drill-regression-<模块>.log`。人工查看 `artifacts/recovery-drills-ui/desktop-1794700564511681200/` 的窄窗表单与中文报告，无横向溢出，Servers 保持首位。
- 独立综合桌面扩展 27 组通过，启用 packages、legacy-host-key 和 isolated-crypto-runtime，覆盖真实 SFTP 字节、原生剪贴板、软件任务和跨进程偏好恢复，不重复计入上述 355 组。日志为 `artifacts/drill-comprehensive.log`，产物为 `artifacts/ui-smoke/run-1794700907484284200/`。
- 显式使用 WSL Ubuntu、项目内 restic 和私有 PostgreSQL 16 夹具，`recovery_drills_integration` 两项真实工具检查通过。目录覆盖中文和字面通配符文件、正确／错误摘要、空文件、符号链接／FIFO 拒绝、已有目标、长输出、SIGINT、子进程组退出及操作锁释放。数据库覆盖真实新库恢复、最小行数失败后保留数据、已有库拒绝；非超级用户恢复 FORCE RLS 表时，即使最小行数为 0，也拒绝把过滤后的计数作为通过。记录为 `artifacts/drill-native-files.log`、`drill-native-postgres.log`。
- 原生预检查最初发现 stderr 混入快照 JSON，现已分离捕获并复验通过。运行器对子进程继承操作锁，即使组长先退出仍终止剩余进程；文件验收逐层拒绝链接，使用非阻塞普通文件描述符，并在摘要前后核对身份。PostgreSQL 固定只读查询、禁止行级过滤，语句 30 秒、子进程 45 秒；不接受自由 SQL。
- 验证边界：界面服务器任务由回环 SSH 夹具应答，真实数据恢复由 WSL systemd 原生检查证明，尚不等同于真实远程 SSH＋sudo 全链路。最终报告来自运行器的结束处理；SIGKILL、掉电或重启无法保证产生最终报告，也不自动续跑。离线来源异机恢复向导、MySQL 数据库演练、SFTP／S3 仓库及多发行版／授权矩阵仍待完善。未新增常驻客户端、调度或远程 Agent，使用正常项目 target，没有 Docker 构建或全局缓存重定向。

## 演练请求保存与中断核验（2026-11-15）

- 在上一阶段基础上，新增演练请求的服务器端持久保存。任务提交前写入公开的执行服务器及 Description，只有写入成功才允许启动；不保存认证文件引用、密码或执行脚本。元数据读取上限为 8 KiB，Description 仍限制 4 KiB，旧记录兼容。查看缺失或矛盾报告时展示原请求及其验收条件，明确不构成执行证明，也不会续跑。
- 严格工作区 Clippy、Windows debug 构建和 123 项单元测试通过（桌面 27、core 96，额外 7 项回环检查保持显式忽略）。新增规则检查验证超过旧 4 KiB 的公开请求可独立解析、错误类型／执行服务器被拒绝，以及请求存在仍不能生成恢复成功证明。日志为 `artifacts/drill-context-clippy.log`、`drill-context-unit.log`、`drill-context-build.log`。
- 扩展真实 restic／WSL systemd 目录检查：在恢复阶段对本测试拥有的 cgroup 发送 SIGKILL，等待进程及操作锁退出。最终报告和退出结果缺失时，独立记录仍读回超过 4 KiB 的完整原请求，状态保持未知；记录容量读取识别新类型和较大的元数据，并保护无确认结果的记录。原 SIGINT、文件字节、摘要、链接／FIFO、拒绝覆盖和保留检查继续通过。私有 PostgreSQL 新库、行数失败及 FORCE RLS 检查也通过，共 2 项真实工具检查；日志为 `artifacts/drill-context-native-files.log`、`drill-context-native-postgres.log`。
- 演练专项扩展为 18 组真实 WebView／回环 SSH 检查。缺失报告和错执行服务器两种结果均在删除全部本地历史、重启后从独立服务器记录核验：请求范围可找回，结果保持未知，不创建任务历史或生成新成功确认。另回归任务记录 27、记录存储 27、清理 18、脚本 13、备份 12、配置 21、任务协调 9，共 145 组相关界面检查，日志为 `artifacts/drill-context-regression-<模块>.log`。人工查看 `artifacts/recovery-drills-ui/desktop-1794702269687527200/missing-report-saved-request.png`，原请求与结果未知状态展示清晰。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、crate 文件清单和 diff 检查通过。记录为 `artifacts/drill-context-fmt.log`、`drill-context-typos.log`、`drill-context-doc.log`、`drill-context-package.txt` 和 `drill-context-diff-check.log`。本阶段没有更换执行器、扩大备份范围或引入调度与客户端常驻；请求存在不保证服务器已接收，不保存阶段检查点，也不承诺强制终止或服务器重启后有最终报告。

## 离线来源异机恢复（2026-11-15）

- 备份策略提供显式异机入口，分别保存原来源编号与访问服务器；来源配置已删除时仍显示策略编号，无须原来源在线或受信任。访问端按原策略／来源标签读取，恢复预检查及执行继续核验完整快照和路径。显式异机模板可选择策略外的已信任恢复目标，普通恢复、备份、初始化、检查、保留及清理不扩大范围，策略列表和版本不作临时改写。
- 严格工作区 Clippy、Windows debug 构建和 124 项单元测试通过（桌面 28、core 96；7 项额外回环检查保持显式忽略），默认资源解析 5 项通过。规则覆盖普通／异机恢复范围隔离、非恢复动作不能启用外部目标、自定义脚本不能获得该能力，以及恢复标签双向翻译。记录为 `artifacts/cross-recovery-clippy.log`、`cross-recovery-unit.log`、`cross-recovery-build.log` 和 `cross-recovery-parsers.log`。
- 扩展现有 2 项真实 restic／WSL systemd／私有 PostgreSQL 检查。目录快照完成后移走本夹具的原来源目录，以不同执行服务器编号恢复并核验实际文件字节，原路径缺失仍通过。PostgreSQL 原连接改为不可用的本地地址，目标保持独立有效连接，仍从仓库导出恢复新库并通过明确表条件；原最小行数、FORCE RLS、拒绝覆盖、SIGINT／SIGKILL 与元数据找回检查继续通过。记录为 `artifacts/cross-recovery-native-files.log`、`cross-recovery-native-postgres.log`；这证明相同 Linux 夹具内的连接和来源分离，不等同于跨物理主机的数据传输矩阵。
- 新增 `tools/cross_recovery_smoke.py` 的 15 组真实 WebView／回环 SSH 检查通过。库存中不存在两个原来源配置，仅有密码＋sudo 无 SFTP 的 Beta、私钥 Gamma 及未信任项；核验多词／分组搜索、拒绝未信任项、错来源响应、错误脱敏、通过 Beta 浏览原来源文件、策略外双目标选定恢复、关闭重启查询和精确确认。还覆盖普通范围不扩大、完整恢复审核、PostgreSQL 新库与演练、切换端点清除凭据和详情、关闭后忽略晚到结果、中文 900×700 及策略／凭据不持久化变更。结果为 `artifacts/cross-recovery-ui/results.json`，日志为 `artifacts/cross-recovery-ui.log`。
- 首次专项在切换中文后的导航／查找阶段超时；同时检查到新增翻译条目的列顺序错误，已按既有中文／英文列约定修正，包括上一阶段的请求兜底文案。专项按已有语言切换测试约定等待常规页标题更新后再导航，完整复验通过；没有据此修改或宣称修复导航实现。新增双向翻译规则检查。人工查看 `artifacts/cross-recovery-ui/desktop-1794703946849164600/` 的中文窄页与双执行端审核，正文换行及页脚正常，Servers 仍为首个业务入口。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、crate 文件清单及 diff 检查通过；对应 `artifacts/cross-recovery-fmt.log`、`cross-recovery-typos.log`、`cross-recovery-doc.log`、`cross-recovery-package.txt`、`cross-recovery-diff-check.log`。使用正常 workspace target，没有 Docker 构建、全局缓存或临时目录重定向。
- 范围限制：来源编号仍须保存在策略中，未知来源的仓库内发现待完善；仓库地址和认证文件路径沿用策略，需要各访问／恢复端提前准备。界面夹具模拟快照／任务应答，真实恢复由独立 Linux 检查验证；SFTP／S3 仓库、跨物理主机 SSH＋sudo、数据库和发行版矩阵继续验收。不自动复制凭据、准备目标环境、覆盖业务原路径、接管应用、调度、重试或服务器重启续跑。

- 完整回归初次在配置记录恢复提交后意外进入 Backups，导致等待任务成功时列表消失。失败页面保存在 `artifacts/configuration-records-ui/desktop-1794704360407191000/failure.png`／`failure.txt`。随后 3 次带事件记录的完整配置记录检查均通过，6 次执行点击与 6 次 DOM 点击对应，无额外 Backups 导航；再进行 1 次普通配置记录检查也通过。记录为 `artifacts/cross-recovery-configuration-traced.log`、`cross-recovery-configuration-events.jsonl` 及最终 `cross-recovery-regression-configuration_records.log`。该现象与较早运维回归的未定位跳页相似，未复现不等于已修复；没有修改导航实现或增加强制跳回逻辑。

- 原有 357 组功能界面检查最终均完成，连同新异机恢复专项共 372 组；重复的配置记录复验不另计数。各模块最终日志为 `artifacts/cross-recovery-regression-<模块>.log`，新增专项为 `cross-recovery-ui.log`。
- 独立综合桌面扩展最终 27 组通过，启用 packages、legacy-host-key 和 isolated-crypto-runtime，覆盖真实 SFTP 字节、原生剪贴板、软件服务器任务及跨进程偏好。最终普通运行日志为 `artifacts/cross-recovery-comprehensive-final.log`，产物为 `artifacts/ui-smoke/run-1794705094218303800/`，不重复计入上述 372 组。
- 综合检查首次在服务器菜单的悬停透明度断言失败（完成前 8 组后结束），失败产物为 `artifacts/ui-smoke/run-1794704721321882500/`，原日志为 `artifacts/cross-recovery-comprehensive.log`。保持原断言，增加只记录事件的临时诊断后完整 27 组通过，保存 2613 个指针事件；随后普通完整复验也通过。跟踪为 `artifacts/cross-recovery-comprehensive-traced.log` 和 `cross-recovery-comprehensive-pointer-events.jsonl`。未复现失败时的事件序列，不能据此判断是外部指针、宿主时序或应用原因；没有修改悬停实现、降低断言或宣称修复该现象。

## 脚本包分享与审核导入（2026-11-15）

- 新增 script-only JSON 格式与客户端的读取／选择／预览／保存流程。复制导入生成新编号，显式替换保留唯一同名项的原编号；相同定义跳过，歧义、非法参数及容量不足在保存前拒绝。计划绑定原库摘要，来源文件后续变化不会改变已审核定义，导入不创建远程任务或获得官方能力。
- 严格工作区 Clippy、Windows debug 构建和 129 项单元测试通过（桌面 28、core 101；7 项额外回环检查仍显式忽略）。新增 5 项核心检查覆盖未知能力／编号字段、格式／大小／数量、重名副本与替换、名称截断及 Unicode、已存在／竞争写入文件保护、完整文件字节、来源冻结、临时文件清理，以及保存前配置序列化容量检查。记录为 `artifacts/script-bundles-clippy.log`、`script-bundles-unit.log`、`script-bundles-build.log`。
- `tools/script_bundles_smoke.py` 的 13 组真实 WebView／回环 SSH 检查通过：无需活动服务器、拒绝伪造能力和外部编号、取消无变更、新副本／显式替换、相同项跳过、来源变更后仍保存审核版本、名称／说明／正文／权限的分享副本、原库不变、目的文件竞争、普通字面参数和双服务器执行、关闭重启查询、清除草稿、中文 900×700、服务器数据及信任保留，以及 200 项容量拒绝。JSON 文件实际读写由本机文件系统完成，远程任务由既有回环 SSH 夹具应答，不声称在生产服务器执行了导入脚本。结果为 `artifacts/script-bundles-ui/results.json`，日志为 `artifacts/script-bundles-ui.log`。
- 原脚本 13、运维 23、导航 6、目标选择 27 组回归通过，连同新专项共 82 组相关界面检查。日志为 `artifacts/script-bundles-regression-<模块>.log`。独立综合桌面扩展另通过 27 组，启用 packages、legacy-host-key 和 isolated-crypto-runtime，覆盖真实 SFTP 字节、原生剪贴板、软件任务和跨进程偏好，不重复计数；日志为 `artifacts/script-bundles-comprehensive.log`，产物为 `artifacts/ui-smoke/run-1794707440547504100/`。
- 人工查看 `artifacts/script-bundles-ui/desktop-1794707366026759200/` 的分享副本审核和中文窄页截图，源脚本与输出副本区分清楚，正文换行及页脚正常，Servers 仍为首个业务入口。指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、crate 文件清单和 diff 检查通过；对应 `artifacts/script-bundles-fmt.log`、`script-bundles-typos.log`、`script-bundles-doc.log`、`script-bundles-package.txt`、`script-bundles-diff-check.log`。
- 范围限制：便携文件仅承载普通用户 Shell 定义，不包含官方检查程序、服务器配置或运行时参数值。分享副本支持用户手动脱敏，不自动判断任意正文是否含秘密。导出只发布新文件，不覆盖旧文件；强制关闭可能遗留自身临时文件，不提供恢复写入或自动重试。文件摘要标识已读取内容，不是发布者签名。未新增后台执行器、调度或服务器工具依赖；采用正常项目 target，没有 Docker 构建或全局缓存重定向。

## 类型参数与本地预设（2026-11-15）

- 新增可选参数定义、六类值校验、默认值及可选空参数；旧脚本的文本规则和原版本摘要保持兼容。预设独立于脚本保存，只记住明确允许的字段，以私有 Template 来源区分官方和用户脚本，失效版本禁止直接载入。分享 v2 携带规则／默认值，v1 继续兼容；分享和新副本不携带本地预设值。
- 严格工作区 Clippy、Windows debug 构建及 136 项单元测试通过（桌面 29、core 107；7 项额外回环检查仍显式忽略）。新增 6 项 core 检查覆盖整数及端口边界、路径根目录／非规范组件、选项／布尔值、控制字符／字节长度、默认值／字段匹配、可选空字面量与注入文本、明确允许与重置、失效规则、来源伪造隔离、10 项及重名限制、旧版本摘要、v2 往返／v1 兼容、禁止额外字段和预设值排除；新增双向翻译检查保留参数名称。记录为 `artifacts/script-parameters-clippy.log`、`script-parameters-unit.log`、`script-parameters-build.log`。
- `tools/script_parameters_smoke.py` 的 11 组真实 WebView／回环 SSH 专项通过：六类运行控件及无效值阻止预检查；仅保存允许字段，载入重置其他值；明确更新及重名保护；重启保留但不自动应用；编辑上下限、失效禁用和移除保存许可；新脚本作者配置；可编辑 v2 分享规则、原脚本保留及独立导入；字面参数／双服务器任务与关闭重启查询；旧脚本、官方／用户预设隔离；失效删除；中文 900×700。结果为 `artifacts/script-parameters-ui/results.json`，最终日志为 `artifacts/script-parameters-ui.log`，截图为 `artifacts/script-parameters-ui/desktop-1794709690606825400/`。本机 JSON 实际读写，远端由现有 SSH 夹具应答；不把夹具结果称为真实服务器软件安装验证。
- 原脚本 13、脚本包 13、运维 23、导航 6、目标选择 27 组均通过，连同新专项共 93 组相关界面检查。补充本地错误翻译并调整分享编辑顺序后，重新构建并完整复验新增 11 与脚本包 13 组。记录为 `artifacts/script-parameters-regression-<模块>.log`。综合桌面扩展另通过 27 组，启用 packages、legacy-host-key 和 isolated-crypto-runtime，覆盖真实 SFTP 字节、原生剪贴板、软件服务器任务及跨进程偏好；日志为 `artifacts/script-parameters-comprehensive.log`，产物为 `artifacts/ui-smoke/run-1794709713982306500/`，不重复计数。
- 新专项首轮失败分别来自测试把框架附带样式类作为错误、以及 has 定位器包含多余外层 dialog。调整为核验 active 类和正确的相对定位后完整通过；没有放宽类型、隔离或恢复断言，也没有因此修改生产校验逻辑。此前未定位的偶发页面切换／菜单悬停问题仍不宣称修复。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、两 crate 文件清单及 diff 检查通过。清单含 `script_parameters.rs`／`script_parameters_ui.rs`，没有 Python 缓存；记录为 `artifacts/script-parameters-fmt.log`、`script-parameters-typos.log`、`script-parameters-doc.log`、`script-parameters-package.txt`、`script-parameters-desktop-package.txt` 和 `script-parameters-diff-check.log`。人工查看英文预设及最终中文窄页截图，类型选项、文字、滚动内容和固定页脚正常，Servers 保持首个业务入口。
- 范围限制：路径类型只核验格式，不证明目录已存在；布尔值传入 true／false 字面量，可选空值不省略参数位置。默认值会随脚本保存和分享，预设是明确允许的本机便利数据，不是凭据保险箱。没有自动应用、定时执行、客户端常驻或额外服务器软件依赖；现有预检查、提交、服务器 systemd 执行与结果查询流程保持原边界。本阶段未扩展发行版／架构、MySQL 或 S3 的真实验收矩阵，仍按 roadmap 继续推进；采用正常 workspace target，没有 Docker 构建或全局缓存／临时目录重定向。

## 安装策略、变更核验与 systemd 变量边界（2026-11-15）

- 新类型化安装定义为 v2，默认保护已有软件包／依赖；升级和已有未配置包的配置需明确选择并重新审核。每目标冻结完整 dpkg 状态摘要及安装／配置版本、架构和数量，实际 APT Pre-Install-Pkgs 协议 3 在持锁、调用 dpkg 之前核对；不以第二次模拟作为原子保证。无变更只验证，不启动 APT。新报告独立保存 plan／install／包及应用检查，旧报告保留读取并提示没有新的审核变更证明。
- 严格工作区 Clippy、Windows debug 构建及 141 项单元测试通过（桌面 30、core 111；7 项额外回环检查仍显式忽略）。新增 3 项变更模型检查覆盖依赖、版本／架构／顺序、重复／不完整动作、默认保护、降级比较命令、移除／异常输出、状态摘要、缺失包和配置修复；另增加 v1／v2 报告证明区分和旧报告范围展示检查。日志为 `artifacts/install-plan-clippy.log`、`install-plan-unit.log`、`install-plan-build.log`。
- `installers_integration` 的 2 项真实 Linux 检查通过。环境为 Ubuntu 24.04 amd64、APT 2.8.4，APT 配置／状态／缓存及 dpkg 根均是唯一私有目录，只安装本地构建的测试包。实际验证首次安装与 Unicode 字节、已有版本、无变更不调用 APT、审核后状态改变、候选版本 1.2.4 → 1.2.5 改变时拒绝且旧包仍为 1.2.3、重新审核后升级为 1.2.5、旧钩子协议拒绝、APT 开始后状态改变仍在 dpkg 前拒绝、降级预检查拒绝、模拟输出超限、环境改变及 APT 失败。另用 dpkg 在私有根制造仅解包未配置状态，默认模式拒绝，明确模式审核 Conf 后实际完成配置。提交 shell 已退出后由 systemd 继续；日志为 `artifacts/install-plan-native-installers.log`。
- 第二项使用提取版真实 Nginx 与私有服务，验证程序、配置、active 状态及有界探针；包获取／状态使用适配，不声称从仓库向系统安装了 Nginx。安装预检查最初把小文件上限应用到 APT，真实缓存写入因此失败；改用有界 FIFO 捕获 stdout／stderr，APT 正常缓存写入保持独立，补充输出超限原生检查后通过。
- 真实安装又发现共享启动层会让 systemd 提前展开 ${...}，使版本字段及 Shell 参数表达式丢失。只在 ExecStart 边界将 $ 转为 $$，恢复传给 Shell 的原始文本。增强 `hosted_tasks_integration`：实际核验参数默认展开、带美元符号／命令替换文本的字面量、真实 PID、awk 字段、百分号、root／UID 1001，以及原有跨用户操作锁、关闭提交进程后继续和结束／终止释放。该项通过，日志为 `artifacts/install-plan-native-tasks.log`；旧任务不重放或补写证明。机制与范围见 development.md 的“安装策略与变更核验”，链接了 APT／systemd 原始文档。
- 因启动层是共享路径，进一步通过真实 restic 备份／恢复、按需诊断、Nginx 配置发布恢复、账户任务记录、配置记录、记录存储、清理／残留导出移除和目录恢复演练 8 项原生回归；日志为 `artifacts/install-plan-native-<模块>.log`。再由隔离 PostgreSQL／MariaDB 夹具通过数据库备份及 PostgreSQL 演练 2 项，日志为 `install-plan-native-database_backups-database.log`、`install-plan-native-recovery_drills-database.log`，夹具服务完成后关闭。本阶段共 13 项真实 Linux 集成测试，重复复验不另计。
- `tools/installers_smoke.py` 的 28 组真实 WebView／回环 SSH 检查通过，覆盖原有安装流程和新增策略、摘要及持久范围、未审核的依赖变化、移除／格式／截断拒绝、明确升级、降级拒绝、状态／计划／协议变化、缺失变更证明、关闭客户端后查询、独立服务器报告、口令脱敏及中英窄页。界面阶段初次失败来自夹具未分发管理员上下文中的新只读查询；增加固定响应后通过，没有改动生产查询判定。远程阶段仍由 SSH 夹具解释，不替代真实 APT 验证。
- 共 21 个运维界面专项、406 组检查最终通过，覆盖安装、脚本、运维、任务协调、备份、数据库、诊断、配置、账户／配置／清理记录、存储、清理／残留处置、演练、快照／异机恢复、目标选择、脚本包、参数预设和导航。诊断／配置中的两处断言仍期待旧的未引用命令展示，调整为核验当前实际引用后完整通过，保留包名及安装目的；没有放宽参数、安全或结果断言。旧报告展示补充后，重新构建并复验相关入口。最终日志为 `artifacts/install-plan-regression-<模块>.log`，按日志核对的汇总为 `artifacts/install-plan-validation-summary.json`。
- 综合桌面扩展另通过 27 组，启用 packages、legacy-host-key 和 isolated-crypto-runtime，覆盖真实 SFTP 字节、原生剪贴板、软件任务和跨进程偏好；不重复计入上述 406。日志为 `artifacts/install-plan-comprehensive.log`，产物为 `artifacts/ui-smoke/run-1794713012297196900/`。人工查看 `artifacts/installers-ui/desktop-1794712615994316400/` 的英文及中文 900×700 安装策略截图，选项、换行、滚动与固定页脚正常，Servers 保持首个业务入口。此前未定位的偶发导航／悬停现象仍不宣称修复。
- 指定 nightly Rustfmt、typos、Python 语法、严格 core 文档、两 crate 清单和 diff 检查通过；清单包含四个 installation_* 源文件，没有 Python 缓存。对应 `artifacts/install-plan-fmt.log`、`install-plan-typos.log`、`install-plan-doc.log`、`install-plan-package.txt`、`install-plan-desktop-package.txt`、`install-plan-diff-check.log`。本轮失败的私有原生探针目录在核对精确绝对路径后移除，记录为 `install-plan-probe-cleanup.log`。
- 范围限制：变更摘要覆盖 dpkg 版本／架构／动作及安装前状态，不固定归档内容，也不证明任意维护脚本行为。APT 下载及已有钩子可能先运行，不提供通用撤销、自动重试或服务器重启续跑。机制适用于内置安装和标准软件入口；普通自定义 Shell 沿用自身审核与退出结果，不自动获得内置安装证明。其他 Debian／Ubuntu 版本、架构、APT 配置和包管理器矩阵仍需验收；本轮不扩展 Oracle MySQL、S3 或真实 SSH＋sudo 矩阵。客户端无需常驻，没有新 Agent、定时调度或 Docker 构建，没有 Cargo／全局缓存／临时环境目录重定向。

## 按需帮助与字号统一（2026-11-15）

- 脚本／任务、备份、巡检、配置、参数及设置的说明收进相关 `?`，支持悬停、点击固定、移入阅读、键盘聚焦、外侧点击和 Esc。提示有可访问名称与描述，跟随视口定位；Esc 先关闭帮助并保留表单。自定义参数说明保持用户原文。实际预检查计划、范围、状态、错误和不可撤销操作的简短提示仍直接展示。
- 统一字号变量：正文 14px、标签 13px、辅助信息至少 12px、代码 13px、标题 16–20px；同步调整控件及文件列宽。`tools/help_readability_smoke.py` 的 5 组真实 WebView 检查通过，覆盖上述交互、页面可见文字最小字号、900×700 布局、中英文、深浅主题、用户说明不被翻译和已连接文件工作区。最终记录为 `artifacts/help-readability-ui/1794717459398448900/`，人工查看脚本、提示、窄窗口和中文文件页截图。
- 141 项单元测试通过（桌面 30、core 111，另 7 项显式忽略）；严格工作区 Clippy 和 Windows debug 构建通过。脚本 13、类型参数 11、安装 28、备份 12、巡检 17、配置 21、记录存储 27、设置结构 5，共 134 组业务界面回归通过。记录为 `artifacts/ui-readability-{unit,clippy,build}.log`、`ui-readability-regression-<模块>.log`。
- 文件上下文回归停在系统剪贴板访问：应用与独立 Python 进程的 `OpenClipboard` 均返回错误 5，未读取剪贴板内容。没有将这次文件回归或综合剪贴板检查记为通过；临时排查改动已撤回，剪贴板实现保持原样。验证限制不替代已有的导航问题跟踪，也不宣称已修复偶发页面切换。

## 真实 OpenSSH／密码 sudo 发布矩阵（2026-11-15）

- 在 Ubuntu 24.04 amd64、systemd 255、OpenSSH 9.6p1 的私有 mount namespace 中，通过真实桌面、公钥 SSH、SFTP 与 PAM 密码 sudo 完成 8 组验证。未安装系统 SSH 服务，未改动宿主账户、密码或 sudoers；APT 下载的包版本与 SHA-256 已和本机 APT 元数据核对，记录在 `artifacts/ssh-native-tools/provenance.json`。
- 验证普通账户无法读取 root 所有的现场文件；错误 sudo 密码拒绝发布；超过 4 KiB 的 Unicode／引号路径候选通过 SFTP 上传并预检查而不提前生效；桌面进程退出后服务器继续健康检查，重启后先显示未知再通过新 SSH 查询原任务；旧版本恢复、健康失败后的真实文件／Nginx 回退、预览后现场变化拒绝覆盖，以及凭据／正文不进入本地持久记录均通过。
- 最终结果为 `artifacts/native-ssh-ui/1794717463157532700/` 和 `artifacts/ui-readability-native-ssh.log`。退出后额外确认本次 7 个服务／任务均 inactive、私有目录已移除，宿主认证文件摘要不变；结果见该目录的 `ssh/post-cleanup.json`。
- 本次采用解包 Nginx 的绝对路径和自定义校验／重载／HTTP 健康命令，仅证明该实际链路。内置 Nginx 的 PATH／包配置、其他发行版和架构、其他 SSH 认证与 sudo 策略仍需分别验收；不新增客户端常驻、服务器 Agent 或定时任务。

## 任务状态与退出结果（2026-11-15）

- 最近批次改为带任务编号、前后服务状态／调用身份和原始退出字节的有界观测。运行中及停止中优先，退出 0 与服务失败冲突、服务状态不可用、结果损坏或查询期间状态变化时不确认成功；可读日志继续展示。实时观察及手动重连共用应用逻辑，安装／演练继续经过完整报告门槛。停止中历史在重启后变为未知，配置恢复入口保护全部活动状态。
- 独立账户记录与最近查询复用原始退出字节读取和 Rust 校验，记录行采用 v2；保存的 `record.json` 格式保持不变，旧元数据缺省任务可读。拒绝把 NUL、多余换行、非规范数字、越界数值或链接文件归一化成有效退出结果。读取不改变服务器任务文件、不部署执行器、不重新提交原任务。
- 严格工作区 Clippy、Windows debug 构建和 147 项单元测试通过（桌面 31、core 116；另 7 项显式忽略）。新增检查覆盖全服务状态／退出组合、状态及调用身份变化、损坏退出字节、弱／不完整协议、旧历史停止中归未知、状态应用及安装报告门槛。日志为 `artifacts/task-observation-{clippy,unit,build}.log`。
- 14 项真实 Linux 集成通过，包含新增 `task_observation_integration`，以及原 systemd 跨用户／操作锁、账户及配置记录、真实 APT／Nginx、restic、诊断、配置发布恢复、存储、清理、目录演练、PostgreSQL／MariaDB 备份及 PostgreSQL 演练。新增用真实运行服务提前写 0、`ExecStopPost=/bin/sleep 3` 保持停止中、`ExecStopPost=/bin/false` 制造主进程 0 而服务失败，证明均不会提前成功；实际文件检查覆盖 NUL、多换行、链接、FIFO 和 48 KiB 日志上限。管理器不可用通过私有 PATH 屏蔽查询模拟，未停用系统管理器。记录为 `task-observation-native-records.log`、`task-observation-native-*-serial.log` 及两项 `*-database.log`。
- 新 `tools/task_observation_smoke.py` 的 8 组真实 WebView／回环 SSH 检查通过，覆盖双目标独立完成、停止中持久化与重启、未知保留新日志、冲突／调用身份／字节校验、错误任务及旧协议拒绝、非零失败、独立记录一致、中英窄页及重启后停止中禁止配置恢复。产物为 `artifacts/task-observation-ui/1794721498618359600/`。配置预检查初次遇到夹具把普通口令回显加到结构化 JSON 前，调整夹具分发顺序后通过，口令校验仍保留；生产解析没有放宽。
- 21 个运维界面专项的最终复验通过，共 406 组，另有上述 8 组新增专项。真实 OpenSSH／SFTP／密码 sudo 的 8 组发布矩阵再次通过，确认新查询可由普通 SSH 用户读取原任务，并验证客户端退出后完成、重连查询、恢复、健康回退、过期预览拒绝及凭据／正文不落本地记录。产物为 `artifacts/native-ssh-ui/1794721588771862000/`，日志为 `task-observation-native-ssh.log`；私有服务、任务及目录退出后已另行核查，宿主认证文件摘要不变。按实际日志核对的汇总为 `artifacts/task-observation-validation-summary.json`。
- 扩展安装原生检查首轮发生 WSL 连接失败／`E_UNEXPECTED`；独立 WSL 查询恢复成功后串行复验通过，没有重启全局 WSL 服务。失败的私有安装目录核对包标识、路径与服务状态后清理，依据为 `task-observation-failed-fixture-cleanup.json`。
- 恢复演练界面首轮又出现意外进入配置页，失败保留在 `task-observation-regression-recovery_drills.log`；新增可选 `ARCNIX_TEST_TRACE_INPUT=1`，仅在测试夹具记录按钮输入、焦点和导航变化，不记录输入值。跟踪复验的 18 组通过，但未重现该跳转，没有修改生产路由或宣称修复。保留此 P0；其他发行版、架构与存量任务完整阶段证明仍需继续验收。

## 最近批次的多服务器结果查询（2026-11-15）

- 展开最近批次后可选择多个原目标一次读取，查询并发为 1–8；单目标入口复用同一身份核对、观测及脱敏。完整核对选中项后才发送请求；未提交、地址／账户改变、当前信任指纹不同或已在查询的目标禁用。每台失败单独回填，错误保留旧日志并清除未确认退出码；安装／演练证明门槛不变，不重复执行任务或持续监视。
- 新 `tools/batch_refresh_smoke.py` 的 7 组真实 WebView／四个回环 SSH 目标检查通过：默认选择、身份与提交范围保护、取消选择／取消清除凭据、实际最多三路与单路排队、认证失败／管理器不可用、重新认证恢复、切页后回填、中英窄页及可选帮助。服务器查询开始／结束审计证明实际并发，每台只记录一次原任务启动，本地仍只有原批次且无认证秘密。最终产物为 `artifacts/batch-refresh-ui/1794723899613821000/`；人工查看身份阻止及中文 900×700 表单，修正计数翻译并复验。
- 真实 OpenSSH／SFTP／密码 sudo 扩为 9 组，通过客户端退出后完成、重启单目标及批次查询、旧配置恢复、健康回退与过期预览拒绝。批次读取前后结果文件身份与 systemd 调用身份一致；该实际 SSH 场景为一台服务器，多服务器并发另由四个回环目标验证。产物为 `artifacts/native-ssh-ui/1794724087647633000/`，`ssh/post-cleanup.json` 独立核对私有目录已移除、7 个相关服务／任务均 inactive、宿主认证文件摘要不变。
- 发现更新安装的空闲判断未等待结果读取：旧构建在新场景中复现“查询期间允许重启”，加入查询集合判定后，`tools/update_smoke.py` 的 6 组通过，覆盖手动安装禁用、自动倒计时等待查询、原安装／回退及自动更新。仅替换产物目录内的测试副本。失败保留为 `artifacts/batch-refresh-update-before-idle-fix.log`，最终为 `batch-refresh-update.log` 和 `artifacts/update-ui/1794726077895825900/`。
- 最终 148 项单元测试通过（桌面 32、core 116；另 7 项显式忽略），严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、typos、Python 语法及 diff 检查通过。桌面 crate 清单包含新模块。相关脚本、状态观测、服务器记录、配置、演练、帮助、导航、新批次查询、真实 SSH 及更新共 10 个专项、120 组通过；按日志核对的汇总为 `artifacts/batch-refresh-validation-summary.json`。不将这次范围替代此前全部原生 Linux 或发行版矩阵。
- 导航另外用原恢复演练跟踪复验及连续 24 次提交／刷新／最近十批轮换检查，产物为 `artifacts/recovery-drills-ui/desktop-1794722587210826000/`、`artifacts/navigation-stress/1794722735208614300/`；均未复现已有偶发跳页。仍保留该 P0，不声称修复。查询期间有意切到备份页的检查只证明本次结果事件不主动导航。
- 一次新增回归因测试未再次填写密码而失败；修正为逐次认证并核对新日志后通过，没有放宽认证或结果断言。失败保留为 `batch-refresh-regression-batch_refresh-missing-query-credential.log`。本轮不新增服务器 Agent、客户端常驻调度、后台备份或 Docker 构建。

## 页面入口清理与导航来源（2026-11-15）

- 全部页面赋值统一到 `navigation::change_page`，补齐任务准备、服务器选择及更新通知离开页面时的临时访问清理；此前这些入口分别清理，备份访问存在遗漏。离开备份页清除内存认证、快照详情和未完成读取，不改变已捕获的任务请求。保留原页面选择和工作区进入逻辑，没有强制返回任务页。
- 可选 debug 跟踪记录导航来源、固定页面名、序号／时间和清理前后是否保留备份访问的布尔值；测试夹具另外记录按钮输入、IPC 激活节点、导航类变化，以及自动点击的开始／结束时间。输入值、普通字符键、选择器、IPC 正文及凭据均不记录，正式构建无 native 跟踪。
- 连续双目标测试在第 25 轮观察到一次从任务页进入配置页。`artifacts/task-navigation-ui/1794726539899216500/` 保存了完整指针按下、松开、点击、对应 IPC 节点和下一次 Rust 侧栏回调；114 次侧栏输入与 114 次回调逐项一致。`input-navigation-audit.json` 记录该次由真实 DOM 点击触发，任务刷新未直接发起导航；该输入来源及早先未跟踪事件仍不能据此确定，保留 P0，不宣称全部修复。该轮曾并行运行另一组界面测试，后续改为串行并补充自动点击调用区间。
- 第二次串行测试连续完成 50 轮，577 次点击均处于自动调用区间，之后 Python 读取正在替换的历史文件遇到 Windows 共享冲突而停止；不计作完整通过。失败日志为 `task-navigation-stress-history-sharing.log`，产物为 `artifacts/task-navigation-ui/1794726867055836200/`。读取改为有限重试可读 JSON，不修改应用持久化或放宽任务结果断言。更早一次新测试误用了 Settings 的类名，保留在 `task-navigation-stress-settings-selector.log`；修正为现有 `settings-link` 后继续。
- 新清理构建的恢复演练跟踪检查扩为 19 组，已通过实际“载入备份访问 → 审核恢复 → 任务准备切页”的清理前后布尔值核验，并继续完成目录／PostgreSQL 演练与客户端重启查询。生产读取、任务提交和恢复证明没有因该跟踪放宽。
- 新构建串行通过恢复演练 19、备份 12、数据库 17、快照文件 25、异机恢复 15、配置 21、配置记录 30、残留处置 20、脚本 13、批次查询 7、导航 6、任务协调 9、更新 6，共 13 个专项、200 组。原演练日志核对到 8 次载入访问的离开，全部清除，记录为 `artifacts/recovery-drills-ui/desktop-1794727225389759300/navigation-cleanup-audit.json`。最终专项日志为 `artifacts/navigation-final-<模块>.log`。
- 完整串行压力检查通过 64 轮、每轮两台回环服务器，轮换最近十批并保留原 SSH 工作区。291 次侧栏点击与 291 次 native 侧栏回调逐项相同，另记录 730 次 IPC 激活；所有点击均落在自动调用区间，没有观察到额外点击或非预期跳页。产物为 `artifacts/task-navigation-ui/1794727524873717200/`，人工查看最终文件工作区与双目标任务页截图；不将 64 轮混入上述 200 组计数。
- 最终 148 项单元测试（桌面 32、core 116；另 7 项显式忽略）、严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、typos、Python 语法及 diff 检查通过。桌面源包包含修改模块；按实际产物核对的汇总及构建摘要为 `artifacts/navigation-trace-validation-summary.json`。此轮使用回环 SSH 验证清理及导航流程，不冒充新增真实发行版／架构矩阵，也不新增客户端常驻、远程 Agent 或后台调度。

## 当前任务快照的读取时间（2026-11-15）

- 最近任务新增可选的 `result_read_at`，采用成功校验原任务观测后 SSH worker 的客户端接收时刻，UI 绘制和排队不补造时间。传输／认证／协议／任务编号失败保留旧日志及原时间；有效但状态未知的观测可以有读取时间，仍不证明任务完成。实时观察继续按内容去重，没有为时间新增心跳或写盘。
- 未提交的预检查阶段隐藏时间栏；有快照的历史显示绝对日期及当前时区，缺少字段的旧历史显示“读取时间未记录”。说明仅放在 `?` 中，重启保留原时间及原状态恢复规则。服务器协议、提交与安装／演练结果门槛不变。
- `tools/task_read_time_smoke.py` 的 6 组真实 WebView／双回环目标检查通过：预检查无伪造时间、实时接收及内容去重、未知状态的新快照、错误协议／其他任务响应拒绝、认证失败保留时间、旧历史及重启、手动查询更新时间、UTC／本地格式切换不改变保存值、中文 900×700 提示及不重复提交。产物为 `artifacts/task-read-time-ui/1794728721616776300/`，人工查看中文时间与帮助截图。初次检查发现预检查目录已存在时显示了无关时间栏，收起该阶段时间栏后复验通过，初次记录保留在 `task-read-time-ui-preflight-visibility.log`。
- 148 项单元测试通过（桌面 32、core 116；另 7 项显式忽略），扩展了历史时间保留／旧字段缺省，以及接收时刻应用、错误保留时间和安装证明门槛检查。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法、两 crate 源包清单及 diff 检查通过。记录为 `artifacts/task-read-time-{unit,clippy,build,fmt-check,doc,typos}.log` 和两项 `*-package.txt`。
- 另有任务观测 8、批次查询 7、脚本 13、安装 28、备份 12、恢复演练 18、配置 21、帮助 5、导航 6、真实 SSH 9，共 10 个专项、127 组通过；加上新增 6 组为 133 组。真实 SSH 继续验证客户端退出后由服务器完成、重启查询、单目标／批次读取不改变原任务结果与服务调用、配置恢复及健康失败回退。产物为 `artifacts/native-ssh-ui/1794728635776734300/`，`ssh/post-cleanup.json` 核对私有目录移除、7 个相关服务／任务均停止、宿主认证文件摘要不变。
- 汇总为 `artifacts/task-read-time-validation-summary.json`。此时间使用客户端时钟，不冒充服务器完成时间或持续健康确认；没有新增监视器、客户端常驻要求、远程 Agent 或调度，也没有扩展已声明之外的发行版／架构矩阵。

## 未提交任务的查询边界（2026-11-15）

- 旧构建用真实预检查流程复现：取消后两个目标均已保存计划目录，“查询批次结果”仍可用；基线失败保留在 `artifacts/unsubmitted-query-before-fix.log`。结果请求此前只以非空目录判断，缺少服务器结果时会把明确取消的目标改成未知。
- 新增 `tasks::Target::may_have_server_result`，排除 Checking、Ready、Queued 和明确 Cancelled；单目标按钮、批次选择及请求校验共用该提交边界。原账户、地址、主机信任检查保留，目录或读取时间不构成提交证明。重启后的 Queued／Submitting 仍按原规则转为 Unknown 并允许核验；不把未知情况误判为未提交。
- 新 `tools/unsubmitted_queries_smoke.py` 的 6 组真实 WebView／双回环目标检查通过：取消预检查及重启不发查询、单槽队列中已接收目标继续完成、未提交目标不启动、不查询、不改变取消状态、混合批次精确查询范围、旧排队／提交历史核验、缺失结果保持未知及中文窄窗口。每台查询与启动次数经过夹具审计；只启动原接受目标一次，没有重放。产物为 `artifacts/unsubmitted-queries-ui/1794729164457509100/`，人工查看中文 900×700 禁用原因与查询范围截图。
- 原批次查询检查强化为“取消目标仍含计划目录及已有时间字段”的情况，继续禁用。148 项单元测试通过（桌面 32、core 116；另 7 项显式忽略），扩展原状态／身份矩阵及历史恢复检查，覆盖取消状态即使含读取时间仍不可查询，以及未知状态不被阻止。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法、两 crate 清单及 diff 检查通过；记录为 `artifacts/unsubmitted-query-{unit,clippy,build,fmt-check,doc,typos}.log`。
- 另有批次查询 7、读取时间 6、任务观测 8、脚本 13、软件／运维入口 23、安装 28、协调 9、配置 21、演练 18、导航 6、真实 SSH 9，共 11 个专项、148 组通过；加上新增 6 组共 154 组。真实 SSH 仍验证客户端退出后完成、重启核验、单目标／批次读取不改变原结果、配置恢复、健康回退及过期预览拒绝。产物为 `artifacts/native-ssh-ui/1794729483611359300/`，`ssh/post-cleanup.json` 确认私有目录移除、7 个相关服务／任务停止及宿主认证文件摘要不变。
- 汇总为 `artifacts/unsubmitted-query-validation-summary.json`。取消和不确定历史的区分由回环流程及明确的历史状态验证，不冒充所有真实网络确认丢失场景；不增加服务器命令、重放、常驻或调度，也不扩展已声明之外的远程平台矩阵。

## 多项证书与配置诊断（2026-11-15）

- 同次按需任务支持多份 PEM 证书及 Nginx／OpenSSH 主配置，合计最多 20 项；新增列表默认空，兼容旧单项请求及 v1 报告。逐项显示保存路径、校验器、状态及有界明细，处理建议放入 `?`。表单拒绝重复、空证书、无效路径、超额及超长脚本；增删行使用稳定界面编号，任务只保存实际请求。
- 多项 v2 报告核对完整保存请求摘要、项目键及顺序；错误摘要、重复、乱序、缺项和混合版本不能显示为完整结论。每项最多 1024 字节，20 项最大报告小于任务日志尾部限额；旧版保持 4096 字节。短密码脱敏仍保留合法结构，不将非法行修补成有效报告。
- 150 项单元测试通过（桌面 32、core 118；另 7 项桌面测试显式忽略），新增请求边界、字面路径、清单绑定、版本兼容、脱敏及最大报告检查。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法及 diff 检查通过；日志为 `artifacts/multi-diagnostic-{unit,clippy,build,fmt-check,doc,typos}.log`。
- 新 `multi_diagnostics_smoke.py` 的 7 组真实桌面／双回环 SSH 检查通过：旧历史、增删与重复拦截、客户端退出后查询 22 条独立结果、三类异常报告和中文窄页；回环响应为夹具模拟，不冒充实际校验器。另有帮助与可读性 5、旧诊断 17、脚本 13、批次查询 7、任务观测 8、备份 12、配置 21、导航 6，共 9 个专项、96 组，均串行运行。产物为 `artifacts/multi-diagnostics-ui/1794732120931961300/` 及 `artifacts/multi-diagnostic-regression-*.log`。
- `diagnostics_integration` 与 `multi_diagnostics_integration` 共 2 项真实 WSL 集成检查通过。环境为 Ubuntu 24.04.1 LTS x86_64、Nginx 1.24.0、OpenSSH 9.6p1／OpenSSL 3.0.13；旧单项路径及基础工具继续通过。新增同一 systemd 任务实际检查正常／临期／过期／缺失证书、有效／无效 Nginx 和 OpenSSH 配置，共 8 项；提交进程退出后使用新进程查询原任务，核对准确状态、保存请求绑定、配置内容未改及没有运行中 PID。Nginx `-t` 可创建空 PID 文件，仍只作为工具行为处理，不解释成服务启动。
- 实际校验只使用私有证书与配置、项目内解包工具，不安装或重载系统服务；OpenSSH 需要的账户和运行目录在私有 mount namespace 内提供。测试核对宿主 passwd／group 摘要不变，自有 unit 为 inactive，私有根目录已移除。原始日志及工具版本为 `artifacts/multi-diagnostic-native{,-tools}.log`，清理审计收进汇总。
- 人工检查发现帮助圆圈采用绝对定位却没有相对按钮定位，滚动后会偏离问号。共享样式改为相对按钮居中，帮助专项增加可见圆圈中心位置检查；悬停、点击固定／关闭、键盘、窄页、主题和字号继续通过。已查看英文逐项结果及中文 900×700 表单／结果截图。
- 汇总为 `artifacts/multi-diagnostic-validation-summary.json`。没有新增持续监控、后台续期、自动修复或客户端常驻要求；本次不宣称证书链／域名／外部 TLS 验证、线上配置效果或其他发行版／架构矩阵已验收。

## 审计与导出存储预览（2026-11-15）

- Tasks → Cleanup reports 新增审计／导出存储视图，跨服务器查看实际分配块与逻辑大小，按最短时间及最近条数跨页选择最多 100 条，再生成只读保留预览。固定两种私有根，不接受任意服务器路径；预览显示精确对象和保护原因，不创建任务、删除文件、停止服务或设置周期维护。导出副本及关联处置证据保持保护。
- 候选必须为完整且停止的已完成清理报告，没有来源原位置／暂存残留，不被恢复记录引用，并符合保留条件与精确结构。活动、failed、未知服务、未完成／损坏记录、链接及不完整引用检查不会成为候选。扫描前后复查根、目录项、引用和文件身份；解析另行校验范围、策略、分页／选择顺序、封闭对象路径和总数。容量不完整时明确标注，不保证候选占用等于最终可释放空间。
- 152 项单元测试通过（桌面 32、core 120；另 7 项桌面测试显式忽略），新检查覆盖请求边界、错误范围／选择、活动／失败／未知候选、越界或重复对象及容量绑定。最终新增翻译后再次通过 32 项桌面测试。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法及 diff 检查通过；日志为 `artifacts/audit-storage-{unit,desktop-final,clippy,build,fmt-check,doc,typos,diff}.log`。
- 新 `audit_storage_smoke.py` 的 6 组真实桌面／双回环 SSH 检查通过：无单机选择的双目标库存、分页与跨页精确预览、引用／残留／活动／最近记录保护、异常响应和读取失败保留快照、不完整引用保护全部选择、非法策略拦截、编辑策略丢弃旧查询、导出副本保护、忘记／重新连接、离开页面拒绝晚到数据，以及中文 900×700 的内容／字号／溢出检查。回环数据是模拟报告，不能替代真实文件系统检查；没有创建本地任务历史或保存私有对象。
- 原清理报告 29、记录存储 27、记录清理 18、残留处置 20、帮助可读性 5、导航 6，共 6 个专项、105 组串行回归通过；加上新增 6 组，共 111 组。产物为 `artifacts/audit-storage-ui/1794734153875070200/` 和 `artifacts/audit-storage-regression-*.log`。人工查看英文精确预览及中文含导出保护结果的窄页截图；移除逐条重复的预览说明，预览显示于列表上方，帮助继续收在 `?` 中。
- `audit_storage_integration` 的 1 项真实 WSL Linux 检查通过：在独立 mount namespace 中把私有库挂到固定路径，核对 111 条报告的分页、完整旧记录候选、实际 active／failed systemd 服务、未完成／残留／引用／损坏／链接／硬链接／嵌套对象保护，以及 16 MiB 稀疏导出文件的逻辑与实际占用差异。读取前后比较全部私有库文件字节摘要、模式、属主、时间及路径；不执行保存的哨兵程序，数据保持不变。故意破坏一条引用后，原候选被全部保护。此测试使用专门生成的历史审计元数据，不冒充线上审计数据或实际清理业务验收。
- 真实检查的两个私有 unit 具有可核对的专用 Description 与有限运行时间；结束确认均无活动状态并移除私有根。早期夹具曾错误地把 systemd 的 StandardOutput 枚举当作文件路径，清理保护因此拒绝继续；已改为核对专用 Description，旧夹具在确认服务停止后精确移除，审计见 `artifacts/audit-storage-failed-fixture-cleanup.json`。最终原始记录为 `artifacts/audit-storage-native.log`，清理状态收进汇总。
- 汇总为 `artifacts/audit-storage-validation-summary.json`。本次新增的是按需库存及保留预览，审计记录删除执行器、来源指针协调、直接下载和业务原路径恢复仍需继续设计；不将预览当成删除授权，不新增客户端常驻、自动重试、周期维护或线上数据操作。

## 独立审核的审计记录清理（2026-11-15）

- 已核验的清理审计候选可进入单独的多服务器表单，固定原端点、账户及信任指纹，清除库存访问后重新认证；所选保护项被排除，未审核服务器不出现在可选目标中。预检查只上传并冻结程序、请求和 prepared 报告；显式执行后才逐项隔离、核验及移除。导出副本、残留来源、被引用记录、保留范围及新的审计清理依据均保持保护。
- `audit_cleanup` 复用从原记录清理提取的精确文件引擎，保留无覆盖移动、原 inode 验证、条目／身份复查、pending／removed 持久写入及空目录核查。执行前重新采样根、引用、保留、服务和对象；过期预览停止。审计登记锁与当前清理准备／残留处置登记互斥；已接受任务由 systemd 持有，不依赖客户端保持打开，不自动重放或续跑，不提供通用撤销及服务器重启恢复承诺。
- 私有源程序上传抽为 `private_program`，残留处置保留原协议，新增审计 staging 仅扩充固定命令白名单；4 MiB 上传、普通 stdin 限额、原账户权限及双摘要核验保留。新准备回执必须匹配原请求摘要、任务编号及候选数，错误回执不能进入 Ready。完整冻结请求留在 root 私有目录，客户端只保存通常的批次、身份、文件审核摘要和有界日志，不保存访问口令或对象身份字段。
- 审计存储新增 `/var/lib/arcnix/audit-cleanups` 结果范围，既可找到准备但未提交的记录，也可找到完成／失败／中断结果，并分别显示保存阶段与实际服务状态。这些清理依据不成为新候选；原任务及独立账户记录保留日志和私有报告路径，完整逐项私有报告交互仍待继续完善。
- 154 项单元测试通过（桌面 32、core 122；另 7 项桌面测试显式忽略），新增根／引用证明要求、保护范围、固定目标、actor／输入边界及准备回执绑定检查。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法和 diff 检查通过；日志为 `artifacts/audit-cleanup-{unit,clippy,build,fmt-check,doc,typos,diff}.log`。
- 新 `audit_cleanup_smoke.py` 的 6 组真实桌面／三回环 SSH 检查通过：候选／保护过滤、取消不产生任务、独立预检查失败、错误请求回执拒绝、精确文件审核、关闭客户端后查询原双目标任务、第三服务器与导出副本不变、独立发现 prepared／completed 清理依据、中文 900×700 实际结果及凭据边界。模拟 peer 只承担 UI／SSH 协议检查，不作为实际删除证明。产物为 `artifacts/audit-cleanup-ui/1794739247425036800/`；人工核对英文文件审核和中文实际结果截图，计划说明已精简，辅助解释位于 `?`。
- 原审计存储 6、清理报告 29、记录存储 27、记录清理 18、残留处置 20、脚本 13、任务观测 8、批次查询 7、帮助可读性 5、导航 6，共 10 个专项、139 组串行回归通过；加上新 6 组共 145 组。原只读预览仍不产生任务，旧清理与残留流程继续通过。
- `audit_cleanup_integration`、`audit_storage_integration`、`record_cleanup_integration` 共 3 项真实 WSL Linux 检查通过，原始最终日志为 `artifacts/audit-cleanup-native-final.log`。新执行器在私有挂载中完成冻结、提交进程退出后的真实文件移除，拒绝变更对象及新引用，保护最近记录、引用来源和导出字节；三次独立实际 SIGTERM 保留部分隔离区与 interrupted 报告，原任务不能重放，结果可独立发现。测试仅在自己的进程使用本地 C interposer 延迟 unlinkat，生产冻结程序字节不变；不是线上数据或任意故障矩阵。
- 原审计存储继续验证 111 条文件系统记录、稀疏导出、活动／失败服务和只读字节不变；原记录清理集成继续覆盖原账户与不同管理员的真实 staging、冻结、执行及残留导出／移除。新审计清理夹具结束核对 6 个自有 unit 无活动状态、cgroup 无进程，再移除私有根；清理审计收进汇总，不以服务名称或停止状态单独代替进程退出证明。
- 初期连续中断复现了“停止信号在写最终报告时再次抛出异常，旧报告停在 running”的问题；失败栈保留于 `artifacts/audit-cleanup-native.log`。新清理程序在第一次中断及失败收尾阶段屏蔽后续 TERM／INT／ALRM，保护短补偿和报告写入；后续三次连续中断在固定版及最终真实集成批次均通过。SIGKILL、存储故障及已冻结的旧程序不据此保证完整报告；保存阶段、实际服务与残留仍需核查。另一次与构建重叠的测试出现有限 unit 超时，最终构建、真实系统检查及 GUI 套件分别串行完成，不将该超时解释为业务数据损坏。
- 汇总为 `artifacts/audit-cleanup-validation-summary.json`。本次只移除经过审核的清理审计文件，不删除导出副本、来源指针或审计清理依据，也不恢复已删除业务文件；它们的保留、报告详情、直接下载及人工恢复仍需继续完善。没有新增客户端常驻、调度、在线服务器操作或自动重试。

## 审计清理逐项详情（2026-11-15）

- 审计清理结果列表新增按服务器独立的查看／刷新／关闭入口，不依赖本地最近批次。固定私有根中的请求和报告核对摘要、编号、来源及目录证明后，逐文件显示报告的移除确认、待确认或未记录，以及原位置／暂存位置当前的匹配、变化、缺失或不可核验状态。文件不存在不会自动解释为移除成功，保存阶段和实际服务分别显示；查看不执行冻结程序，也不重试清理。
- 查询保持只读、有限输出及有限时间，不递归扫描；前后检查文件、目录映射、根和服务状态。刷新失败保留原快照及时间，变化快照给出明确提示；关闭、忘记凭据、切换范围及页面导航后，迟到事件不能重新打开详情。凭据仅在内存，读取不产生本地任务历史。
- 156 项单元测试通过（桌面 32、core 124；另 7 项桌面测试显式忽略）。新增两组覆盖身份／固定路径／结构／状态相互约束、缺失与删除确认区分、待确认唯一性、准备／完成报告约束和不稳定快照校验。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法和 diff 检查通过，日志为 `artifacts/audit-details-{unit,check,clippy,build,fmt-check,doc,typos,diff}.log`。
- 新 `audit_details_smoke.py` 的 5 组桌面／双回环 SSH 检查通过：独立发现，无 SFTP 访问，逐文件待确认及残留区分，单服务器错误与旧快照保留，编号／路径／漏项拒绝，凭据脱敏，不稳定／prepared／completed 结果，关闭或忘记凭据及导航后的迟到结果拒绝，另一服务器详情不受影响。中文 900×700 实际详情、悬停／点击帮助、字号不少于 12 px、无页面横向溢出及无本地任务／口令持久化均通过；模拟 peer 不作为真实文件删除证明。人工核对英文及滚动后的中文完整详情截图，解释置于 `?`。最终产物为 `artifacts/audit-details-ui/1794740686482165100/`。
- 原审计存储 6、清理报告 29、审计清理 6、残留处置 20、帮助可读性 5、导航 6，共 72 组串行回归通过；加上新 5 组共 77 组。各专项退出状态及日志见 `artifacts/audit-details-regressions.json`。
- 扩展后的 `audit_cleanup_integration` 在真实 WSL Linux 私有挂载中通过：prepared 原文件仍匹配且没有执行确认，completed 两个来源有逐文件确认且原位置／暂存位置均缺失，过期执行 failed 保留已变化原文件；三次独立实际 SIGTERM 后详情证明原暂存 inode，区分已确认移除的报告、待确认请求和仍匹配的 runner，查看前后私有报告不变。将已完成任务的冻结程序改为抛错代码后仍可查询，证明读取不依赖其执行。测试结束核对 6 个自有 unit 不活动且 cgroup 无进程，再移除私有根；日志 `artifacts/audit-details-native.log`。没有操作线上服务器。
- 初次在语言切换及缩小窗口后，任务入口点击未使页面离开设置页，失败日志／截图保留于 `artifacts/audit-details-ui.log` 和 `artifacts/audit-details-ui/1794740416617008000/`。增加语言切换完成断言及输入跟踪后，本专项和原导航回归通过；没有据此推断原因或关闭已有偶发导航问题，也未加入强制跳页或自动点击重试。
- 汇总为 `artifacts/audit-details-validation-summary.json`。导出副本及审计清理依据的保留、来源指针协调、直接下载和明确范围的人工恢复仍待继续完善；本次只补充可核查详情，没有新增客户端常驻、周期调度或自动续跑。

## 语言切换后的导航点击丢失（2026-11-15）

- 使用真实桌面分开复验语言切换、窗口缩放、主题与缩放、语言与缩放四种过渡。修复前，单独语言切换及语言与缩放均复现紧接任务入口点击未生效；单独缩放和主题与缩放各 10 轮通过。对照日志为 `artifacts/navigation-input-experiments.json` 及其列出的日志，首次失败另存于 `artifacts/navigation-input-baseline.log`。
- 浏览器输入跟踪显示任务按钮收到 pointerdown／pointerup／click，并发出对应 IPC。临时 Wry 调试副本进一步确认消息到达原生回调和事件循环，但旧节点 `1.13.0` 已不在活动身份表；同一日志记录语言操作后，原应用整树销毁并以新 generation 重建。原始证据为 `artifacts/navigation-input-ui/1794741417666452800/desktop/desktop.log`。结合 `main.rs` 原先按 English 条件选择两个相同应用分支的实现，确认该重挂载使排队中的旧按钮事件失效。
- 改为只挂载一次应用，由现有文本绑定更新语言；保存设置成功后先更新本地化状态，再发布配置变更。没有放宽已销毁节点的校验、重投旧事件、强制跳页或自动补点。临时依赖副本只用于调试，最终使用正常依赖和 `--locked` 构建；Cargo.lock 与调试前逐字节相同。
- 新 `navigation_input_smoke.py` 在四种过渡中各通过 60 轮，共 240 轮、960 次导航点击及 960 次对应 Rust 回调。测试不等待语言文本完成后才点击任务入口，保留原复现顺序；每轮检查原应用、导航栏和全部导航按钮 DOM 实例仍连接，输入、IPC 及回调目标一致，没有额外来源点击。产物与结果见 `artifacts/navigation-input-fixed-results.json`，各原始日志为 `artifacts/navigation-input-fixed-{locale,viewport,theme_viewport,locale_viewport}.log`。
- 156 项单元测试通过（桌面 32、core 124；另 7 项桌面测试显式忽略），严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、typos、Python 语法及 diff 检查通过。日志为 `artifacts/navigation-input-{unit,clippy,build,fmt-check,typos,diff}.log`。
- 审计详情 5、帮助可读性 5、导航 6、工作区偏好 5、设置结构 5、脚本 13、配置 21、备份 12、诊断 17，共 9 个专项、89 组真实桌面／回环 SSH 检查串行通过。包括语言实时切换、中文深浅主题与窄窗口、活动服务器、实际回环传输及编辑、持久设置、任务提交／关闭后查询等旧流程；界面解释继续置于 `?`，没有新增页面说明文字。检查清单为 `artifacts/navigation-input-regressions.json`，人工核对中文深色帮助截图。
- 原 `task_navigation_smoke.py --rounds 24` 另通过 24 轮双目标运行任务与连续切页；任务执行完成，本地最近十批保留规则不变，110 次导航点击与 110 次 Rust 导航回调目标一致，没有动作之外的额外点击。完整日志为 `artifacts/navigation-input-task-churn.log`，产物 `artifacts/task-navigation-ui/1794741944230653300/`。本轮修改仅涉及客户端界面与设置更新顺序，没有新增后端执行路径，因此未重复无关的真实 Linux 删除测试。
- 本轮汇总为 `artifacts/navigation-input-validation-summary.json`，临时调试证据与正常依赖最终构建分开保留。
- 本次明确修复语言重挂载造成的点击丢失；早先意外进入备份页或配置页的其他历史现象，没有据此推断同一原因或宣称全部解决，仍保留输入及回调跟踪。没有新增服务器任务、客户端常驻或自动续跑。

## 残留导出的校验下载（2026-11-15）

- 清理报告的关联导出和独立残留导出库存新增下载 ZIP 入口，分别沿用所选服务器的固定端点、账户、主机指纹及内存管理员凭据；不要求选中／连接单机工作区或提供 SFTP。来源暂存区已移除后仍可下载已完成副本；活动、未完成、失败或无法确认停止状态的记录不能开始下载，也不会触发新的导出任务。
- 服务器从固定私有根按需读取请求、报告及清单，绑定原审核数据／元数据和完整目录条目，使用有界目录枚举；精确清单最多 2 MiB、100 个原目录及 900 个数据文件。ZIP 包含三份审计 JSON、数据、原属性 sidecar 和空目录，不运行保存的程序、不创建服务器临时 ZIP。逐文件核对 SHA-256、长度及身份，完成后再次检查库存和停止状态，才发送匹配精确紧凑请求的最终回执。旧来源不在场不影响副本校验。
- 客户端通过可取消 SSH 增量接收，不保留二进制 stdout 日志尾部；按审核大小限制实际输出，校验 ZIP 标识、完整字节／SHA-256、清单摘要、条目数和退出结果。只有通过全部核验才 fsync、关闭文件并以不覆盖操作发布目标；Windows 复用不带替换标志的原生 rename，不要求硬链接支持。不会自动解压、恢复原业务路径或应用原权限，ZIP 不保留稀疏布局。
- 关闭弹窗、忘记凭据或离开页面取消传输并拒绝迟到结果；正常失败／取消移除 .partial，强制关闭进程可能留下明确命名的未完成文件，不将它标为下载成功，服务器副本保持不变。重新认证后可以明确重新下载，不自动重放导出任务。下载会话／口令／冻结清单不写配置或任务历史，用户保存的 ZIP 按其选择包含审计依据。
- 159 项单元测试通过（桌面 32、core 127；另 7 项桌面测试显式忽略），新增三组覆盖根／编号／原数据与元数据配对、审计身份、字节及条目计数、精确请求回执／哈希／截断／重复回执、命令长度、取消及私有暂存清理。严格工作区 Clippy、Windows debug 构建、指定 nightly Rustfmt、严格 core 文档、typos、Python 语法与 diff 检查通过；日志为 `artifacts/export-download-{unit,check,clippy,build,fmt-check,doc,typos,diff}.log`。
- `export_download_smoke.py` 的 7 组真实桌面／双回环 SSH 检查通过：无本地历史的独立发现、源暂存区不在场、停止状态门槛、取消预览不发起传输、错误编号／来源／路径／元数据遗漏及认证错误拒绝、错误脱敏，两种认证目标的二进制数据／元数据／审计 ZIP，已有及传输期间新建同名目标保留。过期快照、错误哈希／请求回执、截断、缺失／重复回执、非零最终退出、非 ZIP 输出和超出审核大小均不发布最终文件。
- 本专项还覆盖收到实际数据后取消、删除私有暂存、迟到查询拒绝、强制关闭后重新认证下载、来源处置完成后的关联报告入口；中文 900×700 下悬停／点击帮助、字号至少 12 px、无横向溢出通过。服务器种子和导出数据保持不变，不产生任务历史或保存访问口令。模拟 peer 用于控制器、SSH 流及界面协议检查，不代替实际服务器读取证明；最终产物路径见 `artifacts/export-download-ui-final.log` 与汇总。人工核对中文弹窗截图，说明放在 `?`。
- 原清理报告 29、残留处置 20、审计存储 6、审计详情 5、审计清理 6、脚本 13、帮助可读性 5、导航 6，共 8 个专项、90 组串行回归通过；加上新 7 组共 97 组。旧私有上传、普通命令输出、服务器任务及只读访问流程继续通过，清单为 `artifacts/export-download-regressions.json`。
- 扩展 `record_cleanup_integration` 的真实 WSL Linux 检查通过，记录为 `artifacts/export-download-native-final.log`：生产读取器从真实完成导出生成 ZIP，逐项核对 ZIP CRC、文件字节／SHA-256、元数据、审计依据及最终回执；来源暂存移除后仍可再次读取。过期请求、超出审核数量的额外目录项，以及同长度损坏数据、已更新身份但保留原内容摘要的情况均返回错误且没有成功回执。原实际清理、不同管理员私有上传、残留导出及核验移除的检查同时通过。仅使用自有临时数据和唯一编号的夹具记录，没有访问线上服务器。
- 汇总为 `artifacts/export-download-validation-summary.json`。本地下载需要传输期间保持客户端打开，服务器副本完整性不依赖此连接；没有新增客户端常驻、服务器定时调度或自动续跑。ZIP64 写入路径已启用，本轮没有据此宣称验证所有大于 4 GiB、外置盘、掉电或存储故障组合。导出副本及审计清理依据的保留、来源指针协调和明确范围的人工恢复仍待继续完善。


## 独立服务器记录的批量停止（2026-11-15）

- Server records 支持跨页、跨服务器选择最多 100 个活动任务；独立审核重新认证原账户与主机信任，预览只读，明确显示无法验证的任务。只有已核验范围进入停止批次；同一 SSH 端点的不同账户分批操作。说明收进 `?`，仅保留部分变更风险提示；正文沿用 14 px、标签 13 px、辅助文字至少 12 px。
- 新控制任务不占用普通批次控制器，使用独立 Access::Control 绕过被终止任务持有的运维锁。提交前保存本地批次，启动前持久化原任务目录并核对授权；明确未提交失败／取消的目标清除查询目录。服务器冻结精确审核计划并独立执行、写入报告；客户端观察不是完成任务的必要条件，不因丢失回执而自动重放。
- 新增 `task_stops_smoke.py` 的 8 组真实桌面／合成回环 SSH 检查通过：两台服务器跨页选择、密码／密钥与 sudo、普通批次仍运行时独立提交、只读身份核验与拒绝行、局部权限失败后仅重新核验失败服务器、已接收后关闭整个客户端再查询原编号、删除本地历史后从服务器日志找回结果、提交前关闭取消、错误报告配合退出 0 保留未知、迟到响应不重开窗口、同端点不同账户的拒绝，以及清空选择。中文 900×780 的悬停／点击帮助、字号和无横向溢出均验证，配置及历史不保存测试凭据。产物为 `artifacts/task-stops-ui/1794750078127104200/`，日志为 `artifacts/task-stops-ui-final.log`。
- 原服务器记录 27、脚本 13、帮助可读性 5、导航 6、批次查询 7、未提交查询边界 6、导出下载 7，共 7 个相关界面专项另通过 71 组；连同新增 8 组，本轮共 79 组。串行日志为 `artifacts/task-stops-regression-*.log`；新增专项最终结果以 `task-stops-ui-final.log` 为准。
- 真实 WSL systemd 集成检查通过 `task_stops_integration`。只创建唯一编号的自有有限单位与临时记录，实际运行生产只读核验、私有上传／冻结程序和控制启动器。核对原包装、账户、调用身份、进程起始及 cgroup 身份，拒绝其他记录路径／用户的单位。持锁目标另包含忽略 SIGTERM 的子进程；原任务停止中且 cgroup 仍有进程时，控制任务保持运行，最终由 systemd 完成进程组终止后才给出停止证明。提交 shell 退出后控制程序继续完成；同名新调用不受旧计划控制，失败报告保留逐项原因，自然已结束行单列，冻结控制程序拒绝重放。新停止元数据能被实际记录库存识别，并明确保护核查依据。夹具清理先核对自有 Description 与空 cgroup，再移除自有路径。最终日志为 `artifacts/task-stops-native-final.log`。这里直接经 WSL 调用实际 Linux 程序，认证与桌面流程由回环夹具覆盖，不据此宣称真实 SSH／sudo 组合矩阵全部通过。
- 161 项单元测试通过（桌面 32、core 129；另 7 项桌面测试显式忽略），工作区测试另有 5 项无需外部夹具的检查通过。新增单测约束选中账户／编号、原包装证明、拒绝行、冻结计划摘要、结果绑定和缺失证明的成功降级。严格 Clippy、Windows debug 构建、core 严格文档构建、指定 nightly 格式检查、typos、Python 语法及 diff 检查通过；日志前缀为 `artifacts/task-stops-`，汇总为 `artifacts/task-stops-validation-summary.json`。
- 当前需要 Python 3.8+、systemd、支持 JSON 的 busctl 及 cgroup v2。停止不是业务回滚；原记录或运行身份变化、超时、服务器重启和强杀后不能虚构完成。不承诺与任意外部 root 写入者之间的原子条件停止，不自动续跑或恢复。私有控制审计继续保留，尚未接入自动清理。整个功能没有增加客户端常驻执行或定时调度。


## 诊断环境与离线报告导出（2026-11-15）

- Diagnostics 每批及 Server records 的诊断日志新增本地 HTML 导出。打开时固定保存快照，可选择服务器、明确选择是否加入明细，再预览及保存；后台任务后来的更新不改写已打开的报告。默认不含明细，脚本、执行预览及原始协议帧不进入文件。说明放在 `?`，保留正文 14 px 与辅助文字至少 12 px。
- 采集完成、执行状态、退出码与检查结论分别保留；未知／不完整报告不成为已确认检查结论，清单不符时不输出其检查内容。新任务把原诊断 Plan 保存到服务器的有界元数据，本地历史淘汰后仍可核验；旧记录没有原清单或版本时明确标为未核验／未记录。新增每页 100 条大元数据的边界测试：每条元数据合法且接近 8 KiB、编码后整页超过 1 MiB 时仍完整读取，页面与 worker 共用 2 MiB 上限；超限拒绝。
- 新版本数据组保存实际系统、内核／架构及所选工具版本输出，3–8 个有序有界字段。重复、缺少结尾、超限及清单不符均拒绝；脱敏不破坏协议字段，旧无版本报告保持兼容。界面通过服务器卡片 `?` 展示，导出保留服务器采样开始时间、已记录的客户端接收时间、报告捕获时间、采集器修订及客户端版本，不把这些时间当作原子快照或当前健康证明。
- `diagnostic_exports_smoke.py` 的 7 组真实桌面／合成回环 SSH 检查通过：两台目标与无 SFTP 账户、运行中固定快照后任务完成、筛选及明细选择、敏感值脱敏与 HTML 注入文本、预览后出现同名文件仍不覆盖、修改后重新预览、取消与非法文件名、删除本地历史后从服务器元数据恢复原清单并导出、旧范围／版本缺失、中文 900×780 帮助与字号及固定页脚。预览和导出不产生新服务器请求或任务历史。最终产物为 `artifacts/diagnostic-export-ui/1794754276059268000/`。
- 四种报告在自有无头浏览器中实际渲染，核对没有 script／image／iframe／外部样式／表单或事件属性，没有 HTTP 请求及横向溢出；另在浏览器网络离线状态复验均可打开。相关截图与 HTML 在上述产物中，离线复验记录为 `artifacts/diagnostic-export-offline.log`。文件采用 UTF-8、随附样式及 CSP，所有数据文本转义；先写完并同步私有临时文件，再不覆盖地发布，失败不残留本次临时文件。
- 原诊断 17、多项诊断 7、脚本 13、服务器记录 27、任务停止 8、批次查询 7、任务观测 8、帮助可读性 5、导航 6，另 9 个专项通过 98 组；连同新增 7 组，本轮共 105 组。日志为 `artifacts/diagnostic-export-regression-*.log`。最终元数据分页边界调整后再次复验新导出及服务器记录界面，其余相关回归保持已通过。
- 两项实际 Linux 集成通过 `diagnostics_integration` 与 `multi_diagnostics_integration`，直接在自有有限 WSL systemd 服务中运行生产采集器、实际 OpenSSL／Nginx／OpenSSH 等工具与私有配置，核对版本确实来自命令输出。保留证书正常／临近到期／已到期／缺失、TCP 监听、有效／无效 Nginx 和 OpenSSH、原清单绑定及提交 shell 退出后的独立完成。实际检查结论也生成 HTML，文件为 `artifacts/diagnostic-export-native/*.html`；所用字段含明确的本地 WSL 夹具标识，不冒充真实 SSH 认证。日志为 `artifacts/diagnostic-export-native-final.log`，私有 namespace、宿主账户摘要与自有 unit／目录的清理核验同时通过。
- 166 项单元测试通过（桌面 32、core 134；另 7 项桌面测试显式忽略），工作区测试另有 5 项无需外部夹具的检查通过。新增冻结报告、状态区分、原范围元数据、最大页面、完整数据组、脱敏、HTML 转义及同名竞态验证。严格 Clippy、Windows debug 构建、core 严格文档、指定 nightly 格式、typos、Python 语法及 diff 检查通过；汇总为 `artifacts/diagnostic-export-validation-summary.json`。
- 本轮不扩展为持续监控、自动修复、自动重试、外部 TLS／证书链校验或客户端常驻。版本命令失败／缺失／超时明确不可用，不推断程序包或当前 PID 1 的版本。诊断明细可能仍含业务敏感信息，加入后必须按实际预览自行审核；报告不是内容签名，也不承诺掉电及所有文件系统故障组合。服务日志的直接跳转与发行版／授权矩阵仍待继续完善。


## 诊断故障服务日志（2026-11-15）

- 最近诊断及独立服务器记录新增故障服务日志弹窗，完整报告中最多提取 32 个有效字面量服务名。只提供确实保存的故障服务，不推断配置路径对应的服务；缺少原清单或执行未知时仍明确标记采集未确认，不完整或范围不符不提供入口。账户、端点或可信指纹变化时禁止读取，结果显示前再次核对。
- 查询使用独立有限 SSH 连接，保留当前工作区。可选诊断开始前后各 15 分钟、最近一小时、今天或当前启动，及 100／200／500 条。默认普通账户，管理员读取需明确选择。服务器 timeout 20 秒、终止宽限 2 秒，SSH 流 35 秒，stdout／stderr 上限为 2 MiB／64 KiB。JSON 解析后脱敏，读取警告保留；失败标记同一查询的上次快照，改变查询清空旧数据，关闭后丢弃迟到结果。
- 新增 `diagnostic_logs_smoke.py` 的 7 组真实桌面／合成回环 SSH 检查全部通过：原密码及无 SFTP 账户、Beta 主工作区保持连接、普通／显式 sudo 读取、转义及实例名、时间／条数选择不自动发起查询、解码后脱敏、错误保留旧快照、缺少 journal／无效 JSON／超限／真正空结果及警告、关闭后清除凭据与丢弃迟到数据、删除本地历史后的独立服务器记录、账户／端点／指纹变化，以及未知旧结果／不完整报告。读取不创建托管任务或持久化凭据，不写回任务历史。产物为 `artifacts/diagnostic-log-ui/1794756499731511100/`。
- 原诊断 17、多项诊断 7、报告导出 7、服务器记录 27、帮助可读性 5、导航 6，另 6 个专项通过 69 组；连同新增 7 组，共 76 组。日志为 `artifacts/diagnostic-logs-regression-*.log`。中文 900×780 实测 `?` 悬停／点击、正文 14 px 与辅助文字至少 12 px、无横向溢出和可见页脚，已查看实际截图。
- 新增 1 项实际 Linux 集成直接经 WSL 在自有有限 systemd 单元运行生产日志命令，验证字面量 `\xHH` 与实例名不混入相似服务、UNIX 时间窗口、历史空区间不降级为当前日志、读取前后服务仍为 failed。只停止和复位描述／编号匹配的自有单元，并核对 cgroup 清空；日志为 `artifacts/diagnostic-logs-native.log`。该检查不代表真实 SSH／sudo 认证矩阵，认证交互由回环夹具覆盖。
- 168 项单元测试通过（桌面 32、core 136；另 7 项桌面测试显式忽略），另有 5 项无需外部夹具的正常检查通过。严格 Clippy、Windows debug 构建及 core 严格文档检查通过。指定 nightly 格式、typos、Python 语法及 diff 检查也均通过；记录使用 `artifacts/diagnostic-logs-` 前缀，汇总为 `artifacts/diagnostic-logs-validation-summary.json`。
- 日志窗口不绑定原 InvocationID，可能包含同名服务的后续运行；日志轮转／授权限制下空结果不证明没有故障。没有自动刷新、重启／修复、诊断重跑、日志源降级或客户端常驻任务，密码不写入配置／任务历史。


## 任务退出回执与运行实例绑定（2026-11-16）

- 实际 systemd 复现并修复旧数字结果误确认：任务仍运行时写入旧 `0`，强制终止并 reset-failed 后，原读取器会显示 Succeeded。红灯记录为 `artifacts/task-results-native-red.log`。新任务由服务器包装器保存绑定任务编号、实际 INVOCATION_ID 和规范退出值的完整回执，再整份发布为 result；缺少运行实例不执行负载，不补写或重放旧任务。
- 最近任务与独立账户记录共用回执解析和 service／exit 有界读取片段。独立记录第 3 版行核对读取前后状态及运行实例，仍能读取旧版行与原日志。旧数字值、错误任务、错误运行实例、缺少实例、截断／无效回执保持未知；明确缺失的已回收单元可使用完整回执，管理器不可用或错误／超时的部分输出不能替代缺失证明。读取上限为 192 字节，原 49,152 字节日志尾及 2 MiB 页面上限不变；100 条近 8 KiB 元数据的边界单测已使用新回执行复验。
- 清理预览／执行核对完整绑定及仍加载的运行实例，旧数字结果和不匹配回执受保护；不因文件存在或管理器重置而删除未知证据。停止控制器严格识别旧／新包装器后缀；正常 ActiveState／MainPID 变化会在已发送一次 stop 后继续有限核验，真正运行实例变化仍拒绝，不补发 stop。该差异由实际顽固子进程用例触发，已增加确定性状态变化／身份变化检查及完整原 cgroup 停止复验。
- `task_observation_smoke.py` 扩展至 10 组，通过旧 0／非 0、错误任务／运行实例回执、管理器回收后的完整回执、独立记录行及详情、未知结果保留日志、不改写本地历史等流程。产物为 `artifacts/task-observation-ui/1794758251983339800/`。停止界面测试的历史文件读取改为等待可读取的实际文件快照，避免 Windows 原子替换期间的暂时共享冲突；未增加点击重试或生产自动重试。
- 相关回归全部通过：脚本 13、安装 28、备份 12、配置 21、服务器记录 27、停止 8、清理 18、冲突协调 9、诊断 17、报告导出 7、诊断日志 7、帮助 5、导航 6，连同上述 10 组，共 188 组。保留中文窄窗口、悬停／点击 `?`、统一字号、凭据脱敏和跨页操作验证。日志为 `artifacts/task-results-regression-*.log`。
- 8 项实际 Linux 集成通过：新增绑定／旧结果中断 2 项，加上任务独立执行、状态观察、账户记录、存储预览、清理／残留处置及停止各 1 项。实际回执核对 systemd 的 INVOCATION_ID，测试错误任务／实例、旧／不完整字节、管理器回收、普通错误及超时返回部分 not-found 文本，两个读取入口一致保持未知。存储测试还核对有效元数据下的旧数字结果和错误任务回执受保护。Native 日志为 `artifacts/task-results-native-*.log`；认证交互仍由合成回环 SSH 覆盖，不据此扩大真实认证矩阵。
- 170 项单元测试通过（桌面 32、core 138；另 7 项桌面测试显式忽略），另 5 项无需外部夹具的正常检查通过。严格 Clippy、Windows debug 构建、core 严格文档及指定 nightly 格式检查通过；其余静态检查及汇总见 `artifacts/task-results-validation-summary.json`。
- 未改变全局运维锁的互斥范围。已保存在本地的历史状态不会被离线重核验，新查询才应用新规则；旧冻结服务器程序保留其版本。回执不是签名或应用健康证明，不承诺任意外部写入者与服务器故障下的原子证明。不增加客户端常驻、自动重放、强行确认或自动修复。


## 多服务器当前配置对比（2026-11-16）

- 配置中心增加独立的“对比当前文件”，在 2–32 个可信目标读取同一路径并选基准；内容一致性使用完整原始 SHA-256，UID／GID／mode 另行比较，BOM 与换行类型保留。完整且未隐藏的基准只填入空的 Custom 发布草稿，校验／重载／健康命令及目标仍需原流程审核；不自动发布、覆盖草稿或执行保存的命令。
- 新增 4 组 core 单测，覆盖范围／字段／完整字节验证、隐藏秘密前的摘要、空文件与缺失、元数据差异、大差异有界预览、字面路径与显式提权，以及短重复凭据隐藏时不重扫替换标记、不无界膨胀。全工作区共 174 项单元测试通过（桌面 32、core 142；桌面另 7 项显式忽略），另 5 项正常检查通过。
- 真实 WSL Linux 的读取集成 1 项通过：实际普通文件、符号及硬链接、字面量中文／引号／Shell 字符路径、完整 UTF-8／BOM／CRLF、普通 UID 的公开文件及权限拒绝、空／缺失／二进制／超限／FIFO／目录及指向伪文件路径。校验读前后 inode／mode／UID／GID／大小／ctime／mtime 不变（不把 atime 纳入无变化承诺），确定性改写返回 Changed，已删除的当前工作目录不影响生产读取器，测试后精确清除自有文件。日志为 `artifacts/configuration-compare-native-final.log`。这验证实际服务器读取器，不代表真实 SSH 认证矩阵。
- 新增 6 组桌面流程通过：普通密码／无 SFTP 及私钥独立读取保留 Beta 工作区；显式管理员读取、解码后隐藏已知秘密及权限差异；坏 JSON／错误摘要／范围／超限回复保留旧快照但不参与比较；停止丢弃迟到结果、草稿手交且保护原草稿；中文 900px 下悬停／点击帮助、最小字号及对话框底部；6 个实际回环 SSH 对端最多 4 个并行读取，停止后剩余队列不发请求。最终构建产物检查为 `artifacts/configuration-compare-ui/1794763893143913400/`，日志为 `artifacts/configuration-compare-ui-final.log`。
- 相关回归为目标选择 27、配置发布 21、服务器恢复记录 30、脚本 13、帮助与字号 5、更新 6，加上上述 6 组，共 108 项界面检查通过；另语言／窗口变化下连续 60 轮、240 个 rail 输入及对应原生回调通过。日志为 `artifacts/configuration-compare-regression-*.log`。全局正文 14px、标签与代码 13px、辅助信息至少 12px，默认说明及来源／摘要放入 `?`；最终窄窗口实际计算为 grid，差异宽度不少于 320px。
- 严格全工作区 Clippy、指定 nightly 格式检查、core 严格文档、Windows debug 构建及 Python 编译通过。链接器仍有已有 OpenSSL 静态库缺少 PDB 的 LNK4099 提示，不影响构建完成。汇总为 `artifacts/configuration-compare-validation-summary.json`。
- 快照和差异仅存页面内存，离开清除；显式复制的草稿遵循原编辑器生命周期。失败、停止或身份变化的旧结果不参与新比较；已知认证值隐藏不代表文件中其他秘密被识别。读取分时进行，文件上限 128 KiB、差异展示上限 32 KiB，不承诺跨服务器原子快照、当前持续一致、任意特权写入者的防篡改或通用恢复。无自动刷新、周期采集及客户端常驻依赖；多路径与逐目标模板覆盖仍属后续。


## 连接期间资源趋势（2026-11-16）

- 服务器工作区资源页新增 CPU／内存／网络曲线，1／5／15 分钟本地窗口、清除、暂停／继续、鼠标及键盘取样；没有新左侧全局入口或后台采集。CPU／内存固定 0–100%，网络为排除 lo 的接收／发送合计、共用刻度并区分实线／虚线。历史时间使用客户端单调时钟，速率沿用服务器计数，图表以最后接收读取为终点，初期仅展开已有时长。
- 新增 3 项 core 单测，验证首个样本缺失与真实零的区别、失败／页面边界／重启分段、900 秒及 1,024 次读取双重上限、窗口截取及无效值拒绝。全工作区 177 项单元测试通过（桌面 32、core 145，桌面另 7 项显式忽略），另 5 项正常检查通过。日志为 `artifacts/resource-trends-tests-final.log`。
- 新增 8 组真实桌面流程，通过回环 SSH 的可控资源响应验证原采样器、CPU 30%／接收 1 KiB/s／发送 2 KiB/s、首个样本未知和实际时间间距；故障／缺失内存或网络／计数重置；本地范围与清除不发请求；暂停仍可连续手动刷新计算速率；暂停和未暂停时离页均不续采，返回后分段；换服务器和客户端重启清空；不写配置点数据或创建服务器任务。另验证 Tab 可达、左右／Home／End／Esc、悬停／点击 `?`、中文、明暗主题、900px 无横向溢出、至少 12px 字号、SVG 不因父级 padding 整体缩小及页面重建不重复生成图表。
- 慢连接补查先复现并修复跨缺测时段连线的问题，红灯记录为 `artifacts/resource-trends-delay-red.log`。读取完成距前次完成超过两个配置采样间隔时，分段并重建速率基准。真实 SSH 夹具将读取延迟 7 秒，确认 3 秒间隔下只有一个采样请求，迟到点不会沿用旧 CPU／网络值或桥接曲线；后续及时样本恢复计算。暂停只停止自动采样，连续手动读取仍保留可用的计数对。最终 8 组产物为 `artifacts/resource-trends-ui/1794765924964592500/`。
- 最终相关回归：扩展桌面 27、帮助与可读性 5、导航 6，加上上述 8 组，共 46 项界面检查通过。覆盖真实 SFTP、无 SFTP 的 SSH、资源／日志、服务器任务软件安装、主题／语言、分栏与跨进程偏好。日志为 `artifacts/resource-trends-regression-*-final.log`。
- 严格 Clippy、指定 nightly 全工作区格式检查、core 严格文档、Windows debug 构建、typos、Python 编译、内置 JavaScript 语法及差异空白检查通过。汇总为 `artifacts/resource-trends-validation-summary.json`。链接仍只有既有 OpenSSL 静态库 PDB 提示；本轮未改服务端采样命令，不将合成资源响应扩大为新的真实 Linux／发行版／认证矩阵证明。
- 历史仅存当前连接内存，最多保留最后读取之前 15 分钟、1,024 次读取；暂停后可保留旧图，不代表仍在采集。切页停止新请求，已发请求可完成；断线、换连接及客户端退出后丢弃。未知或失败保留间断，不插入零；不提供关闭客户端期间的数据、持续告警、磁盘历史、单接口历史选择或历史导出。说明收进 `?`，数据／状态与实际错误仍直接可见。


## 配置正文参数与草稿升级保护（2026-11-16）

- 配置草稿增加明确启用的正文参数、默认值及逐服务器覆盖；未启用时普通双大括号保持原样。参数只替换一次，中文、引号、Shell 字符和插入值中的参数标记均作为字面内容；文件路径、校验、重载及健康条件仍由同批目标共用。最多 32 个参数、每值 4 KiB 单行、每份候选 128 KiB，所有目标在连接前完成渲染校验。
- 预检查冻结每台服务器的完整候选与摘要，沿用原私有上传、逐目标差异审核和服务器独立执行器；后续编辑不改写已冻结批次。运行表单与编辑器默认值分离，取消或取消选择清除对应运行值，筛选保留已选覆盖。成功开始预检查后释放运行表单值；无效表单保留可编辑内容。
- 本地历史只增加逐目标公开 Record／Spec，不保存配置原文或参数值；恢复按原目标选择自己的摘要，映射缺失或损坏不能回退到其他目标的代表候选。旧共享配置记录保持兼容。编辑器正文、默认值及非默认设置仅存内存，手动重启和自动升级倒计时均等待处理草稿；更新页可返回草稿，丢弃需明确确认，取消保留输入。丢弃不清除已提交任务及服务器版本。
- 新增 4 项 core 单测，覆盖语法、字面替换、UTF-8／换行、精确参数范围、输出上限、不回显值、逐目标候选与冻结范围、历史映射／旧记录／损坏映射。全工作区 181 项单元测试通过（桌面 32、core 149，桌面另 7 项显式忽略），另 5 项无需外部夹具的正常检查通过。日志为 `artifacts/configuration-parameters-tests-final.log`。
- 新增 `configuration_parameters_smoke.py` 的 6 组真实桌面／双回环 SSH 流程通过：普通大括号透传、默认值与覆盖继承／重置、筛选／隐藏目标／取消选择／取消表单、各目标实际上传内容与摘要／差异及冻结后编辑、重启后按 Beta 原摘要恢复、非法参数与超限在本地拒绝且不暂存或写历史。严格恢复夹具核对原 Spec，避免用宽松替身掩盖目标错配；配置文件及历史不含测试参数值。最终产物为 `artifacts/configuration-parameters-ui/1794771172583819500/`。
- 更新专项扩展至 7 组，实际验证草稿阻止手动重启与自动倒计时、返回编辑保留内容、取消丢弃仍等待、明确丢弃后恢复安装；同时保留下载验证、错误版本拒绝、替换成功及失败回滚。产物为 `artifacts/update-ui/1794771068462832200/`，日志为 `artifacts/configuration-parameters-updates-final.log`。
- 相关界面回归通过：目标选择 27、配置发布 21、服务器配置记录 30、当前配置对比 6、脚本 13、脚本参数 11、安装 28、备份 12、诊断 17、帮助与可读性 5、导航 6，加上新参数 6 和更新 7，本轮共 189 组。草稿保护合入最终构建后复验参数、配置发布、配置对比、帮助、导航及更新；其余保留本轮相关回归。中英文、明暗主题、900px 窄窗口、悬停／点击 `?`、至少 12px 字号、主题化代码输入和可见页脚通过，已查看实际参数及升级截图。辅助说明放入 `?`。
- 真实 WSL Linux／Nginx／systemd 集成 1 项通过：实际发布候选由生产参数渲染器生成，核对 UTF-8 正文，再运行原私有发布、恢复、健康失败回退及并发文件变更检查；日志为 `artifacts/configuration-parameters-native.log`。这是实际服务器执行器检查，双目标内容分发与认证交互由回环 SSH 夹具覆盖，不扩大为真实多服务器或发行版／授权矩阵证明。
- 严格工作区 Clippy、指定 nightly 全工作区格式检查、core 严格文档、Windows debug 构建、typos、Python 编译和 diff 检查通过。链接保留既有 OpenSSL 静态库缺少 PDB 的提示。汇总为 `artifacts/configuration-parameters-validation-summary.json`。
- 当前提供内存发布草稿的正文参数，不是持久配置模板库；不参数化路径或命令，不自动做应用语法转义、多文件发布、通用回滚或跨服务器原子发布。候选需按目标审核并通过原生校验。已接收的任务只依赖服务器上的完整文件和工具，不增加桌面常驻、自动续跑或后台保存敏感草稿。


## 已删除文件占用排查（2026-11-16）

- 磁盘／inode 诊断及独立服务器记录增加当前占用排查入口，通过独立 SSH 读取可见进程持有的已删除普通文件。按设备与 inode 去重，分别显示逻辑大小、已分配块及最多八个观察到的持有者，按已分配块展示前 20 项。原账户、端点和可信指纹在读取前后核对，当前工作区保持连接；不读取文件正文、不截断文件、不发送进程信号，不创建服务器任务或写回原诊断结果。
- 生产 Python 读取器固定 `/proc`，目录描述符锚定进程／fd 目录，用 `O_PATH` 固定文件后核对普通文件及零链接数，再读取该固定描述符的名称。进程名称、开始时钟滴答、UID 及 fd 有界读取，不采集命令行或环境。最多 12 秒、10,000 个进程、200,000 个描述符、2,048 个不同文件；服务器外层 15 秒及 2 秒终止宽限，SSH 流 35 秒，响应 256 KiB、stderr 4 KiB。
- 权限不足、进程变化、不可读描述符和扫描上限分别保留，空观察不作为全服务器没有占用的证明。共享块、内存文件、稀疏布局、持续写入和不可见进程影响解释；仅映射持有而没有打开描述符的文件不在当前范围。已分配块不是可释放空间，读取结果不提供终止／恢复／清理操作。说明放入悬停／点击 `?`，保留正文 14px、代码及标签 13px、辅助文字至少 12px。
- 新增 3 项 core 单测，覆盖元数据／汇总、持有者身份、解码后有界脱敏、未知版本／字段、错误／缺失行、重复持有者、控制字符、超限、真实空结果及显式管理员／固定命令时限。全工作区 184 项单元测试通过（桌面 32、core 152，桌面另 7 项显式忽略），另 5 项无需外部夹具的正常检查通过。日志为 `artifacts/deleted-files-tests.log`。
- 新增 1 项真实 WSL Linux 集成，在自有私有 mount／PID namespace 内执行生产命令。实际验证双描述符同 inode 去重、中文／引号／换行／Shell 字符路径、稀疏文件；仍有硬链接、仅名称以 deleted 结尾的现存文件、管道／socket 和已关闭 fd 的内存映射不误纳入。读取前后原文件时间、大小及共享偏移保持不变。25 个新增文件验证前 20 项与全量汇总，12 个引用只展示八个持有者；普通 UID 1001 受到权限限制并明确计数。同一生产扫描器另以测试缩小的描述符上限验证部分结果标记。所有自有描述符和临时文件由夹具收尾，不改变宿主服务或账户。日志为 `artifacts/deleted-files-native-final.log`，不据此宣称真实 SSH／sudo 或发行版矩阵全部通过。
- 新增 `deleted_files_smoke.py` 的 8 组真实桌面／双合成回环 SSH 检查通过：Alpha 密码／无 SFTP 读取保留 Beta 连接、默认普通账户、显式 sudo 与解码后凭据隐藏、权限／扫描上限及真实空观察、25 项前 20 个长名称与可见页脚、同 inode 行的后续值更新。错误 JSON／版本／缺失行／超限／非零退出及非空 stderr 保留并标明旧快照；不把错误降为空成功。账户、端点、指纹和原检查范围变化受到保护，不完整诊断不提供入口。
- 同一专项实际确认关闭待完成读取会关闭其 SSH 通道，重开没有凭据或旧结果，读取不创建托管任务、不持久化观察内容、不改变原任务历史；本地历史删除后可从服务器记录再次进入。中英文、明暗主题、中文 900×780 下悬停／点击帮助、至少 12px 字号、无横向溢出及可见页脚通过，已查看实际明暗截图。最终产物为 `artifacts/deleted-files-ui/1794772993384946900/`，日志为 `artifacts/deleted-files-ui-final.log`。
- 相关回归全部通过：原诊断 17、多项诊断 7、报告导出 7、故障日志 7、服务器记录 27、脚本 13、帮助及可读性 5、导航 6、更新 7，加上上述 8 组，共 104 组。日志为 `artifacts/deleted-files-regression-*.log`。严格全工作区 Clippy、core 严格文档、指定 nightly 全工作区格式、Windows debug 构建、typos、Python 编译和 diff 检查通过；链接仍保留既有 OpenSSL 静态库 PDB 提示。汇总为 `artifacts/deleted-files-validation-summary.json`。
- 查询只在用户明确刷新时执行，关闭后取消并拒绝迟到结果，服务器命令自身有时限，不引入客户端常驻或自动刷新。快照仅在弹窗内存保留，不是进程操作授权或持续监控；服务器缺少 Python、进程不可见或权限受限时仍需按实际错误处理，不自动安装工具或提权。


## 服务器环境与标签（2026-11-16）

- 服务器编辑器新增独立分类页，支持可选环境、最多 16 个标签，每项最多 32 个 Unicode 字符／128 字节；编辑器修剪、按大小写无关去重并支持中文逗号。配置 schema 保持 1，缺省及空字段不新增序列化内容；完整验证拒绝控制字符、超限及重复值，非法保存不覆盖旧配置。分类只是本地备注，不改变 SSH 权限、主机信任或服务器设置。
- 服务器树与共享目标搜索按多词 AND 包含分组、环境和标签；统一任务表单及当前配置对比增加完整值环境／单个标签筛选，与分组及搜索共同生效。筛选保留选择、凭据与逐目标配置参数，审核已选清除所有筛选，有隐藏目标时原守卫仍阻止继续。信任、重复端点、32 台上限及 Template::allows_target 的原作用范围保持；标签不能扩大备份策略或已经审核的目标集合。
- 仅改分类时，合并到保存时的当前 profile，不断开 SSH、不清理传输、不覆盖期间更新的浏览位置；其他编辑沿用原保存与断开规则。目标提示按用户原文展示，单个中文标签不会被界面翻译成另一值。服务器树只补充短环境行，完整分类与说明放入 `?`；正文 14px、标签 13px、辅助文字至少 12px。新增资料使请求枚举超过静态检查阈值，Probe 已改为 Box 传递并在 worker 分支解包，所有构造点更新，连接流程保持一致。
- 新增 3 项单测覆盖标签解析／边界、旧 profile 默认及序列化兼容、只改标签与连接身份变更的区别、组合完整值筛选、隐藏选择及信任／策略范围。扩展现有配置存储测试，验证分类实际落盘／重载、非法重复标签不改旧文件。全工作区 187 项单元测试通过（桌面 32、core 155，桌面另 7 项显式忽略），另 5 项正常检查通过；日志为 `artifacts/server-labels-tests.log`。
- 新增 `server_labels_smoke.py` 的 8 组真实桌面／三个回环 SSH 对端检查通过：旧配置编辑、非法输入本地拒绝、CSV 规范化、取消与重启／清空、多词及 Unicode 搜索、完整值匹配区别于子串、未信任目标保持禁用、隐藏选择拒绝及审核后独立凭据保留、键盘访问、原文标签的悬停／点击帮助。Alpha／Beta 已预检查后，再把 Gamma 改为匹配原分类，也只提交原 Alpha／Beta；备份策略仍排除 Gamma。当前配置对比复用筛选，拒绝隐藏选择时不发查询，审核后实际只读两个目标。
- 专项在真实 SSH／SFTP 协议的限速 16 MiB 上传仍进行时保存分类，核对任务继续完成、原目标路径和全文件 SHA-256、连接状态及认证审计不变；记忆目录保持。传输和 SSH 由本机受控对端提供，不等同于线上主机或发行版矩阵。Peer 辅助类只新增可选 io_delay 参数，默认仍为零，不影响正常对端。没有为本地分类增加新的 Linux 执行器或后台采集测试。
- 最终新增专项产物为 `artifacts/server-labels-ui/1794774901724074400/`，日志为 `artifacts/server-labels-regression-server_labels.log`。实际核对英文窄表单及中文深色编辑器截图；900px 下无横向溢出、可见页脚、悬停／点击 `?` 及最小字号通过。
- 相关回归通过：完整桌面 27（含软件、资源、日志、SFTP、RSA 兼容及隔离运行时）、目标选择 27、配置参数 6、当前配置对比 6、工作区偏好 5、脚本 13、备份 12、帮助可读性 5、导航 6、更新 7。连同上述新增 8 组，共 122 组；另服务器树专项独立通过原生拖动／重载、分组操作、折叠／搜索、键盘、菜单与主题边界，不把其未编号断言混入组数。日志为 `artifacts/server-labels-regression-*.log`。
- 严格全工作区 Clippy、指定 nightly 格式、core 严格文档、Windows debug 构建、typos、Python 编译及 diff 检查通过；链接仍有既有 OpenSSL 静态库 PDB 提示。汇总为 `artifacts/server-labels-validation-summary.json`。标签不自动导入服务器实际状态、授予权限、发起安装或重新匹配已提交任务；本轮没有增加新的全局侧栏入口、客户端常驻、调度或服务器后台工具。


## Servers 本地记录总览（2026-11-16）

- Servers 内增加总览及侧栏入口，空配置保留原添加提示。纯本地汇总按当前 ID、账户／端点、规范化主机指纹及原检查范围核对最近诊断，较新已提交但待确认的任务不回退到旧成功；未提交的预检查不遮盖原报告。磁盘／inode 与故障服务缺失显示 `—`，采样时间与读取时间分开使用，1／7／30 天仅为本地核验标准。问题、未知、不完整、过期和未来时间有独立状态，不输出当前健康保证。
- 备份覆盖遍历全部已分配策略，核对当前版本、仓库及执行账户身份；旧成功与较新失败分别保留。备份、仓库检查和恢复演练不互相替代，跨服务器演练核对明确来源和当前执行端。全局主导航顺序不变，单机操作和连接控制留在具体工作区。
- 共享搜索及完整值标签筛选保留跨页选择，每页最多 50 行，隐藏数量包含其他页；匹配全选超过 32 台或包含重复端点时原选择保持。审核诊断打开完整已选目标，再复用现有预检查和服务器任务。打开／刷新总览、筛选与审核均不发 SSH，服务器查询须主动打开任务。原工作区终端保持，SFTP 仍可使用；隐藏资源页停止周期读取，返回后的基线留出间隔。
- 新增 5 项纯逻辑测试涵盖 IPv6 信任、身份／指纹变化、未提交与新待确认任务、完整范围与 malformed 报告、未知数值、采样时限、未来时间、全部策略覆盖、较新失败、版本变化、检查不算备份，以及跨执行端演练绑定。全工作区 192 项单元测试通过（桌面 32、core 160，桌面另 7 项显式忽略），另 5 项正常检查通过；真实 Linux 集成仍按显式入口单独运行，本轮未新增运行它们。日志为 `artifacts/server-inventory-tests.log`。
- 新增 `server_inventory_smoke.py` 的 8 组真实桌面／回环 SSH 检查通过：合成保存记录的混合状态及策略覆盖、指纹漂移、来源任务入口、无网络刷新／审核、完整值组合筛选、隐藏选择、原子拒绝重复端点／超限、键盘复选、55 台分页与长中英文名称、32 台审核，以及空配置。实际默认四项诊断只提交 Alpha／Beta，桌面结束后受控服务器继续，重开后主动查询才更新结果；核对两端仅各一次提交。连接中的终端内容和认证审计保持，随后完成真实 SFTP 上传并核对内容；资源请求停止／返回间隔通过。
- 界面专项实际检查英文浅色、中文深色 900px 截图、字体最小 12px、主题文本颜色及 18px 复选框（允许布局浮点舍入）。总览说明和逐策略详情放入悬停／点击 `?`，提示框保留在窗口内，无横向溢出。最终产物为 `artifacts/server-inventory-ui/1794777835182148100/`。这些对端提供真实 SSH／SFTP 协议与合成运维输出，不等同于线上机器或发行版矩阵。
- 合成诊断对端现从 v1／v2 脚本提取实际选项，最近查询及独立记录使用对应版本／范围，保留旧版本的受控混合严重度；修复旧固定六项响应与默认四项不符的问题。导航和服务器树的断开／删除选择测试按新默认总览更新，空配置仍断言原添加提示，保留原单机边界验证。
- 相关回归全部通过：导航 6、帮助 5、分类标签 8、资源趋势 8、诊断 17、多文件诊断 7、离线诊断导出 7、诊断服务日志 7、独立任务记录 27、备份 12、目标选择 27、完整桌面 27（含软件、资源、日志、SFTP、RSA 兼容及隔离运行时）。连同新增 8 组，共 166 组；另服务器树专项独立通过拖动、分组、重载、搜索、键盘、菜单及主题检查。日志为 `artifacts/server-inventory-regression-*.log`。
- 严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译及 diff 检查通过。链接仍有既有 OpenSSL 静态库 PDB 提示。汇总为 `artifacts/server-inventory-validation-summary.json`。本轮不增加客户端调度、后台探测、服务器守护进程或备份自动执行；本地最近十批以外的诊断仍通过独立服务器记录主动查看。


## 从服务器核验直接 TLS（2026-11-16）

- 按需诊断增加直接 TLS 端点：连接主机、端口及可选验证名称分开，连接主机末尾点号保留 DNS 搜索语义，DNS 验证名规范化后用于 SNI，IP 身份显式不发送 SNI。支持规范化 DNS、IPv4／IPv6；拒绝 URL、混入端口、控制字符、无效／歧义数字地址及规范化重复范围。最多 16 个端点，与原检查共用总计 20 项和 8192 字节请求上限。表单行按稳定 ID 编辑，删除后保留其他输入；说明收进悬停／点击 `?`。
- 服务器已有 OpenSSL 必须通过链、名称／IP、有效期核验并明确返回成功，再读取同次连接叶证书的主体、颁发者、日期及 SHA-256 指纹和检查到期阈值。可识别的证书核验错误记为严重；网络、工具、非 TLS、超时、解析或缺失正向确认保留无法判断。每次连接限时 15 秒、kill-after 2 秒；X.509 各步骤另有 15 秒时限。子进程只有成功设置限制才运行，stdout／stderr 每文件不超过 128 KiB，v2 每条明细最多 1024 字节。正常退出清理私有 TLS／叶证书临时文件，客户端关闭不承担清理或任务继续执行的职责。
- TLS 项使用 `tls:N` v2 键、完整有序范围和 Plan 摘要，保留原地址／验证身份及 OpenSSL 环境版本；不接受旧 v1 TLS 键。空 `tls` 不序列化，单测逐字比较旧 v2 JSON，保证原摘要不变。原本地 PEM／Nginx／OpenSSH 行为继续沿用。最近批次、独立服务器元数据、离线 HTML 和 Servers 总览识别新项，不能将 TLS 无法判断隐藏在正常磁盘数字之下。
- 新增 3 项单元测试，覆盖输入／参数注入边界、DNS／IP 规范化、IPv6 连接格式、SNI／IP 模式、规范化重复、冻结身份／端口、报告版本／范围及旧 v2 字节兼容，核对原脚本大小限制。全工作区 195 项单元测试通过（桌面 32、core 163，桌面另 7 项显式忽略），另 5 项正常检查通过；日志为 `artifacts/tls-diagnostics-tests.log`。
- `tls_diagnostics_integration` 显式运行通过：Ubuntu WSL、真实 OpenSSL 3.0.13、Python SSL loopback 对端和 systemd 任务，11 项真实网络／证书场景涵盖 IPv4、IPv6、独立 hosts 挂载中的绝对域名、无 IP SNI、DNS SNI 切换到另一叶证书、错误域名、临近到期、过期、不可信链、非 TLS 服务、拒绝连接及停滞超时（部分样本同时覆盖地址／身份）。附加 1 项替代工具持续输出场景，实际捕获 EFBIG／文件大小不超过 131072 字节，仍生成完整且无法判断的报告。停滞读总耗时断言在 14–25 秒，防止即时失败冒充时限验证。提交后独立进程查询原任务；核对元数据和指纹、实际 SNI 记录、私有临时目录清理。最后停止自有服务、移除私有目录并核对主机账户、hosts 及 CA bundle 哈希保持，未安装工具或改系统信任。补充域名场景曾观察到异常 EOF，采集保留未知；现测试 hosts 覆盖带点别名并在连接前核对回环解析，对端完成 TLS 关闭，不放宽 OpenSSL 成功条件。初始失败依据保留为 `artifacts/tls-diagnostics-native-dns-eof.log`。最终日志为 `artifacts/tls-diagnostics-native.log`，原报告保存在 `artifacts/tls-diagnostics-native/`。
- 新增 `tls_diagnostics_smoke.py` 的 7 组真实桌面／回环 SSH 协议检查通过：输入及重复范围本地拒绝、不联网审核、固定双目标预检查、客户端关闭后查询、各 TLS 项不同严重度、总览未知项、原名称绑定、离线导出不发请求、移除本地历史后独立记录仍能核验及导出。两端仅各提交一次，报告默认不含原脚本或原始细节。改变保存的验证名称后，两目标均拒绝显示旧已核验发现。最终产物为 `artifacts/tls-diagnostics-ui/1794780677117148400/`。此对端的运维输出为合成数据，真实密码学和超时依据来自上述 WSL 专项。
- 实际检查中英文、深浅主题 900px 截图，确认完整 TLS 字段、至少 12px 字号、键盘顺序、提示边界、无横向溢出和语言切换保留草稿。补充排版截图为 `artifacts/tls-diagnostics-layout/1794779954201049100/`。
- 相关回归通过：原诊断 17、多文件诊断 7、诊断导出 7、服务日志 7、独立任务记录 27、Servers 总览 8、帮助 5、导航 6。连同新增 7 组，共 91 组；日志为 `artifacts/tls-diagnostics-regression-*.log`。严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译和 diff 检查通过；链接保留既有 OpenSSL 静态库 PDB 提示。汇总为 `artifacts/tls-diagnostics-validation-summary.json`。
- 结果是所选服务器、该时刻、该路由和 CA 库的单次直接 TLS 视角，不能替代 HTTP 健康、浏览器、所有 DNS／负载均衡节点或吊销核验；不提供 STARTTLS、客户端证书、自动续期或常驻客户端采集。实际环境矩阵为本次 Ubuntu／OpenSSL 3.0.13，不将受控桌面结果扩展为其他发行版、OpenSSL 实现或 CA 配置已全部验证。

## 客户端配置备份与恢复（2026-11-16）

- 设置常规页增加私密配置 JSON 快照及审核恢复，不增加全局导航或后台任务。复用 schema 1、模型验证和普通原子保存；保留服务器分类、脚本与类型参数／预设、策略及历史确认、偏好和路径引用，不采集引用的密钥正文、会话密码、任务日志或发布草稿。导出冻结审核快照，SHA-256 核对实际文件，现有目标文件不覆盖。恢复候选忽略导入信任，保留本机当前缓存；历史确认沿用原采样时间，不形成新结果。
- 新增 6 项核心测试通过：完整模型往返及不读取密钥、冻结导出／同名竞争／保留设备名／临时清理、源／内存／磁盘变化拒绝、原信任与逐字节原文件保持、损坏文件／缺失文件和目录恢复，以及实际 Windows 文件共享锁阻止替换。边界测试实际写入并重载完整 4 MiB 文件，再增加一字节验证拒绝并保留旧文件；另验证旧 4 MiB 加换行的损坏文件仍可完整保存。源与候选上限 4 MiB，保留原文件上限 16 MiB。全工作区通过 201 项单元测试（桌面 32、core 169，桌面另 7 项显式忽略）及 5 项正常检查；需要显式环境的 Linux 集成仍独立执行，本轮没有新增运行。日志为 `artifacts/configuration-backup-tests.log`。
- `configuration_backup_smoke.py` 的 8 组真实桌面／回环 SSH 检查通过：导出保留连接及终端、不收集密钥／密码／草稿／历史、同名及取消、源与磁盘变更拒绝且连接不断、成功后才断开并保留原字节／信任／历史／发布草稿，以及恢复的参数预设仍可使用。导入 Gamma 密钥仍不能授权目标。真实限速 16 MiB SFTP 上传期间拒绝恢复审核，上传按原路径完成并核对完整 SHA-256 后才重新开放，不设置本地恢复队列。
- 损坏启动配置专项实际保存含 NUL 的原始坏字节，不展示虚假的零项当前配置；恢复成功后无需先重启，配置备份及脚本导入立即重新启用，后续主题修改成功落盘并在新进程重载。可主动打开前一次原文件重新审核，不自动恢复。源文件、当前内存及磁盘分别复查；原字节先保留，最终保存失败不先断开连接。此流程不声明跨无关编辑器事务。
- 实际检查中文深色 900 × 700 恢复截图、UTF-8 64 KiB 预览边界、悬停／点击 `?`、无横向溢出、可见提交页脚及最小 12px 字号。新专项产物为 `artifacts/configuration-backup-ui/1794783699319799100/`；帮助回归另核对中英文、深浅主题、点击固定／移出关闭、键盘及窗口边界，产物为 `artifacts/help-readability-ui/1794784189159866300/`。正文 14px、标签／代码 13px，辅助说明收进提示，操作结果与关键警告保留在原处。
- 相关回归通过：设置结构 5、脚本参数 11、脚本包 13、脚本 13、目标选择 27、备份 12、工作区偏好 5、分类标签 8、配置参数 6、更新 7、导航 6、帮助 5、完整桌面 27（包含软件、资源、日志、SFTP、RSA 兼容及隔离运行时）。连同新增 8 组，共 153 组；另服务器树未编号专项独立通过拖动／重载、分组、键盘、搜索、菜单及主题检查。日志为 `artifacts/configuration-backup-regression-*.log`。
- 严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译及 diff 检查通过；链接仍有既有 OpenSSL 静态库 PDB 提示。汇总为 `artifacts/configuration-backup-validation-summary.json`。当前验证为 Windows 文件系统和真实回环 SSH／SFTP 协议，不扩展为其他平台的权限或服务器发行版矩阵；本轮未增加 Linux 执行器、客户端常驻或定时调度。

## 常用脚本收藏（2026-11-16）

- Scripts 增加星标和收藏列表，跨官方／个人库搜索并使用当前定义；保留原全局导航顺序及逐服务器审核。配置只保存来源和脚本标识，空值兼容旧文件；不可用引用不阻止整个配置加载，也不产生可执行条目。官方与个人脚本使用相同标识或名称时保持隔离，混合列表按实际来源提供安装信息、版本及编辑入口，列表 key 包含来源。
- 新增 2 项核心测试通过，实际保存／重载完整配置，核对引用不复制正文、同 ID 来源隔离、编辑后读取当前正文、删除只清理个人引用、空收藏不序列化，以及旧配置／失效引用兼容。重复、控制字符和超限保存实际拒绝且原字节保持；新增引用必须解析到当前脚本。完整配置快照的原往返测试也加入收藏。全工作区通过 203 项单元测试（桌面 32、core 171，桌面另 7 项显式忽略）及 5 项正常检查；日志为 `artifacts/script-favorites-tests.log`。
- `script_favorites_smoke.py` 的 9 组真实桌面／回环 SSH 检查通过：旧配置空列表、星标只保存本地偏好且不联网／建任务、同名同 ID 来源区分、编辑／重命名保留、复制不继承、导入替换保留原标识、脚本分享不携带收藏，以及当前正文搜索和新进程重载。收藏打开原参数和双目标审核，接受后关闭桌面、重开主动查询成功，两端仅各提交一次；服务器运维输出由受控对端合成，执行器本身没有修改，本轮不新增运行 Linux 生命周期专项。
- 实际 Windows 文件共享锁阻止保存时，错误就地显示，旧配置和实心星标保持；解锁后才能保存成功。个人删除取消保留引用，确认后同 ID 的官方收藏仍在；损坏启动配置可浏览官方脚本，收藏写入全部禁用且坏字节不变。专项最终产物为 `artifacts/script-favorites-ui/1794785306663784500/`，日志为 `artifacts/script-favorites-ui.log`。
- 实际检查中文深色 900 × 700 收藏截图，官方名翻译与用户原文分别保留；空心／实心 SVG、键盘 Enter 切换、至少 12px 字号、无横向溢出，以及悬停／点击固定的 `?` 通过。收藏明确引用当前定义，不固定旧版本或免除运行审核；分享包继续只包含脚本，完整客户端配置备份包含收藏。
- 相关回归通过：脚本参数 11、脚本包 13、脚本 13、目标选择 27、配置备份 8、帮助 5、导航 6、完整桌面 27（含软件、资源、日志、SFTP、RSA 兼容及隔离运行时）。连同新增 9 组，共 119 组；另服务器树未编号专项独立通过。配置备份专项已加入官方／个人收藏，实际 JSON 全模型相等及审核恢复仍通过，产物为 `artifacts/configuration-backup-ui/1794785571796845500/`；完整桌面产物为 `artifacts/ui-smoke/run-1794785632603107400/`。另实际查看脚本回归的英文浅色空心星标截图，产物为 `artifacts/scripts-ui/desktop-1794785552399704700/official-scripts.png`。
- 严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译及 diff 检查通过；汇总为 `artifacts/script-favorites-validation-summary.json`。保留既有静态 OpenSSL PDB／Git 换行提示。本轮没有更改远程执行器、凭据处理、备份计划或后台驻留行为。

## 服务器系统名称解析（2026-11-16）

- 按需诊断增加 ASCII／punycode 主机名及可选必需 IP，保留最终点号与原预期范围。IPv4／IPv6 预期按规范化值去重，必须全部包含，允许其他结果；每名最多 8 个预期、16 个实际地址。系统解析可能使用 DNS、hosts 或缓存，不输出来源判断、可达性、DNSSEC、TTL 或所有应答保证。不修改解析设置或新增后台任务；说明放入悬停／点击 `?`。
- 新增 2 项核心测试通过：输入及数字／十六进制 IP 别名拒绝、IPv6 规范化及重复、逗号和上限、最终点号区别、固定命令参数、v2 选择／预期绑定及空字段保持原序列化摘要，以及默认四项＋名称＋TLS 能在原 8192 字节上限中组装。每次最多 16 个名称，与其他检查共用 20 项；过长请求仍须拆分。全工作区通过 205 项单元测试（桌面 32、core 173，桌面另 7 项显式忽略）及 5 项正常检查，显式 Linux 集成单独运行；日志为 `artifacts/name-resolution-tests.log`。
- 新原生 `name_resolution_integration` 显式通过：WSL Ubuntu 的实际 Python／glibc、systemd、私有挂载与网络命名空间及回环 UDP DNS。14 项观察包括 hosts 双栈、真实 DNS A／AAAA、缺少必需地址、搜索扩展与最终点号区别、NXDOMAIN、超过 16 个实际地址、名称与证书／配置函数混合检查、实际 15 秒超时、解释器不可用、退出 0 但缺少正向回执，以及实际输出 EFBIG。超时断言在 14–25 秒，输出实测大于零且不超过 131072 字节；未知不成为成功。正常结束删除私有临时结果，停止全部自有任务并移除私有目录，核对宿主 passwd／group／hosts／resolv.conf／nsswitch.conf 哈希保持；没有安装工具或改变宿主解析。日志为 `artifacts/name-resolution-native.log`，逐任务报告在 `artifacts/name-resolution-native/`。
- 原 TLS Linux 专项重新通过 11 项实际网络／证书场景及 1 项输出边界场景，日志为 `artifacts/name-resolution-regression-native-tls.log`。证书／配置函数机械拆分后，没有名称查询时仍原样组装；有名称时仅插入实际选用函数，避免为常见名称＋TLS 组合放大请求上限。环境协议新增 Python 键和最多 9 行，原 key 与摘要兼容，Python 回执及缺失版本继续区分。
- 新增 `name_resolution_smoke.py` 7 组真实桌面／回环 SSH 协议检查通过：无网络输入验证、稳定行删除、双服务器与 TLS 混合审核、客户端关闭后原任务查询、地址数量／不符／未知各自显示、总览保留新增发现及未知项、离线导出默认不含明细／脚本，以及更改预期后拒绝旧结论。移除本地历史后，服务器原元数据仍可核验和导出，两端仅各一次提交。对端运维输出为合成数据，真实解析及 DNS／时限依据来自上述原生专项。
- 最终界面专项产物为 `artifacts/name-resolution-ui/1794786949474116600/`；实际检查中英文与深浅主题的 900px 截图、主机名与预期输入保持、至少 12px 字号、无横向溢出、键盘顺序及提示边界。报告保存通知已补齐中文，切换语言后也按原路径正确显示；报告正文保留原用户名称和服务器输出。
- 相关回归全部通过：原诊断 17、多文件诊断 7、TLS 7、诊断导出 7、服务日志 7、独立任务记录 27、Servers 总览 8、帮助 5、导航 6。连同新增 7 组，共 98 组；日志为 `artifacts/name-resolution-regression-*.log`。严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译及 diff 检查通过，汇总为 `artifacts/name-resolution-validation-summary.json`；保留既有静态 OpenSSL PDB／Git 换行提示。
- 验证环境为 Windows 桌面、受控 SSH 协议和 WSL Ubuntu 的 root 私有命名空间；不扩大为其他 NSS／libc／Python 实现、发行版、账户认证或缓存服务矩阵。客户端不持有解析任务继续运行的责任；服务器重启恢复、DNS 配置修改、DNS 服务器选择及持续监测仍不在此功能内。

## TCP 监听与端口占用排查（2026-11-16）

- 从近期 TCP 诊断和独立服务器记录提供按需入口，固定原服务器身份并预填端口；独立 SSH 查询保留当前工作区，支持密码认证及无 SFTP 的账户。可查看 IPv4／IPv6 监听、可见 PID／进程名和观察到的 cgroup 服务线索；元数据与覆盖明细放入 `?`，关键缺失／上限提示仍可见。服务线索不替代服务状态，未发现监听或进程不表示端口空闲。
- 新增 2 项核心测试通过：端口排序／去重／范围与上限、实际命令参数及响应作用范围绑定、已知凭据脱敏；拒绝错误版本、部分／超限数据、重复监听和错误计数，同时接受明确标记的有限、无持有者观察。全工作区 207 项单元测试通过（桌面 32、core 175，桌面另 7 项显式忽略）及 5 项正常检查。静态检查整理了一处闭包写法及宏内赋值间距，相关核心测试重新通过；日志为 `artifacts/tcp-listeners-tests.log` 和 `artifacts/tcp-listeners-unit.log`。
- `tcp_listeners_integration` 显式运行通过：WSL Ubuntu、真实 IPv4／IPv6、SO_REUSEPORT 的同端口多监听、同 socket 超过八个描述符、实际 PID／start ticks／cgroup 单元关联，以及 nobody 读取时仍见监听但不虚构 root 持有者。全量查询展示 20 行，筛选查询单独保存。检查后原监听仍接受连接，数据文件偏移保持 17，FIFO 原字节保持，输出不含命令行或文件正文标记；停止自有服务、移除私有目录并核对 passwd／group 哈希。原生日志为 `artifacts/tcp-listeners-native.log`，原始查询在 `artifacts/tcp-listeners-native/`。
- 同一原生入口另运行 9 个明确的私有 proc 样本：正常关联、PID 起始时间变化、描述符变化、监听替换、进程／描述符／时间限制，以及 cgroup／IPv6 表不可读。变更不保留旧进程身份，cgroup 缺失可保留已核对的 PID，预算触及时仍核对后续表和明确上限；每份结果都通过客户端协议验证。这些为受控竞态模拟，不扩大为所有内核／命名空间竞态证明。
- `tcp_listeners_smoke.py` 的 8 组真实桌面／回环 SSH 协议检查通过：Alpha 的密码／无 SFTP 查询保留 Beta 连接和终端；规范化端口及本地错误拒绝；有限和空观察不自动刷新；损坏、范围不符、超大、缺少工具或 stderr 警告保留带时间的旧快照；管理员读取显式选择并脱敏，已有行更新进程值；关闭取消通道、清空凭据并忽略迟到事件。查询没有新增服务器任务或改变配置／历史字节，源账户／端点／指纹和检查范围变化不能重新授权。删除本地历史后，独立记录仍能查询原服务器；对端提供真实 SSH 和合成元数据，内核读取依据为上述 WSL 专项。
- 实际检查英文浅色及中文深色 900px 截图，确认 IPv6、至少 12px 字号、可见页脚与悬停／点击帮助，页面没有常驻解释段落。初始专项产物为 `artifacts/tcp-listeners-ui/1794789056510050200/`；最终完整回归与汇总另列下文。
- 最终专项再次通过 8 组，管理员场景完整检查结果区不含两种已知凭据，产物为 `artifacts/tcp-listeners-ui/1794790155394682300/`。相关回归通过：原诊断 17、多文件诊断 7、已删除文件占用 8、独立任务记录 27、服务日志 7、Servers 总览 8、帮助 5、导航 6、完整桌面 27（含软件、资源、日志、SFTP、RSA 兼容及隔离运行时）。连同新增 8 组，共 120 组；完整桌面产物为 `artifacts/ui-smoke/run-1794790304876794900/`，日志为 `artifacts/tcp-listeners-regression-*.log`。
- 严格全工作区 Clippy、core 严格文档、指定 nightly 格式、Windows debug 构建、typos、Python 编译及 diff 检查通过，汇总为 `artifacts/tcp-listeners-validation-summary.json`。保留既有静态 OpenSSL PDB／Git 换行提示；当前验证为 Windows 桌面、受控真实 SSH 协议，以及 WSL Ubuntu 内核／proc 的 root 与 nobody 场景，不扩大为全部内核、架构、容器、网络命名空间或认证矩阵。只检查 TCP LISTEN，不提供 UDP、所有占用状态或服务健康结论。

## 交付核对与 Windows 历史文件占用（2026-11-16）

- [交付对照](enhancement-roadmap.md#当前交付对照) 汇总报告中的入口、批量脚本、服务器任务、备份恢复、按需维护和界面要求，保留实际适配范围及后续路线。日常使用不新增客户端常驻、定时调度或自动重放任务。
- 初次 64 轮连续操作检查在第 62 轮提交前遇到“配置或任务历史只读”。此前 61 轮完成；失败时仍在 Scripts 的审核表单，按钮输入与 IPC 已送达，没有观测到额外页面跳转。证据保留于 `artifacts/final-acceptance-task-navigation.log` 及 `artifacts/task-navigation-ui/1794791005216314200/`。原失败未保存底层 I/O 错误，不能据此确定具体占用者或解释更早的导航现象。
- Windows 原生文件句柄用例复现短读阻止历史替换，修复前实际返回 `Access is denied (os error 5)`，日志为 `artifacts/final-acceptance-history-sharing-before.log`。替换现对 5／32／33 最多等待 250ms，再尝试同一份已同步临时内容；其他平台保留原 rename 行为。持续占用仍返回错误、保留旧字节并清理临时文件，释放后可正常保存，不放宽提交保护或重新执行服务器任务。
- 新用例验证短暂读取释放后保存、持续持锁时原文件与临时清理、释放后正常保存，最终通过，日志为 `artifacts/final-acceptance-history-sharing-after.log`。全工作区 208 项单测通过（桌面 32、core 176；另 7 项桌面显式忽略），另有 5 项普通检查通过；严格全工作区 Clippy、指定 nightly 格式检查及 Windows debug 构建通过，日志为 `artifacts/final-acceptance-{unit,clippy,fmt,build}.log`。
- 人工复核最近帮助专项的中文脚本提示和文件工作区截图，以及 TCP 中文深色 900px 截图；正文 14px、标签／代码 13px、辅助文字至少 12px，说明置于可悬停／点击的 `?`。图片分别保留于 `artifacts/help-readability-ui/1794790293763191000/desktop/` 及 `artifacts/tcp-listeners-ui/1794789056510050200/desktop/`，本次不新增页面解释段落。
- 修复后的连续操作验收完整通过 64 轮、128 个目标：提交与查询期间交替切页、修改草稿／主题及轮换最近十批，291 次侧栏输入与 291 次 Rust 回调逐项一致，730 次 IPC 激活均按原测试输入转发，没有动作之外的点击。原 SSH 工作区保留，最近十批及每台结果核对通过。日志为 `artifacts/final-acceptance-task-navigation-fixed.log`，产物为 `artifacts/task-navigation-ui/1794791756189989400/`。这次通过不证明更早的偶发跳页已解决。
- 最终相关界面回归通过 41 组：脚本 13、任务观测 10、批次查询 7、导航 6、帮助／字号 5。日志为 `artifacts/final-acceptance-regression-*.log`；本轮新帮助产物为 `artifacts/help-readability-ui/1794792106841329600/`。typos 与差异检查通过，按实际日志核对的汇总为 `artifacts/final-acceptance-validation-summary.json`。Windows 历史替换以外的服务器执行路径未变，沿用各专项已有原生证据及明确的兼容范围。
