# LightCone Gallery 交接文档

> 交接时间：2026-10-10
> 项目路径：`E:\LightCone Gallery`
> 状态：**脚手架完成，可编译、可测试、可打包；UI 端到端路径尚未人工验证**（详见第 4 节）

---

## 1. 这个项目是什么

本地图库桌面应用。用户添加图片目录 → 后端递归索引 → 前端网格浏览、放大查看、打标签、加相册、收藏评分。原图始终留在用户自己的目录里，应用只读不写。

技术栈（版本均为实际解析结果，非 package.json 里的范围声明）：

| 层 | 实际版本 |
| --- | --- |
| Tauri | 2.12.2 |
| Svelte / SvelteKit | 5.57.2 / 2.70.3 |
| Vite | 8.3.4 |
| TypeScript | 6.0.3 |
| Tailwind CSS | 4.3.3（`@tailwindcss/vite` 插件，无 `tailwind.config.js`） |
| rusqlite | 0.40.2（`bundled`） |
| image | 0.25.10 |
| walkdir | 2.5.0 |

代码量约 3570 行：Rust 后端 2018 行，前端 1555 行。

---

## 2. 环境前置（本机已就绪，含两个陷阱）

本机已满足全部构建条件，**不需要重新安装任何东西**：

| 组件 | 状态 |
| --- | --- |
| Rust | stable 1.99.0，本次会话通过 rustup 安装，位于 `C:\Users\NachoNeko\.cargo\bin` |
| Node / npm | 24.19.0 / 11.18.0（pnpm 11.7.0 也可用） |
| MSVC | Visual Studio 18（Community + BuildTools），link.exe 版本 14.51.36231 |
| Windows SDK | 10.0.22621.0 与 10.0.26100.0 |
| WebView2 Runtime | 154.0.4258.62 |

### 陷阱一：`cargo` 不在 Git Bash 的 PATH 里

rustup 把 PATH 写进了注册表，但 Git Bash 启动早于该写入，因此**每个新 shell 都要手动加**：

```bash
export PATH="/c/Users/NachoNeko/.cargo/bin:$PATH"
```

不这么做所有 cargo 命令都会报 `command not found`（不是没装）。

### 陷阱二：Windows SDK 在 D 盘

`C:\Program Files (x86)\Windows Kits\10` **不存在**，SDK 实际装在 `D:\Windows Kits\10`（注册表 `KitsRoot10` 指向此处）。排查链接错误时不要据此判定"缺少 SDK"。

---

## 3. 命令速查

```bash
cd "E:/LightCone Gallery"
export PATH="/c/Users/NachoNeko/.cargo/bin:$PATH"   # 每个新 shell 都要

npm install                                    # 装前端依赖
npm run tauri:dev                              # 开发模式（Vite + Rust，开窗口）
npm run tauri:build                            # 打包 MSI + NSIS 安装包
npm run build                                  # 仅构建前端到 build/
npm run check                                  # svelte-check 类型检查
cd src-tauri && cargo test                     # Rust 单元测试（11 个）
cd src-tauri && cargo build                    # 仅编译后端
```

产物位置：`src-tauri/target/release/bundle/msi/` 与 `.../nsis/`。

---

## 4. 验证状态（交接的最关键部分）

### 已实测通过

| 项目 | 结果 |
| --- | --- |
| `cargo test` | 11 passed / 0 failed |
| `cargo build` | 成功，**0 warning** |
| `npm run check` | 0 errors / 0 warnings |
| `npm run build` | 成功，产物在 `build/` |
| `npm run tauri:dev` | Vite 在 1420 端口监听，应用窗口打开，标题正确 |
| `npm run tauri:build` | 产出 MSI（3.64 MiB）与 NSIS setup（2.59 MiB） |
| release exe 直接启动 | 正常，窗口标题 `LightCone Gallery` |
| 数据库初始化 | 8 张业务表全部建出，位于 `%APPDATA%\com.lightcone.gallery\gallery.db` |
| 退出时 WAL checkpoint | 主库从 4096 增至 98304 字节，WAL 归零（见第 6 节） |

