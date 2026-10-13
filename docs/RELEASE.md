# 发行清单

两条发行线：**桌面（Windows）** 与 **Android**。桌面那条差的是「打包前的配置」
（签名、安装器元数据）；Android 那条的**工具链与文件访问模型都已落地**，
真机能跑通全流程，剩下的是签名与上架相关的收尾。

下面所有「现状」都在本机核对过（看的是 `tauri.conf.json`、依赖树、以及 Tauri CLI
二进制里内嵌的 Android 模板），不是照抄官方文档。Android 一列已按 **2026-10-12
真机结果**更新 —— 文中仍留着几段当时的设计推演，它们记录了「为什么最后选了
这条路」，不要当成待办。

## 现状一览

| 项 | 桌面（Windows） | Android |
| --- | --- | --- |
| 工具链 | ✅ rustc 1.99 + MSVC + Windows SDK + WebView2 | ✅ SDK `F:\Android\Sdk` + NDK 29 + CMake 3.22.1 + zulu-24；`ANDROID_HOME` 已写进用户环境变量 |
| Rust target | ✅ `x86_64-pc-windows-msvc` | ✅ 四个 android target 全装 |
| JDK | 不适用 | ✅ zulu-24 跑通 Gradle 9.6.1 + AGP 9.3.1 |
| 移动端入口 | — | ✅ `mobile_entry_point` 与 `crate-type` 已就位 |
| 原生依赖 | ✅ | ✅ `aws-lc-sys` 的 CMake 已装；SQLite / libwebp 走 `cc` 无碍 |
| 文件访问 | ✅ 路径式（walkdir + `std::fs` + `asset://`） | ✅ 自研 Kotlin 桥走 SAF 选目录、拿回物理路径，之后复用同一套管线 |
| 触摸交互 | — | ✅ 捏合缩放 / 单指拖动 / 陀螺仪视差 / 返回手势关闭栈 |
| 图标 | ✅ `src-tauri/icons/` 全套 | ⚠️ 需 `tauri icon` 生成 mipmap |
| 签名 | ⚠️ 无证书，SmartScreen 会拦 | ✅ 已配（走调试密钥库，release 可直接覆盖升级 debug 包） |
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

### 2. 工具链（已装齐）

需要的东西与本机现状（2026-10-11 装齐，2026-10-12 复核）：

| 组件 | 要求 | 本机（2026-10-11 实测已装齐） |
| --- | --- | --- |
| Android SDK | `platforms;android-36` + platform-tools（装在 `F:\Android\Sdk`）。**注意 `platforms;android-37` 在稳定仓库里不存在**（API 37 还在 canary 频道），生成的 Gradle 里 compileSdk/targetSdk 要手动从 37 改成 36 | ✅ |
| build-tools | `build-tools;37.0.0`（稳定仓库里有，与 AGP 9.3.1 配合正常） | ✅ |
| NDK | **`ndk;29.0.13846066`**（r29-beta3，2.4GB，装在 `F:\Android\Sdk\ndk\29.0.13846066`） | ✅ |
| CMake | `cmake;3.22.1`（`aws-lc-sys` 用，SDK 自带 ninja） | ✅ |
| JDK | 17+。**实测 zulu-24 跑通了 Gradle 9.6.1 + AGP 9.3.1**（zulu-25 跑 sdkmanager 也没问题） | ✅ zulu-24 |
| Rust target | `aarch64-linux-android` `armv7-linux-androideabi` `i686-linux-android` `x86_64-linux-android` | ✅ 四个全装 |

**2026-10-11 实测记录（第一次 aarch64 debug APK 已产出）**：

- **下载源**：`dl.google.com` 与 sdkmanager 直连可达（无需代理）；但 **Gradle 发行包**
  会重定向到 GitHub Releases——代理半死时会拿到 502/空文件。解法：从腾讯云镜像
  `mirrors.cloud.tencent.com/gradle/gradle-9.6.1-bin.zip` 下载，铺进
  `~/.gradle/wrapper/dists/gradle-9.6.1-bin/<hash>/`（hash = URL 的 MD5 转 base36）。
