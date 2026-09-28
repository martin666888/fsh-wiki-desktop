# 飞书文档轻客户端

将飞书文档网页版封装为 Windows 桌面窗口。基于 Tauri 2、React 与系统 WebView2，支持多标签、自定义显示字体和分阶段更新。运行数据保存在安装目录的 `data\` 中。

## 功能与边界

- 每个页面使用独立 WebView，切换时隐藏而不销毁，保留当前页面状态。标签列表独立滚动，窗口控制按钮固定可达；支持中键关闭与键盘操作。
- 顶部「分屏」支持单页、左侧文章加右侧选页、左右双文档三种状态。复用原标签的 WebView，分屏、切换和退出均不重新导航页面。
- 无边框主窗口支持拖动、双击最大化和窗口控制；设置窗口复用时可从最小化恢复，关闭主窗口会同时关闭设置窗口。
- 西文、中文、代码字体分别设置，只改变本地显示，不修改文档。代码与图标子树单独保护；关闭字体后，已有标签重新导航同样清除覆盖样式。
- 飞书/Lark 域的标准 HTTPS 链接在应用内打开；其他 HTTP/HTTPS 链接交给系统浏览器。带用户名密码的 URL、文件链接及未知协议不交给系统执行。
- `target="_blank"` 由原生新窗口回调分流，内部文档打开为标签。WebView2 默认右键菜单被拦截，飞书页面自己的菜单继续由页面处理。
- 单实例：再次启动时还原并激活已有主窗口。开发版与安装版共用应用标识，会相互排斥。
- 更新可自动检查、可选自动下载，安装始终需要点击「重启并安装」。

## 代码入口与权限

`src-tauri/src/lib.rs` 负责应用装配和设置窗口；`tabs.rs` 管理标签和工作区快照；`layout.rs` 定义布局状态与尺寸约束；`native.rs` 接入 WebView2 焦点、加载结果、预览和分隔线鼠标捕获；`fonts.rs` 管理字体；`navigation.rs` 统一链接分类；`storage.rs` 提供原子写入；`updates.rs` 管理更新任务、签名缓存和安装。

主窗口使用 `WindowBuilder`，再通过 `add_child()` 挂载高 42 逻辑像素的本地 `ui` 标签栏、本地 `workspace` 选页与分隔线、独立内容 WebView。窗口过窄时显示布局提示；布局按 `scale_factor` 换算成物理像素。布局操作串行执行，原生位置统一在主线程更新。涉及 WebView 增删的命令保持 `async`，避免主线程重入阻塞。

标题通过 Tauri 的原生 `on_document_title_changed` 回调读取，Rust 保存完整标签快照并通知本地标签栏和工作区。应用命令由 `build.rs` 声明 ACL，仅本地 `ui/settings/workspace` WebView 获得各自所需能力；远程飞书页面没有本地 IPC 权限。CSP 限制本地界面资源，开发 CSP 单独允许 Vite 连接；这不替代飞书网页自己的 CSP。

前端入口为 `src/App.tsx`、`src/Workspace.tsx` 和 `src/Settings.tsx`。设置保存失败会显示原因，只有后端保存成功才确认新值；更新状态包含检查、下载、取消和安装阶段。

## 分屏浏览

- 分屏属于单个标签。点击顶部「分屏」后，当前标签进入选页；选择另一个独立标签，会将其页面合并进当前标签，复用原 WebView。已有分屏组合不会被选页操作拆散。
- 顶部「＋」、Ctrl+T 和文章中的新标签链接创建独立的完整页面标签。分屏内的飞书首页、云盘列表作为选页入口，点击文章直接在所在栏打开。原来的分屏组合、焦点和宽度保存在原标签内；切回即可继续阅读。多个标签可以各自分屏。
- 选页区的「在此打开首页」只用于给当前分屏添加第二个页面，与顶部新建标签明确区分。选页支持搜索、预览、Tab、方向键、Enter，Esc 退出选页。
- 关闭分屏标签会关闭其中两个页面。退出分屏会将两个页面拆为独立标签，最近操作的页面留在当前标签；退出选页则恢复原页面。以上布局操作不重新加载文章。
- 分隔线宽 6 逻辑像素，可拖动，双击恢复均分；获得焦点后支持方向键、Shift 加方向键、Home/End、Enter，拖动时 Esc 撤销。每栏最小宽度 480，窗口至少 966 逻辑像素宽才可分屏。变窄时暂时显示当前页面，扩大后恢复原比例。
- 页面加载状态和失败重试入口位于标签栏。
- 标签与分屏布局只在本次运行中保留，重启仍打开首页。

## 数据目录

```text
<安装目录>\
├── feishu-desktop.exe
├── uninstall.exe
└── data\
    ├── fonts.json         字体配置
    ├── updates.json       更新偏好
    ├── update\
    │   └── pending.json   安装包、签名与版本组成的原子缓存
    └── webview\           WebView2 缓存与登录状态
