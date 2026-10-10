# LightCone Gallery — 阶段一交接（给 DS V4.1）

> ⚠️ **本文档已过期，不要照着实施。** 阶段一后来走了另一条路：`artworks` 是一张**独立表**
> （`id / pid / file_path / page / created_at` + `UNIQUE(pid, page)`），而不是本文推荐的
> `artworks` + `images.artwork_id` 两层模型；阶段二也已经落地（有损 WebP 缩略图 + 虚拟滚动）。
> 当前实现的权威说明见 `README.md`，实际代码见 `src-tauri/src/artworks.rs` 与 `src/lib/components/VirtualGrid.svelte`。
> 保留此文件仅作为当时的决策记录。

> 编写时间：2026-10-10
> 仓库路径：`E:\LightCone Gallery`
> 配套文档：`HANDOFF.md`（环境、架构、已踩的坑的完整参考）

---

## 0. 先看这里：阶段一 spec 与仓库现状存在分歧

你要交给 DS V4.1 的阶段一 Prompt，和仓库里**已经存在的代码**不是同一套东西。直接照那份 Prompt 执行会撞车。

| 维度 | 阶段一 Prompt 假设 | 仓库实际情况 |
| --- | --- | --- |
| 入口文件 | `src/App.svelte` | SvelteKit 路由，入口是 `src/routes/+page.svelte`（**没有 App.svelte，建了也不会被加载**） |
| Tailwind 配置 | `tailwind.config.js` | Tailwind **v4**，走 `@tailwindcss/vite` 插件 + CSS 内 `@theme`，**不需要也不需要读 config 文件** |
| 数据表 | 新建 `artworks(id, pid, file_path, created_at)` | 已有 `images` 表（13 个字段，含尺寸/大小/mtime/收藏/评分）+ 8 张业务表 |
| PID 概念 | 文件名 `{pid}_p{num}.jpg` 解析出 pid | **仓库里完全没有 pid 概念**（已 grep 确认） |
| 图片显示 | 「前端绝对不能直接加载原图」 | 现有 UI 通过 `asset://` 协议**已经能显示原图**（Lightbox）并生成缩略图 |
| 缩略图 | 「本阶段不做」 | **已经做好了**（`thumbnails.rs`，带磁盘缓存，有单元测试覆盖） |

也就是说：**仓库是一个通用图库，你的阶段一是要做成 Pixiv 风格的作品库。** 两者在领域模型上是分叉的，不是"加个功能"的关系。

在往下走之前，你需要决定走哪条路（见第 5 节）。

---

## 1. 我在这份 spec 里发现的 4 个问题

这些是执行前就该改掉的，否则 DS V4.1 会照着写出问题。

### 问题 1（最重要）：`pid` 加唯一索引会丢图

spec 说「实现基于 PID 的简单去重（唯一索引）」，表结构是 `artworks(id, pid, file_path, created_at)`。

但文件名 `{pid}_p{num}.jpg` 里的 `_p{num}` **就是页码**。Pixiv 一张作品多页是常态：

```
123456_p0.jpg
123456_p1.jpg
123456_p2.jpg
```

这三行的 `pid` 都是 `123456`。如果按 spec 对 `pid` 建唯一索引，`p1` 和 `p2` 会被 `INSERT OR IGNORE` 静默丢掉——**一本多页作品只剩第一页，而且没有任何报错**。

正确做法是把「作品」和「页」拆成两层（见第 2 节）。`pid` 在 `artworks` 表里唯一是对的，因为一个 pid 确实只对应一个作品；但 `file_path` 不该放在那张表上，它是"页"的属性。

### 问题 2：`tailwind.config.js` 不适用

Tailwind v4 不再需要也不自动读取 `tailwind.config.js`。设计令牌写在 CSS 里：

```css
/* src/app.css 已存在 */
@import "tailwindcss";
@theme {
  --color-surface-950: oklch(0.17 0.012 265);
  /* ... */
}
```

让 DS V4.1 建这个文件只会留下一个永远不生效的死文件。

### 问题 3：`src/App.svelte` 不会被加载

仓库是 SvelteKit（adapter-static，SPA 模式）。SvelteKit 只认 `src/routes/` 下的文件。`src/App.svelte` 需要手动在入口里 import 才会渲染，而现在的入口是 `src/routes/+page.svelte`。

直接改 `src/routes/+page.svelte` 即可，不要另起 App.svelte。

### 问题 4：「前端绝对不能直接加载原图」和现有 UI 冲突

这条约束本身是好设计（大图直出会拖垮滚动性能），但仓库的 Lightbox **已经**在用 `asset://` 加载原图了。

所以这不是"要不要做"的问题，而是"要不要把已有功能撤掉"的问题。我的建议：**保留**。阶段一的「只显示路径」只是网格视图的简化，等到阶段二做了缩略图再自然接上，不需要专门去禁用 Lightbox。

---

## 2. 推荐的数据模型

