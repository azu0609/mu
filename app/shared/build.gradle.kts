// The app's UI, in Compose Multiplatform. It knows nothing of the Rust core:
// the platform app hands it what to show.
plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kotlin.multiplatform.library)
    alias(libs.plugins.compose.multiplatform)
    alias(libs.plugins.compose.compiler)
}

kotlin {
    android {
        namespace = "dev.azuki.mu.shared"
        compileSdk = 37
        minSdk = 26
        // What the flake's dev shell provides; AGP cannot install into the Nix store.
        buildToolsVersion = "37.0.0"
    }

    sourceSets {
        commonMain.dependencies {
            implementation(libs.compose.runtime)
            implementation(libs.compose.foundation)
            implementation(libs.compose.ui)
            implementation(libs.miuix)
        }
    }
}
