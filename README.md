# LightCone Gallery · 光锥映画

**本地优先的 Pixiv 图库桌面应用。** 原图留在你自己的目录里、应用只读不写；索引与元数据
全部存在本机 SQLite；联网只用来补 Pixiv 的标题与标签，而且可选 —— 不填代理，其余功能照常。

界面走「机箱陈列」式的工业配色。检视弹窗是一套纯 CSS 的三层光锥视差：没有 Three.js，
没有 WebGL，`transform-style: preserve-3d` 加几层 Z 轴就够了。

> **发行状态**：桌面（Windows）现在就能打包出安装包；**Android 端尚未适配** ——
> 差的不是一条命令，是文件访问模型，详见 [docs/RELEASE.md](docs/RELEASE.md#二android)。

## 能做什么

**通用图库**（`/`）
- 添加任意根目录，增量扫描：先把 `path -> (大小, mtime)` 读进内存快照，只对新增 / 变化的文件写库
- 搜索、排序、收藏、评分；相册与标签两套手动组织方式
- 从图库移除只删索引，不动磁盘上的文件

**作品库**（侧栏「作品库 (PID)」，与「全部图片 / 收藏」同款的原地切换）
- 只收录文件名符合 `{pid}_p{num}` 的图片，按 Pixiv 作品号归类，一行代表「某作品的某一页」
- 定高网格虚拟滚动：5000 条数据单屏最多渲染 45 个格子，元数据按 120 条一块拉取
- 缩略图在扫描阶段就预生成好并写回数据库，列表自带地址 —— 3000 条整趟滚动下来缩略图相关 IPC 只有 1 次

**画师维度**
- 从目录名离线反推（pixez 这类下载器落盘时就是 `<画师名>_<画师ID>/<pid>_p<n>.<ext>`），零网络成本
- 按 **uid** 归组而不是名字：同一 uid 在本地出现过 12 组不同名字（改过名、或把展会后缀写进了目录名）

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
- 全屏、复制图片到剪贴板、左右翻页

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
npm run check    # svelte-check 类型检查
npm test         # 前端自检：虚拟滚动几何（Node 24 原生跑 TS，未引入测试框架）
cd src-tauri && cargo test   # 后端测试
```

## 数据放在哪

| 内容 | 路径 |
| --- | --- |
| 数据库 | `<AppData>/com.lightcone.gallery/gallery.db`（Windows：`%APPDATA%\com.lightcone.gallery\`） |
| 缩略图缓存 | `<AppData>/com.lightcone.gallery/thumbnails/<归属域>/<尺寸>/<id>.webp`（作品库用 400） |

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
│  │  ├─ thumbnailQueue.ts       # 缩略图请求的并发闸门
│  │  ├─ format.ts               # 展示层格式化
│  │  └─ components/
│  │     ├─ Sidebar.svelte       # 图库 / 相册 / 标签导航
│  │     ├─ Toolbar.svelte       # 搜索、排序、扫描
│  │     ├─ ImageGrid.svelte     # 通用图库网格 + 无限滚动
│  │     ├─ ImageCard.svelte     # 单张卡片
│  │     ├─ VirtualGrid.svelte   # 定高网格虚拟滚动（纯 CSS，无滚动库）
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
│  └─ virtual-geometry.test.ts   # 虚拟滚动几何自检（npm test）
├─ docs/
│  ├─ DESIGN-NOTES.md            # 数据库设计、命令清单、实现说明与踩过的坑
│  └─ RELEASE.md                 # 桌面 + Android 发行清单
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
         ├─ artworks.rs          # 阶段一 / 二：作品扫描、分页、缩略图
         └─ system.rs
```

## 测试

```bash
npm test                      # 前端：虚拟滚动几何 10 个用例
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

打包前要补的配置、代码签名、自动更新，以及**Android 端需要先做什么**，
都写在 [docs/RELEASE.md](docs/RELEASE.md) 里。一句话概括 Android：本机工具链全缺
（SDK / NDK / CMake / 合适的 JDK / 四个 Rust target），而且 `tauri-plugin-dialog`
在 Android 上不支持目录选择、返回的是 `content://` URI —— 现在这套
「选目录 → walkdir → `std::fs` → `asset://`」的链路在 Android 上跑不通，
得先决定走「侧载 + 所有文件访问」还是「SAF 重写」。

## 文档

| 文件 | 内容 |
| --- | --- |
| [docs/DESIGN-NOTES.md](docs/DESIGN-NOTES.md) | 数据库每一版的来由、后端命令一览、关键实现说明、完整的已知限制清单 |
| [docs/RELEASE.md](docs/RELEASE.md) | 桌面与 Android 的发行清单 |
| `HANDOFF.md` / `HANDOFF-PHASE1.md` | 开发过程中的交接笔记（内部资料，`HANDOFF-PHASE1` 已过期） |

## 值得先知道的几条限制

- **联网元数据要分批补**：单次上限 300 条、每条间隔 800ms，2653 条约需 35 分钟连续请求。
  没补过的作品在弹窗里显示「这条还没取过元数据」，而不是「没有标签」——这两件事必须分清。
- **国内必须走代理**：直连 pixiv 不通，不填代理时批量抓取会连续失败并中止。
  走代理意味着出口 IP 是共享的，别人撞脏了会影响你，反之亦然。
- **HEIC / RAW 无法索引**：`image` crate 只启用了 jpeg / png / gif / webp / bmp / tiff / ico，
  这些文件会计入 `skipped` 而不是被静默丢弃。
- **EXIF 尚未解析**：`images.taken_at` 字段已预留但当前为 `NULL`，排序回退到文件修改时间。
- **虚拟滚动是定高网格**：格子必须同宽同高（`object-cover` 裁切），做瀑布流要另写组件。
- **作品库与通用图库是两套并存的索引**：同一个目录两边都索引会存两份缩略图，
  合并需要给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。

完整的三十来条（含每条的对策与扩展点）在
[docs/DESIGN-NOTES.md](docs/DESIGN-NOTES.md#已知限制与扩展点)。

## 许可

`package.json` 里声明的是 MIT，但仓库里**还没有 LICENSE 文件** —— 发行前补上。
