import groovy.json.JsonSlurper
import java.io.StringReader
import java.math.BigDecimal
import javax.inject.Inject
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element
import org.xml.sax.InputSource

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.compose.compiler)
}

val rust = rootProject.layout.projectDirectory.dir("..")
val rustSources = files(rust.file("Cargo.toml"), rust.file("Cargo.lock"), rust.dir("crates"))
val rustAbis =
    providers.gradleProperty("mu.abis").map { abis ->
        abis.split(",").map(String::trim).filter(String::isNotEmpty)
    }

android {
    namespace = "dev.azuki.mu"
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
        ndk { abiFilters += rustAbis.get() }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(libs.compose.runtime)
    implementation(libs.compose.foundation)
    implementation(libs.compose.ui)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.miuix.ui)
    implementation(libs.miuix.preference)
    implementation(libs.miuix.nav)
    implementation(libs.markdown.renderer)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.annotation)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.jna) { artifact { type = "aar" } }
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.compose)
    implementation(libs.zxing.cpp)
}

abstract class CargoNdk : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

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
                    listOf(
                        "--output-dir",
                        output.path,
                        "build",
                        "--package",
                        "mu-ffi",
                        "--lib",
                        "--profile",
                        profile.get(),
                    )
            )
        }
    }
}

abstract class UniffiBindings : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val library: RegularFileProperty

    @get:Internal abstract val workspace: DirectoryProperty

    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction
    fun generate() {
        val output = output.get().asFile
        output.deleteRecursively()
        exec.exec {
            workingDir = workspace.get().asFile
            commandLine(
                "cargo",
                "run",
                "--quiet",
                "--package",
                "mu-ffi",
                "--features",
                "bindgen",
                "--bin",
                "uniffi-bindgen",
                "--",
                "generate",
                "--library",
                library.get().asFile.path,
                "--language",
                "kotlin",
                "--no-format",
                "--out-dir",
                output.path,
            )
        }
    }
}

// Solar, from the Iconify package the flake pins: the icons solar.txt names as Kotlin for the UI,
// and every bold duotone one as a resource for hosts and snippets to pick from.
abstract class SolarIcons : DefaultTask() {
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val iconify: DirectoryProperty

    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val names: RegularFileProperty

    @get:OutputDirectory abstract val code: DirectoryProperty

    @get:OutputDirectory abstract val resources: DirectoryProperty

    private data class Style(
        val fill: Boolean = true,
        val stroke: Boolean = false,
        val evenOdd: Boolean = false,
        val alpha: BigDecimal = BigDecimal.ONE,
        val width: String = "1",
        val cap: String = "butt",
        val join: String = "miter",
    )

    private class Shape(val path: String, val style: Style)

    @TaskAction
    fun generate() {
        val set = JsonSlurper().parse(iconify.file("icons.json").get().asFile) as Map<*, *>
        val metadata = JsonSlurper().parse(iconify.file("metadata.json").get().asFile) as Map<*, *>
        @Suppress("UNCHECKED_CAST") val icons = set["icons"] as Map<String, Map<String, Any>>
        @Suppress("UNCHECKED_CAST")
        val categories = metadata["categories"] as Map<String, List<String>>
        fun shapes(name: String): List<Shape> {
            val icon = icons[name] ?: error("Solar has no $name")
            val body = "<svg>${icon["body"]}</svg>"
            val svg =
                DocumentBuilderFactory.newInstance()
                    .newDocumentBuilder()
                    .parse(InputSource(StringReader(body)))
                    .documentElement
            return buildList { draw(name, svg, Style()) }
        }

        val glyphs = names.get().asFile.readLines().map(String::trim).filter(String::isNotEmpty)
        code.get().file("dev/azuki/mu/design/Solar.kt").asFile.apply {
            parentFile.mkdirs()
            writeText(solarKt(glyphs.associateWith(::shapes)))
        }

        val picks = categories.flatMap { (category, names) ->
            names
                .filter { it.endsWith("-bold-duotone") && icons[it]?.get("hidden") != true }
                .map { name ->
                    val paths =
                        shapes(name).map { shape ->
                            val style = shape.style
                            check(!style.stroke) { "$name strokes, which pictograms don't" }
                            val rule = if (style.evenOdd) "evenodd" else "nonzero"
                            "${style.alpha.toPlainString()} $rule ${shape.path}"
                        }
                    (listOf(category, "solar:$name") + paths).joinToString("\t")
                }
        }
        resources.get().file("dev/azuki/mu/design/pictograms.tsv").asFile.apply {
            parentFile.mkdirs()
            writeText(picks.joinToString("\n", postfix = "\n"))
        }
    }

