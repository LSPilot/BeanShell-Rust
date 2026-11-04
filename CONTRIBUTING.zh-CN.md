# 贡献指南

感谢参与 BeanShell-Rust。本文说明构建方式、提交前要跑什么，以及如何反馈问题。

[English](CONTRIBUTING.md)

## 开始之前

- 先搜索已有 issue，避免重复。
- 较大的改动请先开 issue 讨论设计。
- 保持改动聚焦，无关重构请单独提 PR。

## 构建与测试

需要 Rust 1.75+，Java 互操作需要 JDK 8+。

```sh
cargo build --release
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

Java 反射桥由 `crates/bsh-jvm/build.rs` 调用 `javac` 编译；`javac` 不在 `PATH`
时请设置 `JAVA_HOME` 或 `JAVAC`。

### 差分测试

```sh
BSH_ORACLE=/path/to/beanshell/run.sh tools/oracle-diff.py
```

## Android

```sh
ANDROID_NDK_HOME=/path/to/ndk \
  cargo build --release --target aarch64-linux-android -p bsh-rs

mkdir -p android/bsh-rs/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libbsh_rs.so \
   android/bsh-rs/src/main/jniLibs/arm64-v8a/

cd android && ./gradlew :bsh-rs:assembleRelease :app:assembleRelease
```

## Pull Request

- 描述问题与解决方式。
- 关联 issue，例如 `Fixes #123`。
- 行为变化要补测试。
- 提交信息用祈使句，例如 `fix: resolve catch matching for simple names`。
- CI 通过后再请求 review。

## 反馈问题

请使用 bug report 模板，并附上：

- 平台与系统版本，
- Rust 与 JDK 版本，
- 最小可复现脚本，
- 期望结果与实际结果。

## 代码风格

- Rust：`cargo fmt`，`cargo clippy` 不新增警告。
- Java：4 空格缩进，JNI 方法名与 Rust 侧保持一致。
- Markdown：大约 80 列换行。

## 许可证

提交即表示你的改动按 Apache License 2.0 授权。
