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
    buildToolsVersion = "36.0.0" // Nix SDK provides only 36.0.0; AGP's default (35.0.0) would try to auto-install into the read-only store

    defaultConfig {
        applicationId = "art.infinitescroll.control"
        minSdk = 33 // typed GATT write/read callbacks, BLUETOOTH_SCAN/CONNECT permissions
        targetSdk = 36
        versionCode = 8
        versionName = "0.1.7"
    }

    // Release signing. Credentials come from environment variables (see
    // docs/android-build-nix.md), so nothing secret lives in the repo and
    // debug builds work without them. IS_KEYSTORE is a JKS/PKCS12 whose key
    // and store passwords are equal (keytool's PKCS12 limitation).
    signingConfigs {
        create("release") {
            System.getenv("IS_KEYSTORE")?.let { path ->
                storeFile = file(path)
                storePassword = System.getenv("IS_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("IS_KEY_ALIAS") ?: "release"
                keyPassword = System.getenv("IS_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true      // R8 shrinks and obfuscates
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = signingConfigs.getByName("release")
        }
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
