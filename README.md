# LightCone Gallery

基于 **Tauri 2 + Svelte 5 + Vite + TypeScript + Tailwind CSS** 的本地图库桌面应用，
图片索引与元数据全部存放在本机 SQLite（Rust 端通过 **rusqlite** 操作）。

## 技术栈

| 层 | 选型 | 说明 |
| --- | --- | --- |
| 桌面外壳 | Tauri 2 | 使用系统 WebView（Windows 上是 WebView2），产物体积小 |
| 前端框架 | SvelteKit + Svelte 5 | `adapter-static` 的 SPA 模式，SSR 关闭 |
| 构建 | Vite 8 | 开发端口固定 1420 |
| 语言 | TypeScript（严格模式） | 前端类型与 Rust 模型手工对齐 |
| 样式 | Tailwind CSS 4 | 走 `@tailwindcss/vite` 插件，无需 `tailwind.config.js` |
| 数据库 | SQLite + rusqlite | `bundled` 特性内嵌 SQLite 源码，用户机器无需装 sqlite3 |
| 图像处理 | `image` + `webp` | `image` 负责解码与缩放，`webp`（libwebp 绑定）负责**有损** WebP 编码 |
## 环境要求

- **Node.js** ≥ 20
- **Rust** stable（用 `rustup` 安装）
- **Windows**：Visual Studio Build Tools（含 MSVC v143+ 与 Windows SDK）、WebView2 Runtime
- **macOS**：Xcode Command Line Tools
- **Linux**：`webkit2gtk-4.1`、`libappindicator3`、`librsvg2` 等

首次运行 `npm run tauri:dev` 需要编译约 400 个 crate，耗时较长；之后走增量缓存。

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
│  │     ├─ ArtworkThumbnail.svelte # 直接用列表里的 thumbnailPath + 失败回退
│  │     ├─ ArtworkLightbox.svelte  # 检视弹窗（唯一加载原图的地方）
│  │     ├─ Thumbnail.svelte     # 通用图库的懒加载缩略图
│  │     └─ Lightbox.svelte      # 大图查看 + 标签 / 评分编辑
│  └─ routes/
│     ├─ +layout.svelte
│     ├─ +page.svelte            # 通用图库（网格 / 搜索 / 标签 / 相册）
│     └─ artworks/+page.svelte   # 作品库（PID 维度，虚拟滚动缩略图网格）
├─ tests/
│  └─ virtual-geometry.test.ts   # 虚拟滚动几何自检（npm test）
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
      ├─ thumbnails.rs           # 缩略图生成与磁盘缓存（有损 WebP）
      ├─ db/
      │  ├─ mod.rs               # 连接、PRAGMA、时间工具
      │  ├─ schema.rs            # 版本化迁移
      │  └─ models.rs            # 数据模型与查询参数
      └─ commands/               # Tauri 命令，按领域拆分
         ├─ library.rs
         ├─ images.rs
         ├─ albums.rs
         ├─ tags.rs
         ├─ artworks.rs          # 阶段一 / 二：作品扫描、分页、缩略图
         └─ system.rs
```

## 数据存放位置

| 内容 | 路径 |
| --- | --- |
| 数据库 | `<AppData>/com.lightcone.gallery/gallery.db`（Windows：`%APPDATA%\com.lightcone.gallery\`） |
| 缩略图缓存 | `<AppData>/com.lightcone.gallery/thumbnails/<归属域>/<尺寸>/<id>.webp`（作品库用 400） |

`<归属域>` 取 `images` 或 `artworks`。分目录不是洁癖：`images.id` 与 `artworks.id` 是两张表各自的自增主键，共用一层目录会让 `images#1` 和 `artworks#1` 落到同一个文件上、互相覆盖缩略图。

原图始终留在用户自己的目录里，应用**只读不写**；从图库移除记录不会删除磁盘文件。

## 数据库设计

迁移基于 `PRAGMA user_version`，见 `src-tauri/src/db/schema.rs`。当前为 **v2**：

**通用图库（v1）**

- `libraries` — 用户添加的根目录
- `images` — 索引到的图片（`path` 唯一，含尺寸、大小、mtime、收藏、评分）
- `albums` / `album_images` — 手动组织的相册，`position` 保留加入顺序
- `tags` / `image_tags` — 用于交叉筛选的标签
- `thumbnails` — 缩略图缓存登记表
- `settings` — 预留的键值设置

**作品维度（v2 新增，v3 改列名）**

- `artworks` — 只收录文件名符合 `{pid}_p{num}` 的图片，一行代表「某作品的某一页」；`original_path` 是原图，`thumbnail_path` 是扫描阶段预生成的缩略图
- `artwork_roots` — 作品库扫描过的根目录