```

目录以 exe 所在位置为基准，运行账户必须能写入。开发模式对应 `src-tauri/target/debug/data/`，与安装版各自维护登录数据。安装目录数据、登录状态、私钥、依赖及本地验证产物不入库。卸载后如需彻底清理登录状态，请检查并手动移除遗留 `data\`；删除前先备份需要保留的数据。

配置和更新缓存采用同目录临时文件写完、同步后替换，失败保留原文件。旧版本的独立 `pending-*.exe` 没有保存完整验证信息，新版本不会直接信任或安装它们，也不会自动删除；需要重新下载一次更新。

## 更新

1. 检查：正式构建启动时可自动检查，也可手动触发。已有有效待安装缓存时跳过启动自动检查，避免覆盖「已下载」。开发构建不自动查询线上更新。
2. 下载：默认手动，可在「关于」中开启自动下载。进度累计所有数据块；总长度未知时显示已下载大小。检查和下载可以取消，检查总超时 30 秒，下载总超时 5 分钟，网络读取停滞也有超时。
3. 安装：下载后将 bytes、原始签名、受保护版本及元数据一起保存。重启恢复与安装前均用内置公钥重新验签，并确认签名版本、缓存版本和当前版本关系；有效缓存支持完全离线安装。损坏、半文件、版本不匹配或不兼容缓存不会启动安装器。

点击「重启并安装」后进入安装中状态，阻止重复提交。当前产品只接受 NSIS 包；经验证的安装器写入随机临时目录，使用禁止改写/删除的读取句柄再次验签后启动。安装器成功启动才退出当前进程，随后由安装器重新打开应用。安装启动失败会保留错误反馈，不会把失败显示为成功。

缓存包上限为 128 MiB。有效缓存不会因离线或检查失败消失；重新下载失败也不会覆盖原有完整包。更新服务地址配置在 `src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints`，当前使用本仓库最新正式 Release 中的 `latest.json`。

## 开发与本地检查

需要 Node.js 22 或更高版本、Rust stable、Windows C++ 构建工具和 WebView2 Runtime。CI 使用 Node.js 22；本机依赖版本以锁文件为准。

```powershell
npm ci
npm run tauri dev
```

启动开发版前应退出安装版；本地检查无需启动客户端，也不会读取生产登录资料或使用生产签名私钥：

```powershell
npm run check
npm run check:rust
```

- `check:project`：核对 npm、Cargo、Tauri、两份锁文件的版本，并检查 CSP、远程权限与签名发布配置。指定 tag 可运行 `npm run check:project -- --tag v0.1.13`。
- `check:signatures`：使用 Node 内置 `assert/crypto` 和公开签名向量验证 Ed/ED 兼容、bytes 与可信注释篡改拒绝、版本绑定和稳定版顺序，不使用私钥或可执行安装器。
- `check`：依次执行上述脚本及 TypeScript/Vite 构建。
- `check:rust`：依次运行 Rust 格式检查、锁定依赖检查、严格 Clippy 和 Rust 自带的单元测试。已有依赖齐全时，可单独运行 cargo check/clippy/test 并增加 `--offline`。

没有引入第三方测试框架。GitHub 的常规提交/PR 检查使用相同命令；这些检查不能替代真实飞书登录、混合 DPI、读屏器与真实安装升级验收。原生操作验证应使用独立测试账户和临时数据目录，避免影响日常安装版。

## 构建与签名

`bundle.createUpdaterArtifacts` 保持开启，发布构建必须取得原有签名私钥。公钥在 `tauri.conf.json` 中，私钥及密码保存在仓库之外；不要打印到日志或提交到 Git。

```powershell
$keyDir = Join-Path $env:USERPROFILE '.tauri'
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -Raw (Join-Path $keyDir 'fsh-wiki.key')).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content -Raw (Join-Path $keyDir 'fsh-wiki.key.password')).Trim()
npm run tauri -- build --ci -- --locked
```

`--ci` 禁止交互式询问，缺少必要签名配置时应直接失败。产物位于 `src-tauri/target/release/bundle/nsis/`：

- `飞书文档轻客户端_<版本>_x64-setup.exe`：NSIS 安装器，同时用作更新包。
- 同名 `.sig`：更新签名，必须随安装器保存并上传；`latest.json` 引用它的内容。

Tauri 2 的 NSIS 更新产物是安装器与签名，无需 v1 的 `.nsis.zip` 包装。当前产品配置使用中文 NSIS；Tauri 本身也支持 MSI，但本项目的离线更新和发布校验明确只支持 NSIS。

## 发版流程

推送 `v<SemVer>` tag 会触发 Release 工作流；这一步会对外发布，需要先完成本地检查并获得发版授权。版本与 tag 必须保持一致。

准备新版本时同步修改 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`，再更新 `package-lock.json` 和 `src-tauri/Cargo.lock` 中对应的应用版本。不得只改 tag。先验证待发布版本，例如：

