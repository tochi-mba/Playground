import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// CI sets these (see .github/workflows/release.yml). Local builds fall back to dev values.
val ciVersionCode = providers.environmentVariable("VERSION_CODE").map { it.toInt() }.orElse(1)
val ciVersionName = providers.environmentVariable("VERSION_NAME").orElse("0.1.0-dev")

// Optional real release key, supplied through environment variables by CI secrets.
val releaseKeystore = providers.environmentVariable("KEYSTORE_FILE")
val releaseKeystorePassword = providers.environmentVariable("KEYSTORE_PASSWORD")
val releaseKeyAlias = providers.environmentVariable("KEY_ALIAS")
val releaseKeyPassword = providers.environmentVariable("KEY_PASSWORD")

android {
    namespace = "dev.tochi.mobileharness"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.tochi.mobileharness"
        minSdk = 26
        targetSdk = 35
        versionCode = ciVersionCode.get()
        versionName = ciVersionName.get()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        // Checked-in key shared by every debug/CI build so a newer build always installs
        // over the previous one on the phone. Test-only: never ship a store build with it.
        getByName("debug") {
            storeFile = rootProject.file("signing/debug.keystore")
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
        create("release") {
            if (releaseKeystore.isPresent) {
                storeFile = file(releaseKeystore.get())
                storePassword = releaseKeystorePassword.get()
                keyAlias = releaseKeyAlias.get()
                keyPassword = releaseKeyPassword.get()
            } else {
                initWith(getByName("debug"))
            }
        }
    }

    buildTypes {
        debug {
            // The phone harness installs debug builds. The suffix keeps them from
            // ever overwriting a real install of the app on the same device.
            applicationIdSuffix = ".debug"
        }
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2024.12.01")
    implementation(composeBom)
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    testImplementation("junit:junit:4.13.2")

    androidTestImplementation(composeBom)
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
}
