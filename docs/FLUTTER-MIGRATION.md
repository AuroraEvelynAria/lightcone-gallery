# LightCone Gallery → Flutter 迁移方案

> 状态：草案，待确认后进入实施
> 决策前提：保留 Rust 后端，通过 `flutter_rust_bridge` 复用；桌面与 Android 同等重要；
> 目标是解决整体流畅度与交互体验问题。

---

## 一、为什么换 Flutter（结论先行）

当前痛点是「滚动 / 列表卡顿 + 弹窗 / 查看器卡顿 + 整体交互有问题」。这些问题的根因
不是代码写得差——恰恰相反，现有 Svelte 代码已经做了大量优化（虚拟滚动、rAF 直接写
DOM、缓存布局矩形、绕开 Svelte 渲染管线等）。问题出在 **WebView 渲染模型的天花板**：

1. **滚动性能受限于合成层数量**：WebView 里即使做了虚拟滚动，大量图片卡片的 `transform`
   / `opacity` 动画仍要走 GPU 合成，低端 Android WebView 容易掉帧。
2. **手势跟手性差**：Pointer Events 经过 WebView → 系统手势分发，延迟比原生高；
   双指捏合、滑动关闭等交互在 WebView 里始终不如原生跟手。
3. **光锥视差的 CSS 3D 层叠**：`preserve-3d` + 多层 `translateZ` 在部分 Android
   WebView 上渲染不稳定，且与 `position: fixed` 的包含块规则互相牵制（现有代码里有
   大量注释在处理这个矛盾）。
4. **窗口材质（Mica / Acrylic）**：Tauri 依赖系统 WebView 透出，Android 上根本不存在
   这套，只能用 CSS 假模糊。

Flutter 的 Skia / Impeller 渲染引擎直接画到画布，不受 WebView 合成层限制；手势系统
是原生级的；动画走自己的渲染管线，帧率稳定。这正是解决上述问题的正确方向。

---

## 二、目标技术栈

| 层 | 现有 | 迁移后 |
| --- | --- | --- |
| 桌面 / 移动外壳 | Tauri 2 | Flutter（自绘引擎） |
| 前端框架 | Svelte 5 + SvelteKit | Flutter（Dart） |
| 后端 | Rust（Tauri commands） | Rust（`flutter_rust_bridge` 暴露） |
| 数据库 | SQLite + rusqlite | 不变（仍由 Rust 管理） |
| 图像处理 | `image` + `webp` crate | 不变 |
| 状态管理 | Svelte 5 runes | Riverpod 2.x（推荐，见 §四） |
| 路由 | SvelteKit（静态 SPA） | go_router |
| 样式 | Tailwind CSS 4 | Flutter 主题 + 自定义 Design Tokens |
| 构建 | Vite + cargo | Flutter toolchain + cargo |

**保留的 Rust 模块**（全部不动逻辑，只改暴露层）：
- `db/`（schema、models、迁移）
- `scanner.rs`、`artworks.rs`、`artists.rs`、`thumbnails.rs`
- `pixiv.rs`（联网抓取 + 限流）
- `wallpaper.rs`（Windows FFI）
- `state.rs`、`error.rs`

**需要改造的 Rust 模块**：
- `commands/`：从 `#[tauri::command]` 改为 `flutter_rust_bridge` 可暴露的普通函数
- `lib.rs`：从 Tauri builder 改为 FRB 初始化
- `media.rs`（Android 桥）：从 Tauri 插件改为 Flutter Platform Channel 或 FRB 直接调 JNI

---

## 三、Rust 后端改造

### 3.1 总体思路

Tauri 的命令本质是「Rust 函数 + 宏生成 IPC 序列化」，`flutter_rust_bridge`（FRB）
做的是同一件事，只是把序列化格式从 Tauri 的 JSON 换成了 FFI + 内存零拷贝。**业务逻辑
一行都不用改**，只需要：

1. 把 `#[tauri::command]` 宏去掉，函数签名保持不变
2. 把 `State<'_, AppState>` 参数改为通过 FRB 的 `rust_bridge` 全局单例获取
3. 把 `tauri::Emitter` / 事件推送改为 FRB 的 `StreamSink` 或 Dart 端轮询
4. 重新生成 Dart 绑定代码