`artworks` 与 `images` 是**两套并存的索引**：`images` 是通用图库（标签 / 相册 / 缩略图挂在它上面），`artworks` 只做作品维度。两者互不影响，所以作品库的新增不会动到已有图库功能。

> **唯一约束是 `UNIQUE(pid, page)` 而不是 `UNIQUE(pid)`。** 这条不能改：`123456_p0.jpg` 与 `123456_p1.jpg` 的 pid 相同，若只对 pid 加唯一索引，多页作品从第 2 页起会被 `INSERT OR IGNORE` **静默丢弃**且不报错。回归测试见 `db::schema::tests::artworks_dedupes_by_pid_and_page_not_by_pid_alone`。

> **v3 的三处改动各有理由**：`file_path` 改名 `original_path`，是因为查询命令对外返回的就是 `pid / original_path / thumbnail_path`，列名跟着走免得每层翻译一次；新增 `thumbnail_path` 让列表自带缩略图地址，卡片不必再往返 IPC；`artwork_roots` 落盘是因为 `asset://` 的作用域**只存在于内存中**（见启动流程），不记下来就没法在重启后重新授权原图目录。
>
> 启动时还有一步幂等修复：如果库里有作品行但 `artwork_roots` 是空的（老库升级上来就是这个状态），就从现有的 `original_path` 反推所在目录补登一次。不做的话那些原图不在授权范围内，弹窗会莫名加载失败。登记过就只花一次 `COUNT` 查询。

外键均带 `ON DELETE CASCADE`，连接建立时开启 `foreign_keys`；写库使用 WAL 与 `synchronous=NORMAL`。

**新增迁移时**：把 `SCHEMA_VERSION` 加一，并**追加**一个新的 `if version < N` 分支，不要修改历史分支。

## 后端命令一览

所有命令都在 `lib.rs` 的 `generate_handler!` 中注册，前端封装见 `src/lib/api.ts`。

| 分类 | 命令 |
| --- | --- |
| 图库 | `list_libraries` `add_library` `remove_library` `scan_library` `scan_all_libraries` |
| 图片 | `list_images` `get_image` `toggle_favorite` `set_rating` `thumbnail_for` |
| 相册 | `list_albums` `create_album` `rename_album` `delete_album` `add_images_to_album` `remove_images_from_album` `set_album_cover` |
| 标签 | `list_tags` `create_tag` `update_tag` `delete_tag` `add_tag_to_image` `remove_tag_from_image` |
| 系统 | `library_stats` `thumbnail_cache_usage` `clear_thumbnail_cache` |
| 作品（阶段一 / 二） | `scan_artworks_folder` `list_artworks` `thumbnail_for_artwork` |

扫描期间后端通过 `scan://progress` 事件推送进度（发现阶段 + 索引阶段），前端在 `gallery.svelte.ts` 中订阅。

`scan_artworks_folder(path)` 接收一个文件夹路径：遍历图片、解析 `{pid}_p{num}` 入库，**再为缺缩略图的作品批量生成 WebP 并把路径写回数据库**。返回的 `inserted` / `duplicates` 直接体现去重效果（重复扫描同一目录时 `inserted` 为 0），`thumbnailsGenerated` / `thumbnailsSkipped` / `thumbnailsFailed` 反映本轮缩略图 pass 的产出。`list_artworks(offset, limit)` 分页返回 `pid / original_path / thumbnail_path`，`limit` 上限 200，按 pid 数值倒序（`CAST(pid AS INTEGER)`，否则字符串排序会让 `999` 排在 `12345` 之后）。`thumbnail_for_artwork(id)` 只是自愈回退，正常路径上前端不会调它。

## 关键实现说明

**并发模型**：rusqlite 是同步 API，直接在异步命令里调用会占住 async runtime 的工作线程。因此所有数据库与图像解码操作统一经 `state::with_db` / `tauri::async_runtime::spawn_blocking` 丢到阻塞线程池，命令本身是 `async fn`。

**单连接 + Mutex**：图库场景下写入集中在索引阶段，读取都是小查询。单连接配合 WAL 已经足够，也避免了连接池与 rusqlite 的版本耦合。若要并行读取，把 `AppState` 里的连接换成 r2d2 池即可，命令层无需改动。

**图片显示**：走 Tauri 的 `asset://` 自定义协议（`convertFileSrc`），而不是把图片转 base64 走 IPC。该协议的允许目录只存在于内存中，因此 `lib.rs` 在每次启动时都会为已登记的图库根目录与缩略图目录重新授权。

