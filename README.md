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
| 图像处理 | `image` crate | 只解析文件头读尺寸；缩略图按需生成并缓存 |

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
```

## 目录结构

```
├─ src/                          # SvelteKit 前端
│  ├─ app.css                    # Tailwind 入口 + 设计令牌（@theme）
│  ├─ app.html
│  ├─ lib/
│  │  ├─ api.ts                  # 所有 invoke 调用的类型化封装
│  │  ├─ types.ts                # 与 Rust models.rs 对齐的类型定义
│  │  ├─ gallery.svelte.ts       # 全局状态（Svelte 5 runes）
│  │  ├─ format.ts               # 展示层格式化
│  │  └─ components/
│  │     ├─ Sidebar.svelte       # 图库 / 相册 / 标签导航
│  │     ├─ Toolbar.svelte       # 搜索、排序、扫描
│  │     ├─ ImageGrid.svelte     # 网格 + 无限滚动
│  │     ├─ ImageCard.svelte     # 单张卡片
│  │     ├─ Thumbnail.svelte     # 懒加载缩略图
│  │     └─ Lightbox.svelte      # 大图查看 + 标签 / 评分编辑
│  └─ routes/
│     ├─ +layout.svelte
│     ├─ +page.svelte            # 通用图库（网格 / 搜索 / 标签 / 相册）
│     └─ artworks/+page.svelte   # 作品库（PID 维度，只列路径不加载原图）
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
      ├─ thumbnails.rs           # 缩略图生成与磁盘缓存
      ├─ db/
      │  ├─ mod.rs               # 连接、PRAGMA、时间工具
      │  ├─ schema.rs            # 版本化迁移
      │  └─ models.rs            # 数据模型与查询参数
      └─ commands/               # Tauri 命令，按领域拆分
         ├─ library.rs
         ├─ images.rs
         ├─ albums.rs
         ├─ tags.rs
         ├─ artworks.rs          # 阶段一：作品扫描与分页
         └─ system.rs
```

## 数据存放位置

| 内容 | 路径 |
| --- | --- |
| 数据库 | `<AppData>/com.lightcone.gallery/gallery.db`（Windows：`%APPDATA%\com.lightcone.gallery\`） |
| 缩略图缓存 | `<AppData>/com.lightcone.gallery/thumbnails/<尺寸>/<图片 id>.jpg` |

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

**作品维度（v2，阶段一新增）**

- `artworks` — 只收录文件名符合 `{pid}_p{num}` 的图片，一行代表「某作品的某一页」

`artworks` 与 `images` 是**两套并存的索引**：`images` 是通用图库（标签 / 相册 / 缩略图挂在它上面），`artworks` 只做作品维度。两者互不影响，所以作品库的新增不会动到已有图库功能。

> **唯一约束是 `UNIQUE(pid, page)` 而不是 `UNIQUE(pid)`。** 这条不能改：`123456_p0.jpg` 与 `123456_p1.jpg` 的 pid 相同，若只对 pid 加唯一索引，多页作品从第 2 页起会被 `INSERT OR IGNORE` **静默丢弃**且不报错。回归测试见 `db::schema::tests::artworks_dedupes_by_pid_and_page_not_by_pid_alone`。

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
| 作品（阶段一） | `scan_artworks_folder` `list_artworks` |

扫描期间后端通过 `scan://progress` 事件推送进度（发现阶段 + 索引阶段），前端在 `gallery.svelte.ts` 中订阅。

`scan_artworks_folder(path)` 接收一个文件夹路径，遍历其中的图片、解析 `{pid}_p{num}` 后写入 `artworks` 表，返回的 `inserted` / `duplicates` 直接体现去重效果（重复扫描同一目录时 `inserted` 为 0）。`list_artworks(offset, limit)` 分页返回，`limit` 上限 200，按 pid 数值倒序（`CAST(pid AS INTEGER)`，否则字符串排序会让 `999` 排在 `12345` 之后）。

## 关键实现说明

