package com.lightcone.gallery.media

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.MediaStore
import androidx.activity.result.ActivityResult
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.tauri.plugin.Plugin
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.io.File

/**
 * MediaStore 桥：本应用在 Android 上是「索引层」——pixez 负责下图，
 * 这里只负责把全机图片里符合 pixez 命名（`{pid}_p{n}.{ext}`）的条目
 * 枚举出来交给 Rust。
 *
 * 为什么走 MediaStore 而不是文件系统：分区存储下其他应用的图片按路径
 * 直读不可靠，MediaStore 是图库类应用的正规通道，`READ_MEDIA_IMAGES`
 * 是普通运行时权限。查询自带宽高与大小，索引阶段连文件头都不用读。
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.READ_MEDIA_IMAGES], alias = "mediaImages")
  ]
)
class MediaStoreBridge(private val activity: Activity) : Plugin(activity) {
  private val pixivName =
    Regex("^(\\d+)_p(\\d+)\\.(jpg|jpeg|png|gif|webp|bmp)$", RegexOption.IGNORE_CASE)

  /** 查询数据列的顺序与 readRow 的索引一一对应。 */
  private val projection = arrayOf(
    MediaStore.Images.Media._ID,
    MediaStore.Images.Media.DISPLAY_NAME,
    MediaStore.Images.Media.RELATIVE_PATH,
    MediaStore.Images.Media.SIZE,
    MediaStore.Images.Media.DATE_MODIFIED,
    MediaStore.Images.Media.WIDTH,
    MediaStore.Images.Media.HEIGHT,
  )

  /** 当前权限状态。MediaStore 在未授权时只回自己拥有的条目（对我们=空），所以扫描前必须问一次。 */
  @Command
  override fun checkPermissions(invoke: Invoke) {
    val result = JSObject()
    result.put("granted", hasMediaPermission())
    invoke.resolve(result)
  }

  /** 触发系统权限对话框；结果经 @PermissionCallback 回来再 resolve。 */
  @Command
  override fun requestPermissions(invoke: Invoke) {
    requestPermissionForAlias("mediaImages", invoke, "permissionsCallback")
  }

  @PermissionCallback
  fun permissionsCallback(invoke: Invoke) {
    val result = JSObject()
    result.put("granted", hasMediaPermission())
    invoke.resolve(result)
  }

  private fun hasMediaPermission(): Boolean =
    activity.checkSelfPermission(Manifest.permission.READ_MEDIA_IMAGES) ==
      PackageManager.PERMISSION_GRANTED

  @Command
  fun listPixivImages(invoke: Invoke) {
    val items = ArrayList<JSObject>()
    activity.contentResolver.query(
      MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
      projection,
      null,
      null,
      null,
    )?.use { cursor ->
      while (cursor.moveToNext()) {
        val name = cursor.getString(1) ?: continue
        val match = pixivName.find(name) ?: continue
        val relativePath = (cursor.getString(2) ?: "").trimEnd('/')
        val row = JSObject()
        row.put("id", cursor.getLong(0))
        row.put("name", name)
        // 真实文件路径：FUSE 会把带 READ_MEDIA_IMAGES 的应用放行到媒体文件
        // （Android 11+），Rust 侧的缩略图管线按路径读就行
        row.put("path", "/storage/emulated/0/$relativePath/$name")
        row.put("pid", match.groupValues[1])
        row.put("page", match.groupValues[2].toInt())
        row.put("size", cursor.getLong(3))
        row.put("modified", cursor.getLong(4))
        row.put("width", cursor.getInt(5))
        row.put("height", cursor.getInt(6))
        items.add(row)
      }
    }
    val result = JSObject()
    result.put("items", JSArray.from(items.toTypedArray()))
    invoke.resolve(result)
  }

  /**
   * 兜底读取：把某个 MediaStore 条目拷进应用私有 cache。路径直读万一被
   * 设备策略挡住（部分机型的 FUSE 实现不一致），缩略图与弹窗原图改走这条。
   */
  @Command
  fun copyToCache(invoke: Invoke) {
    val args = invoke.parseArgs(CopyArgs::class.java)
    val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, args.id)
    val dir = File(activity.cacheDir, "bridge").apply { mkdirs() }
    val target = File(dir, args.name)
    activity.contentResolver.openInputStream(uri)?.use { input ->
      target.outputStream().use { output -> input.copyTo(output) }
    }
    val result = JSObject()
    result.put("path", target.absolutePath)
    invoke.resolve(result)
  }

  /**
   * 全机图片的目录清单：每个有图的文件夹 + 图片数（含子文件夹），按数量倒序。
   *
   * 给应用内的「选目录」面板用 —— 系统 SAF 选择器（DocumentsUI）浏览
   * Pictures 这种上万文件的目录会崩溃循环（HyperOS 缺陷，实测），而
   * MediaStore 里现成就有分桶数据，自己列又快又稳。
   *
   * **祖先目录也聚合**：`Pictures/pixez/画师A` 的图会同时累进
   * `Pictures/pixez` 和 `Pictures` —— pixez 下直属没有图片（图都在画师
   * 子文件夹里），不展开祖先的话主人想「整个 pixez 收进来」就找不到条目。
   * 选中任何一级都是 walkdir 递归扫描，语义一致。
   *
   * 用简单查询 + Kotlin 侧聚合，不碰 content provider 的 SQL 聚合
   * （各 ROM 的 MediaProvider 对 projection 校验不一，最兼容）。
   */
  @Command
  fun listBuckets(invoke: Invoke) {
    val counts = HashMap<String, Int>()
    activity.contentResolver.query(
      MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
      arrayOf(MediaStore.Images.Media.RELATIVE_PATH),
      "${MediaStore.Images.Media.RELATIVE_PATH} IS NOT NULL",
      null,
      null,
    )?.use { cursor ->
      val column = cursor.getColumnIndexOrThrow(MediaStore.Images.Media.RELATIVE_PATH)
      while (cursor.moveToNext()) {
        val dir = cursor.getString(column)?.trimEnd('/') ?: continue
        if (dir.isEmpty()) continue
        // 每张图对路径的每一级祖先 +1：叶子目录是直属数，父目录是子孙总数
        val parts = dir.split('/')
        for (depth in 1..parts.size) {
          val ancestor = parts.take(depth).joinToString("/")
          counts[ancestor] = (counts[ancestor] ?: 0) + 1
        }
      }
    }
    val buckets = JSArray()
    counts.entries
      .sortedByDescending { it.value }
      .forEach { (dir, count) ->
        val o = JSObject()
        o.put("path", dir)
        o.put("count", count)
        buckets.put(o)
      }
    val result = JSObject()
    result.put("buckets", buckets)
    invoke.resolve(result)
  }

  /**
   * 前台服务开关：元数据补全跑几十分钟，跑的时候把应用挂到前台服务上
   * （通知栏常驻），豁免「应用后台断网」的系统省电策略 —— 不然主人一切
   * 走应用，网络立刻被掐（真机实测 netpolicy blocked=APP_BACKGROUND）。
   */
  @Command
  fun keepAlive(invoke: Invoke) {
    // 参数直读原始 JSON（parseArgs 的 Jackson 坑见 immersive 的注释）
    val enabled = org.json.JSONObject(invoke.getRawArgs().toString()).optBoolean("enabled", false)
    if (enabled) MetaKeepAliveService.start(activity) else MetaKeepAliveService.stop(activity)
    invoke.resolve(JSObject())
  }

  @app.tauri.annotation.InvokeArg
  data class CopyArgs(val id: Long, val name: String)

  /**
   * SAF 目录选择器：主人自选一个文件夹作为图库根目录 —— 和桌面端同一套
   * 「选目录 → walkdir 扫描」的心智，不像 MediaStore 扫描那样只能全机收。
   *
   * 流程：系统目录选择器 → 持久化授权 → tree URI 折算成真实物理路径
   * （`primary:Pictures/pixez` → `/storage/emulated/0/Pictures/pixez`）。
   * Rust 那边拿到路径后照常 walkdir，读文件走 FUSE（READ_MEDIA_IMAGES
   * 已持有，公共目录的媒体文件按路径可读）。
   */
  @Command
  fun pickDirectory(invoke: Invoke) {
    // Rust 侧经 spawn_blocking 进来，调用线程不是主线程 —— ActivityResultLauncher
    // 的 launch 必须在主线程，否则 SAF 窗口行为不稳定，切回 UI 线程再启动。
    //
    // 优先显式调用**第三方文件管理器**的 SAF 实现：设备上装的播放器/文件管理器
    // 只要注册了 OPEN_DOCUMENT_TREE 就能接这个活（它们的浏览 UI 不会像系统
    // DocumentsUI 那样浏览上万文件的目录就崩）。没有第三方才回落系统默认。
    val safIntent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
    val thirdParty = activity.packageManager
      .queryIntentActivities(safIntent, 0)
      .firstOrNull {
        it.activityInfo.packageName != "com.google.android.documentsui" &&
          it.activityInfo.packageName != "com.android.documentsui"
      }
    if (thirdParty != null) {
      safIntent.setClassName(
        thirdParty.activityInfo.packageName,
        thirdParty.activityInfo.name,
      )
    }
    // CLEAR_TASK：DocumentsUI 的 PickActivity 会卡在半死状态（HyperOS 上「允许
    // 访问」流程中断后，窗口 visibleRequested=false 的实例留在任务里）——之后
    // 每次 launch 都 warm-start 这个僵尸实例、一恢复就 finish（用户看到的
    // 「允许窗口闪一下就回去」）。每次清任务强制全新实例。
    safIntent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
    activity.runOnUiThread {
      startActivityForResult(invoke, safIntent, "directoryCallback")
    }
  }

  /**
   * 结果回调。签名必须是 `(Invoke, ActivityResult)` 两个参数 —— PluginHandle
   * 用反射按这个形状调（第一版写成 `(Invoke, Int, Intent?)` 三个参数，反射
   * 直接抛 Wrong number of arguments，主人每选一次目录就闪退一次，血的教训）。
   */
  @ActivityCallback
  fun directoryCallback(invoke: Invoke, result: ActivityResult) {
    val payload = JSObject()
    val uri: Uri? = result.data?.data
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      // 主人取消或选择器异常：path 置空，上层静默返回
      payload.put("path", "")
      invoke.resolve(payload)
      return
    }
    // 持久化授权：应用重启后如果需要走 content:// 兜底读取仍然有效
    try {
      activity.contentResolver.takePersistableUriPermission(
        uri,
        Intent.FLAG_GRANT_READ_URI_PERMISSION,
      )
    } catch (_: SecurityException) {
      // 个别机型的选择器不给 persistable 权限：不致命，路径直读照常
    }
    val path = treeUriToPath(uri)
    payload.put("path", path ?: "")
    invoke.resolve(payload)
  }

  /**
   * SAF tree URI → 物理路径。只支持主存储（`primary:` 前缀）—— 内置存储
   * 的映射是稳定的；SD 卡的 tree documentId 不是稳定文件路径，返回 null
   * 让上层如实告诉主人「暂不支持外置存储卡」。
   */
  private fun treeUriToPath(uri: Uri): String? {
    val docId = try {
      DocumentsContract.getTreeDocumentId(uri)
    } catch (_: IllegalArgumentException) {
      return null
    }
    if (!docId.startsWith("primary:")) return null
    val relative = docId.substringAfter(':')
    if (relative.isEmpty()) return null
    val base = System.getenv("EXTERNAL_STORAGE") ?: "/storage/emulated/0"
    return "$base/$relative"
  }

  /**
   * 沉浸式全屏：隐藏 / 恢复状态栏与手势条。
   *
   * 手机上没有「窗口全屏」这回事，wry 的 WebChromeClient 对 HTML Fullscreen
   * 也是空壳（onShowCustomView 直接回调 onCustomViewHidden），所以检视弹窗的
   * 全屏按钮走这里 —— insets controller 隐藏系统栏，从屏幕边缘滑动临时呼出、
   * 松手自动收回。
   *
   * 参数用 getRawArgs 直读（org.json），不走 parseArgs —— 那条路是 Jackson
   * 反序列化，运行时缺 Kotlin module 时 data class 会报
   * 「no property-based Creator」（实测）。
   */
  @Command
  fun immersive(invoke: Invoke) {
    // tauri 的 JSObject 不是完整 org.json：没有 optBoolean。转成原始 JSON
    // 文本再交给 Android 自带的 org.json 解析，绕开 parseArgs 的 Jackson。
    val enabled = org.json.JSONObject(invoke.getRawArgs().toString()).optBoolean("enabled", false)
    activity.runOnUiThread {
      val window = activity.window
      val controller = WindowCompat.getInsetsController(window, window.decorView)
      val bars = WindowInsetsCompat.Type.statusBars() or WindowInsetsCompat.Type.navigationBars()
      if (enabled) {
        controller.systemBarsBehavior =
          WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(bars)
      } else {
        controller.show(bars)
      }
      invoke.resolve(JSObject())
    }
  }
}