### 3.2 AppState 改造

现有 `AppState` 通过 Tauri 的 `app.manage()` 托管，命令通过 `State<'_, AppState>` 注入。
FRB 没有这个机制，需要改成 `OnceLock<AppState>` 或 `lazy_static` 全局单例：

```rust
// src-tauri/src/state.rs
use std::sync::OnceLock;
use std::sync::Mutex;

static APP_STATE: OnceLock<Mutex<AppState>> = OnceLock::new();

pub fn init_state(state: AppState) {
    APP_STATE.set(Mutex::new(state)).expect("AppState already initialized");
}

pub fn app_state() -> &'static Mutex<AppState> {
    APP_STATE.get().expect("AppState not initialized")
}
```

所有命令把 `state: State<'_, AppState>` 参数去掉，函数体里用 `app_state().lock().unwrap()`
获取（锁的粒度与现有 Tauri 的 `Mutex<Connection>` 一致）。

### 3.3 命令改造示例

以 `list_libraries` 为例：

```rust
// 现有（Tauri）
#[tauri::command]
pub async fn list_libraries(state: State<'_, AppState>) -> Result<Vec<Library>, AppError> {
    let db = state.db.lock().unwrap();
    // ... 查询逻辑
}

// 迁移后（FRB）
pub async fn list_libraries() -> Result<Vec<Library>, AppError> {
    let db = app_state().lock().unwrap();
    // ... 查询逻辑完全不变
}
```

FRB 会自动把 `Result<T, E>` 转成 Dart 的 `Future<Result<T, E>>` 或抛异常（取决于配置）。
推荐用 `anyhow` 风格的错误 + FRB 的 `Result` 支持，Dart 端 try/catch 即可。

### 3.4 事件推送改造

现有三个事件频道：
- `SCAN_PROGRESS_EVENT`（扫描进度）
- `THUMBNAIL_PROGRESS_EVENT`（缩略图生成进度）
- `META_PROGRESS_EVENT`（元数据抓取进度）

FRB 支持 `StreamSink<T>`，把事件改成 Rust 端向 sink 推送，Dart 端 `await for` 消费：

```rust
// Rust
pub fn scan_library_stream(
    id: i64,
    sink: StreamSink<ScanProgress>,
) -> Result<ScanSummary, AppError> {
    // 扫描过程中 sink.add(ScanProgress { ... })
    // 结束时返回 ScanSummary
}
```

```dart
// Dart
final stream = api.scanLibraryStream(id: libraryId);
stream.listen((progress) {
  // 更新进度条
});
final summary = await stream.last;
```

**注意**：FRB 的 Stream 是单次的（调用一次返回一个 Stream），与 Tauri 的全局事件总线
不同。如果需要多订阅者（比如多个页面都监听扫描进度），Dart 端用 `StreamController.broadcast()`
包一层即可。

### 3.5 asset:// 协议替代

现有 Tauri 用自定义 `asset://` 协议让 WebView 直接加载本地图片（缩略图、原图），
不需要把文件读进内存再转 base64。Flutter 没有这个概念，替代方案：

- **缩略图 / 原图**：Flutter 的 `Image.file(File(path))` 直接从磁盘加载，
  走 Flutter 自己的图片缓存（`PaintingBinding.instance.imageCache`）。
  这比 WebView 的 `asset://` 更直接，零拷贝进 GPU 纹理。
- **需要授权的目录**（Android SAF）：Rust 端通过 `flutter_rust_bridge` 直接返回
  文件路径，Flutter 端用 `File(path)` 读取。SAF 的权限授予在 Rust 端通过 JNI 完成，
  与现有 `media-store-bridge` 的逻辑一致。

### 3.6 Android 桥接改造

现有 Android 专属功能：
- `android_scan_media_store`：枚举 MediaStore 里的 Pixiv 图片
- `android_immersive`：沉浸式全屏
- `android_pick_directory` / `android_list_subdirs`：SAF 目录选择
- `android_keep_alive`：前台服务
- `set_wallpaper`：`WallpaperManager.setStream`
- `copy_image_to_clipboard`：剪贴板