覆盖核心逻辑的单元测试共 11 个，其中三个是端到端的：

- `scanner::scan_indexes_then_detects_changes_and_removals` — 建真实临时目录与真实 PNG，验证首次索引、无变化重扫不重复写、文件删除后清理记录
- `thumbnails::ensure_thumbnail_generates_reuses_and_invalidates_cache` — 真实解码 1200×800 PNG，验证等比缩放、缓存复用、原图更新后重算
- `db::schema::migrate_creates_schema_and_is_idempotent` — 迁移幂等 + 级联删除

### 未实测（不要假设它能用）

1. **UI 端到端路径完全没走过。** 我验证了窗口能开、能连数据库，但从没在界面里真正点过"添加图库 → 扫描 → 缩略图显示 → 点开大图 → 打标签"。这是**最大的盲区**，建议接手后第一件事就是人工过一遍。
2. **asset:// 协议的实际图片渲染未验证。** 后端缩略图生成已被单元测试覆盖，但 WebView 侧通过 `convertFileSrc` 是否真能把图显示出来，没有实证。若界面上图片是空白/破图，优先怀疑这里是 `assetProtocol` 的 scope 没生效。
3. **MSI / NSIS 安装包的安装与卸载流程**没跑过。
4. **非 Windows 平台**完全未验证。`tauri.conf.json` 里 `targets: "all"`，代码本身没有平台分支，只有 `main.rs` 顶部一个标准的 `windows_subsystem` 属性（Release 下隐藏控制台窗口），macOS / Linux 理论上能编但没试过。
5. **大数据量性能未压测**（没试过上万张图的目录）。
6. **扫描是单线程的**，大目录下可能慢。

### 已实现但 UI 无入口的后端命令

以下 6 个命令后端已实现、`api.ts` 已封装，但没有任何组件调用：

`renameAlbum`、`deleteAlbum`、`removeImagesFromAlbum`、`setAlbumCover`、`updateTag`、`deleteTag`

也就是说：**相册建好后在界面上改不了名、删不掉；图片加进相册后在界面上移不出；标签改不了名、删不掉。** 这些是明确的待补 UI，不是 bug。

---

## 5. 代码地图

```
src/                                  # 前端（SvelteKit，SPA 模式，SSR 关闭）
├─ app.css                            # Tailwind 入口 + @theme 设计令牌
├─ lib/
│  ├─ api.ts                          # 所有 invoke 的类型化封装（组件不直接 invoke）
│  ├─ types.ts                        # 与 Rust models.rs 手工对齐的类型
│  ├─ gallery.svelte.ts               # 全局状态（runes class），筛选/查询逻辑唯一出处
│  ├─ format.ts                       # 字节/日期/尺寸格式化
│  └─ components/
│     ├─ Sidebar.svelte               # 图库/相册/标签导航 + 缓存清理
│     ├─ Toolbar.svelte               # 搜索（防抖 250ms）、排序、评分筛选、扫描
│     ├─ ImageGrid.svelte             # 网格 + IntersectionObserver 无限滚动
│     ├─ ImageCard.svelte             # 卡片 + 收藏按钮
│     ├─ Thumbnail.svelte             # 进视口才请求缩略图（rootMargin 400px）
│     └─ Lightbox.svelte              # 大图 + 标签/评分/相册编辑 + 键盘导航
└─ routes/{+layout.svelte,+layout.ts,+page.svelte}

src-tauri/src/                        # 后端
├─ lib.rs                             # 启动流程、命令注册、退出 checkpoint
├─ state.rs                           # AppState + with_db（spawn_blocking 封装）
├─ error.rs                           # AppError，可直接序列化成前端消息
├─ scanner.rs                         # 递归发现 + 增量索引
├─ thumbnails.rs                      # 缩略图生成与磁盘缓存
├─ db/
│  ├─ mod.rs                          # 连接、PRAGMA、is_unique_violation
│  ├─ schema.rs                       # 版本化迁移（user_version）
│  └─ models.rs                       # 数据模型 + ImageQuery + SortBy 白名单
└─ commands/                          # 26 个 Tauri 命令，按领域分 5 个文件
```