不动现有的 8 张表，**追加**两张表（v2 迁移），让"作品"和"页"分层：

```sql
-- v2 迁移
CREATE TABLE artworks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    pid        TEXT    NOT NULL UNIQUE,   -- Pixiv 作品号，唯一是合理的
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

ALTER TABLE images ADD COLUMN artwork_id INTEGER REFERENCES artworks(id) ON DELETE SET NULL;
ALTER TABLE images ADD COLUMN page       INTEGER;   -- 文件名里的 num

CREATE INDEX idx_images_artwork ON images(artwork_id);
CREATE UNIQUE INDEX idx_images_artwork_page ON images(artwork_id, page)
    WHERE artwork_id IS NOT NULL;
```

为什么这样分层：

- `pid` 唯一放在 `artworks` 上 —— 满足 spec 的去重意图，且不会丢页
- `file_path` 留在 `images` 上 —— 一个作品有多个文件，文件路径属于"页"
- 现有 `tags` / `albums` / `thumbnails` 都外键指向 `images.id`，**不用改**，标签和相册功能自动适用于新索引进来的图片
- 独立的 `artworks` 表还能挂以后要加的东西（标题、作者、R-18 标记、收藏数）

如果你坚持要 spec 里那种扁平的 `artworks(id, pid, file_path, created_at)`，那就必须把唯一约束改成 `UNIQUE(pid, page)` 并且把列名从 `pid` 改成 `pid + page`，否则一定丢图。

---

## 3. 修正版 Prompt（可直接复制给 DS V4.1）

````text
在已存在的 Tauri 2 + SvelteKit + rusqlite 项目 E:\LightCone Gallery 上做增量开发。
不要重建项目，不要新建 src/App.svelte，不要新建 tailwind.config.js。

## 背景
仓库已是一个通用本地图库：扫描目录 -> 存 SQLite -> 网格浏览，缩略图与 asset:// 显示都已可用。
本阶段的目标是给它加上 Pixiv 作品维度的建模：从文件名解析出作品号 PID。

## 现有代码地图（先读，再改）
- src-tauri/src/db/schema.rs   版本化迁移（PRAGMA user_version），当前 v1，8 张表
- src-tauri/src/scanner.rs     目录递归 + 增量索引（内存快照比对，只写变化的文件）
- src-tauri/src/commands/images.rs   list_images / get_image 等，26 个命令之一
- src-tauri/src/lib.rs         命令注册 + 启动流程
- src/lib/api.ts               所有 invoke 的类型化封装（组件不直接 invoke）
- src/lib/gallery.svelte.ts    全局状态，筛选/查询逻辑的唯一出处
- src/routes/+page.svelte      页面入口（不是 App.svelte）

## 本阶段要做的 4 件事

1. 文件名解析
   在 src-tauri/src/scanner.rs 中新增 `pub fn parse_pid(filename: &str) -> Option<(String, i64)>`，
   从 `{pid}_p{num}.{ext}` 解析出 (pid, page)。要求：
   - `123456_p0.jpg` -> Some(("123456", 0))
   - `123456_p12.png` -> Some(("123456", 12))
   - `123456_p0_master1200.jpg` -> Some(("123456", 0))   容忍 Pixiv 下载器追加的后缀
   - `IMG_1234.jpg` / `123456.jpg` -> None                不匹配就返回 None
   用正则 `^(\d+)_p(\d+)` 匹配文件名开头即可，加 `regex = "1"` 依赖。
   必须为这个函数写单元测试，覆盖上面 4 种情况。

2. 数据库迁移到 v2
   在 src-tauri/src/db/schema.rs 中：把 SCHEMA_VERSION 改成 2，**追加**一个新的
   `if version < 2 { ... }` 分支（不要修改 v1 分支）。内容：
     - 新建 artworks 表：id INTEGER PRIMARY KEY AUTOINCREMENT, pid TEXT NOT NULL UNIQUE,
       page_count INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL
     - 给 images 表加两列：artwork_id INTEGER REFERENCES artworks(id) ON DELETE SET NULL, page INTEGER
     - 建索引 idx_images_artwork(artwork_id)
     - 建部分唯一索引 idx_images_artwork_page ON images(artwork_id, page) WHERE artwork_id IS NOT NULL
   注意 ALTER TABLE ADD COLUMN 在 SQLite 里不能加带 UNIQUE 的列，唯一性要靠上面的单独索引。

3. 扫描时写入作品归属
   改 src-tauri/src/scanner.rs 的 scan_library：对每个文件调 parse_pid。
   解析成功的，先 `INSERT INTO artworks (pid, created_at) VALUES (?, ?)
   ON CONFLICT(pid) DO NOTHING`，再查回 id，把 artwork_id 和 page 写进 images 的 upsert 语句。
   解析失败的照常入库，artwork_id 留 NULL（不要跳过这些文件）。
   扫完后更新每个作品的 page_count。

