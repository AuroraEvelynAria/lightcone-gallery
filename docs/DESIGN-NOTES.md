# 设计笔记

这份文档是 README 的背面：**为什么这么写**，而不是**怎么用**。
数据库每一版的来由、后端命令清单、以及踩过的坑与它们的对策都在这里；
想先跑起来的话看 [README](../README.md)，想打包发行看 [RELEASE.md](RELEASE.md)。

## 数据存放位置

| 内容 | 路径 |
| --- | --- |
| 数据库 | `<AppData>/com.lightcone.gallery/gallery.db`（Windows：`%APPDATA%\com.lightcone.gallery\`） |
| 缩略图缓存 | `<AppData>/com.lightcone.gallery/thumbnails/<归属域>/<尺寸>/<id>.webp`（作品库用 400） |

`<归属域>` 取 `images` 或 `artworks`。分目录不是洁癖：`images.id` 与 `artworks.id` 是两张表各自的自增主键，共用一层目录会让 `images#1` 和 `artworks#1` 落到同一个文件上、互相覆盖缩略图。

原图始终留在用户自己的目录里，应用**只读不写**；从图库移除记录不会删除磁盘文件。

## 数据库设计

迁移基于 `PRAGMA user_version`，见 `src-tauri/src/db/schema.rs`。当前为 **v9**：

**通用图库（v1）**

- `libraries` — 用户添加的根目录
- `images` — 索引到的图片（`path` 唯一，含尺寸、大小、mtime、收藏、评分）
- `albums` / `album_images` — 手动组织的相册，`position` 保留加入顺序
- `tags` / `image_tags` — 用于交叉筛选的标签
- `thumbnails` — 缩略图缓存登记表
- `settings` — 预留的键值设置

**作品维度（v2 新增，v3 改列名，v4 加忽略名单）**

- `artworks` — 只收录文件名符合 `{pid}_p{num}` 的图片，一行代表「某作品的某一页」；`original_path` 是原图，`thumbnail_path` 是扫描阶段预生成的缩略图
- `artwork_roots` — 作品库扫描过的根目录
- `ignored_artworks` — 被移出过作品库的 `(pid, page)`，扫描时跳过它们
- `artists` — 画师（`uid` 主键 + 显示名），`artworks` / `images` 各带一列 `artist_uid`

**Pixiv 联网元数据（v6 新增，v7 补 `images.pid`）**

- `illust_meta` — 按 PID 从网页接口取回的标题 / 画师 / 页数 / R-18 等级；**失败的抓取也占一行**，否则每次都会从头再撞。这一行有三种状态，靠 `error` 与 `missing` 两列区分：

  | 状态 | `error` | `missing` | 批量会重抓吗 |
  | --- | --- | --- | --- |
  | 成功 | NULL | 0 | 不会（已完成） |
  | **作品在 Pixiv 上没了**（404） | 有值 | **1** | **不会**（终态） |
  | 网络 / 限流失败 | 有值 | 0 | 会（留在待补全里） |

  **「已删除」是什么意思**：Pixiv 对这个 PID 返回 404 —— 作者删了、转私密、或下架了。**本地文件还在**，只是 Pixiv 那一侧没了（实测过几个这样的 PID，一律 404，同时拿一个正常 PID 作对照是 200）。Pixiv 对「删除了」和「转私密了」都返回 404、无法区分，而私密作品以后可能放回来，所以留了「重试已删除的 N 条」这个出口 —— 终态不该是单向门。
- `illust_tags` — 官方标签多对多。标签**只能**从接口来，路径里没有
- `images.pid` — 从文件名反推的作品号，是通用图库通往 Pixiv 维度的**唯一桥梁**（没有它，弹窗显示不了官方标签、图库页也没法按标签筛选）

`artworks` 与 `images` 是**两套并存的索引**：`images` 是通用图库（标签 / 相册 / 缩略图挂在它上面），`artworks` 只做作品维度。两者互不影响，所以作品库的新增不会动到已有图库功能。