### 一次"浏览图片"请求的完整路径

```
组件 → gallery.svelte.ts（拼 ImageQuery）→ api.ts（invoke）
  → commands/images.rs::list_images
  → state::with_db → spawn_blocking → Mutex<Connection>
  → 动态 SQL（? 占位符 + Vec<Value>；排序列走 SortBy::column() 白名单）
  → 返回 PagedImages
```

缩略图是另一条路径：`Thumbnail.svelte` 进视口 → `thumbnail_for` → 查 source path → 生成/读缓存 → 返回**文件路径** → 前端 `convertFileSrc` 转 `asset://` URL。

---

## 6. 三个已踩过的坑（重踩成本高，务必先读）

### 坑一：Tailwind v4 的 `@apply` 在组件里需要 `@reference`

Tailwind v4 中组件的 `<style>` 是**独立编译单元**，看不到 `app.css` 里的 `@theme`。在组件 `<style>` 里直接用 `@apply` 会报：

```
Cannot apply unknown utility class `bg-black/50`.
Are you using CSS modules or similar and missing `@reference`?
```

修法：在 `<style lang="postcss">` 顶部加一行 `@reference "../../app.css";`。`Sidebar.svelte` 和 `Lightbox.svelte` 已加，新增用到 `@apply` 的组件照做。

### 坑二：Svelte 5 的 `class:` 指令名不能含冒号

`class:group-hover:opacity-100={...}` 这种 Tailwind 变体写法**不合法**（冒号不是合法的指令名字符）。要用模板字符串三元：

```svelte
class="... {cond ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}"
```

`class:text-rose-400`（只含连字符）是合法的，可以用。

### 坑三：Tauri 退出时跳过析构，WAL 永远合并不回主库

**这是实测发现的真实缺陷，已修复但值得知道原因。**

Tauri 的退出路径直接结束进程、跳过 Rust 析构，`Connection` 不走 `Drop`，SQLite 的 WAL 就没有机会 checkpoint。实测退出后主库仍停在 4096 字节、WAL 里积压 156 KB。后果是单独备份 `gallery.db` 会丢数据。

修法在 `lib.rs` 末尾：用 `Builder::build()` + `app.run(|handle, event| ...)` 捕获 `RunEvent::Exit`，退出前显式 `PRAGMA wal_checkpoint(TRUNCATE);`。修复后主库正常增长到 98304 字节、WAL 归零。

**如果你要重构 `run()` 里的退出逻辑，别把这行删掉。**

---

## 7. 关键决策（请勿轻易撤销）

| 决策 | 理由 |
| --- | --- |
| `Mutex<Connection>` 单连接，不用连接池 | 写入集中在索引阶段，读取都是小查询，WAL 下单连接足够；避免连接池与 rusqlite 的版本耦合。要换 r2d2 只需改 `AppState`，命令层不用动 |
| 所有 DB/解码操作走 `state::with_db` + `spawn_blocking` | rusqlite 是同步 API，直接在 async 命令里调用会占住 async runtime 工作线程 |
| 图片走 `asset://` 协议而非 base64 IPC | 避免大图走 IPC 的内存开销。代价是 scope 不持久化，每次启动要重新 `allow_directory`（见下） |
| 启动时重新授权 asset scope | `asset_protocol_scope()` 只存在于内存，因此在 `lib.rs` setup 里为缩略图目录和所有图库根目录重新登记 |
| 时间统一用秒级 Unix 整数 | 避免引入 chrono |
| 缩略图统一输出 JPEG | 图库主体是照片，JPEG 体积/解码速度最均衡。代价：PNG/ICO 的透明通道会丢 |
| 排序字段走 `SortBy::column()` 白名单 | 排序字段来自前端，不能拼接原始字符串，否则是注入点 |
| 只索引能读出尺寸的文件 | `image_dimensions` 只解析文件头。读不出（HEIC/RAW）的计入 `skipped`，不静默丢弃 |