4. 新增一个分页查询 command
   在 src-tauri/src/commands/ 下新增 artworks.rs：
     `list_artworks(state, offset: i64, limit: i64) -> ArtworkPage`
   返回按 pid 排的作品列表，每项含 pid、页数、以及该作品第一页的 file_path。
   用 `params_from_iter` + `?` 占位符绑定参数，**不要拼 SQL 字符串**。
   limit 要 clamp 到 1..=200，offset 不小于 0。
   在 src/lib/api.ts 加对应的 `listArtworks(offset, limit)` 封装，
   在 src-tauri/src/lib.rs 的 generate_handler! 里注册。

## 前端
改 src/routes/+page.svelte（或在 src/lib/components/ 下加一个新组件并挂进去）：
用 `grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3` 的网格
展示 list_artworks 的分页结果，**每格只显示 file_path 文本和下方的 pid**，不渲染图片。
底部放「上一页 / 下一页」两个按钮，显示当前页码和总数。

## 硬性约束
- 所有数据库操作必须走 `state::with_db`（内部是 spawn_blocking），不要写成同步阻塞命令
- 代码注释用中文，与现有代码风格一致
- 不要删除或重写现有的缩略图、asset:// 显示、标签、相册相关代码
- 改完必须让 `cargo test`、`npm run check`、`npm run build` 三个都通过

## 环境（必须注意）
- cargo 不在 Git Bash 的 PATH 里，每条 cargo 命令前先执行：
  `export PATH="/c/Users/NachoNeko/.cargo/bin:$PATH"`
- 组件 <style> 里用 @apply 必须先在顶部写 `@reference "../../app.css";`（Tailwind v4 要求）
- Svelte 5 的 class: 指令名不能含冒号，`class:group-hover:x` 非法，要用模板字符串三元

## 交付
列出所有改动文件的完整路径，说明 v2 迁移怎么验证，并贴出三个检查命令的实际输出。
````

---

## 4. 必须一并带过去的环境信息

这些是实操中会卡住 DS V4.1 的点，`HANDOFF.md` 里有更详细的版本：

| 事项 | 内容 |
| --- | --- |
| **cargo 不在 PATH** | 每个新 shell 先 `export PATH="/c/Users/NachoNeko/.cargo/bin:$PATH"`，否则报 command not found（不是没装） |
| **Windows SDK 在 D 盘** | 实际的 SDK 在 `D:\Windows Kits\10`，`C:\Program Files (x86)\Windows Kits\10` 不存在，别据此判定缺 SDK |
| **Tailwind v4 的 `@reference`** | 组件 `<style>` 是独立编译单元，用 `@apply` 前必须 `@reference "../../app.css";` |
| **Svelte 5 的 `class:` 不能含冒号** | `class:group-hover:opacity-100` 非法，用模板字符串三元 |
| **迁移只追加不改历史** | `schema.rs` 里新增变更必须追加 `if version < N` 分支 |
| **不是 git 仓库** | 建议 DS V4.1 动手前先 `git init` + 提交一次基线，否则改坏了没法回退 |

---

## 5. 你需要先做的决定

仓库现状和阶段一 spec 分叉了，有两条路，选一条：

**路线 A：在现有仓库上增量做（推荐）**

保留通用图库的全部能力（缩略图、标签、相册、asset:// 显示），按第 3 节的 Prompt 追加 PID 层。
优点是已有的 2018 行 Rust 代码和 11 个测试全部可用，标签相册等功能不用重做；
代价是领域模型比 spec 复杂一点（作品 / 页两层）。

**路线 B：按 spec 从零重建**

把 `artworks` 做成 spec 里那种扁平表，删掉或绕开现有的 `images` 体系。
优点是严格贴合你的阶段划分；
代价是要重写扫描器、丢掉现有 11 个测试、标签和相册要重做，而且**必须先解决第 1 节的丢页问题**。

我推荐 A。如果你的阶段划分是有意为之（比如后续阶段要替换掉现有 UI），那选 B，但无论选哪条，`pid` 唯一索引的丢页问题都得先修。

---

## 6. 验收清单

无论走哪条路，阶段一做完时应该能满足：

- [ ] `cargo test` 通过，且 `parse_pid` 有覆盖 4 种文件名格式的测试
- [ ] `npm run check` 0 error
- [ ] `npm run build` 成功
- [ ] 在 UI 里添加一个装有 `{pid}_p{num}.jpg` 文件的目录，扫描后网格显示路径 + PID
- [ ] **多页作品不丢页**：放一组 `123456_p0.jpg` / `123456_p1.jpg` / `123456_p2.jpg`，验证三行都在，`page_count = 3`（这是对第 1 节问题的回归验证）
- [ ] 不符合命名规范的文件（如 `IMG_1234.jpg`）照常入库，PID 留空，不被跳过
- [ ] 二次扫描不产生重复行（增量索引仍然有效）
- [ ] 退出应用后 `gallery.db` 文件本身包含全部数据（WAL 已 checkpoint）
