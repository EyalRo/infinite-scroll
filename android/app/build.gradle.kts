plugins {
    id("com.android.application")
    kotlin("android")
    kotlin("plugin.compose")
}

android {
    namespace = "art.infinitescroll.control"
    // Pixel on Android 17 is the target device; raise compileSdk/targetSdk to
    // the installed API 37 platform when building against it.
    compileSdk = 36

    defaultConfig {
        applicationId = "art.infinitescroll.control"
        minSdk = 33 // typed GATT write/read callbacks, BLUETOOTH_SCAN/CONNECT permissions
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation(project(":protocol"))
    implementation(platform("androidx.compose:compose-bom:2025.05.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.0")
}