**增量索引**：扫描时先把该图库的 `path -> (size, mtime)` 读进内存快照，只对新增 / 变化的文件写库，再清理磁盘上已消失的记录。单张图片的尺寸用 `image_dimensions` 读取——只解析文件头，不解码整图。

**缩略图缓存**：`<app data>/thumbnails/<归属域>/<size>/<id>.webp`，命中条件是「缓存不早于原图」，因此原图被替换后会自动重算。允许的尺寸白名单在 `thumbnails::ALLOWED_SIZES`。

**为什么缩略图是有损 WebP**：`image` crate 自带的 WebP 编码器只实现 VP8L 无损（其 `lib.rs` 的格式表里写着 *Only lossless encoding is currently supported*），无损压照片会比 JPEG 更大，做缩略图没有意义。因此编码走 `webp`（libwebp 绑定）的有损通道，质量 80。400px 缩略图实测：平滑插画感的画面约为 JPEG q82 的 **15%**，含噪照片感约 **71%** —— 插画越平滑，WebP 赢得越多。附带好处是 WebP 支持 alpha，PNG / ICO 的透明区域不再被压掉。

**缩略图在扫描阶段预生成**：`generate_thumbnails` 在索引之后跑一遍，把缺图的作品批量补齐并把路径回填数据库。它是幂等的 —— 缓存仍然新鲜的一律跳过（`is_cached` 比对文件与修改时间），所以重复扫描同一目录几乎不花时间；缓存文件被手动删掉也会被它重新补齐。

**缩略图生成是并行且可观测的**：解码 / 缩放 / 编码全在 CPU 上，实测单张约 23ms，串行跑一万张要近四分钟。`thumbnails::ensure_parallel` 用 `std::thread::scope` + 原子游标摊到最多 8 个线程（没有为此引入 rayon），SQLite 写入留在主线程串行落库。进度经 `artworks://thumbnail-progress` 事件推送，按 25 张节流一次，前端在扫描按钮上显示 `生成缩略图 320/1374`。

**列表自带缩略图地址**：`thumbnail_path` 在扫描阶段就写进库，`list_artworks` 随每条记录一起返回，卡片直接用 `<img src>` 渲染 —— **实测 3000 条数据整趟滚动下来，缩略图相关的 IPC 只有 1 次**（而且那一次是刻意造的坏图触发的回退）。旧设计（每张卡片问一次后端）同一场景要发几百次请求。`thumbnail_for_artwork` 保留为自愈回退：缓存被清空后 `<img>` 会加载失败，卡片此时才调一次它把图补回来。

**原图只在弹窗里加载**：列表永远是缩略图；点击卡片后 `ArtworkLightbox` 才用 `original_path` 取原图。原图目录必须登记在 `artwork_roots` 里并在每次启动时重新授权 `asset://`（该作用域不持久化），扫描时也会立刻授权一次，免得用户得重启才能看到原图。

**虚拟滚动**（阶段二）：`/artworks` 用定高网格虚拟滚动，只把视口内的行渲染成真实 DOM，用一块总高度占位撑住滚动条比例。几何全部落在 `virtualGeometry.ts` 的纯函数里，`npm test` 有 10 个用例覆盖——最容易出错的正是「列数 → 格宽 → 行高 → 可视区间 → 定位偏移」这条链，算错了通常只表现为滚动条轻微抖动或偶尔白屏，靠肉眼很难发现。实测 5000 条数据单屏最多渲染 45 个格子。

**元数据分块 + 像素懒加载**：元数据廉价、像素昂贵。列表按 120 条一块向后端要（`BLOCK_SIZE`），虚拟滚动只请求可视区覆盖到的块；缩略图则是格子进入视口附近才生成。所以滚动开销与列表长度解耦。一次从顶到底的实测：13 次分块请求、399 次缩略图请求（去重后 344 个作品），而不是 5000 次。

**缩略图请求闸门**：`thumbnailQueue.ts` 用固定并发度 4 的先进先出队列压住在途请求（实测并发峰值恰好为 4）。两个地方会用到它：通用图库的 `Thumbnail.svelte`（那边仍然是每张卡片问一次后端），以及作品库的失败回退 —— 缓存被清空时整屏卡片的 `<img>` 会同时报错，不拦一下就会同时抛上百个「解码整图 → 缩放 → 编码」任务，内存峰值能到 GB 级。

