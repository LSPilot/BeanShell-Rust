package me.yun.bsh

import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Typeface
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.rosemoe.sora.widget.CodeEditor
import io.github.rosemoe.sora.widget.schemes.SchemeDarcula
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    private val runSignal = MutableStateFlow(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Draw behind the system bars and keep their icons in sync with the theme.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(Color.Transparent.toArgb(), Color.Transparent.toArgb()),
            navigationBarStyle = SystemBarStyle.auto(Color.Transparent.toArgb(), Color.Transparent.toArgb())
        )
        // Debug builds run a tiny self-test so JNI/ART integration is easy to verify.
        if ((applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
            Thread {
                val result = try {
                    BshBridge.nativeEval("print(\"self-test: \" + (6 * 7));")
                } catch (t: Throwable) {
                    "self-test failed: " + t
                }
                Log.i("bsh-rs", result.trim())
            }.start()
        }
        if (intent?.getBooleanExtra("autorun", false) == true) {
            runSignal.value = 1
        }
        setContent {
            BshTheme {
                EditorScreen(runSignal)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.getBooleanExtra("autorun", false)) {
            runSignal.value += 1
        }
    }
}

private fun Color.toArgb(): Int = android.graphics.Color.argb(
    (alpha * 255).toInt(), (red * 255).toInt(), (green * 255).toInt(), (blue * 255).toInt()
)

@Composable
private fun BshTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) {
        darkColorScheme(
            primary = Color(0xFF9CC7FF),
            surface = Color(0xFF141417),
            surfaceVariant = Color(0xFF232328),
            background = Color(0xFF0F0F12)
        )
    } else {
        lightColorScheme(
            primary = Color(0xFF2F5DA8),
            surface = Color(0xFFFFFBFE),
            surfaceVariant = Color(0xFFE8E8EE),
            background = Color(0xFFF7F7FA)
        )
    }
    MaterialTheme(colorScheme = colors, content = content)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditorScreen(runSignal: StateFlow<Int>) {
    var output by remember { mutableStateOf("准备就绪。\n点击右上角「运行」执行脚本。") }
    var running by remember { mutableStateOf(false) }
    var editor by remember { mutableStateOf<CodeEditor?>(null) }
    var handledSignal by remember { mutableStateOf(-1) }
    val scope = rememberCoroutineScope()
    val outputScroll = rememberScrollState()

    fun runScript() {
        val code = editor?.text?.toString() ?: return
        if (running) return
        running = true
        output = "运行中…"
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                try {
                    BshBridge.nativeEval(code).ifEmpty { "(无输出)" }
                } catch (t: Throwable) {
                    "native error: " + t
                }
            }
            Log.i("bsh-rs", result)
            output = result
            running = false
        }
    }

    val signal by runSignal.collectAsState()
    LaunchedEffect(signal, editor) {
        if (editor != null && signal > handledSignal) {
            handledSignal = signal
            runScript()
        }
    }
    LaunchedEffect(output) {
        outputScroll.animateScrollTo(outputScroll.maxValue)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text("bsh-rs", fontWeight = FontWeight.SemiBold)
                        Text(
                            "Rust BeanShell · sora-editor",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                },
                actions = {
                    Button(
                        onClick = { runScript() },
                        enabled = !running,
                        modifier = Modifier.padding(end = 12.dp)
                    ) {
                        if (running) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(16.dp),
                                strokeWidth = 2.dp,
                                color = MaterialTheme.colorScheme.onPrimary
                            )
                        } else {
                            Text("运行")
                        }
                    }
                }
            )
        }
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
                .imePadding()
        ) {
            AndroidView(
                factory = { context ->
                    CodeEditor(context).apply {
                        setColorScheme(SchemeDarcula())
                        typefaceText = Typeface.MONOSPACE
                        setTextSize(14f)
                        setText(SAMPLE_SCRIPT)
                    }
                },
                modifier = Modifier
                    .weight(3f)
                    .fillMaxWidth(),
                update = { editor = it }
            )

            HorizontalDivider()

            Surface(
                color = MaterialTheme.colorScheme.surfaceVariant,
                modifier = Modifier.fillMaxWidth()
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 16.dp, end = 8.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        "输出",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f)
                    )
                    TextButton(onClick = { output = "" }) {
                        Text("清空")
                    }
                }
            }

            Box(
                modifier = Modifier
                    .weight(2f)
                    .fillMaxWidth()
                    .background(Color(0xFF141417))
            ) {
                SelectionContainer {
                    Text(
                        text = output,
                        color = Color(0xFFD7D7DC),
                        fontFamily = FontFamily.Monospace,
                        fontSize = 13.sp,
                        lineHeight = 19.sp,
                        modifier = Modifier
                            .fillMaxSize()
                            .verticalScroll(outputScroll)
                            .padding(16.dp)
                    )
                }
            }
        }
    }
}

private const val SAMPLE_SCRIPT = "print(\"Hello, BeanShell on Android!\");\n" +
    "\n" +
    "int sum = 0;\n" +
    "for (int i = 1; i <= 10; i++) sum += i;\n" +
    "print(\"1 + ... + 10 = \" + sum);\n" +
    "\n" +
    "// closures\n" +
    "makeAdder(n) { return x -> x + n; }\n" +
    "add5 = makeAdder(5);\n" +
    "print(\"add5(37) = \" + add5(37));\n" +
    "\n" +
    "// Java interop (the app's ART runtime is the JVM)\n" +
    "java.util.function.Function twice = x -> (int) x * 2;\n" +
    "print(\"twice(21) = \" + twice.apply(21));\n" +
    "print(\"java version: \" + java.lang.System.getProperty(\"java.version\"));\n" +
    "\n" +
    "// a Java thread running script code (implement an interface, not a subclass)\n" +
    "java.lang.Runnable r = new java.lang.Runnable() {\n" +
    "    public void run() { print(\"thread: custom run\"); }\n" +
    "};\n" +
    "new java.lang.Thread(r).start();\n"