    private fun MutableList<Shape>.draw(icon: String, element: Element, inherited: Style) {
        if (element.tagName == "defs") {
            // A clip to the whole 24×24 box changes nothing.
            val clips = element.children()
            check(
                clips.all { clip ->
                    val rect = clip.children().singleOrNull()
                    clip.tagName == "clipPath" &&
                        rect?.tagName == "rect" &&
                        rect.getAttribute("width") == "24" &&
                        rect.getAttribute("height") == "24" &&
                        !rect.hasAttribute("x") &&
                        !rect.hasAttribute("y")
                }
            ) {
                "$icon clips"
            }
            return
        }
        val style = style(icon, element, inherited)
        fun shape(path: String) {
            check(style.fill != style.stroke) {
                "$icon has a shape that isn't just filled or stroked"
            }
            add(Shape(path, style))
        }
        fun ellipse(rx: String, ry: String) {
            val cx = BigDecimal(element.getAttribute("cx"))
            val cy = BigDecimal(element.getAttribute("cy"))
            val x = BigDecimal(rx)
            val arc = "a$rx $ry 0 1 0"
            shape(
                "M${(cx - x).toPlainString()} ${cy.toPlainString()}$arc ${(x + x).toPlainString()} 0$arc ${(-x - x).toPlainString()} 0Z"
            )
        }
        when (element.tagName) {
            "svg",
            "g" -> element.children().forEach { draw(icon, it, style) }
            "path" -> shape(element.getAttribute("d"))
            "circle" -> element.getAttribute("r").let { ellipse(it, it) }
            "ellipse" -> ellipse(element.getAttribute("rx"), element.getAttribute("ry"))
            else -> error("$icon has a <${element.tagName}>")
        }
    }

    private fun style(icon: String, element: Element, inherited: Style): Style {
        fun paint(value: String) =
            when (value) {
                "currentColor" -> true
                "none" -> false
                else -> error("$icon paints $value")
            }
        var style = inherited
        for (i in 0 until element.attributes.length) {
            val attribute = element.attributes.item(i)
            val value = attribute.nodeValue
            style =
                when (attribute.nodeName) {
                    "fill" -> style.copy(fill = paint(value))
                    "stroke" -> style.copy(stroke = paint(value))
                    "fill-rule" -> style.copy(evenOdd = value == "evenodd")
                    "opacity" -> style.copy(alpha = style.alpha * BigDecimal(value))
                    "stroke-width" -> style.copy(width = value)
                    "stroke-linecap" -> style.copy(cap = value)
                    "stroke-linejoin" -> style.copy(join = value)
                    "d",
                    "cx",
                    "cy",
                    "r",
                    "rx",
                    "ry",
                    "clip-rule",
                    "clip-path" -> style
                    else -> error("$icon has a ${attribute.nodeName}")
                }
        }
        return style
    }

    private fun Element.children(): List<Element> =
        (0 until childNodes.length).mapNotNull { childNodes.item(it) as? Element }

    private fun solarKt(glyphs: Map<String, List<Shape>>): String = buildString {
        appendLine("// Generated from solar.txt. Solar Icons by 480 Design")
        appendLine("// (https://www.figma.com/community/file/1166831539721848736), CC BY 4.0.")
        appendLine("package dev.azuki.mu.design")
        appendLine()
        appendLine("import androidx.compose.ui.graphics.StrokeCap")
        appendLine("import androidx.compose.ui.graphics.StrokeJoin")
        appendLine("import androidx.compose.ui.graphics.vector.ImageVector")
        appendLine()
        appendLine("object Solar {")
        for ((name, shapes) in glyphs) {
            val property = name.split('-').joinToString("") { it.replaceFirstChar(Char::uppercase) }
            appendLine("    val $property: ImageVector by lazy {")
            appendLine("        icon(\"$property\") {")
            for (shape in shapes) {
                val style = shape.style
                val alpha = style.alpha.toPlainString()
                val arguments = buildList {
                    add("\"${shape.path}\"")
                    if (alpha != "1") add("alpha = ${alpha}f")
                    if (style.fill) {
                        if (style.evenOdd) add("evenOdd = true")
                    } else {
                        if (style.width != "1.5") add("width = ${style.width}f")
                        if (style.cap != "round")
                            add("cap = StrokeCap.${style.cap.replaceFirstChar(Char::uppercase)}")
                        if (style.join != "miter")
                            add("join = StrokeJoin.${style.join.replaceFirstChar(Char::uppercase)}")
                    }
                }
                appendLine(
                    "            ${if (style.fill) "fill" else "stroke"}(${arguments.joinToString()})"
                )
            }
            appendLine("        }")
            appendLine("    }")
            appendLine()
        }
        appendLine("}")
    }
}

androidComponents {
    onVariants { variant ->
        val name = variant.name.replaceFirstChar(Char::uppercase)
        val cargo =
            tasks.register<CargoNdk>("cargoNdk$name") {
                sources.from(rustSources)
                profile = if (variant.buildType == "release") "android" else "dev"
                abis = rustAbis
                platform = variant.minSdk.apiLevel
                workspace = rust
                output = layout.buildDirectory.dir("rust/${variant.name}/jniLibs")
            }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargo, CargoNdk::output)

        val bindings =
            tasks.register<UniffiBindings>("uniffiBindings$name") {
                library = cargo.flatMap { cargo ->
                    cargo.output.file(rustAbis.map { "${it.first()}/libmu_ffi.so" })
                }
                workspace = rust
                output = layout.buildDirectory.dir("generated/uniffi/${variant.name}")
            }
        variant.sources.kotlin?.addGeneratedSourceDirectory(bindings, UniffiBindings::output)

        val solar =
            tasks.register<SolarIcons>("solarIcons$name") {
                iconify.fileProvider(providers.environmentVariable("SOLAR_ICONS").map(::File))
                names = layout.projectDirectory.file("solar.txt")
                code = layout.buildDirectory.dir("generated/solar/${variant.name}/kotlin")
                resources = layout.buildDirectory.dir("generated/solar/${variant.name}/resources")
            }
        variant.sources.kotlin?.addGeneratedSourceDirectory(solar, SolarIcons::code)
        variant.sources.resources?.addGeneratedSourceDirectory(solar, SolarIcons::resources)
    }
}