这些走的是 Kotlin 桥（`media-store-bridge`）。迁移到 Flutter 后有两条路：

**方案 A（推荐）：Rust 端通过 JNI 直接调 Java/Kotlin API**
- 现有 `media-store-bridge` 的 Kotlin 代码大部分可以保留
- Rust 端用 `jni` crate 直接调用，不经过 Tauri 插件
- FRB 把这些暴露给 Dart，Dart 端无感知

**方案 B：Flutter Platform Channel（MethodChannel）**
- 用 Flutter 社区插件（`image_gallery_saver`、`flutter_windowmanager` 等）替换
- 剪贴板用 `flutter/services` 的 `Clipboard` API
- 壁纸设置用 `wallpaper_manager` 插件或自写 Platform Channel

**建议**：核心扫描 / MediaStore 走方案 A（逻辑复杂，已有 Rust+Kotlin 实现），
简单系统 API（剪贴板、沉浸式、前台服务）走方案 B（生态成熟，减少自维护）。

### 3.7 窗口材质（Mica / Acrylic）

Tauri 的 `window_vibrancy` 插件提供窗口材质。Flutter 桌面端需要用：
- **Windows**：`flutter_acrylic` 插件（封装 DWM 的 SetWindowAttribute）
- **macOS**：`flutter_acrylic` 同样支持 NSVisualEffectView
- **Linux**：无原生材质，用 Flutter 的 `BackdropFilter` 做半透明模糊

这部分与现有 `set_window_material` 命令的语义对齐，Dart 端封装一个 `WindowMaterialService`。

---

## 四、Dart 前端架构

### 4.1 项目结构

```
lib/
├── main.dart                    # 入口，初始化 Rust + 全局 Provider
├── app.dart                     # MaterialApp + 主题 + 路由
├── core/
│   ├── rust/                    # FRB 生成的绑定（不手改）
│   ├── api/                     # 对 FRB 绑定的类型化封装（对应原 api.ts）
│   ├── models/                  # 数据模型（对应原 types.ts）
│   ├── theme/                   # 主题系统（配色 × 材质 × 明暗）
│   └── router/                  # go_router 配置
├── features/
│   ├── gallery/                 # 通用图库
│   │   ├── providers/           # Riverpod providers（对应 gallery.svelte.ts）
│   │   ├── widgets/             # ImageGrid, ImageCard, VirtualGrid
│   │   └── pages/
│   ├── artworks/                # 作品库
│   │   ├── providers/           # 对应 artworks.svelte.ts
│   │   └── widgets/             # ArtworksView, ArtworkCard, VirtualMasonry
│   ├── viewer/                  # 图片查看器
│   │   └── widgets/             # ImageViewer（光锥视差 + 手势）
│   ├── sidebar/                 # 侧栏导航
│   ├── filter/                  # 筛选面板
│   ├── appearance/              # 外观设置页
│   └── shared/                  # 通用组件（Thumbnail, Lightbox 等）
└── utils/                       # 工具函数（format 等）
```

### 4.2 状态管理选型：Riverpod 2.x

现有 Svelte 5 runes 的模式是「单例 store + 响应式状态」，与 Riverpod 的 Provider 模式
高度对齐：

| 现有 | 迁移后 |
| --- | --- |
| `gallery.svelte.ts` 的 `GalleryStore` 单例 | `galleryProvider`（NotifierProvider） |
| `artworks.svelte.ts` 的 `ArtworkIndex` 单例 | `artworksProvider`（NotifierProvider） |
| `theme.svelte.ts` 的 `ThemeStore` 单例 | `themeProvider`（NotifierProvider） |
| `$state` 字段 | Notifier 的 `state` 字段 |
| `$derived` | `ref.watch` + 派生 Provider |
| `$effect` | `ref.listen` / `Widget.build` 内的 `ref.watch` |

选择 Riverpod 而非 Bloc 的理由：
- 现有代码大量使用「直接修改对象字段然后触发重渲染」（如 `image.isFavorite = ...`），
  Riverpod 的 Notifier 允许这种写法（只要调用 `state = state` 或用 `update`），
  Bloc 需要为每个操作定义 Event，迁移成本高。