> **唯一约束是 `UNIQUE(pid, page)` 而不是 `UNIQUE(pid)`。** 这条不能改：`123456_p0.jpg` 与 `123456_p1.jpg` 的 pid 相同，若只对 pid 加唯一索引，多页作品从第 2 页起会被 `INSERT OR IGNORE` **静默丢弃**且不报错。回归测试见 `db::schema::tests::artworks_dedupes_by_pid_and_page_not_by_pid_alone`。

> **v3 的三处改动各有理由**：`file_path` 改名 `original_path`，是因为查询命令对外返回的就是 `pid / original_path / thumbnail_path`，列名跟着走免得每层翻译一次；新增 `thumbnail_path` 让列表自带缩略图地址，卡片不必再往返 IPC；`artwork_roots` 落盘是因为 `asset://` 的作用域**只存在于内存中**（见启动流程），不记下来就没法在重启后重新授权原图目录。
>
> 启动时还有一步幂等修复：如果库里有作品行但 `artwork_roots` 是空的（老库升级上来就是这个状态），就从现有的 `original_path` 反推所在目录补登一次。不做的话那些原图不在授权范围内，弹窗会莫名加载失败。登记过就只花一次 `COUNT` 查询。

> **v4 加 `ignored_artworks` 是为了让「移除」真的算数。** 在此之前移除只是删行，重新扫描同一目录会把它们原封不动地收回来，用户看到的就是「移除了又回来了」。记的是 `(pid, page)` 而不是 `artworks.id`：id 是自增的，重扫会换一个，只有 `(pid, page)` 稳定。代价是误移之后光重扫没用，得先清空名单 —— 所以作品库页头留了一个「已忽略 N 条 · 清空」的出口。

> **每一版迁移都与它自己的 `user_version` 在同一个事务里提交**（见 `schema.rs` 的 `apply`）。这不是洁癖：开发时改到一半被 `tauri dev` 重启，就出现过「`CREATE TABLE` 成功、版本号没落盘」，下一次启动重跑同一版迁移直接撞 `table ... already exists`，应用起不来。`user_version` 存在数据库头部、本身受事务保护，所以 `BEGIN; DDL; PRAGMA user_version = N; COMMIT;` 是原子的。`MIGRATION_V4` 另加了 `IF NOT EXISTS` 让已经处于半吊子状态的库自愈，回归测试见 `db::schema::tests::recovers_from_a_half_applied_migration`。

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
| 外观（阶段三） | `set_window_material` `load_settings` `save_settings` |
| 作品（阶段一 / 二 / 四） | `scan_artworks_folder` `list_artworks` `thumbnail_for_artwork` `remove_artworks` `count_ignored_artworks` `clear_ignored_artworks` |
| 画师 | `list_artists` |
| Pixiv 元数据 | `illust_detail` `count_missing_meta` `fetch_missing_meta` `list_pixiv_tags` `pixiv_meta_stats` |

扫描期间后端通过 `scan://progress` 事件推送进度（发现阶段 + 索引阶段），前端在 `gallery.svelte.ts` 中订阅。

`scan_artworks_folder(path)` 接收一个文件夹路径：遍历图片、解析 `{pid}_p{num}` 入库，**再为缺缩略图的作品批量生成 WebP 并把路径写回数据库**。返回的 `inserted` / `duplicates` 直接体现去重效果（重复扫描同一目录时 `inserted` 为 0），`thumbnailsGenerated` / `thumbnailsSkipped` / `thumbnailsFailed` 反映本轮缩略图 pass 的产出。`list_artworks(offset, limit)` 分页返回 `pid / original_path / thumbnail_path`，`limit` 上限 200，按 pid 数值倒序（`CAST(pid AS INTEGER)`，否则字符串排序会让 `999` 排在 `12345` 之后）。`thumbnail_for_artwork(id)` 只是自愈回退，正常路径上前端不会调它。`remove_artworks(ids)` 把作品移出作品库（删记录 + 清缩略图缓存 + 记入忽略名单），**不删除原图**；`count_ignored_artworks()` / `clear_ignored_artworks()` 是那份名单的读与清空。

