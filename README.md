# 飞书文档轻客户端

Windows 桌面客户端，将飞书文档网页版封装为原生窗口应用。支持多标签、无边框窗口与自定义显示字体。运行数据存放于安装目录的 `data\` 目录。

基于 Tauri 2 与系统 WebView2，安装包约 3.4 MB。

## 功能

- 多标签：每个标签对应一个独立 WebView，切换时仅隐藏而不销毁，页面状态得以保留。支持中键关闭标签。
- 无边框窗口：标题栏与最小化、最大化、关闭按钮均为自绘。标签栏空白区域可拖动窗口，双击可切换最大化。
- 自定义显示字体：西文、中文、代码字体可分别设置。样式于 document-start 注入，首帧即生效。
- 链接分流：飞书域名内的链接在应用内打开，其余链接交由系统浏览器处理。
- 新窗口拦截：页面中 `target="_blank"` 不创建原生窗口，改为新建标签页；相同 URL 在 500ms 内去重。
- 更新：支持后台检查新版本，下载与安装均需手动确认，详见「更新」一节。

## 实现要点

主窗口不包含 WebView，由 `WindowBuilder` 创建，再通过 `window.add_child()` 挂载子 WebView：`ui` 为高 42px 的标签栏（React 实现），其余各对应一个飞书标签页。

内容区尺寸依据 `scale_factor` 将 42 逻辑像素换算为物理像素，以避免高 DPI 环境下错位。标签切换通过 `set_position`、`set_size`、`show` 显示目标 WebView，并将其余 WebView 隐藏。

飞书页面不属于本应用，无法直接读取其路由与标题，因此注入 `init.js` 轮询 `document.title`，经 IPC 事件 `feishu-title` 上报至标签栏。可调用 IPC 的远程页面限定在 `capabilities/remote-feishu.json` 中，仅 feishu.cn 与 larksuite.com 域。

涉及 WebView 增删的 Tauri 命令必须声明为 `async`：`add_child` 与 `close` 会切换至主线程并阻塞等待，同步命令将导致死锁。

## 数据目录

```
<安装目录>\
├── feishu-desktop.exe
├── uninstall.exe
└── data\
    ├── fonts.json          字体配置
    ├── updates.json        更新偏好
    ├── update\             已下载待安装的安装包
    └── webview\            WebView2 数据（缓存与登录状态）
```

`data\` 目录以 exe 所在位置为基准。卸载程序仅删除 `feishu-desktop.exe` 与 `uninstall.exe`，`data\` 目录需手动删除。

开发模式下 exe 位于 `src-tauri/target/debug/`，数据目录相应为 `src-tauri/target/debug/data/`，与安装版各自维护独立的登录状态。

## 更新

更新分为检查、下载、安装三个阶段，其中安装阶段始终需要手动触发。

1. 检查。启动时自动执行，可在「关于」页关闭；亦可随时通过「检查更新」手动触发。
2. 下载。发现新版本后可自动执行（默认关闭），亦可手动触发。
3. 安装。需点击「重启并安装」。该操作会关闭应用、运行 NSIS 安装器，随后自动重新启动应用。

存在待处理的更新时，标签栏 ⚙ 按钮显示蓝色标记，点击将直接打开「关于」页。

已下载但未安装的安装包存放于 `data\update\`，重启应用后仍保持「已下载」状态，无需重新下载。

## 开发

```bash
npm install
npm run tauri dev
```

## 构建

启用 `bundle.createUpdaterArtifacts` 后，缺少签名私钥将导致 `tauri build` 失败。

```powershell
$keyDir = "$env:USERPROFILE\.tauri"
$env:TAURI_SIGNING_PRIVATE_KEY          = (Get-Content -Raw "$keyDir\fsh-wiki.key").Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content -Raw "$keyDir\fsh-wiki.key.password").Trim()

npm run tauri build
```

产物位于 `src-tauri/target/release/bundle/nsis/`：

- `飞书文档轻客户端_<版本>_x64-setup.exe`：安装包，同时用作更新包
- 同名 `.sig` 文件：签名，供自动更新验证，无需手动分发

Tauri v2 中 NSIS 的更新产物为安装器本体加 `.sig` 文件；v1 为 `.nsis.zip`。

## 发版

推送 tag 触发 GitHub Actions，自动完成构建、签名、创建 Release 与生成 `latest.json`。

需先修改三处版本号：`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`package.json`。

```bash
git commit -am "release v0.1.9"
git tag v0.1.9
git push && git push --tags
```

客户端读取的地址为 `https://github.com/martin666888/fsh-wiki-desktop/releases/latest/download/latest.json`。

每次发版均须修改版本号，否则客户端判定为无新版本，不会提示更新。

### 签名密钥

```bash
npm run tauri signer generate -- -w ~/.tauri/fsh-wiki.key -p "<密码>"
```

公钥填入 `tauri.conf.json` 的 `plugins.updater.pubkey`。私钥与密码存入两个仓库 Secret：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码 |

两个 Secret 均为必需，即使私钥未设置密码。Windows 不允许空值环境变量（赋空字符串等价于删除该变量），仅提供私钥时 tauri 将停留在交互式密码输入提示，导致本地脚本与 CI 阻塞。

私钥与密码需另行备份。私钥丢失后，已分发的旧版本将无法自动更新，因为签名验证必然失败，只能手动重新安装。

更新清单 `latest.json` 由 workflow 中的 PowerShell 步骤生成，未使用 `tauri-action` 的 `includeUpdaterJson`：后者仅识别 v1 风格的 `.nsis.zip.sig`，而当前 CLI 产出 `.exe.sig`，会输出 `Signature not found for the updater JSON` 并跳过上传。此外 GitHub 会移除资源名中的非 ASCII 字符，`飞书文档轻客户端_0.1.9_x64-setup.exe` 实际存为 `_0.1.9_x64-setup.exe`，因此该步骤从 Release API 读取实际资源名。

## 已知限制

仅发布 Windows 安装包，依赖 WebView2（Win10/11 已内置）。不提供 MSI：Tauri 的自动更新在 Windows 上仅支持 NSIS 安装器，且 MSI 的 `InstallScope` 固定为 perMachine，安装至 Program Files 后普通权限无法在程序所在目录写入 `data\`。

字体样式对 `*` 使用 `!important`，个别图标的字形可能受影响。已排除 `i`、`svg`、代码块及类名含 `icon` 的元素。

## 技术栈

Tauri 2、React 18、TypeScript、Vite 6、WebView2