- Riverpod 的依赖注入比 Provider 更灵活，适合多层嵌套的状态依赖。
- 代码生成（`riverpod_generator`）让 Provider 声明更简洁。

### 4.3 数据层（api 封装）

现有 `api.ts` 把所有 Tauri invoke 收口在一个对象里。Dart 端做同样的事：

```dart
// lib/core/api/api.dart
class GalleryApi {
  final RustApi _rust; // FRB 生成的 API 实例
  GalleryApi(this._rust);

  Future<List<Library>> listLibraries() => _rust.listLibraries();
  Future<Library> addLibrary(String path) => _rust.addLibrary(path: path);
  // ... 其余 50+ 个方法
}
```

**类型对齐**：Rust 的 `serde` 序列化格式与现有 TS 类型一致（camelCase）。
FRB 会根据 Rust struct 自动生成 Dart class，字段名用 `snake_case` 还是 `camelCase`
取决于 FRB 配置。建议在 Rust 端加 `#[serde(rename_all = "camelCase")]`，
FRB 生成的 Dart 代码也用 camelCase，与现有 TS 类型完全一致，减少心智负担。

### 4.4 主题系统

现有主题是「配色方案 × 窗口材质 × 明暗」三轴，9 套配色 + 4 种材质 + 明暗。
Flutter 端用 `ThemeExtension` 实现：

```dart
class AppTheme extends ThemeExtension<AppTheme> {
  final Palette palette;       // 当前生效的调色板
  final MaterialId material;   // 窗口材质
  final bool dark;             // 实际明暗
  // ...
}
```

配色令牌（`--tone-*`）转为 Dart 的 `Color` 常量，材质透明度（`--alpha-*`）
转为 `Color.withValues(alpha: ...)`。窗口材质通过 `flutter_acrylic` 插件下发。

**首帧不闪烁**：现有用 `localStorage` 做首帧缓存。Flutter 端用 `SharedPreferences`
在 `main()` 里同步读取（`await`），然后构建 `MaterialApp`，第一帧就是正确主题。

---

## 五、核心模块迁移方案

### 5.1 虚拟滚动（VirtualGrid / VirtualMasonry）

现有两套虚拟滚动：
- `VirtualGrid.svelte`：定高网格，纯 CSS + 常量几何
- `VirtualMasonry.svelte`：瀑布流，绝对定位 + 最短列优先

Flutter 端的对应方案：

**定高网格**：用 `GridView.builder` + `SliverGridDelegateWithFixedCrossAxisCount`，
Flutter 的 `GridView` 本身就是虚拟化的（只构建可见子项）。**不需要自己写虚拟滚动**，
这是 Flutter 比 WebView 最大的优势之一。

```dart
GridView.builder(
  gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
    crossAxisCount: columns,
    crossAxisSpacing: gap,
    mainAxisSpacing: gap,
    childAspectRatio: aspect,
  ),
  itemCount: total,
  itemBuilder: (context, index) {
    final artwork = ref.watch(artworksProvider).at(index);
    if (artwork == null) return const SkeletonCard();
    return ArtworkCard(artwork: artwork);
  },
)
```

**瀑布流**：Flutter 没有内置瀑布流，用 `flutter_staggered_grid_view` 的
`MasonryGridView.count`，同样是虚拟化的。高宽比从 `aspects()` 里取。

**保留阅读位置**：现有用 `initialIndex` 记录「当前看的是第几条」。Flutter 端用
`ScrollController.initialScrollOffset` 或 `PageStorageKey` 保存滚动位置。
网格 ↔ 瀑布流切换时，用数据下标换算目标偏移。

### 5.2 图片查看器（ImageViewer）

这是迁移难度最高的模块。现有实现包含：
- 三层光锥视差（`preserve-3d` + 多层 `translateZ`）
- 鼠标跟随倾斜 + 扫光
- 滚轮缩放（锚在光标下）
- 双击铺满
- 拖拽平移
- 触屏：双指捏合、单指滑动切图、下滑关闭
- 重力感应（手机）
- 全屏、复制图片、设为壁纸

Flutter 端的实现策略：