扫描返回值里 `ignored` 是被忽略名单主动跳过的条数，与 `duplicates`（撞唯一索引）区分开 —— 前者是「用户不想要」，后者是「目录里有重复文件」。

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

**外观：三个正交的轴**（阶段三）。配色 × 材质 × 明暗，三者互不耦合。

- **配色**取自 [RhinE](https://github.com/ericzhang12111-cell/RhinE)（一个 foobar2000 主题）的 `tokens/tokens.json`：5 套方案（ARCHIVE / HAZARD / FLARE / FIELD / COLD FRONT）× 明暗两版 × 15 个语义令牌，共 150 个色值。它那套词汇（`bg` / `panel` / `case` / `well` / `hair` / `line-*` / `accent` / `on-accent`）本来就是给「机箱陈列」这类工业界面设计的，直接沿用没有另起名字。`src/lib/themes.ts` 由脚本从该 JSON 生成，不是手抄。
- **材质**是 Tauri 的真实系统效果（DWM），不是 CSS 假模糊：`Effect::Mica` / `Acrylic` / `Tabbed` / `Blur`。窗口必须 `transparent: true` 且 `html` 背景透明，材质才不会被自己的底色盖住；材质生效时应用底色与面板按 `--alpha-*` 让出透明度。
- **明暗**只影响配色，不影响材质；材质本身有明暗变体的（Mica / Tabbed）由应用显式指定，不交给系统偏好决定。

**令牌层**：`app.css` 里的 `--color-*` 全部指向运行时写入的 `--tone-*` 自定义属性。换配色只改这 15 个属性，Tailwind 生成的所有工具类跟着变，组件一行都不用动。前景层次按「强调度」编号（`text-fg` 最强 → `text-mark` 最弱）而不是明度，所以浅色方案里 fg 是深色、深色方案里 fg 是浅色，组件不必知道自己在哪种模式下。

**材质染色**：只有 Acrylic / Blur 吃 `tint` 参数。这两个材质几乎不挡壁纸 —— 实测在浅色壁纸下会把深色方案冲淡到读不清字，所以用配色方案自己的底色染一遍。

**外观偏好的两条持久化路径**：数据库 `settings` 表是**权威来源**（WebView 的 localStorage 会被清缓存顺手清掉，用户挑的外观不该跟着消失）；localStorage 只作**首帧缓存**，构造时同步读一次并立刻落到 DOM 上，页面第一帧就是用户选的样子。读取时对取值做校验（`isSchemeId` 等），免得以后删掉某个方案后旧设置把界面指到一个不存在的配色上。

**检视弹窗：一层共享的查看器内核**。`ImageViewer.svelte` 承载光锥 3D 视差、缩放平移与全屏，通用图库的 `Lightbox`（带信息面板）与作品库的 `ArtworkLightbox`（带 PID 栏）都只是它的外壳 —— 之前这套视差只长在作品库那边，而用户真正在浏览的是通用图库。两条硬约束写在文件头注释里：

- **缩放只能加在图片上，不能加在裁切框上**。圆角裁切那层带 `overflow: hidden`，`scale()` 会把圆角一起放大。所以缩放加在它里面的 `<img>` 上，裁切框保持不动 —— 观感上是「透过一张卡片看放大的图」。
- **图片显示尺寸用 JS 算，不用 `max-height: 100%`**。百分比 `max-height` 在「高度 auto 的祖先链」里会被当成 `none` 直接失效；而平移必须知道精确的显示尺寸才能把位移夹在「图片边缘不越过裁切框」的范围内。所以 `natural` × 容器盒子在 JS 里算成整数像素写死在元素上。

全屏走 Tauri 的 `setFullscreen`（capabilities 需要 `core:window:allow-set-fullscreen`，`core:default` 只给了查询没给设置），在纯浏览器里回退到 Fullscreen API；全屏时通知外壳收起侧边面板，可用区域变大后显示尺寸重算，图片自动撑满。放大后自动关掉倾斜 —— 鼠标改负责平移，再叠 3D 会晕。

**画师：不用联网，路径里一直有。** pixez 这类下载器落盘时就按 `<画师名>_<画师ID>/<pid>_p<n>.<ext>` 分好了目录，所以画师维度**零网络成本**——实测本地 2653 条作品里 2621 条（98.8%）能从路径解析出来，共 1238 位。解析规则在 `src-tauri/src/artists.rs`，**不能简化成「按第一个下划线切一刀」**：本地数据里画师名含下划线的有 47 个（`Ciza_`、`B_P`）、含数字的有 71 个（`AkiZero1510`、`0032`），所以用「贪婪匹配 + 锚定末尾连续数字」。

> **`uid` 才是身份，名字只是显示标签。** 同一 uid 在本地出现过 12 组不同名字（画师改过名，或下载时把展会后缀写进了目录名），例如 `27848445 -> Coria / Coria@COMIC1☆24 / Coria@コミ1【E31b】`。分组始终按 uid；界面不要假设名字唯一。

**Pixiv 元数据：按 PID 联网取，但不需要 OAuth。** 走公开的网页接口 `GET https://www.pixiv.net/ajax/illust/{pid}?lang=zh`，普通作品连 cookie 都不要。字段形状是在真机上逐个核出来的（见 `src-tauri/src/pixiv.rs` 的模块注释），其中两个最容易写错：**`body.tags` 是对象不是数组**（标签在 `body.tags.tags[].tag`），**作品不存在时 `body` 是空数组而不是对象**（所以必须先判 `error`）。

接口比路径强的地方有两点：**标签只能从这里来**；**画师名是未清洗的** —— 路径里的目录名是 Windows 文件名，`/` 这类字符被删掉了（接口给 `-273℃/望歩`，目录里是 `273℃望歩`），所以补全时用接口的名字**覆盖**路径派生的名字。uid 两边一致（真机比对 10/10）。

**限流是这个功能的核心约束，不是附加项。** 一次批量抓取就是上百个请求打在同一个出口 IP 上。做法：**串行 + 每次间隔 800ms**（抄 Pixiv-Archive 的 `SYNC_DELAY_MS` 默认值）、**非 200 一律不重试**、**连续失败 5 次中止整批**、单次上限 300 条。更要紧的是**把「被限流」和「作品没了」分开**：混为一谈会把限流永久记成「已删除」（污染数据且不再重试），同时不触发中止而继续撞 —— 那才是真会出事的做法。所以 429 / 403 直接判为限流，200 里的报错文案跨语种匹配 `limit` / `rate` / `频繁` / `制限` 等信号，命中就**立刻停**。

> **国内必须走代理。** 实测本机直连 pixiv 是 `HTTP 000`（DNS 被污染成 `face:b00c` 段，直连 IP 也不通）。所以客户端支持显式配置代理，存在 `settings` 表的 `pixiv_proxy` 里，作品库页面上有输入框。

**筛选：两个维度的多选语义不一样。** 画师是**任一**（一个作品只有一个画师），标签是**同时包含**（一个作品有很多标签，多选是为了收窄而不是发散），后者用 `GROUP BY pid HAVING COUNT(DISTINCT tag) = N` 实现。

**SQL 注入防护**：筛选条件全部走 `?` 占位符 + `Vec<Value>` 绑定；排序字段通过 `SortBy::column()` 的白名单映射落地，不拼接前端传来的字符串。

**退出时 checkpoint**：Tauri 的退出路径会直接结束进程、跳过 Rust 析构，`Connection` 不会走 `Drop`，WAL 也就永远合并不回主库（实测退出后主库仍停在 4096 字节、WAL 有 156 KB）。因此 `lib.rs` 用 `app.run(|handle, event| ...)` 捕获 `RunEvent::Exit`，在退出前显式执行 `PRAGMA wal_checkpoint(TRUNCATE)`。这样主库文件始终是自洽的——单独备份 `gallery.db` 不会丢数据。

## 已知限制与扩展点

- **元数据只补了一部分**：`fetch_missing_meta` 单次上限 300 条、每条间隔 800ms，所以补全整个库要分批点（2653 条约 35 分钟连续请求）。没补过的作品在弹窗里显示「这条还没取过元数据」，而不是「没有标签」——这两件事必须分清。
- **联网功能的账号风险**：这个接口**不带任何身份**（没有 cookie、没有 token），所以 Pixiv 无法把请求关联到账号，「封号」不是现实的失败模式；现实的是**出口 IP 被限流**，可恢复。但走代理意味着出口 IP 是**共享**的，别人撞脏了会影响主人，反之亦然。**账号风险真正变大的地方在 OAuth 那条路**（每日合集榜单、下载原图都要 refresh_token），那部分没做。
- **代理是硬前提**：国内直连 pixiv 不通，不填代理时批量抓取会连续失败并中止（界面会提示「检查代理是否开着」）。
- **可重试的失败会一直留在待补全里**：网络断了、代理没开时攒下的失败记录，下次批量会重新尝试 —— 这是有意的。只有「已删除」（404）才是终态，而它可以用「重试已删除的 N 条」放回队列。
- **`pixiv_meta_stats` 与批量的判据必须一致**：曾经批量那边多带一个 `artist_uid IS NOT NULL`，结果界面的「待补全」比实际能抓的多 32 条（那 32 条文件直接躺在图库根目录下、路径推不出画师），永远归不了零；而且那个条件恰好把最该抓的挡在外面 —— 接口是能给出画师的。现在三处查询共用同一个谓词常量。
- **多选标签的「反悔」是全清**：没有「只去掉一个」之外的批量操作，但每个胶囊点一下就能取消，够用。
- **画师胶囊只铺前 24 位**：1238 位全铺既慢又没法看，所以搜索框是必需的入口（按名字或 uid 搜）。
- **标签筛选只在作品库页有效**：v7 之后 `images` 也有 `pid` 了，通用图库的标签筛选在 SQL 层已经就绪（`ImageQuery.pixiv_tags`），但侧栏还没有入口。
- **两个弹窗的原图加载没有预取**：翻页时要等下一张原图下载完。项目的硬约束是「原图只在弹窗里出现」，预取等于把没在看的原图也拉进内存。
- **「平滑」没有客观验证**：缩放与视差已改为 rAF 逐帧插值 + 直接写 DOM（绕开 Svelte 渲染管线），放大时也关掉了两片 `mix-blend-mode` 扫光。但帧率没法靠截图测，手感只能人肉判断。
- **材质是否生效无法检测**：Tauri 的 `apply_effects` 把底层 `window_vibrancy` 的返回值整个丢掉了（见 `tauri/src/vibrancy/windows.rs`），所以不支持的平台上 `set_window_material` 会「成功」但什么也没发生。外观页把平台要求直接标在选项旁，不假装能检测。Mica / Tabbed 需要 Windows 11，Acrylic 需要 Windows 10 v1809+。
- **「模糊」材质没有实测**：另外四种（纯色 / 云母 / 亚克力 / 标签页）已在真机窗口逐一确认；Blur 是同一个代码路径，但 Windows 11 22621 上拖动窗口会明显卡顿（Tauri 的类型注释里也写了），所以没细看。
- **语义状态色还没进配色体系**：错误 / 警告用的是 Tailwind 的 `rose` / `amber`，不随配色方案变化。浅色方案下 `bg-rose-950/60` 这种深底会显得突兀，后续应该抽成 `danger` / `warn` 两个令牌。
- **EXIF 尚未解析**：`images.taken_at` 字段已预留但当前为 `NULL`，排序回退到文件修改时间。接入 `kamadak-exif` 后写入该列即可启用「按拍摄时间排序」。
- **HEIC / RAW 无法索引**：`image` crate 只启用了 jpeg / png / gif / webp / bmp / tiff / ico。扫描时这些文件会计入 `skipped` 而不是被静默丢弃。要支持需更换解码后端（如 `libheif-rs`）。
- **换成 WebP 后旧缓存会变孤儿**：缓存路径从 `<尺寸>/<id>.jpg` 改成 `<归属域>/<尺寸>/<id>.webp`，旧 JPEG 不会再被读取，但会留在磁盘上。侧栏底部的「清理」按钮可以一次性清掉，下次浏览按需重建。
- **虚拟滚动是定高网格**：格子尺寸只由「列数 × 容器宽度」推出，所以所有格子必须同宽同高（`object-cover` 裁切）。要做行高不齐的瀑布流得另写组件——那需要逐格测量与位置缓存，本项目的 3D 视差弹窗（阶段四）不依赖它。
- **扫描会连带生成缩略图，因此首次索引慢**：约 23ms/张（并行后 2000 张约 12s、10000 张约 58s）。这是「扫描时预生成」换来的浏览零等待。若更在意扫描速度，把 `generate_thumbnails` 从 `scan_artworks_folder` 里摘出去、改成按需调用即可，缓存与回退逻辑都不用动。
- **弹窗翻页只覆盖「已加载」的范围**：通用图库那边翻到已加载列表的边界会自动 `loadMore()` 再翻，但一次只补一页；作品库那边靠 `artworks.fetchAt()` 等数据块到位，等不到就原地不动（不会切到空白）。两边都不做相邻原图预加载 —— 项目的硬约束是「原图只在弹窗里出现」，预加载等于把没在看的原图也拉进内存。
- **移除的「反悔」是整表清空，不是逐条撤销**：移除会把 `(pid, page)` 记进 `ignored_artworks`，所以重扫不会收回来；但想撤销只能清空整份名单（作品库页头的「已忽略 N 条 · 清空」），没有逐条恢复。名单通常很小，暂不值得为它做一套管理界面。
- **作品库不刷新已移动文件的路径**：`scan_artworks_folder` 用 `ON CONFLICT DO NOTHING` 实现去重（这也让 `duplicates` 计数能如实反映去重效果），代价是文件被移动后 `original_path` 仍指向旧位置，缩略图会随之变成「源文件缺失」。若要自动刷新，把冲突分支改成 `DO UPDATE SET original_path = excluded.original_path`，但那样就分不清「新增」和「更新」了。
- **作品库只能扫「目录」，不能复用已登记的图库**：`artworks` 是独立索引，只有通过作品库页面的「选择文件夹并扫描」明确扫过的目录才会进去；侧栏「添加图库」登记的是通用图库（`images` 表）。同一个目录两边都索引会存两份缩略图（320px + 400px）。要合并成一套需要给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。
- **扫描是单线程的**：`scanner.rs`（通用图库）顺序遍历；作品库那边的**缩略图生成**已经并行（见 `ensure_parallel`），但目录遍历本身仍是串行。目录很大时可引入 `rayon` 并行读取元数据，注意 SQLite 写入仍需串行。
- **CSP 当前关闭**：`tauri.conf.json` 中 `security.csp` 为 `null` 以便开发；发布前应配置严格 CSP，并把 `asset:` 加入 `img-src`。
- **移除图库不会清理缩略图缓存**：缓存可能残留孤儿文件。侧栏底部显示缓存占用，旁边的「清理」按钮调用 `clear_thumbnail_cache` 一次性清空（作品库下次浏览会按需重建）。
- **作品库与通用图库是两套索引**：同一个目录既可以被「添加图库」索引进 `images`，也可以被「作品库扫描」索引进 `artworks`，两边的元数据不共享。若后续要合并成一套，需要决定是否给 `images` 加 `pid` / `page` 列并废弃 `artworks` 表。
