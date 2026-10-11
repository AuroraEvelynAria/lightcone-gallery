package com.lightcone.gallery.media

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat

/**
 * 元数据补全的前台服务：显示一条常驻通知，把应用标成「用户可感知的前台任务」，
 * 豁免系统的后台断网策略 —— 否则主人点完「自动补全」一切走应用，网络立刻被
 * 系统掐掉，几十分钟的补全全部超时（真机实测）。
 */
class MetaKeepAliveService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        acquireLocks()
        startForeground(NOTIFICATION_ID, buildNotification())
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // 服务被系统回收后重启时也保持前台通知与锁
        acquireLocks()
        startForeground(NOTIFICATION_ID, buildNotification())
        return START_STICKY
    }

    override fun onDestroy() {
        wifiLock?.release()
        wakeLock?.release()
        wifiLock = null
        wakeLock = null
        super.onDestroy()
    }

    /**
     * WiFi 高性能锁 + CPU 部分 Wake 锁：部分 ROM 的后台断网判定会参考这些锁，
     * 标准 Android 上也是长任务的标准配置。MIUI 的私有省电策略可能仍然无视
     * （那种情况只能主人在设置里把省电策略改成「无限制」）。
     */
    private var wifiLock: WifiManager.WifiLock? = null
    private var wakeLock: PowerManager.WakeLock? = null

    private fun acquireLocks(): Unit {
        if (wifiLock == null) {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "lightcone:meta")
              .apply { setReferenceCounted(false); acquire() }
        }
        if (wakeLock == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "lightcone:meta")
              .apply { setReferenceCounted(false); acquire(45 * 60 * 1000L) }
        }
    }

    private fun buildNotification(): Notification {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        // Android 8+ 必须先建渠道；IMPORTANCE_LOW = 不响铃不弹横幅，安静常驻
        manager.createNotificationChannel(
          NotificationChannel(CHANNEL_ID, "元数据补全", NotificationManager.IMPORTANCE_LOW),
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
          .setContentTitle("光锥映画")
          .setContentText("正在补全元数据，请保持应用在后台运行")
          .setSmallIcon(android.R.drawable.stat_notify_sync)
          .setOngoing(true)
          .build()
    }

    companion object {
        private const val CHANNEL_ID = "meta_keep_alive"
        private const val NOTIFICATION_ID = 1001

        fun start(context: Context): Unit {
            val intent = Intent(context, MetaKeepAliveService::class.java)
            context.startForegroundService(intent)
        }

        fun stop(context: Context): Unit {
            context.stopService(Intent(context, MetaKeepAliveService::class.java))
        }
    }
}
