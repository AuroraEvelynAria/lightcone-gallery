# LightCone Gallery · 光锥映画

[![CI](https://github.com/AuroraEvelynAria/lightcone-gallery/actions/workflows/ci.yml/badge.svg)](https://github.com/AuroraEvelynAria/lightcone-gallery/actions/workflows/ci.yml)

**本地优先的 Pixiv 图库桌面应用。** 原图留在你自己的目录里、应用只读不写；索引与元数据
全部存在本机 SQLite；联网只用来补 Pixiv 的标题与标签，而且可选 —— 不填代理，其余功能照常。

界面走「机箱陈列」式的工业配色。检视弹窗是一套纯 CSS 的三层光锥视差：没有 Three.js，
没有 WebGL，`transform-style: preserve-3d` 加几层 Z 轴就够了。

> **发行状态**：桌面（Windows）现在就能打包出安装包；**Android 也已在真机跑通**
> （Redmi K70 Ultra / HyperOS 实测：SAF 选目录 → 扫描 → 缩略图 → 检视弹窗 → 标签展示）。
> 移动端怎么绕开分区存储、以及踩过的坑，见 [docs/RELEASE.md](docs/RELEASE.md#二android)。

## 能做什么

**通用图库**（`/`）
- 添加任意根目录，增量扫描：先把 `path -> (大小, mtime)` 读进内存快照，只对新增 / 变化的文件写库
- 搜索、排序、收藏、评分；相册与标签两套手动组织方式
- 从图库移除只删索引，不动磁盘上的文件
- **虚拟滚动**：无论加载了多少条，DOM 里始终只有视口那几十格。这一点是真机实测逼出来的 ——
  早先把已加载的全渲染出来，一翻页就有 60~100ms 的长任务（一次性挂载 120 张卡片），
  滑久了 DOM 上万个节点；换虚拟滚动后同一台手机上 p99 帧间隔从 41.6ms 降到 **8.4ms**
  （正好是那块 120Hz 屏的刷新周期），卡顿帧从 150 降到 2

**作品库**（侧栏「作品库 (PID)」，与「全部图片 / 收藏」同款的原地切换）
- 只收录文件名符合 `{pid}_p{num}` 的图片，按 Pixiv 作品号归类，一行代表「某作品的某一页」
- 定高网格虚拟滚动：5000 条数据单屏最多渲染 45 个格子，元数据按 120 条一块拉取
- 缩略图在扫描阶段就预生成好并写回数据库，列表自带地址 —— 3000 条整趟滚动下来缩略图相关 IPC 只有 1 次
- **两种排版可切换**（工具条上的「网格 / 瀑布流」，选择记在 localStorage；窄屏首次默认瀑布流）
  - *网格*：定高方格，整齐；原图裁成正方
  - *瀑布流*：每格按自己的高宽比占位、插进当前最矮的列，**原图长宽比完整保留**（参考 pixez）。
    本机 4430 张实测：只有 **2.6%** 的作品因为超过 `[0.5, 2.5]` 的极端比例被裁一点，
    而正方网格是 100% 全裁
  - 两种排版共用同一份卡片内容与同一套列宽公式（`virtualGeometry` 的 `columnsFor` /
    `columnWidthFor`），切换时格子大小不变，只是摆放方式变了
- **筛选两端各取所长，且都带出入场动效**
  - *桌面*：一排**维度入口**（分级 / 画师 / 标签），点哪个哪个从按钮下方浮出锚点面板，
    **盖在列表上、不推挤内容**（京东 PC 那种）。入口上挂着该维度的已选条数，收起也看得见
  - *手机*：入口收进 header 的「筛选」按钮，面板是浮层，**底部浮起 / 右侧滑出可切换**
    （开关就在面板头部，选择记在 localStorage）。两种形态的人机功效各有取舍 ——
    底部浮层只占下半屏、一眼还看得见图，拇指也够得着；右侧抽屉一屏放得下更多条件 ——
    所以都留着让人自己挑
  - 两端共用同一份维度内容（`ratingGroup` / `artistGroup` / `tagGroup`），不再各写一套
- **元数据补全面板默认收起**：统计 / 代理输入 / 补单批 / 重查已删除这几行是「配一次就
  不碰」的东西，手机上实测它们一个人吃掉 **13.4% 的屏高**（112 CSS / 835 CSS），
  比筛选那一整行还多。收起后只剩一行（现状数字 + **主操作常驻**），点箭头才展开
- **缩略图从骨架淡入**（150ms，只动 opacity —— 交给合成器，不碰滚动性能）
- **切换视图带淡入**（全部图片 ↔ 作品库）。刻意**只淡不位移**：位移要写 `transform`，
  而带 `transform` 的元素会成为后代里 `fixed` 元素的包含块，筛选浮层 / 批量条 /
  检视弹窗会整块跳到屏幕中间
- **切换排版时保留阅读位置**：网格 ↔ 瀑布流的几何完全不同，像素没法换算，但**数据
  下标**可以 —— 切换前记下当前看的是第几条，新组件用自己的几何算出该滚到哪，
  不会在几千条里翻到一半时被弹回顶部
- **窄屏改两列**（`minColumnWidth` 在 767px 以下从 200 降到 150，与通用图库同值）。
  手机可用宽只有约 333 CSS px，200 的最小值只排得下一列 —— 卡片一列铺满时格子的
  **物理**边长到 1084px（屏密度 3.25），而缩略图当时是 400px 长边、3:2 的图短边只有
  267px，被放大四倍，肉眼可见地糊。两列把格子压到 522 物理像素，再把缩略图提到
  640（短边 427）就只剩 1.22 倍，已看不出差别

**画师维度**
- 从目录名离线反推（pixez 这类下载器落盘时就是 `<画师名>_<画师ID>/<pid>_p<n>.<ext>`），零网络成本
- 按 **uid** 归组而不是名字：同一 uid 在本地出现过 12 组不同名字（改过名、或把展会后缀写进了目录名）

**标签图库**（侧栏「标签图库」一节）
- 把 NSFW 从日常浏览里分出去：内置「全年龄 / R-18」两条，按 `illust_meta.x_restrict`
  这个 **Pixiv 官方分级**筛，不依赖作者有没有打 R-18 标签
- 还能自己建：在作品库的筛选面板里配好「标签 + 分级」，点「存成图库」起个名字即可，
  之后侧栏一点直达。**主列表行为不变**，只是多几个入口
- 侧栏上的条数实时算，不存列 —— 作品库随时在增删，存下来的计数必然过期
- 没抓过元数据的作品**既不算全年龄也不算 R-18**（是未知），界面上会单独提示条数

**Pixiv 联网元数据**（可选）
- 标题 / 画师名 / 官方标签 / 页数 / R-18 等级，走公开网页接口 —— 不带 cookie、不碰 OAuth
- 限流是核心约束：串行 + 每次间隔 800ms、非 200 一律不重试、连续失败 5 次中止整批、单次上限 300 条
- 「被限流」与「作品在 Pixiv 上没了」严格分开：前者留在待补全里下次再试，后者才是终态

**筛选**
- 画师是「任一」（一个作品只有一个画师），标签是「同时包含」（多选是为了收窄），两套语义不同
- 筛选条件全走 `?` 占位符绑定，排序字段走白名单映射

**检视弹窗**
- 三层光锥视差 + 鼠标跟随扫光；放大后自动关掉倾斜，鼠标改负责平移
- 滚轮缩放锚在光标下的那一点（不是屏幕中心），双击「正好铺满」，放大后拖拽平移
- 全屏、复制图片到剪贴板、**设为桌面壁纸**、左右翻页

**设为壁纸**（两端同一颗按钮，但实现完全不同）
- **先选目标再设**：桌面 / 锁屏 / 桌面 + 锁屏。直接替人决定「设哪个」是不对的 ——
  锁屏和桌面是两种观感（一个是自己解锁后看的，一个是别人拿起来先看到的）
- Windows 桌面：`SystemParametersInfoW(SPI_SETDESKWALLPAPER)`，**自己声明 FFI**
  （不为一个函数引 `windows-sys`）
- Windows 锁屏：Win32 没有对应 API，走 WinRT 的 `UserProfilePersonalizationSettings`
  （`windows` crate 本来就在依赖树里，加它不增加编译负担）
- Android：交给 Kotlin 的 `WallpaperManager.setStream` —— 由系统按壁纸尺寸自己解码，
  不必先算 `inSampleSize`（算小了 OOM、算大了壁纸糊）
- **Windows 不认的格式（WebP / ICO）会先转成 PNG** 再设。直接传过去 Windows 会**静默失败**：
  返回成功、桌面却不变，最难查
- 转码结果落在应用数据目录而不是临时目录 —— 注册表里存的是那个路径，重启后还得读得到

**外观**
- 5 套配色 × 4 种 Windows 原生材质（云母 / 亚克力 / 标签页 / 模糊）× 明暗，三个正交的轴
- 材质是 DWM 的真实系统效果，不是 CSS 假模糊；偏好存数据库（localStorage 只作首帧缓存）

## 技术栈

| 层 | 选型 | 说明 |
| --- | --- | --- |
| 桌面外壳 | Tauri 2 | 用系统 WebView（Windows 上是 WebView2），产物体积小 |
| 前端框架 | SvelteKit + Svelte 5 | `adapter-static` 的 SPA 模式，SSR 关闭 |
| 构建 | Vite | 开发端口固定 1420 |
| 语言 | TypeScript（严格模式） | 前端类型与 Rust 模型手工对齐 |
| 样式 | Tailwind CSS 4 | 走 `@tailwindcss/vite` 插件，无需 `tailwind.config.js` |
| 数据库 | SQLite + rusqlite | `bundled` 特性内嵌 SQLite 源码，用户机器无需装 sqlite3 |
| 图像处理 | `image` + `webp` | `image` 负责解码与缩放，`webp`（libwebp 绑定）负责**有损** WebP 编码 |
| 外观 | RhinE 配色 + DWM 材质 | 配色源自 RhinE 的 5 套（FLARE 浅色按反馈改为暖纸底）+ 自设计 4 套，共 9 套；材质是 Windows 原生 Mica / Acrylic |

## 环境要求

- **Node.js** ≥ 22.18（`npm test` 靠 Node 原生剥离 TS 类型跑，该行为 22.18 / 23.6 起默认开启；实测 Node 24）
- **Rust** stable（用 `rustup` 安装）
- **Windows**：Visual Studio Build Tools（含 MSVC v143+ 与 Windows SDK）、WebView2 Runtime
- **macOS**：Xcode Command Line Tools
- **Linux**：`webkit2gtk-4.1`、`libappindicator3`、`librsvg2` 等
- **Android**（只在要打移动端包时才需要）：Android SDK（platform 36 + build-tools 37）、
  NDK `29.0.13846066`、CMake 3.22.1、JDK 17+，以及四个 `*-linux-android` Rust target。
  **还必须把 `ANDROID_HOME` 指向 SDK 根目录** —— Tauri CLI 只认这个环境变量，
  `local.properties` 顶不上（实测确认）。本机的 SDK 在 `F:\Android\Sdk`，
  不在默认的 `%LOCALAPPDATA%\Android\Sdk`。

首次运行 `npm run tauri:dev` 要编译约 560 个 crate（`Cargo.lock` 里的包数，含 `reqwest`
带来的 TLS 依赖树），耗时较长；之后走增量缓存。

**想用联网补全元数据的话，国内需要能连上 pixiv 的代理**（直连实测是 `HTTP 000`）。
在作品库页面的「元数据」一栏填入代理地址（如 `http://127.0.0.1:7897`）并保存即可。
不填也不影响其它功能 —— 画师维度完全离线。

## 快速开始

```bash
npm install          # 安装前端依赖
npm run tauri:dev    # 开发模式（同时拉起 Vite 与 Rust，打开应用窗口）
npm run tauri:build  # 打包安装包 / 可执行文件
```

其他常用脚本：

```bash
npm run dev      # 仅启动 Vite（浏览器里调 UI，此时无法调用 Tauri 命令）
npm run build    # 仅构建前端到 build/
npm run check    # svelte-check 类型检查（CI 也会跑这条）
npm test         # 前端自检：两套排版的几何（Node 24 原生跑 TS，未引入测试框架）
cd src-tauri && cargo test   # 后端测试
```

## 数据放在哪

| 内容 | 路径 |
| --- | --- |
| 数据库 | `<AppData>/com.lightcone.gallery/gallery.db`（Windows：`%APPDATA%\com.lightcone.gallery\`） |
| 缩略图缓存 | `<AppData>/com.lightcone.gallery/thumbnails/<归属域>/<尺寸>/<id>.webp`（作品库用 640、通用图库用 320） |

原图始终留在用户自己的目录里，应用**只读不写**；从图库移除记录不会删除磁盘文件。
退出时会显式 `PRAGMA wal_checkpoint(TRUNCATE)`，所以单独备份 `gallery.db` 不会丢数据。
细节（为什么缩略图分两层目录、为什么偏好存数据库不存 localStorage）见
[docs/DESIGN-NOTES.md](docs/DESIGN-NOTES.md#数据存放位置)。

## 目录结构

```
├─ src/                          # SvelteKit 前端
│  ├─ app.css                    # Tailwind 入口 + 设计令牌（@theme）
│  ├─ app.html
│  ├─ lib/
│  │  ├─ api.ts                  # 所有 invoke 调用的类型化封装
│  │  ├─ types.ts                # 与 Rust models.rs 对齐的类型定义
│  │  ├─ gallery.svelte.ts       # 通用图库的全局状态（Svelte 5 runes）
│  │  ├─ artworks.svelte.ts      # 作品列表的分块窗口缓存（按需拉取）
│  │  ├─ themes.ts               # 配色方案与材质定义（由 RhinE 的 tokens.json 生成）
│  │  ├─ theme.svelte.ts         # 外观三轴状态（配色 × 材质 × 明暗）
│  │  ├─ virtualGeometry.ts      # 定高网格的几何推导（纯函数，可脱离浏览器测）
│  │  ├─ masonryGeometry.ts      # 瀑布流的几何推导（最短列优先 + 按列二分求可见区间）
│  │  ├─ thumbnailQueue.ts       # 缩略图请求的并发闸门
│  │  ├─ format.ts               # 展示层格式化
│  │  ├─ backStack.svelte.ts     # Android 返回手势的关闭栈（弹窗 → 抽屉 → 才轮到退出）
│  │  └─ components/
│  │     ├─ Sidebar.svelte       # 图库 / 相册 / 标签导航
│  │     ├─ Toolbar.svelte       # 搜索、排序、扫描
│  │     ├─ ImageGrid.svelte     # 通用图库网格（虚拟滚动 + 按需续页）
│  │     ├─ ImageCard.svelte     # 单张卡片
│  │     ├─ VirtualGrid.svelte   # 定高网格虚拟滚动（纯 CSS，无滚动库）
│  │     ├─ VirtualMasonry.svelte # 瀑布流虚拟滚动（绝对定位，纯 CSS）
│  │     ├─ FilterPanel.svelte  # 筛选面板外壳（底部浮层 / 右侧抽屉，可切）
│  │     ├─ ArtworkCard.svelte   # 作品格子（可点击）
│  │     ├─ ArtworksView.svelte # 作品库视图（嵌在主界面，侧栏原地切换）
│  │     ├─ ArtworkThumbnail.svelte # 直接用列表里的 thumbnailPath + 失败回退
│  │     ├─ ArtworkLightbox.svelte  # 作品库检视弹窗（PID 栏 + 查看器内核）
│  │     ├─ ImageViewer.svelte   # 查看器内核：光锥 3D / 缩放 / 平移 / 全屏（唯一加载原图的地方）
│  │     ├─ Thumbnail.svelte     # 通用图库的懒加载缩略图
│  │     └─ Lightbox.svelte      # 大图查看 + 标签 / 评分编辑
│  └─ routes/
│     ├─ +layout.svelte
│     ├─ +layout.ts
│     ├─ +page.svelte            # 通用图库（网格 / 搜索 / 标签 / 相册）
│     ├─ appearance/+page.svelte # 外观（配色 / 材质 / 明暗）
│     └─ artworks/+page.svelte   # 旧入口重定向（作品库已并入主界面侧栏）
├─ tests/
│  ├─ virtual-geometry.test.ts   # 定高网格几何自检（npm test）
│  └─ masonry-geometry.test.ts   # 瀑布流几何自检（npm test）
├─ docs/
│  ├─ DESIGN-NOTES.md            # 数据库设计、命令清单、实现说明与踩过的坑
│  ├─ RELEASE.md                 # 桌面 + Android 发行清单
│  └─ SYNC.md                    # 跨设备同步的设计（参考 Resilio Sync，尚未实现）
└─ src-tauri/                    # Rust 后端
   ├─ Cargo.toml
   ├─ tauri.conf.json
   ├─ capabilities/default.json  # 权限（核心 API / 文件对话框 / 打开链接）
   └─ src/
      ├─ main.rs
      ├─ lib.rs                  # 启动流程 + 命令注册
      ├─ state.rs                # AppState 与 spawn_blocking 辅助
      ├─ error.rs                # 统一错误类型（可直接序列化给前端）
      ├─ scanner.rs              # 目录递归 + 增量索引（通用图库）
      ├─ artworks.rs             # PID 解析 + 作品维度索引（阶段一）
      ├─ artists.rs              # 画师从路径反推（含「为什么 uid 才是身份」）
      ├─ pixiv.rs                # 网页接口客户端 + 代理 + 限流分类（不碰 OAuth）
      ├─ wallpaper.rs            # 设为桌面壁纸（Windows FFI + 格式转码）
      ├─ thumbnails.rs           # 缩略图生成与磁盘缓存（有损 WebP）
      ├─ db/
      │  ├─ mod.rs               # 连接、PRAGMA、时间工具
      │  ├─ schema.rs            # 版本化迁移
      │  └─ models.rs            # 数据模型与查询参数
      └─ commands/               # Tauri 命令，按领域拆分
         ├─ mod.rs
         ├─ library.rs
         ├─ pixiv.rs             # 批量补全元数据（限流 / 中止 / 进度事件）
         ├─ images.rs
         ├─ albums.rs
         ├─ tags.rs
         ├─ artworks.rs          # 阶段一 / 二：作品扫描、分页、缩略图、筛选拼装
         ├─ tag_galleries.rs     # 标签图库：自建图库的增删查 + 分级计数
         ├─ media.rs             # Android 桥命令：沉浸式 / SAF 选目录 / 保活服务 / 子目录列举
         └─ system.rs
```

## 测试

```bash
npm test                      # 前端：两套排版的几何共 22 个用例（定高网格 10 + 瀑布流 12）
cd src-tauri && cargo test    # 后端：数据库迁移、去重、画师解析、PID 解析等
npm run check                 # svelte-check 类型检查
```

前端不引测试框架：Node 能原生跑 TS，几个断言加退出码就够，不值得为此塞一个 vitest 进来。
几何全部落在 `virtualGeometry.ts` 的纯函数里 —— 最容易出错的正是「列数 → 格宽 → 行高 →
可视区间 → 定位偏移」这条链，算错了通常只表现为滚动条轻微抖动或偶尔白屏，靠肉眼很难发现。

## 构建与发行

**桌面（Windows）**：

```bash
npm run tauri:build
# 产物：src-tauri/target/release/bundle/msi/*.msi 与 bundle/nsis/*-setup.exe
```

打包前要补的配置、代码签名、自动更新，都写在 [docs/RELEASE.md](docs/RELEASE.md) 里。

**Android**：

```bash
npm run tauri -- android build --target aarch64            # release，28MB，日常用这个
npm run tauri -- android build --target aarch64 --debug    # debug，~700MB，只在要调 WebView 时用
```

| 构建 | 体积 | 用途 |
| --- | --- | --- |
| **release** | **28MB** | 装到手机上日常用。已 strip + LTO + R8，比 pixez（Flutter，53MB）还小一半 |
| debug | 360–720MB | 只在需要用 Chrome DevTools 连 WebView 调试时用 —— **只有 debuggable 的包才开 `webview_devtools_remote` 端口** |

**debug 包为什么能大到 700MB**：`app/build.gradle.kts` 的 debug 构建类型明确要求
`jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")`，于是那个 `.so` **不剥离符号**，
单个文件 354MB；打包偶尔还会在它前面留一段等长的零字节预留区（所以 `unzip -l` 的
内容总和只有 362MB，文件却是 718MB）。**这跟应用本身多大没关系**，release 一 strip
就掉到 27.9MB。

三个容易踩的点：

- **`ANDROID_HOME` 必须指向 SDK 根**（本机 `F:\Android\Sdk`）。没设它时 Tauri CLI 会报
  「Android SDK not found」并去 `%LOCALAPPDATA%\Android\Sdk` 找 —— 本机那个路径下
  只有一个**空目录**，看到它别误判成「SDK 没装」。
- **`src-tauri/gen/android/local-maven/` 里那份 19KB 的 `rustls-platform-verifier` AAR
  要跟着仓库走**。Android 上 `reqwest` 靠它做系统证书校验，缺了会在运行时报
  「failed to call native verifier」（编译期完全看不出来）。
- **release 的签名配置在 `gen/android/keystore.properties`**（该文件被 gitignore）。
  现在指向 Android 调试密钥库 —— 刻意为之：release 包与 debug 包**签名相同**，
  可以直接覆盖升级，**不用卸载**（卸载会连带清掉应用数据：索引、标签、元数据）。
  将来换正式 keystore 就必须先卸载重装。

> **R8 会吃掉 Kotlin 桥，而 debug 包不会暴露这个问题。** Tauri 的 Android 插件全靠反射：
> 按**类名字符串**建插件实例、再按**方法名**匹配 `@Command`。R8 从代码里看不见这些引用，
> 默认会把整个类删掉 —— 而 debug 包不混淆所以完全正常。保护规则在
> `media-store-bridge/consumer-rules.pro`（这个文件原先被声明却不存在，
> 靠 `android.proguard.failOnMissingFiles=false` 掩盖着）。改完记得用
> `apkanalyzer dex packages --defined-only <apk> | grep lightcone` 确认那几个命令方法还在。

## 文档

| 文件 | 内容 |
| --- | --- |
| [docs/DESIGN-NOTES.md](docs/DESIGN-NOTES.md) | 数据库每一版的来由、后端命令一览、关键实现说明、完整的已知限制清单 |
| [docs/RELEASE.md](docs/RELEASE.md) | 桌面与 Android 的发行清单 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 协作规矩：分支模型、**哪几类改动要先打招呼**（迁移、`gen/android`、出包）、签名与真机验收由谁做 |
| [docs/SYNC.md](docs/SYNC.md) | 跨设备同步（共用收藏 / 元数据互通）的设计：配对与加密、发现、合并规则、分三期的计划。**尚未实现**，先把结论写下来免得重新推一遍 |
| `HANDOFF.md` / `HANDOFF-PHASE1.md` | 开发过程中的交接笔记（内部资料，`HANDOFF-PHASE1` 已过期） |

## 值得先知道的几条限制

- **联网元数据要分批补**：单次上限 300 条、每条间隔 800ms，2653 条约需 35 分钟连续请求。
  没补过的作品在弹窗里显示「这条还没取过元数据」，而不是「没有标签」——这两件事必须分清。
- **国内必须走代理**：直连 pixiv 不通，不填代理时批量抓取会连续失败并中止。
  走代理意味着出口 IP 是共享的，别人撞脏了会影响你，反之亦然。
- **HEIC / RAW 无法索引**：`image` crate 只启用了 jpeg / png / gif / webp / bmp / tiff / ico，
  这些文件会计入 `skipped` 而不是被静默丢弃。
- **EXIF 尚未解析**：`images.taken_at` 字段已预留但当前为 `NULL`，排序回退到文件修改时间。
- **「设为壁纸」的「锁屏」在个别设备/系统上会被拒**：厂商 ROM 和企业策略都可能禁掉它，
  两端都会把拒绝原因原样显示在按钮的 tooltip 里。另外 **Windows 的锁屏那条路没有自动化
  测试覆盖** —— 测一次就会换掉用户的锁屏图，而 Windows 没有「读回当前锁屏图路径」的正规
  接口，换掉了还原不回去。测试只验到 `IsSupported()`（WinRT 套间初始化 + 类型激活）。
- **两套虚拟化并存**：通用图库只有定高网格（`VirtualGrid`）；作品库两套都有
  （`VirtualGrid` + `VirtualMasonry`），靠工具条上的「网格 / 瀑布流」切。
  瀑布流比定高网格多一个硬前提 —— **必须先拿到全部作品的高宽比**才算得出滚动条长度
  （总高 = 最高那一列有多高），所以它走一条单独的 `list_artwork_aspects` 命令，
  一次性返回全表（4430 条约 70KB）。老库的宽高由这条命令首次调用时现量
  （读缩略图文件头，多线程，量文件头在数据库锁之外）+ 写回 `artworks.width/height`，
  只发生一次；网格模式下一分钱都不花。
- **改缩略图尺寸会触发一轮重算，而且是「滑到哪张补哪张」**：缓存按尺寸分目录，
  所以改了边长之后库里存的还是旧路径 —— 文件在、图能打开，只是分辨率不够，**不报错**。
  列表把这种路径当「没有缩略图」返回，卡片于是走一次重生成回退，顺手把新尺寸写回库，
  **并删掉同一个 id 在旧尺寸目录下的那份**（不删就是白留一整份，本机实测 4430 张 91MB）。
  想一次性重算完则重扫一次目录，那条路会走带进度条的批量生成。
- **`surface-900 / 850 / 950` 是半透明的**（`color-mix` 出来的，为了透出窗口材质）。
  铺在 `body` 上当页面底色没问题，但**浮层一旦盖在内容上，底下的文字会直接透出来
  看得见** —— 而且层级完全正确，`elementsFromPoint` 查出来浮层就在最上面，
  很容易往错的方向查。全站的做法是浮层一律 `bg-surface-900/95 + backdrop-blur`。
- **给容器加动效前先想清楚它里面有没有 `fixed` 后代**：`transform` / `filter` /
  `backdrop-filter` / `contain` 都会把元素变成固定定位的**包含块**，其后代里所有
  `position: fixed` 的浮层会改为相对它定位、整块跑到屏幕中间。所以视图切换那层只能
  用 `opacity`（只建栈上下文，不建包含块），而排版切换那层可以用位移 —— 因为网格容器
  里没有 `fixed` 后代，浮层都是它的兄弟节点。
- **作品库与通用图库是两套并存的索引**：同一个目录两边都索引会存两份缩略图，
  合并需要给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。

完整清单（含每条的对策与扩展点）在
[docs/DESIGN-NOTES.md](docs/DESIGN-NOTES.md#已知限制与扩展点)。

## 许可

MIT，见仓库根目录的 [LICENSE](LICENSE)，与 `package.json` 的声明一致。
