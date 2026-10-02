pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
    // Versions only; a plugin is resolved when a module applies it, so the
    // Android plugins are fetched only when :app is included.
    plugins {
        kotlin("jvm") version "2.1.21"
        kotlin("android") version "2.1.21"
        kotlin("plugin.serialization") version "2.1.21"
        kotlin("plugin.compose") version "2.1.21"
        id("com.android.application") version "8.10.1"
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "infinite-scroll-android"
include(":protocol")
// The Android application module needs the Android SDK; include it only when
// one is configured, so the protocol module builds and tests anywhere.
if (System.getenv("ANDROID_HOME") != null || System.getenv("ANDROID_SDK_ROOT") != null || file("local.properties").exists()) {
    include(":app")
}
