package com.lightcone.gallery

import android.graphics.Color
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  /** wry 异步创建的 WebView；返回手势靠它把事件转发给前端 */
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // 应用永远是深色界面：系统栏图标钉死为浅色，不跟系统主题走 ——
    // 否则系统浅色模式下图标会被画成深色，压在深色界面上看不见
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
    )
    super.onCreate(savedInstanceState)
    // 全面屏返回手势拦截：TauriActivity 关掉了 wry 的历史后退
    // （handleBackNavigation=false），默认行为是直接结束 Activity（应用退出）。
    // 这里把返回转发给前端：弹窗 → 抽屉先关，都空了前端再自己调 exit_app。
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val view = webView
        if (view == null) {
          // 冷启动 WebView 还没建好：保持系统默认（退出）
          isEnabled = false
          onBackPressedDispatcher.onBackPressed()
          isEnabled = true
          return
        }
        view.evaluateJavascript("window.dispatchEvent(new CustomEvent('lc-android-back'))", null)
      }
    })
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
  }
}
