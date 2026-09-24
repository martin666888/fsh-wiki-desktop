# 飞书文档轻客户端

一个 Windows 桌面客户端，把飞书文档（网页版）装进一个原生窗口里用：多标签、无边框、可自定义显示字体，并且**所有运行数据都待在安装目录里**，不散落到 AppData。

基于 Tauri 2 + 系统 WebView2，不打包浏览器内核，安装体积约 10 MB。

---

## 特性

- **多标签浏览** —— 每个标签是一个独立 WebView，切换不销毁页面，状态保留；中键点击标签关闭
- **无边框窗口** —— 自绘标题栏与最小化/最大化/关闭按钮，标签栏即拖动区，双击空白最大化
- **显示字体自定义** —— 可分别设置西文 / 中文 / 代码字体；在 `document-start` 注入，**首帧即生效**，无默认字体跳变
- **链接分流** —— 飞书域内的链接留在应用内，外部链接交给系统浏览器打开
- **新窗口拦截** —— 页面里 `target="_blank"` 不再弹原生窗口，改为新建标签（带 500ms 同 URL 去重）
- **自动更新** —— 启动时静默检查 GitHub Release，有新版本则下载、验签、更新并自动重启

## 工作原理

| 关注点 | 做法 |
|---|---|
| 渲染 | `WebviewUrl::External` 直接加载线上 `https://www.feishu.cn/drive/home/`，登录态存在本地 WebView2 profile |
| 窗口结构 | 一个**不带 WebView 的原生窗口**，通过 `window.add_child()` 挂多个子 WebView：`ui`（React 标签栏，高 42px）+ `tab-N`（各一个飞书页面） |
| 布局 | 按 `scale_factor` 把 42 逻辑像素换算为物理像素，内容区 = 窗口内高 − 42 |
| 标签切换 | 目标 WebView `set_position` / `set_size` / `show` + `set_focus`，其余 `hide`（不销毁重建） |
| 标签标题 | 注入 `init.js` 轮询 `document.title`，通过 IPC 事件 `feishu-title` 上报 |
| 页面 IPC 白名单 | `capabilities/remote-feishu.json` 只放行 `feishu.cn` / `larksuite.com` 域 |
| 字体注入 | `.initialization_script()`，早于页面任何脚本执行 |

> 涉及 WebView 增删的 Tauri 命令必须是 `async`：`add_child` / `close` 会 hop 到主线程并阻塞等待，同步命令会自死锁。

## 数据目录

全部数据落在 **exe 同级的 `data/`** 下，删除安装目录即彻底清除：

```
<安装目录>\
├── feishu-desktop.exe
├── uninstall.exe
└── data\
    ├── fonts.json          # 字体配置
    └── webview\            # WebView2 用户数据（缓存 / 登录态，可能涨到数百 MB）
```

> 卸载程序**不会**删除 `data/`，需要手动删掉安装目录。
> 首次切换数据目录位置后需要重新登录一次。

## 开发

```bash
npm install
npm run tauri dev
```

> dev 模式下 `current_exe()` 指向 `src-tauri/target/debug/`，所以**开发版的数据目录是 `src-tauri/target/debug/data/`**，与安装版各存一份登录态。

## 构建

```powershell
# 两个变量缺一不可：Windows 不允许"空值环境变量"（设为空串等于删除该变量），
# 所以即便密钥本身没有密码，也必须显式提供密码变量，
# 否则 tauri build 会停在交互式密码提示上、无法在脚本 / CI 里跑。
$keyDir = "$env:USERPROFILE\.tauri"
$env:TAURI_SIGNING_PRIVATE_KEY          = (Get-Content -Raw "$keyDir\fsh-wiki.key").Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content -Raw "$keyDir\fsh-wiki.key.password").Trim()

npm run tauri build
```

产物在 `src-tauri/target/release/bundle/nsis/`：

- `飞书文档轻客户端_<版本>_x64-setup.exe` —— 安装包
- `飞书文档轻客户端_<版本>_x64-setup.nsis.zip` + `.sig` —— 更新包与签名（自动更新用，不用手动分发）

## 发版与自动更新

**打 tag 即发布**，GitHub Actions 全自动完成：

```bash
# 1. 升版本号（三处，必须同步）
#    src-tauri/tauri.conf.json
#    src-tauri/Cargo.toml
#    package.json
# 2. 提交并打 tag
git commit -am "chore: release v0.1.5"
git tag v0.1.5
git push && git push --tags
```

流水线会构建 → 签名 → 建 Release → 上传安装包和更新包 → 生成 `latest.json`。

客户端启动时拉取：

```
https://github.com/martin666888/fsh-wiki-desktop/releases/latest/download/latest.json
```

比对版本 → 下载 → 验签 → 以 passive 模式静默运行 NSIS 安装器（`/P /UPDATE /R`）→ 自动重启。

### 三条铁律

1. **每次发版必须升版本号**，否则客户端认为没有新版本，不会更新。
2. **签名私钥必须备份**。私钥丢失后，已安装的旧版本将永远无法自动更新（验签必然失败），只能手动重装。
3. **私钥密码也必须备份**。它是解密私钥的唯一凭据，丢了等同于丢了私钥。

签名密钥生成方式：

```bash
npm run tauri signer generate -- -w ~/.tauri/fsh-wiki.key -p "<密码>"
```

两个仓库 Secret，缺一不可：

| Secret | 内容 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件内容 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 私钥密码 |

对应地，公钥写进 `tauri.conf.json` 的 `plugins.updater.pubkey`。

> **注意**：不要用无密码的私钥。Windows 无法传递"空值环境变量"，无密码密钥在本地脚本和 CI 里都会卡在交互式密码提示上。
>
> **私钥和密码都不要提交进仓库**（本机存放在 `~/.tauri/`，仓库外）。

## 已知限制

- **仅 Windows**。依赖 WebView2（Win10/11 自带），更新链路使用 NSIS 安装包。
- **只出 NSIS，不出 MSI**。Tauri 的自动更新在 Windows 上只能驱动 NSIS 安装器。
- **MSI 版无法把数据收进安装目录**：MSI 的 `InstallScope` 固定为 `perMachine`，装进 `Program Files` 后普通权限无权在程序旁边写文件。
- 字体样式对 `*` 使用 `!important`，极端情况下可能影响个别图标的字形渲染（已排除 `i` / `svg` / 代码块 / `*icon*` 类名）。
- 更新时不会提示确认，属于静默行为；安装阶段会显示系统进度条。

## 技术栈

Tauri 2 · React 18 · TypeScript · Vite 6 · WebView2
