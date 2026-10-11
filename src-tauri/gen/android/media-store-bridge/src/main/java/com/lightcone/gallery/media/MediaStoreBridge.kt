package com.lightcone.gallery.media

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.pm.PackageManager
import android.provider.MediaStore
import app.tauri.plugin.Plugin
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

  @app.tauri.annotation.InvokeArg
  data class CopyArgs(val id: Long, val name: String)
}
