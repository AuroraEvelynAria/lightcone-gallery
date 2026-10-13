import java.util.Properties
import org.gradle.api.provider.ValueSource
import org.gradle.api.provider.ValueSourceParameters
import org.gradle.api.file.RegularFileProperty
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

// ---- rustls-platform-verifier 的 Kotlin 组件 -------------------------------
// reqwest 0.13 在 Android 上的 TLS 证书校验靠它（Rust 侧经 JNI 调 Java 的
// CertificateVerifier），没进 APK 的话每次 HTTPS 都报
// 「failed to call native verifier」。组件（pom/aar）已用镜像下载进本地
// maven 目录（gen/android/local-maven），构建期零网络依赖。
repositories {
    maven { url = uri(rootProject.file("local-maven")) }
}

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion("0.2.0")
            because("native component version must be identical to version of the rustls-platform-verifier crate (0.7.1)")
        }
    }
}
// ---------------------------------------------------------------------------

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "com.lightcone.gallery"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.lightcone.gallery"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    /**
     * release 的签名配置，从 `gen/android/keystore.properties` 读（该文件被 .gitignore
     * 忽略，口令绝不进仓库）。文件不存在时整块跳过 —— 于是 `assembleRelease` 仍然能跑，
     * 只是产出一个装不上的 `-unsigned.apk`。
     *
     * 本机现在指向 **Android 调试密钥库**（`~/.android/debug.keystore`）。这是刻意的：
     * release 包和之前一直装的 debug 包**签名相同**，可以直接覆盖升级，不用卸载 ——
     * 卸载会连带清掉应用数据，手机里那套索引和元数据就没了。
     *
     * 将来要正式发行时换成自己的 keystore 即可，代价是必须先从手机上卸载现有应用。
     */
    val keystoreProps = Properties().apply {
        val propsFile = rootProject.file("keystore.properties")
        if (propsFile.exists()) propsFile.inputStream().use { load(it) }
    }
    signingConfigs {
        if (keystoreProps.isNotEmpty()) {
            create("release") {
                storeFile = file(keystoreProps.getProperty("storeFile"))
                storePassword = keystoreProps.getProperty("storePassword")
                keyAlias = keystoreProps.getProperty("keyAlias")
                keyPassword = keystoreProps.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            if (keystoreProps.isNotEmpty()) {
                signingConfig = signingConfigs.getByName("release")
            }
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation(project(":media-store-bridge"))
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    // reqwest 0.13 在 Android 上的 TLS 校验 Kotlin 组件（版本动态对齐 Cargo.lock）
    implementation("org.rustls:rustls-platform-verifier:0.2.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