```powershell
npm run check:project -- --tag v0.1.13
npm run check
npm run check:rust
```

只有源码与锁文件已经全部升级到示例版本，上述 tag 检查才会通过。每次正式发布需高于当前最新稳定版；已公开版本禁止重新上传覆盖。

流水线顺序为：检查源码/版本 → 构建签名安装器并上传 draft → 从 draft 重新下载实际资源 → 验证安装器签名及签名中的版本 → 生成并上传 `latest.json` → 再次下载验证安装器、签名和清单 → 单次操作公开 Release。公开之前失败只留下草稿，稳定入口继续指向上一版。修复后可重跑未公开的草稿。

Release 工作流全局串行；最终公开前再次拒绝低于或等于当前稳定版的任务。包含预发布标识的 tag（如 `v0.2.0-beta.1`）发布为 prerelease，并明确不修改 latest。当前客户端只跟随稳定入口，不自动接收这些预发布版本。

清单脚本使用构建步骤返回的 Release ID 读取草稿，并按资源 ID 下载验证，避免按 tag 查询仅公开版本时遗漏草稿。下载链接使用 API 的真实资源名，避免 GitHub 规范化中文文件名造成链接错误。`includeUpdaterJson` 关闭，由 `scripts/release.mjs` 统一生成和校验清单。`release:prepare`、`release:manifest`、`release:publish` 是供 CI 使用的发布操作，需要 `GH_TOKEN`、`GITHUB_REPOSITORY` 和 `RELEASE_TAG`；后两步还需构建输出的 `RELEASE_ID`。不要把它们当成普通本地检查命令。

### 签名保留

仓库 Secrets 保留既有 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥只传给构建步骤，后续校验只使用公开配置；PR 检查无需签名 Secrets。维护签名密钥及密码的独立备份，发布修复不能重新生成或替换公钥，否则现有安装版无法验证新更新。

只有从未对外分发过的初始项目才可新建签名密钥：

```powershell
npm run tauri -- signer generate -w ~/.tauri/fsh-wiki.key -p '<密码>'
```

已有项目的密钥轮换需另行安排迁移。丢失当前私钥时无法为旧公钥签发可信更新，必须通过明确的手动重新安装或预先设计的迁移流程处理。

## 已知范围

仅发布 Windows x64 NSIS 安装包，依赖可用的 WebView2 Runtime；精简系统或缺少 Runtime 的设备需要安装运行时。字体规则对图标和代码子树做了保护，但飞书页面结构可能改变，应结合真实文档回归。当前没有承诺重启恢复全部已打开标签、离线阅读正文或多账户隔离。

公开签名向量的第三方许可见 `scripts/NOTICE-minisign.txt`。
