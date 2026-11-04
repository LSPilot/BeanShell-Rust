plugins {
    id("com.android.library")
}

android {
    namespace = "me.yun.bsh.rust"
    compileSdk = 37

    defaultConfig {
        minSdk = 28
        consumerProguardFiles("consumer-rules.pro")
    }

    // libbsh_lspilot.so for each ABI is copied to src/main/jniLibs/<abi>/
    // before assembling; AGP picks that directory up by default.

    buildFeatures {
        buildConfig = false
        aidl = false
        viewBinding = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
