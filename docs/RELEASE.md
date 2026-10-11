# 发行清单

两条发行线：**桌面（Windows）** 与 **Android**。桌面那条差的是「打包前的配置」；
Android 那条差的是**工具链 + 文件访问模型** —— 不是换一条命令就能出包。

下面所有「现状」都是 2026-10-10 在本机核对过的（看的是 `tauri.conf.json`、依赖树、
以及 Tauri CLI 二进制里内嵌的 Android 模板），不是照抄官方文档。

## 现状一览

| 项 | 桌面（Windows） | Android |
| --- | --- | --- |
| 工具链 | ✅ rustc 1.99 + MSVC + Windows SDK + WebView2 | ❌ 无 `ANDROID_HOME`、无 SDK、无 NDK、无 CMake |
| Rust target | ✅ `x86_64-pc-windows-msvc` | ❌ 四个 android target 都没装 |
| JDK | 不适用 | ⚠️ 本机 JDK 25；模板用的是 Gradle 9.6.1 + AGP 9.3.1 |
| 移动端入口 | — | ✅ `mobile_entry_point` 与 `crate-type` 已就位 |
| 原生依赖 | ✅ | ⚠️ `aws-lc-sys` 要 CMake；SQLite / libwebp 走 `cc` 无碍 |
| 文件访问 | ✅ 路径式（walkdir + `std::fs` + `asset://`） | ❌ 分区存储，且插件在 Android 上不支持选目录 |
| 触摸交互 | — | ❌ 需要捏合缩放与单指拖动 |
| 图标 | ✅ `src-tauri/icons/` 全套 | ⚠️ 需 `tauri icon` 生成 mipmap |
| 签名 | ⚠️ 无证书，SmartScreen 会拦 | ⚠️ 无 keystore |
| LICENSE | ❌ 文件缺失（`package.json` 已声明 MIT） | 同左 |

---

## 一、桌面（Windows）

### 1. 打包前要补的配置

`src-tauri/tauri.conf.json` 的 `bundle` 段现在只有 `active` / `targets` / `icon` 三项，
发行版至少要补这些：

| 字段 | 现状 | 说明 |
| --- | --- | --- |
| `publisher` | 未设 | 默认取 `identifier` 的第二段（这里是 `lightcone`）。它是 MSI 的 Manufacturer 属性，也是 deb 的 Maintainer 字段 —— 写清楚「谁做的」 |
| `copyright` / `category` / `shortDescription` / `longDescription` | 未设 | 安装程序与「应用和功能」里显示的元数据 |
| `licenseFile` | 未设 | 指向 LICENSE 文件，安装向导会展示 |
| `windows.nsis.languages` | 未设 | 默认只有 `["English"]`，中文安装界面要写 `["SimpChinese"]`（可选 `displayLanguageSelector: true` 让用户选） |
| `windows.wix.language` | 未设 | 默认 `en-US`，中文 MSI 写 `"zh-CN"` |
| `windows.wix.upgradeCode` | 未设 | **建议显式写死一个 GUID**。默认由 productName 推导，改了产品名 Windows 就会当成另一个应用，用户机器上会出现两份 |
| `windows.nsis.installMode` | 未设 | 默认 `currentUser`（装进用户目录、不弹 UAC）；要装进 `Program Files` 用 `perMachine` |

另外两处与发行直接相关：

- **`app.security.csp` 现在是 `null`**（开发方便）。发布前应换成严格 CSP，
  并记得把 `asset:` 与 `http://asset.localhost` 放进 `img-src` —— 图片全走
  `asset://` 协议，漏了这一条界面会整片空白。
- **`app.security.assetProtocol.scope` 保持空数组是对的**。作用域在运行时由
  `grant_asset_scope` 逐个目录授权（用户添加的图库根目录 + 缩略图目录）。
  别为了省事写成 `["**"]`：那等于把用户整个磁盘读给前端。

### 2. 版本号在三个地方