### 数据库迁移约定

迁移基于 `PRAGMA user_version`，见 `db/schema.rs`。**新增变更时**：把 `SCHEMA_VERSION` 加一，并**追加**一个新的 `if version < N` 分支，**不要修改历史分支**。

当前 v1 共 8 张业务表：`libraries`、`images`、`albums`、`album_images`、`tags`、`image_tags`、`thumbnails`、`settings`（另有 SQLite 为 AUTOINCREMENT 自动建的 `sqlite_sequence`）。外键全部 `ON DELETE CASCADE`，连接建立时开启 `foreign_keys`。

---

## 8. 当前工作区状态

- **不是 git 仓库**（`git rev-parse` 失败）。如果你要开始改代码，建议先 `git init` 并提交当前状态作为基线。
- **无残留进程**，1420 端口已释放。
- **应用数据目录有我的测试残留**：`%APPDATA%\com.lightcone.gallery\` 下有一个 98304 字节的 `gallery.db`（schema 已建，但**没有任何图库记录和图片**，是干净的空库）+ 一个空的 `thumbnails/` 目录。想让首次运行绝对干净可以删掉整个目录，应用会自动重建；不删也没有副作用。
- `node_modules`、`build`、`.svelte-kit`、`src-tauri/target` 都已存在于本地（依赖装好了，不用重新装）。
- `src-tauri/icons/` 还是脚手架默认的 Tauri 图标，**没有换成项目自己的图标**。

---

## 9. 建议的下一步（按优先级）

**P0 — 先做这个**

1. **人工过一遍 UI 端到端**：`npm run tauri:dev`，加一个有图片的目录，看扫描进度、网格缩略图、点开大图是否都正常。第 4 节列的 6 个盲区里，这一条的信息量最大。若图片显示不出来，先查 `assetProtocol` 的 scope（`lib.rs` setup 里的 `grant_asset_scope`）。

**P1 — 补完功能闭环**

2. **接上 6 个无入口的命令**：相册的重命名/删除/移除图片/设封面，标签的重命名/删除。后端和 `api.ts` 都好了，只差组件。
3. **EXIF 解析**：`images.taken_at` 字段已建好但当前为 `NULL`，排序回退到文件修改时间。接 `kamadak-exif` 后写入该列，即可启用"按拍摄时间排序"。
4. **相册封面展示**：`albums.cover_image_id` 字段已就绪，侧栏目前只显示名称和数量。

**P2 — 工程加固**

5. **扫描并行化**：`scanner.rs` 现在是顺序遍历，大目录下慢。可引入 `rayon` 并行读取元数据（注意 SQLite 写入仍需串行）。
6. **生产 CSP**：`tauri.conf.json` 里 `security.csp` 目前是 `null`（方便开发）。发布前应配严格 CSP，并把 `asset:` 加进 `img-src`。
7. **大目录压测**：上万张图验证索引耗时与内存占用。
8. **换应用图标**：`src-tauri/icons/`。
9. **支持 HEIC / RAW**：需要换解码后端（如 `libheif-rs`），`image` crate 不支持。

---

## 10. 工作约定

- **所有面向用户的文字用中文**（用户明确要求过）。代码标识符保持英文，**注释用中文**——现有代码全部是中文注释，请保持一致。
- 前端类型与 Rust 模型是**手工对齐**的（`types.ts` ↔ `models.rs`），改一边记得改另一边。序列化统一 camelCase。
- 组件不直接 `invoke`，一律经 `api.ts`；筛选/查询逻辑只写在 `gallery.svelte.ts` 里，不要分散到组件。
- 涉及数据库或图像解码的新命令，必须走 `state::with_db` / `spawn_blocking`，不要写成同步阻塞。
- 新增迁移只追加分支，不改历史分支。