**观察器的 root 必须指向滚动容器**：`IntersectionObserver` 的 `rootMargin` 只在 root 上生效。把 root 留空（默认视口）时，滚动容器自身的裁剪会把 `rootMargin` 完全吃掉，于是「提前一屏预取」永远不会发生——实测可见缩略图数会从 25 掉到 15。所以 `Thumbnail.svelte` 会向上找到最近的滚动祖先再建观察器。

**虚拟滚动的复用陷阱**：格子按索引作 key，`Thumbnail` 会拿到新 `id` 却沿用旧实例。因此它在 `id` 变化时会清空 `source` / `failed` / `requestedId`，并用「请求期间 id 是否又被换掉」丢弃过期响应。另外 `VirtualGrid` 的格子带 `data-index`——虚拟滚动的 DOM 没有位置语义，这是唯一能把节点和数据下标对上的凭据。

**SQL 注入防护**：筛选条件全部走 `?` 占位符 + `Vec<Value>` 绑定；排序字段通过 `SortBy::column()` 的白名单映射落地，不拼接前端传来的字符串。

**退出时 checkpoint**：Tauri 的退出路径会直接结束进程、跳过 Rust 析构，`Connection` 不会走 `Drop`，WAL 也就永远合并不回主库（实测退出后主库仍停在 4096 字节、WAL 有 156 KB）。因此 `lib.rs` 用 `app.run(|handle, event| ...)` 捕获 `RunEvent::Exit`，在退出前显式执行 `PRAGMA wal_checkpoint(TRUNCATE)`。这样主库文件始终是自洽的——单独备份 `gallery.db` 不会丢数据。

## 已知限制与扩展点

- **EXIF 尚未解析**：`images.taken_at` 字段已预留但当前为 `NULL`，排序回退到文件修改时间。接入 `kamadak-exif` 后写入该列即可启用「按拍摄时间排序」。
- **HEIC / RAW 无法索引**：`image` crate 只启用了 jpeg / png / gif / webp / bmp / tiff / ico。扫描时这些文件会计入 `skipped` 而不是被静默丢弃。要支持需更换解码后端（如 `libheif-rs`）。
- **换成 WebP 后旧缓存会变孤儿**：缓存路径从 `<尺寸>/<id>.jpg` 改成 `<归属域>/<尺寸>/<id>.webp`，旧 JPEG 不会再被读取，但会留在磁盘上。侧栏底部的「清理」按钮可以一次性清掉，下次浏览按需重建。
- **虚拟滚动是定高网格**：格子尺寸只由「列数 × 容器宽度」推出，所以所有格子必须同宽同高（`object-cover` 裁切）。要做行高不齐的瀑布流得另写组件——那需要逐格测量与位置缓存，本项目的 3D 视差弹窗（阶段四）不依赖它。
- **扫描会连带生成缩略图，因此首次索引慢**：约 23ms/张（并行后 2000 张约 12s、10000 张约 58s）。这是「扫描时预生成」换来的浏览零等待。若更在意扫描速度，把 `generate_thumbnails` 从 `scan_artworks_folder` 里摘出去、改成按需调用即可，缓存与回退逻辑都不用动。
- **检视弹窗还不能翻页**：只有单张显示 + Esc / 点空白关闭。RhinE 工业科技风的重构与 3D 视差弹窗是阶段三 / 四的内容。
- **作品库不刷新已移动文件的路径**：`scan_artworks_folder` 用 `ON CONFLICT DO NOTHING` 实现去重（这也让 `duplicates` 计数能如实反映去重效果），代价是文件被移动后 `original_path` 仍指向旧位置，缩略图会随之变成「源文件缺失」。若要自动刷新，把冲突分支改成 `DO UPDATE SET original_path = excluded.original_path`，但那样就分不清「新增」和「更新」了。
- **扫描是单线程的**：`scanner.rs`（通用图库）顺序遍历；作品库那边的**缩略图生成**已经并行（见 `ensure_parallel`），但目录遍历本身仍是串行。目录很大时可引入 `rayon` 并行读取元数据，注意 SQLite 写入仍需串行。
- **CSP 当前关闭**：`tauri.conf.json` 中 `security.csp` 为 `null` 以便开发；发布前应配置严格 CSP，并把 `asset:` 加入 `img-src`。
- **移除图库不会清理缩略图缓存**：缓存可能残留孤儿文件。侧栏底部显示缓存占用，旁边的「清理」按钮调用 `clear_thumbnail_cache` 一次性清空（作品库下次浏览会按需重建）。
- **作品库与通用图库是两套索引**：同一个目录既可以被「添加图库」索引进 `images`，也可以被「作品库扫描」索引进 `artworks`，两边的元数据不共享。若后续要合并成一套，需要决定是否给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。
