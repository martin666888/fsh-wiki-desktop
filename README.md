# 飞书文档轻客户端

Windows 桌面客户端，把飞书文档网页版装进一个原生窗口用。多标签、无边框、可换显示字体。运行数据全部写在安装目录的 `data\` 下，不散到 AppData。

Tauri 2 + 系统 WebView2，安装包 3.4 MB。

## 功能

- 多标签：每个标签是一个独立 WebView，切换时只隐藏不销毁，页面状态保留。中键点标签可关闭。
- 无边框窗口：标题栏和最小化/最大化/关闭按钮都是自绘的。标签栏空白处拖动窗口，双击最大化。
- 自定义显示字体：西文、中文、代码字体分开设置。样式在 document-start 注入，第一帧就是设定的字体，不会先闪一下默认字体。
- 链接分流：飞书域名内的链接留在应用内，其余交给系统浏览器。
- 新窗口拦截：页面里 `target="_blank"` 不弹原生窗口，改建新标签，同 URL 在 500ms 内去重。
- 更新：可以在后台检查新版本，但下载和安装都要你手动确认。见下面「更新」。

## 实现要点

窗口本身不含 WebView。主窗口是用 `WindowBuilder` 建的裸窗口，再用 `window.add_child()` 往上挂子 WebView：一个高 42px 的 `ui`（React 写的标签栏），其余每个对应一个飞书标签页。

内容区尺寸按 `scale_factor` 把 42 逻辑像素换算成物理像素，高 DPI 屏下不会错位。切换标签就是把目标 WebView `set_position` / `set_size` / `show`，其余的 `hide`。

标签标题拿不到现成的：飞书不是我们的页面，没法从外面读它的路由。所以注入了 `init.js` 轮询 `document.title`，通过 IPC 事件 `feishu-title` 上报给标签栏。能调 IPC 的远程页面限定在 `capabilities/remote-feishu.json`，只有 feishu.cn 和 larksuite.com 域。

涉及 WebView 增删的 Tauri 命令必须写成 `async`。`add_child` 和 `close` 会 hop 到主线程并阻塞等待，同步命令会把自己锁死。

## 数据目录

```
<安装目录>\
├── feishu-desktop.exe
├── uninstall.exe
└── data\
    ├── fonts.json          字体配置
    ├── updates.json        更新偏好
    ├── update\             已下载待安装的更新包
    └── webview\            WebView2 数据（缓存和登录态，会涨到几百 MB）
```

卸载程序只删 exe 和 uninstall.exe，**不删 `data\`**，要清干净得手动删安装目录。这是有意的：覆盖安装新版时安装目录里的 `data\` 不受影响，所以升级不会掉登录。

数据目录跟着 exe 走。开发模式下 exe 在 `src-tauri/target/debug/`，所以开发版的数据在 `src-tauri/target/debug/data/`，和安装版各有一份登录态。

## 更新

分三步，安装永远要你点：

1. **检查**。启动时自动检查，可以在「关于」里关掉；也可以随时手动点「检查更新」。
2. **下载**。发现新版本后可以自动下载（默认关），也可以手动点「下载更新」。
3. **安装**。必须点「重启并安装」。它会关掉应用、跑 NSIS 安装器、再自动把应用打开。没有静默强更。

有更新可处理时，标签栏的 ⚙ 上会出现一个蓝点，点它直接落到「关于」页。

已经下载但没安装的包放在 `data\update\`，重启应用后仍然显示为「已下载」，不用重下。

## 开发

```bash
npm install
npm run tauri dev
```

## 构建

`bundle.createUpdaterArtifacts` 打开之后，没有签名私钥 `tauri build` 会直接失败。

```powershell
$keyDir = "$env:USERPROFILE\.tauri"
$env:TAURI_SIGNING_PRIVATE_KEY          = (Get-Content -Raw "$keyDir\fsh-wiki.key").Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content -Raw "$keyDir\fsh-wiki.key.password").Trim()

npm run tauri build
```

产物在 `src-tauri/target/release/bundle/nsis/`：

- `飞书文档轻客户端_<版本>_x64-setup.exe`：安装包，同时就是更新包
- 同名的 `.sig`：签名，自动更新验签用，不用手动分发

Tauri v2 下 NSIS 的更新产物就是安装器本体加一个 `.sig`，不是 v1 时代的 `.nsis.zip`。

## 发版

推 tag 触发 GitHub Actions，自动构建、签名、建 Release、生成 `latest.json`。

先改三处版本号，`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`package.json`，然后：

```bash
git commit -am "release v0.1.8"
git tag v0.1.8
git push && git push --tags
```

客户端读的是 `https://github.com/martin666888/fsh-wiki-desktop/releases/latest/download/latest.json`。

版本号每次都改。没改的话客户端会认为没有新版本，不会提示。

### 签名密钥

```bash
npm run tauri signer generate -- -w ~/.tauri/fsh-wiki.key -p "<密码>"
```

公钥填进 `tauri.conf.json` 的 `plugins.updater.pubkey`。私钥和密码存成两个仓库 Secret：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码 |

两个都必须有，哪怕密钥本身不设密码。Windows 不允许空值环境变量（设成空串等于把变量删掉），只给私钥不给密码的话 tauri 会停在交互式密码提示上，本地脚本和 CI 都会卡住。

私钥和密码都要另外备份。私钥丢了，已经装出去的旧版本就再也无法自动更新了，验签必然失败，只能让用户手动重装。

更新清单 `latest.json` 是 workflow 里一个 PowerShell 步骤生成的，没用 `tauri-action` 的 `includeUpdaterJson`。它只认 v1 风格的 `.nsis.zip.sig`，而现在 CLI 产出的是 `.exe.sig`，于是它会打印 `Signature not found for the updater JSON` 然后跳过，清单根本不会上传。另外 GitHub 会剥掉资源名里的非 ASCII 字符，`飞书文档轻客户端_0.1.8_x64-setup.exe` 实际上存成 `_0.1.8_x64-setup.exe`，所以那一步是从 Release API 读真实资源名，不自己拼。

## 已知限制

只出 Windows 包，依赖 WebView2（Win10/11 自带）。MSI 不出了：Tauri 的自动更新在 Windows 上只能驱动 NSIS 安装器，而且 MSI 的 `InstallScope` 固定是 perMachine，装进 Program Files 后普通权限没法在程序旁边写 `data\`。

字体样式对 `*` 用了 `!important`，个别图标的字形可能受影响，已经排除了 `i`、`svg`、代码块和类名带 `icon` 的元素。

## 技术栈

Tauri 2、React 18、TypeScript、Vite 6、WebView2