**光锥视差**：用 `Stack` + `Transform` + `Transform.translate` 实现多层 Z 轴。
Flutter 的 `Transform` 支持 `Matrix4` 做 3D 变换（`setEntry(3, 2, 0.001)` 加透视）。
倾斜用 `Matrix4.rotationX` / `rotationY`，扫光用 `ShaderMask` 或自定义 `Painter`。

**缩放 / 平移**：用 `InteractiveViewer`（Flutter 内置），它原生支持缩放、平移、
双指捏合，且性能优秀。但 `InteractiveViewer` 的缩放锚点是中心，要实现「滚轮锚在
光标下」需要自定义 `TransformationController`，在 `onInteractionUpdate` 里
根据光标位置调整 `transformation`。

**手势**：`InteractiveViewer` 处理了缩放和平移，但滑动切图 / 下滑关闭需要
叠加 `GestureDetector`。注意手势竞争（`GestureArena`）——捏合时不能触发滑动。

**重力感应**：用 `sensors_plus` 插件的 `accelerometerEventStream` 或
`gyroscopeEventStream`，与现有 `GravitySensor` / `deviceorientation` 逻辑对齐。

**建议拆分**：把查看器拆成两层——
1. `ViewerCore`：纯图片显示 + 缩放平移（用 `InteractiveViewer`）
2. `ViewerEffects`：光锥视差、扫光、倾斜（用 `Stack` + `Transform` 叠在上面）
两层解耦，特效可以开关（对应现有 `effectsEnabled`）。

### 5.3 筛选面板（FilterPanel）

现有两种形态：底部浮层（sheet）/ 右侧抽屉（drawer），可切换。
Flutter 端用 `showModalBottomSheet` 或自定义 `SlideTransition` 实现。
筛选维度（分级 / 画师 / 标签）的数据从 `galleryProvider` / `artworksProvider` 取。

### 5.4 侧栏导航（Sidebar）

桌面常驻侧栏 + 手机抽屉。Flutter 端：
- 桌面：`NavigationRail` 或自定义 `Drawer`（常驻）
- 手机：`Drawer`（滑出）+ `Scaffold.drawer`
- 侧栏数据（图库、相册、标签、标签图库、分级计数）从 `galleryProvider` 取

### 5.5 缩略图加载

现有缩略图路径由 Rust 生成并存储，卡片直接用路径渲染。Flutter 端：
- 用 `Image.file(File(thumbnailPath))` + `cacheWidth` / `cacheHeight` 限制解码尺寸
- 失败回退：调用 `api.thumbnailForArtwork(id)` 重新生成
- 骨架淡入：`FadeInImage` 或自定义 `AnimatedOpacity`

### 5.6 外观设置页

现有 `/appearance` 页面展示 9 套配色 × 4 材质 × 明暗。Flutter 端用 `GridView`
展示配色方案卡片，材质用 `SegmentedButton`，明暗用 `Switch`。
状态写入 `themeProvider`，持久化到 Rust 的 `settings` 表（通过 `saveSettings` API）。

---

## 六、分阶段实施计划

### 阶段 0：技术验证（1 周）

**目标**：验证关键风险点，避免大规模投入后发现走不通。

**任务**：
1. 初始化 Flutter 项目，集成 `flutter_rust_bridge`
2. 把 `list_libraries` + `list_images` 两个命令从 Tauri 改为 FRB 暴露
3. Flutter 端调用成功，显示在一个简单的 `ListView` 里
4. 验证 `GridView.builder` 对 5000 条数据的滚动性能（与现有对比）
5. 验证 `InteractiveViewer` 的缩放跟手性
6. 验证 Rust 端 Stream（扫描进度）能否推送到 Dart

**交付物**：可运行的最小 Demo，证明 FRB + 虚拟滚动 + 手势可行

**通过标准**：
- Flutter 端能拿到 Rust 返回的数据
- 5000 条 GridView 滚动帧率 ≥ 60fps
- InteractiveViewer 缩放无明显卡顿

### 阶段 1：数据层 + 状态层（2 周）

**目标**：把所有 Rust 命令迁移到 FRB，Dart 端建好 api 封装和 Riverpod providers。

