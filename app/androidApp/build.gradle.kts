import javax.inject.Inject

// The Android app: the shared UI, plus the Rust core (mu-ffi) and its UniFFI bindings.
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

// The Cargo workspace this app lives in.
val rust = rootProject.layout.projectDirectory.dir("..")
val rustSources = files(rust.file("Cargo.toml"), rust.file("Cargo.lock"), rust.dir("crates"))
val rustAbis = providers.gradleProperty("mu.abis").map { abis -> abis.split(",").map(String::trim).filter(String::isNotEmpty) }

android {
    namespace = "dev.azuki.mu.android"
    compileSdk = 37
    // What the flake's dev shell provides; AGP cannot install into the Nix store.
    buildToolsVersion = "37.0.0"
    // AGP strips the Rust library with it.
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "dev.azuki.mu"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        // Only where the Rust core is built for, whatever else dependencies bring.
        ndk { abiFilters += rustAbis.get() }
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(project(":shared"))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.annotation)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.jna) { artifact { type = "aar" } }
}

// Builds mu-ffi for each ABI with cargo-ndk, into `output/<abi>/libmu_ffi.so`.
abstract class CargoNdk : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val sources: ConfigurableFileCollection

    @get:Input abstract val profile: Property<String>

    @get:Input abstract val abis: ListProperty<String>

    @get:Input abstract val platform: Property<Int>

    @get:Internal abstract val workspace: DirectoryProperty

    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction
    fun build() {
        val output = output.get().asFile
        output.deleteRecursively()
        exec.exec {
            workingDir = workspace.get().asFile
            commandLine(
                listOf("cargo", "ndk", "--platform", platform.get().toString()) +
                    abis.get().flatMap { listOf("--target", it) } +
                    listOf("--output-dir", output.path, "build", "--package", "mu-ffi", "--lib", "--profile", profile.get()),
            )
        }
    }
}

// Generates the Kotlin bindings from a built mu-ffi, with the uniffi-bindgen
// that comes with it, so the two always match.
abstract class UniffiBindings : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val library: RegularFileProperty

    @get:Internal abstract val workspace: DirectoryProperty

    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction
    fun generate() {
        val output = output.get().asFile
        output.deleteRecursively()
        exec.exec {
            workingDir = workspace.get().asFile
            commandLine(
                "cargo", "run", "--quiet", "--package", "mu-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen", "--",
                "generate", "--library", library.get().asFile.path, "--language", "kotlin", "--no-format",
                "--out-dir", output.path,
            )
        }
    }
}

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar(Char::uppercase)
        val cargo = tasks.register<CargoNdk>("cargoNdk$name") {
            sources.from(rustSources)
            // Release builds unwind on panic; see the workspace's Cargo.toml.
            profile = if (variant.buildType == "release") "android" else "dev"
            abis = rustAbis
            platform = variant.minSdk.apiLevel
            workspace = rust
            output = layout.buildDirectory.dir("rust/${variant.name}/jniLibs")
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdk::output)

        val bindings = tasks.register<UniffiBindings>("uniffiBindings$name") {
            library = cargo.flatMap { cargo -> cargo.output.file(rustAbis.map { "${it.first()}/libmu_ffi.so" }) }
            workspace = rust
            output = layout.buildDirectory.dir("generated/uniffi/${variant.name}")
        }
        variant.sources.kotlin?.addGeneratedSourceDirectory(bindings, UniffiBindings::output)
    }
}