- **crates.io 的移动端新依赖**（simd_cesu8 等，桌面构建从没拉过）走 Fake-IP 的
  TUN 路由，**节点抖动时会「transfer too slow」连续重试失败**——重试一次往往就通。
- `set_window_material` 的 `set_effects` 在 mobile 上不存在，已用 `#[cfg(desktop)]`
  门控（移动端 no-op，材质选项前端应隐藏）。
- debug APK 358MB 是正常体型（未剥离的 Rust 调试符号）；release + strip 会小一个量级。

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

**`ANDROID_HOME` 是硬要求，别指望 `local.properties`**（2026-10-12 实测）：
只写 `gen/android/local.properties` 里的 `sdk.dir` 时，`tauri android build` 依然会报

```
Info ANDROID_HOME not set, trying to locate Android SDK...
Info Using installed Android SDK: C:\Users\NachoNeko\AppData\Local\Android\Sdk
failed to ensure Android environment: ...
```

Tauri CLI 自己那层环境探测只认 `ANDROID_HOME` / `ANDROID_SDK_ROOT` 环境变量，
`local.properties` 是给 Gradle 用的、救不了它。本机已用
`setx ANDROID_HOME "F:\Android\Sdk"`（连同 `ANDROID_SDK_ROOT`）写进用户环境变量，
**新开的终端**才生效。顺带记一笔：`%LOCALAPPDATA%\Android\Sdk` 下确实有个同名空目录，
它是 CLI 探测时留下的壳子，别把它当成「SDK 装到 C 盘去了」或者「SDK 没装」。

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

两个例外要留意：`local-maven/`（那份 rustls AAR，见第 5 节，**必须提交**）
和 `local.properties`（机器相关，**必须忽略** —— 它现在的角色只是给 Gradle 兜底，
真正让 Tauri CLI 找到 SDK 的是 `ANDROID_HOME`）。

### 4. 触摸交互（已实现）

**2026-10-12 真机跑通**，实际做法与下面推演有两处出入，记一笔免得下次重新推：

- **视差走陀螺仪，但要用 `deviceorientation` 而不是 `GravitySensor`。** Generic Sensor API
  的 `GravitySensor` 在 K70 Ultra 的 WebView 里**构造器存在、却永远不发读数**——
  只靠 `typeof GravitySensor !== "undefined"` 判断会挑中一条死管线。现在的做法是两条
  管线都起，700ms 内没等到有效读数（`null`，或三轴幅值 < 0.5）就回退到
  `deviceorientation`（实测约 36Hz，够用）。
- **返回键**在 `MainActivity` 挂 `OnBackPressedCallback`，向前端派发 `lc-android-back`
  事件；前端 `backStack.svelte.ts` 维护一个关闭栈（检视弹窗 → 筛选抽屉 → …），
  栈空了才调 `exit_app` 真退出。**别让系统默认行为接管**——那会直接退出应用。
- **全屏**走 `WindowCompat.getInsetsController` 隐藏状态栏与手势条（`android_immersive`），
  配合 `enableEdgeToEdge` + CSS `env(safe-area-inset-*)` 补安全区内边距。

下面这段是当时的设计推演，保留作背景：

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

#### 2026-10-12：Android 上真正跑通联网踩到的三层

**第一层，`rustls-platform-verifier` 在 Android 上不会自己初始化。** 编译期一切正常，
运行时第一条 HTTPS 请求就 panic：

```
Expect rustls-platform-verifier to be initialized
```

它需要一个 JNI `Context`。两边都得做：Rust 侧在 `setup()` 里取
`ndk_context::android_context()` 再调 `rustls_platform_verifier::android::init_with_env(env, context)`；
Gradle 侧必须把它的 Kotlin 组件也打进去（`org.rustls:rustls-platform-verifier:0.2.0`），
缺了会退化成 `failed to call native verifier`。本机把它放在
`src-tauri/gen/android/local-maven/`（19KB，**要提交进仓库**）—— 当初是从 ghproxy
镜像下的，上游仓库国内直连不稳。

**第二层，MIUI / HyperOS 会在应用退到后台时掐网。** 系统给 uid 打上
`netpolicy blocked=APP_BACKGROUND`，表现是「切出去看一眼，回来全超时」。
现在的对策是元数据补全期间挂一个前台服务（`MetaKeepAliveService`：通知栏常驻 +
`WifiLock` + `PARTIAL_WAKE_LOCK`），跑完立刻撤。

