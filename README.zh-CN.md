# BeanShell-Rust

<p align="center">
  <a href="https://github.com/Lspilot/BeanShell-Rust/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Lspilot/BeanShell-Rust/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/Lspilot/BeanShell-Rust/releases"><img alt="Release" src="https://img.shields.io/github/v/release/Lspilot/BeanShell-Rust?sort=semver&display_name=tag"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"></a>
  <a href="https://www.rust-lang.org"><img alt="Rust" src="https://img.shields.io/badge/rust-1.75%2B-orange.svg"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/platform-linux%20%7C%20macOS%20%7C%20android-lightgrey.svg">
</p>

BeanShell-Rust 是 [BeanShell](https://github.com/beanshell/beanshell) 解释器的 Rust 重写。
词法分析、语法分析、值模型与求值器均由 Rust 实现；涉及 Java 互操作时，通过 JNI
内嵌一个 JVM，并用一个小型 Java 反射桥完成调用。

[English](README.md) | **简体中文**

---

## 目录

- [主要特性](#主要特性)
- [项目结构](#项目结构)
- [环境要求](#环境要求)
- [构建](#构建)
- [使用](#使用)
- [JVM 定位](#jvm-定位)
- [测试](#测试)
- [Android](#android)
- [文档](#文档)
- [贡献](#贡献)
- [许可证](#许可证)
- [致谢](#致谢)

## 主要特性

- 完整 BeanShell 语法：语句、表达式、方法定义、闭包、异常、数组、增强 for、
  switch 语句与表达式、try-with-resources、lambda、record、sealed、
  instanceof 模式、文本块与字符串模板。
- Java 互操作：`new`、静态与实例成员、可变参数、装箱、代理对象，
  以及可继承 Java 类的脚本类。
- 便于嵌入：`bsh-runtime` 可直接作为库使用，`bsh-rs` 以动态库形式提供
  Android JNI 入口。
- 使用差分测试与 Java 实现逐脚本对比行为。

## 项目结构

| Crate | 作用 |
| --- | --- |
| `crates/bsh-syntax` | 词法分析、语法分析与 AST |
| `crates/bsh-runtime` | 值模型、求值器、作用域与内建 |
| `crates/bsh-jvm` | 基于 JNI 的内嵌 JVM 与 Java 反射桥 |
| `crates/bsh-cli` | `bsh` 命令行 |
| `crates/bsh-rs` | Android JNI 绑定（cdylib） |

辅助目录：

| 路径 | 内容 |
| --- | --- |
| `tests/corpus` | 语法与语义用例 |
| `tests/upstream-scripts` | 取自上游 BeanShell 的脚本 |
| `tools/` | 桥接重建、Android 链接器包装、差分测试脚本 |
| `docs/` | 项目图示 |

## 环境要求

- Rust 1.75 或更新版本
- Java 互操作需要 JDK 8 或更新版本（提供 `libjvm` 即可）

## 构建

```sh
cargo build --release
```

Java 反射桥由 `crates/bsh-jvm/build.rs` 调用 `javac` 编译后嵌入 Rust 库，
仓库中不保存 class 文件。

## 使用

```sh
# 运行脚本文件
cargo run --release --bin bsh -- script.bsh

# 单行求值
cargo run --release --bin bsh -- -e 'print(new java.math.BigInteger("12").add(new java.math.BigInteger("30")));'

# 从标准输入读取整段脚本
echo 'print("hi");' | cargo run --release --bin bsh

# REPL
cargo run --release --bin bsh
```

`run.sh` 会在需要时自动构建，然后以相同参数运行脚本。

## JVM 定位

`libjvm` 按以下顺序查找：

1. 环境变量 `BSH_RS_LIBJVM`
2. `JAVA_HOME/lib/server/libjvm.so`
3. `java -XshowSettings:properties` 报告的 `java.home`
4. 常见目录（如 `/usr/lib/jvm/*`）

没有 JVM 时解释器仍可启动，但只能使用 Rust 内建的宿主类。
使用 `--no-jvm` 或设置 `BSH_RS_JVM=0` 可显式关闭 JVM。

## 测试

```sh
cargo test --workspace
```

与 Java 实现做差分测试需要一个可运行的 BeanShell：

```sh
BSH_ORACLE=/path/to/beanshell/run.sh tools/oracle-diff.py
```

未设置 `BSH_ORACLE` 时只运行 Rust 侧。

## Android

Android 产物发布为 AGP 构建的 AAR，内含 `arm64-v8a`、`armeabi-v7a`、`x86_64`
三个 ABI 的 JNI 库：

```text
bsh-rs-<version>.aar
  AndroidManifest.xml
  classes.jar
  R.txt
  META-INF/com/android/build/gradle/aar-metadata.properties
  jni/arm64-v8a/libbsh_rs.so
  jni/armeabi-v7a/libbsh_rs.so
  jni/x86_64/libbsh_rs.so
```

Gradle 中使用 `implementation(files("libs/bsh-rs.aar"))` 引用。
库导出形如 `Java_me_yun_lspilot_bsh_Native_*` 的 JNI 入口。

AAR 由 `android/` 下的 AGP library 模块打包，该模块只负责把预编译的
`libbsh_rs.so` 打进标准 AAR。本地构建：

```sh
# 1. 先按需要的 ABI 构建 JNI 库
ANDROID_NDK_HOME=/path/to/ndk \
  cargo build --release --target aarch64-linux-android -p bsh-rs

# 2. 放到 AGP 约定的目录
mkdir -p android/bsh-rs/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libbsh_rs.so \
   android/bsh-rs/src/main/jniLibs/arm64-v8a/

# 3. 生成 AAR
cd android && ./gradlew :bsh-rs:assembleRelease
```

rustc 自身即可生成 AArch64 目标文件，`tools/android-linker.sh` 只负责链接：
它转换 rustc 的参数后直接调用 NDK 的 `ld.lld`，并带上 Android 15 起要求的
16 KB 页对齐参数。aarch64 主机上若存在 `qemu-x86_64-static`，则用它执行
x86_64 的 `ld.lld`。该包装器由 `.cargo/config.toml` 指定，需要设置
`ANDROID_NDK_HOME`。

### 示例 App

`android/app` 是示例兼冒烟测试 App，包名 `me.yun.bsh`，依赖 AAR 模块，
直接运行输入框里的脚本。

```sh
adb install -r bsh-demo-<version>.apk

# 不打开界面直接跑脚本
adb shell am broadcast -n me.yun.bsh/.RunReceiver -a me.yun.bsh.RUN \
  --es code 'print("demo ok " + (40 + 2));'

adb logcat -s bsh-rs
```

### Java API

AAR 的 `classes.jar` 内提供一个精简 Java API：

```text
me.yun.bsh.rust.BshEngine   每个对象对应一个解释器实例
me.yun.bsh.rust.Native      JNI 绑定（包内可见）
bshrs.Bridge               运行时使用的 Java 反射桥
bshrs.Native               代理回调入口
```

```kotlin
val engine = BshEngine()
engine.importClass("java.util.ArrayList")
val list = engine.evalValue("new java.util.ArrayList()")
engine.setVariable("answer", 42)
engine.close()
```

AAR 同时携带 `proguard.txt`，避免混淆后 native 方法名失配。

## 文档

- [更新记录](CHANGELOG.md)：条目与仓库 tag 对应
- [发布页](https://github.com/Lspilot/BeanShell-Rust/releases)：版本号以最新 tag 为准
- [许可证](LICENSE) 与 [NOTICE](NOTICE)

## 贡献

构建、测试与提交流程见 [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md)。
欢迎提交 issue 与 pull request。提交前请运行 `cargo fmt` 与
`cargo test --workspace`；CI 会检查格式、以 release 模式构建，并在 Linux 与
macOS 上运行完整测试。

## 许可证

本项目使用 Apache License 2.0，详见 [LICENSE](LICENSE) 与 [NOTICE](NOTICE)。

## 致谢

BeanShell 由 Patrick Niemeyer 创建，后捐赠给 Apache 软件基金会。
本项目的语言语义、测试脚本与部分设计来自上游 Java 实现。
