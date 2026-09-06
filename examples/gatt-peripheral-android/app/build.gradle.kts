plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.example.gattperipheral"
    compileSdk = 36
    defaultConfig {
        applicationId = "com.example.gattperipheral"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
        ndk { abiFilters += "arm64-v8a" }
    }
    ndkVersion = "27.0.12077973"
    buildFeatures { compose = true }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets["main"].kotlin.directories.add(layout.buildDirectory.dir("generated/source/uniffi/java").get().asFile.path)
    sourceSets["main"].jniLibs.directories.add(layout.buildDirectory.dir("generated/jniLibs").get().asFile.path)
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
    implementation(platform("androidx.compose:compose-bom:2026.03.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("androidx.annotation:annotation:1.9.1")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
}

// Generate Kotlin before compilation and package the native library with JNA's AAR.
val workspace = rootProject.projectDir.resolve("../..").canonicalFile
val nativeOutput = layout.buildDirectory.dir("generated/jniLibs")
val generatedKotlin = layout.buildDirectory.dir("generated/source/uniffi/java")
val sdkPath = androidComponents.sdkComponents.sdkDirectory.get().asFile.absolutePath

val buildRust by tasks.registering(Exec::class) {
    workingDir(workspace)
    environment("ANDROID_HOME", sdkPath)
    environment("ANDROID_NDK_HOME", "$sdkPath/ndk/27.0.12077973")
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "--platform", "26",
        "-o", nativeOutput.get().asFile.absolutePath,
        "build", "--locked", "--release", "-p", "gatt-peripheral-android", "--lib")
    inputs.files(fileTree(workspace.resolve("btstack-core/src")), fileTree(workspace.resolve("btstack-gatt/src")),
        fileTree(workspace.resolve("btstack-nusb/src")), fileTree(workspace.resolve("btstack-sys")),
        fileTree(workspace.resolve("vendor/btstack/src")), fileTree(workspace.resolve("vendor/btstack/platform/embedded")),
        fileTree(rootProject.file("native/src")))
    inputs.files(workspace.resolve("Cargo.lock"), workspace.resolve("Cargo.toml"), rootProject.file("native/Cargo.toml"),
        workspace.resolve("btstack-core/Cargo.toml"), workspace.resolve("btstack-gatt/Cargo.toml"),
        workspace.resolve("btstack-nusb/Cargo.toml"))
    outputs.dir(nativeOutput)
}
val generateUniFFIBindings by tasks.registering(Exec::class) {
    dependsOn(buildRust)
    workingDir(workspace)
    commandLine("cargo", "run", "--locked", "-p", "gatt-peripheral-android", "--features", "bindgen",
        "--bin", "uniffi-bindgen", "--", "generate", "--library",
        nativeOutput.get().file("arm64-v8a/libgatt_peripheral_android.so").asFile.absolutePath,
        "--language", "kotlin", "--config", rootProject.file("native/uniffi.toml").absolutePath,
        "--out-dir", generatedKotlin.get().asFile.absolutePath)
    inputs.dir(nativeOutput)
    inputs.file(rootProject.file("native/uniffi.toml"))
    outputs.dir(generatedKotlin)
}
tasks.named("preBuild") { dependsOn(generateUniFFIBindings) }