**第三层，剩下的就真的是节点问题了。** 拿电脑 `curl` 交叉验证过：同一时刻
pixiv 间歇性失败而 baidu 正常，说明是代理节点在抖，不是应用的问题。所以
`pixiv.rs` 里的错误链会一路展开到最底层原因，并在节点类错误后面补一句
「代理的节点连不上 Pixiv，去代理软件里换一个节点试试」——不然用户只会看到
一句 `operation timed out`，根本无从下手。

### 6. 签名与产物

**签名已经配好了**（2026-10-12）：`app/build.gradle.kts` 读 `gen/android/keystore.properties`
（该文件被 gitignore，口令不进仓库），文件不存在时整块跳过 —— 于是 `assembleRelease`
仍然能跑，只是产出一个装不上的 `-unsigned.apk`。

现在那个文件指向 **Android 调试密钥库**（`~/.android/debug.keystore`），是刻意为之：

| 做法 | 后果 |
| --- | --- |
| **调试密钥库**（当前） | release 包与一直装的 debug 包**签名相同**，可以**直接覆盖升级** —— 不用卸载，应用数据（索引 / 标签 / 元数据）全在 |
| 自己 `keytool` 生成的新 keystore | 必须先**卸载**现有应用才能装（签名不同），而**卸载会清掉应用数据**。将来正式发行时走这条 |

```bash
npm run tauri -- android build --target aarch64    # -> app-universal-release.apk（28MB，已签名）
```

Play 上架要 AAB，`versionCode` 由 `bundle.android.versionCode` 决定，不填则按
`major * 1000000 + minor * 1000 + patch` 从版本号换算。

#### ★ release 专属陷阱：R8 会吃掉 Kotlin 桥

Tauri 的 Android 插件全靠反射（按**类名字符串**建实例、按**方法名**匹配 `@Command`），
R8 看不见这些引用，默认会把整个类删掉或改名。**debug 包不混淆，所以这个问题只在
release 上出现**，而且表现为「扫描 / 选目录 / 沉浸式这些命令全部报找不到」。

保护规则在 `media-store-bridge/consumer-rules.pro`。这个文件原先被
`consumerProguardFiles("consumer-rules.pro")` 声明却**并不存在** —— 靠
`gradle.properties` 里的 `android.proguard.failOnMissingFiles=false` 掩盖着，等于裸奔。

改完务必验一次（只读，不用装机）：

```bash
"F:/Android/Sdk/cmdline-tools/latest/bin/apkanalyzer.bat" dex packages --defined-only \
  src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk \
  | grep lightcone
# 期望看到 com.lightcone.gallery.media.MediaStoreBridge 以及 pickDirectory / keepAlive /
# listBuckets / immersive / copyToCache / listPixivImages / checkPermissions / directoryCallback
```

#### 体积：28MB vs 700MB

| 构建 | 体积 | 原因 |
| --- | --- | --- |
| release | **28MB** | `strip = true` + LTO + `panic=abort`，Rust `.so` 只剩 27.9MB |
| debug | 360–720MB | debug 构建类型显式要求 `jniLibs.keepDebugSymbols`，`.so` 不剥离符号（354MB）；打包偶尔还在它前面留一段等长的零字节预留区，于是文件 718MB 而 `unzip -l` 的内容总和只有 362MB |

**只有 debuggable 的包才开 `webview_devtools_remote`** —— 需要用 Chrome DevTools 连
WebView 调试时必须装 debug 包，日常用 release。

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
- [ ] Android 额外：`keystore.properties` 与 `local.properties` 不被提交、`local-maven/` 被提交
- [ ] Android release：R8 之后用 `apkanalyzer dex packages` 确认 `MediaStoreBridge` 的命令方法还在
- [x] Android 交互（2026-10-12 真机验证）：SAF 选目录、捏合缩放、单指拖动、返回手势、全屏沉浸
- [x] Android 联网（2026-10-12 真机验证）：rustls JNI 初始化 + 保活服务，元数据补全可跑