**并发模型**：rusqlite 是同步 API，直接在异步命令里调用会占住 async runtime 的工作线程。因此所有数据库与图像解码操作统一经 `state::with_db` / `tauri::async_runtime::spawn_blocking` 丢到阻塞线程池，命令本身是 `async fn`。

**单连接 + Mutex**：图库场景下写入集中在索引阶段，读取都是小查询。单连接配合 WAL 已经足够，也避免了连接池与 rusqlite 的版本耦合。若要并行读取，把 `AppState` 里的连接换成 r2d2 池即可，命令层无需改动。

**图片显示**：走 Tauri 的 `asset://` 自定义协议（`convertFileSrc`），而不是把图片转 base64 走 IPC。该协议的允许目录只存在于内存中，因此 `lib.rs` 在每次启动时都会为已登记的图库根目录与缩略图目录重新授权。

**增量索引**：扫描时先把该图库的 `path -> (size, mtime)` 读进内存快照，只对新增 / 变化的文件写库，再清理磁盘上已消失的记录。单张图片的尺寸用 `image_dimensions` 读取——只解析文件头，不解码整图。

**缩略图缓存**：`<app data>/thumbnails/<size>/<id>.jpg`，命中条件是「缓存不早于原图」，因此原图被替换后会自动重算。允许的尺寸白名单在 `thumbnails::ALLOWED_SIZES`。

**SQL 注入防护**：筛选条件全部走 `?` 占位符 + `Vec<Value>` 绑定；排序字段通过 `SortBy::column()` 的白名单映射落地，不拼接前端传来的字符串。

**退出时 checkpoint**：Tauri 的退出路径会直接结束进程、跳过 Rust 析构，`Connection` 不会走 `Drop`，WAL 也就永远合并不回主库（实测退出后主库仍停在 4096 字节、WAL 有 156 KB）。因此 `lib.rs` 用 `app.run(|handle, event| ...)` 捕获 `RunEvent::Exit`，在退出前显式执行 `PRAGMA wal_checkpoint(TRUNCATE)`。这样主库文件始终是自洽的——单独备份 `gallery.db` 不会丢数据。

## 已知限制与扩展点

- **EXIF 尚未解析**：`images.taken_at` 字段已预留但当前为 `NULL`，排序回退到文件修改时间。接入 `kamadak-exif` 后写入该列即可启用「按拍摄时间排序」。
- **HEIC / RAW 无法索引**：`image` crate 只启用了 jpeg / png / gif / webp / bmp / tiff / ico。扫描时这些文件会计入 `skipped` 而不是被静默丢弃。要支持需更换解码后端（如 `libheif-rs`）。
- **带透明通道的缩略图**：统一编码为 JPEG，PNG / ICO 的透明区域会丢失。
- **扫描是单线程的**：`scanner.rs` 顺序遍历。目录很大时可引入 `rayon` 并行读取元数据，注意 SQLite 写入仍需串行。
- **CSP 当前关闭**：`tauri.conf.json` 中 `security.csp` 为 `null` 以便开发；发布前应配置严格 CSP，并把 `asset:` 加入 `img-src`。
- **移除图库不会清理缩略图缓存**：缓存可能残留孤儿文件。侧栏底部显示缓存占用，旁边的「清理」按钮调用 `clear_thumbnail_cache` 一次性清空（下次浏览会按需重建）。
- **作品库不刷新已移动文件的路径**：`scan_artworks_folder` 用 `ON CONFLICT DO NOTHING` 实现去重（这也让 `duplicates` 计数能如实反映去重效果），代价是文件被移动后 `file_path` 仍指向旧位置。若要自动刷新，把冲突分支改成 `DO UPDATE SET file_path = excluded.file_path`，但那样就分不清"新增"和"更新"了。
- **作品库与通用图库是两套索引**：同一个目录既可以被「添加图库」索引进 `images`，也可以被「作品库扫描」索引进 `artworks`，两边的元数据不共享。若后续要合并成一套，需要决定是否给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。
