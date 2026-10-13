# Tauri 的 Android 插件桥**全靠反射**，R8 从代码里看不见这些引用：
#
#   · `register_android_plugin("com.lightcone.gallery.media", "MediaStoreBridge")`
#     是按**类名字符串**建插件实例的 —— 类被改名就找不到；
#   · 建好之后再扫方法上的 `@Command` / `@ActivityCallback` 注解，按**方法名**
#     匹配 Rust 侧传来的命令名（`pickDirectory` / `keepAlive` / `listBuckets` …）
#     —— 方法被改名或删掉就报「命令不存在」。
#
# 默认混淆会把整个类删掉（R8 认为没人引用它），而 **debug 包不混淆所以完全正常**，
# 于是这个问题只在 release 上出现，特别容易漏。
#
# 本文件由 media-store-bridge/build.gradle.kts 的 `consumerProguardFiles` 引入 ——
# 之前那个声明指向的文件并不存在（被 gradle.properties 的
# `android.proguard.failOnMissingFiles=false` 掩盖了），等于没有任何保护。
-keep class com.lightcone.gallery.media.** { *; }

# 注解本身也要留着，否则扫不到 @Command / @ActivityCallback
-keepattributes RuntimeVisibleAnnotations,AnnotationDefault