**任务**：
1. 全部 50+ 个命令改为 FRB 暴露，生成 Dart 绑定
2. 三个事件（扫描 / 缩略图 / 元数据进度）改为 FRB Stream
3. `lib/core/api/api.dart` 封装所有 API
4. `lib/core/models/` 定义所有数据模型（或用 FRB 生成的）
5. `galleryProvider`、`artworksProvider`、`themeProvider` 迁移完成
6. Android 桥接（MediaStore、壁纸、沉浸式）通过 JNI 或 Platform Channel 跑通

**交付物**：Dart 端可以调用全部后端功能，但还没有完整 UI

### 阶段 2：核心浏览功能（3 周）

**目标**：通用图库 + 作品库的浏览体验可用。

**任务**：
1. 主界面布局（侧栏 + 内容区 + 工具条）
2. 通用图库：`GridView.builder` 虚拟滚动 + 卡片 + 搜索 + 排序
3. 作品库：网格 / 瀑布流切换 + 虚拟滚动
4. 缩略图加载 + 失败回退
5. 筛选面板（分级 / 画师 / 标签）
6. 侧栏导航（图库 / 相册 / 标签 / 作品库 / 标签图库）
7. 收藏 / 评分操作

**交付物**：可以浏览、搜索、筛选、收藏图片，但还没有查看器

### 阶段 3：图片查看器（2 周）

**目标**：检视弹窗功能完整，交互流畅。

**任务**：
1. `ViewerCore`：`InteractiveViewer` 缩放平移 + 双击铺满 + 滚轮锚点
2. `ViewerEffects`：光锥视差 + 鼠标倾斜 + 扫光（桌面）
3. 重力感应（手机）
4. 触屏手势：双指捏合、滑动切图、下滑关闭
5. 全屏、复制图片、设为壁纸
6. 键盘快捷键（Esc / ←→ / +- / 0 / 数字键评分 / F）

**交付物**：完整的图片查看体验

### 阶段 4：完善功能（2 周）

**目标**：补齐剩余功能，达到与现有版本对等。

**任务**：
1. 扫描功能（添加图库目录 + 扫描进度）
2. 相册管理（创建 / 重命名 / 删除 / 添加图片）
3. 标签管理（创建 / 编辑 / 删除 / 打标签）
4. Pixiv 元数据补全（批量抓取 + 进度 + 中止）
5. 标签图库（创建 / 删除）
6. 外观设置页（配色 × 材质 × 明暗）
7. 缓存管理（查看 / 清空缩略图缓存）
8. Android 返回手势栈

**交付物**：功能与现有版本完全对等

### 阶段 5：性能优化 + 打磨（1 周）

**目标**：流畅度达标，修复边角问题。

**任务**：
1. 帧率 profiling，优化掉帧场景
2. 图片缓存调优（内存 + 磁盘）
3. 动画曲线调优（与现有手感对齐）
4. 桌面 / Android 两端真机验收
5. 产物体积优化（Rust strip + Flutter tree-shake）

**交付物**：可发布的 Flutter 版本

---

## 七、工作量评估

| 阶段 | 预估工时 | 说明 |
| --- | --- | --- |
| 阶段 0 技术验证 | 1 周 | 关键风险验证 |
| 阶段 1 数据层 + 状态层 | 2 周 | 50+ 命令迁移 + 3 个 provider |
| 阶段 2 核心浏览 | 3 周 | 两套虚拟滚动 + 筛选 + 侧栏 |
| 阶段 3 图片查看器 | 2 周 | 最复杂的模块，手势 + 3D 视差 |
| 阶段 4 完善功能 | 2 周 | 相册 / 标签 / 元数据 / 外观 |
| 阶段 5 优化打磨 | 1 周 | profiling + 真机验收 |
| **合计** | **11 周** | 约 2.5 个月（单人） |

**不确定因素**：
- FRB 对 Rust 复杂类型（嵌套 struct、enum、Result）的支持可能有坑，阶段 1 可能 +1 周
- ImageViewer 的光锥视效用 Flutter 实现可能比预期难，阶段 3 可能 +1 周
- Android MediaStore 桥接如果走 Platform Channel 而非 JNI，可能需要额外写 Kotlin 代码