`0.1.0` 同时写在 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`。
三处要一起改（Tauri 取 `tauri.conf.json` 的 `version`，但另外两处不一致会让
「装的是哪个版本」变得含糊）。

### 3. 打包

```bash
npm run tauri:build
```

首次构建会**联网**下载 NSIS 与 WiX 两套安装器工具链（各自几十 MB）。
`bundle.targets: "all"` 在 Windows 上就是 **MSI + NSIS** 两种。

### 4. 产物

| 路径 | 内容 |
| --- | --- |
| `src-tauri/target/release/lightcone-gallery.exe` | 裸可执行文件（可当绿色版） |
| `src-tauri/target/release/bundle/msi/*.msi` | WiX 打的 MSI |
| `src-tauri/target/release/bundle/nsis/*-setup.exe` | NSIS 打的安装程序（推荐发给个人用户） |

`[profile.release]` 已经调好（`lto` / `strip` / `codegen-units = 1` / `panic = "abort"`），
不用再动。WebView2 的默认安装策略是 `downloadBootstrapper`：Win10 老机器上装的时候
会去微软下载运行时，离线环境要改 `windows.webviewInstallMode`。

### 5. 可选：代码签名

没有证书时 Windows SmartScreen 会拦（「未知发布者」）。要签名就配：

```jsonc
"bundle": {
  "windows": {
    "certificateThumbprint": "<证书 SHA1>",
    "digestAlgorithm": "sha256",
    "timestampUrl": "http://timestamp.digicert.com",
    "tsp": true
  }
}
```

用云签名服务的话改用 `signCommand`。

### 6. 可选：自动更新

需要三件事：`bundle.createUpdaterArtifacts: true`、前端加 `tauri-plugin-updater`、
一对签名密钥（公钥进配置，私钥留在 CI）。不打算做的话忽略即可 —— 现在没有更新通道，
用户得手动换包。

---

## 二、Android

### 1. 移动端的定位：pixez 是采集层，本应用是索引层

这个应用的每一条桌面数据通路都建立在**文件系统路径**上：

```
选目录 → walkdir 递归 → std::fs 读元数据 / 读图片字节
       → 缩略图写进 app_data_dir
       → 原图用 asset:// 协议交给 WebView
```

Android 上这套有三处硬碰，但主人的定位把它全绕开了：**手机上 pixez 负责下图，
本应用只负责整理与归类**——「只要 pixez 能下图，数据库就是无限大」。于是：

1. **不需要「选目录」**。pixez 落盘命名是 `{pid}_p{n}.{ext}`，而作品库的归档
   本来就是文件名驱动的——枚举全机图片、按文件名正则一过，是 pixez 下的
   就自动进库。目录选择、SAF 树、授权持久化统统不需要
   （`tauri-plugin-dialog` 在 Android 上没有目录选择、返回的是 `content://`
   URI 而非路径——已核实 `src/mobile.rs` 只有 `pick_file` / `pick_files` /
   `save_file` / `show_message_dialog`——但这不再是障碍，因为入口不存在了）。
2. **枚举全机图片有正规通道：MediaStore**。`READ_MEDIA_IMAGES` 是普通运行时
   权限（Play 友好），查询自带文件名、尺寸、大小、修改时间——连图片头都不用
   读，索引一万张几乎零成本。Kotlin 桥做三件事：查询 MediaStore、按命名
   正则过滤、把命中行的 `_id / display_name / relative_path / size /
   date_modified / width / height` 分批回给 Rust。
3. **像素读取走「Kotlin 拷临时文件」桥**：缩略图生成时把源图拷进应用 cache
   目录，Rust 现有管线（解码 → WebP → 落盘）原样复用；弹窗原图同理按需拷贝
   ——cache 目录本来就在 asset:// 的授权清单里，原图显示这一环也接上了。

所以要改的只有：**一个 Kotlin 桥插件 + `artworks` 扫描入口的分叉（桌面走
walkdir，Android 走 MediaStore 桥）**。SQLite、虚拟滚动、分块缓存、缩略图
队列、检视弹窗全部原样。

> 顺带：联网补全在手机上有意外的好消息——国内安卓普遍用 VPN 模式代理
> （TUN 在 IP 层带走流量），`reqwest` 直连就能通，代理输入框可以降级成「可选」。

#### 备选路线（按需）

- **侧载 + `MANAGE_EXTERNAL_STORAGE`**：保留全部路径式代码（walkdir / `std::fs`
  / `asset://` 直读），只把「选目录」换成固定根 + 手填路径。改动最小，但该权限
  上 Play 审核基本过不了，且比 MediaStore 方案没有额外好处——弃为备选。
- **SAF 全重写**：树 URI + `ContentResolver` 全链路。为「任选任意目录」这种
  本应用不需要的能力付出最大代价——弃为备选。

### 2. 装工具链（本机全缺）

需要的东西（括号里是本机现状）：

| 组件 | 要求 | 本机 |
| --- | --- | --- |
| Android SDK | `platforms;android-37`（CLI 模板的 `compileSdk` / `targetSdk` 都是 37） | ❌ 没有 |
| build-tools | 跟随 AGP 9.3.1（先装 `build-tools;37.0.0`，若报缺别的版本按它报的装） | ❌ 没有 |
| NDK | **`29.0.13846066`** —— CLI 二进制里只出现这一个 NDK 版本号（`At least NDK …` 校验与 `sdkmanager --install "ndk;…"` 提示共用它） | ❌ 没有 |
| CMake | 必须。`aws-lc-sys`（rustls 的加密后端）用 CMake 构建 | ❌ 没有 |
| JDK | 17 或更高。本机是 JDK 25，处于 AGP/Gradle 支持范围的上沿、没实测过；稳妥起见装 JDK 21 LTS | ⚠️ 有 JDK 25 |
| Rust target | `aarch64-linux-android` `armv7-linux-androideabi` `i686-linux-android` `x86_64-linux-android` | ❌ 只有 windows-msvc |

装法（cmdline-tools 版，不必装整个 Android Studio）：

```bash
# 1) 从 developer.android.com 下 command-line-tools，解压到 <SDK>/cmdline-tools/latest
sdkmanager "platform-tools" "platforms;android-37" "build-tools;37.0.0"
sdkmanager "ndk;29.0.13846066"
sdkmanager "cmake;3.22.1"

# 2) 环境变量（Tauri CLI 认这几个）
#    ANDROID_HOME / ANDROID_SDK_ROOT = <SDK>
#    NDK_HOME   = <SDK>/ndk/29.0.13846066
#    JAVA_HOME  = <JDK 21>

# 3) Rust 交叉编译目标
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
```

### 3. 初始化与首次构建

```bash
npm run tauri -- android init     # 生成 src-tauri/gen/android（Gradle 工程）
npm run tauri -- android dev      # 连真机 / 模拟器跑
npm run tauri -- android build    # 出 APK
npm run tauri -- android build --aab   # 出 AAB（上架 Play 用）
```

`gen/android` 是**要提交进仓库的**（它是工程的一部分），但里面的
`app/build/`、`.gradle/`、`local.properties`、keystore 不该提交 ——
`tauri android init` 会自带一份 `.gitignore`，核对一下再提交。

### 4. 触摸交互

`ImageViewer.svelte` 现在靠三样鼠标专属的东西：滚轮缩放（`onwheel`）、
指针悬停更新 3D 倾斜（`onpointermove`）、指针捕获拖拽。Android 上前两样都不存在：

- **缩放**：加双指捏合（`touchstart` / `touchmove` 里算两指间距），
  复用现有的 `zoomAt()` —— 它只吃「倍率 + 锚点坐标」，与输入设备无关，这部分不用重写。
- **平移**：单指拖动。现在的 `onpointerdown` 只在放大态才进入拖拽，触屏上还要区分
  「点击」（关弹窗 / 翻页）与「拖动」，建议加一个移动距离阈值。
- **视差**：触屏没有 hover，放大前的 3D 倾斜不会触发。要么在移动端关掉视差只留缩放平移，
  要么用 `devicemotion`（陀螺仪）驱动 —— 后者是纯 CSS transform，Android WebView
  是 Chromium，渲染本身没问题。

另外两处移动端专属的：

- **返回键**：现在弹窗是前端状态、不进路由历史，返回键多半会直接退出应用而不是关弹窗。
  需要单独拦一次（实现方式得实测）。
- **外观页**：`set_window_material` 在 Android 上是空操作（`set_effects` 编译得过，
  但材质是 Windows DWM 专有），移动端应该把材质选项隐藏或标注不可用。
  `tauri.conf.json` 里的 `transparent: true` 同理无意义；界面底色由 `body` 的
  `bg-surface-950` 提供，所以不会漏底。

### 5. 联网功能

`reqwest` 这边是好消息：依赖树里**没有 OpenSSL**（`reqwest 0.13.5` 的
`default-tls = ["rustls"]`，走 rustls + aws-lc-rs + `rustls-platform-verifier`，
Android 上直接用系统证书库）。唯一的额外要求就是上面那张表里的 CMake。

但要注意：Cargo.toml 里 reqwest 关掉了默认特性、也没开 `system-proxy`，
所以**不会自动走系统代理**。手机上的 Clash 一般开在局域网端口，让用户在
「元数据」栏手填 `http://<电脑IP>:7897` 即可 —— 桌面版已经是这个交互。

### 6. 签名与产物

```bash
keytool -genkey -v -keystore lightcone.jks -keyalg RSA -keysize 2048 -validity 10000 -alias lightcone
```

在 `gen/android/app/build.gradle.kts` 里配 `signingConfigs`（keystore 路径与口令走
环境变量或 `keystore.properties`，**别写进仓库**）。Play 上架要 AAB，`versionCode`
由 `bundle.android.versionCode` 决定，不填则按
`major * 1000000 + minor * 1000 + patch` 从版本号换算。

图标：`tauri icon src-tauri/icons/icon.png` 会一并生成 Android 的 mipmap 各密度版本。

---

## 三、两条线共同的收尾检查

```bash
cd src-tauri && cargo test     # 后端 49 项
npm test                       # 前端虚拟滚动几何 10 项
npm run check                  # svelte-check
```

- [ ] 三处版本号一致
- [ ] `LICENSE` 文件补上（`package.json` 已经声明 MIT，但仓库里没有这个文件）
- [ ] `security.csp` 不再是 `null`，且 `img-src` 放行了 `asset:`
- [ ] `bundle` 元数据（publisher / copyright / category / 描述 / licenseFile）补齐
- [ ] 安装器语言设成中文（NSIS `SimpChinese` / WiX `zh-CN`）
- [ ] `windows.wix.upgradeCode` 显式写死
- [ ] 装机实测一遍：添加图库 → 扫描 → 缩略图 → 弹窗原图 → 复制图片 → 全屏 → 外观切换
- [ ] Android 额外：`MANAGE_EXTERNAL_STORAGE` 声明、捏合缩放、返回键、keystore 不进仓库