---

## 八、关键风险与应对

### 风险 1：FRB 对现有 Rust 代码的兼容性

**问题**：FRB 要求函数参数和返回值是「FRB 支持的类型」。现有代码用了 `State<'_, AppState>`、
`tauri::Emitter` 等 Tauri 专属类型，以及一些复杂的 serde 类型。

**应对**：阶段 0 先挑最复杂的几个命令（如 `list_artworks` 返回带嵌套字段的 struct、
`fetch_missing_meta` 返回带可选字段的 struct）验证 FRB 能否正确生成绑定。
如果不行，在 Rust 端加一层薄薄的 DTO 转换。

### 风险 2：光锥视差在 Flutter 上的还原度

**问题**：现有光锥视差靠 CSS 的 `preserve-3d` + 多层 `translateZ` + `mix-blend-mode`
实现。Flutter 的 3D Transform 能力有限（没有真正的 3D 空间，只有矩阵变换），
`mix-blend-mode` 也需要 `ColorFilter` 或自定义 `Painter` 模拟。

**应对**：阶段 0 做一个光锥视差的最小 Demo，对比视觉效果。如果差距大，
考虑简化特效（去掉混合模式层，保留倾斜 + 扫光），或者用 `CustomPainter` 手绘。

### 风险 3：Android 端的文件访问权限

**问题**：Android 11+ 的分区存储限制，现有通过 SAF + `media-store-bridge` 绕过。
Flutter 端如果用 `Image.file(path)` 直接读，可能遇到权限问题。

**应对**：Rust 端已经处理了 SAF 权限授予（通过 JNI），Flutter 端只需拿到路径即可。
但需要验证：Rust 端授予的 URI 权限，Flutter 进程能否直接用 `File(path)` 读取。
如果不行，需要在 Rust 端把文件内容读出来返回给 Flutter（性能差，不推荐），
或者通过 ContentResolver 读取。

### 风险 4：产物体积

**问题**：现有 Tauri release 包 Android 只有 28MB。Flutter 自带引擎（约 5-10MB），
加上 Rust 编译产物，可能涨到 40-50MB。

**应对**：
- Flutter 开启 `--split-debug-info` + `--obfuscate`
- Rust 开启 `lto = true` + `strip = true`
- 移除未使用的 Flutter 插件
- 目标是 ≤ 35MB，仍比同类 Pixiv 客户端（pixez 53MB）小

### 风险 5：迁移期间的功能对等

**问题**：现有版本功能完整，如果迁移周期长，用户可能需要在旧版本和新版本之间切换。

**应对**：
- Rust 后端的数据库 schema 不变，新旧版本共用同一个 `gallery.db`
- 缩略图缓存目录结构不变，新旧版本共用
- 迁移期间不改动 Rust 业务逻辑，只改暴露层，确保旧版本仍可构建

---

## 九、不迁移的部分（保持不变）

- 数据库 schema 与迁移逻辑（`db/schema.rs`）
- 扫描、索引、去重逻辑（`scanner.rs`、`artworks.rs`）
- 缩略图生成（`thumbnails.rs`）
- Pixiv 联网抓取（`pixiv.rs`）
- 壁纸设置的 Windows FFI 部分（`wallpaper.rs`）
- 所有 `DESIGN-NOTES.md` 里记录的设计决策与坑

这些是项目的核心资产，迁移时一行都不应该改。

---

## 十、需要确认的问题

1. **光锥视差的还原标准**：是必须 1:1 还原现有视觉效果，还是可以接受简化版
   （保留倾斜 + 扫光，去掉混合模式层）？这直接影响阶段 3 的工时。

2. **桌面平台范围**：现有桌面支持 Windows / macOS / Linux。Flutter 桌面端
   macOS 和 Linux 的窗口材质支持不如 Windows，是否需要三端都支持，还是优先 Windows？

3. **数据库兼容性**：是否要求新旧版本共用同一个数据库？如果是，Rust 端的 schema
   在迁移期间不能改。

4. **迁移期间是否继续维护现有 Svelte 版本**：如果 Flutter 迁移需要 2-3 个月，
   期间现有版本的 bug 修复和功能迭代如何处理？
