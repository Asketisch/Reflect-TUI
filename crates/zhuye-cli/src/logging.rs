//! TUI 专用 tracing 初始化。
//!
//! # 为什么不直接写 stderr
//!
//! TUI 画面渲染到 **stdout 的 alternate screen**
//! (`CrosstermBackend::new(std::io::stdout())`,见 `app.rs`)。而 tracing 若
//! 写 **stderr**,stderr 不在 alt-screen 缓冲区里 → 日志直接落到真实终端
//! 滚动区,TUI 无法覆盖/清除,表现为切换 permission mode(Shift+Tab)时
//! `permission mode changed` 之类的 INFO 行"漏"到界面上。所有 `reflect*`
//! crate 的 INFO/WARN/ERROR 都有此问题,Shift+Tab 只是最显眼的触发点。
//!
//! # 策略(对标业界最佳实践)
//!
//! - stderr **是 tty**(用户没重定向)→ 日志写文件
//!   `~/.zhuye/logs/tui-<YYYY-MM-DD>.log`,终端保持干净。
//! - stderr **不是 tty**(用户 `2>debug.log`)→ 仍写 stderr,保留
//!   `RUST_LOG=debug reflect tui 2>debug.log` 调试工作流(见 USER_GUIDE)。
//!
//! 判定用 `std::io::IsTerminal`(Rust 1.70+ stable,与 `terminal_title.rs`
//! 一致)。best-effort:文件打开失败则回退 stderr(宁可泄露也不丢日志)。

use std::io::IsTerminal;
use std::sync::Once;

use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;

/// 持有一个 append 打开的日志文件,供 `fmt` layer 同步写入。
///
/// `MakeWriter` 返回的 writer 持有 `MutexGuard`,fmt layer 在单次 `write_event`
/// 期间持有该 writer,故 guard 生命周期足够;无需 `Send` writer(同步写)。
struct LogFile(Mutex<std::fs::File>);

impl LogFile {
    fn open(path: &std::path::Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        // 首行写分隔,便于多次启动区分(同日 append)。
        let now = chrono::Local::now().to_rfc3339();
        let header = format!(
            "=== zhuye-cli started {now} pid={} ===\n",
            std::process::id()
        );
        use std::io::Write;
        let _ = (&file).write_all(header.as_bytes());
        Ok(Self(Mutex::new(file)))
    }
}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = LogFileWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogFileWriter(self.0.lock())
    }
}

/// 持有 `MutexGuard<File>` 的 writer。`Write` 直接透传给底层文件。
struct LogFileWriter<'a>(parking_lot::MutexGuard<'a, std::fs::File>);

impl<'a> std::io::Write for LogFileWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

/// 解析默认日志文件路径:`$HOME/.zhuye/logs/tui-<YYYY-MM-DD>.log`。
/// HOME 缺失时回退 `./.zhuye/logs/...`。
fn default_log_path() -> std::path::PathBuf {
    let date = chrono::Local::now().format("%Y-%m-%d");
    let base = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join(".zhuye")
        .join("logs")
        .join(format!("tui-{date}.log"))
}

/// 持有 OTLP exporter 的 guard —— 必须活到进程退出,否则 provider 被
/// shutdown,in-flight span 全部丢失。泄漏到进程全局(`OnceLock`)。
static OTEL_GUARD: std::sync::OnceLock<zhuye_config::ExporterGuard> = std::sync::OnceLock::new();

/// 安装 tracing subscriber。仅生效一次(`Once` 守护)。
///
/// 默认 EnvFilter 仍为 `"warn,reflect=info"`(与 `zhuye-exec` 一致),
/// 仅替换 writer:stderr 是 tty → 文件;否则 → stderr。`try_init` 结果
/// 忽略(测试可能已装 subscriber,best-effort)。
///
/// `analytics` = `[analytics]` 段。给了且 `enabled` 时启动 OTLP/HTTP
/// exporter 并把 `OpenTelemetryLayer` 挂进 registry —— 业务侧的
/// `#[tracing::instrument]` / `info_span!` 自动转成 OTLP span。
/// 此前 TUI 完全没有这层,`[analytics] enabled = true` 是死配置
/// (headless `zhuye-exec` 有)。exporter 启动失败只降级 + warn,
/// 不阻塞启动(与 zhuye-exec 同款 fail-closed 语义)。
pub fn init(analytics: Option<&zhuye_config::AnalyticsSection>) {
    static TRACING_INIT: Once = Once::new();
    TRACING_INIT.call_once(|| {
        // v1.3 接线:先起 OTLP exporter(设全局 TracerProvider),再把
        // OpenTelemetryLayer 加进 registry。失败 → None,不阻塞。
        let otel_guard = analytics.and_then(zhuye_config::init_exporter);

        // stderr 是 tty → 用户没重定向,写文件避免污染 TUI。
        // 否则(stderr 已被 `2>file` 重定向)→ 仍写 stderr,保留文档工作流。
        let wrote_to_file = if std::io::stderr().is_terminal() {
            let path = default_log_path();
            match LogFile::open(&path) {
                Ok(log_file) => {
                    install_subscriber(log_file, otel_guard.as_ref());
                    true
                }
                Err(e) => {
                    // 文件打开失败:回退 stderr,并在终端留一行提示(此时
                    // 还没进 alt-screen,lib.rs 在 install guard 之前调用)。
                    eprintln!(
                        "zhuye-cli: 打开日志文件 {} 失败,回退 stderr: {e}",
                        path.display()
                    );
                    false
                }
            }
        } else {
            false
        };
        if !wrote_to_file {
            install_subscriber(std::io::stderr, otel_guard.as_ref());
        }

        // 把 guard 泄漏到进程全局,确保退出前不会 drop(否则 provider 被提前
        // shutdown,in-flight span 丢失)。
        if let Some(g) = otel_guard {
            let _ = OTEL_GUARD.set(g);
        }
    });
}

/// 装 subscriber(fmt + 可选 OTLP layer)。OTLP layer 仅在 exporter 起来时
/// 挂载,避免空 subscriber 增加开销。
fn install_subscriber<W>(writer: W, otel_guard: Option<&zhuye_config::ExporterGuard>)
where
    // `Send + Sync` 是必需的:subscriber 最终要装进全局 `Dispatch`,后者要求
    // `Subscriber + Send + Sync + 'static`。泛型 writer 若不带这两个 bound,
    // 编译器无法证明整条 Layered 链满足要求 —— headless 那边 writer 是具体
    // 类型(`std::io::stderr`)所以没暴露这个问题。
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
{
    use tracing_subscriber::prelude::*;

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,reflect=info"));
    // layer 形态的 fmt 没有 `with_env_filter`(那是 `fmt::Subscriber` 的 API);
    // `EnvFilter` 本身实现 `Layer`,作为独立一层挂进 registry —— 与
    // headless `zhuye-exec::runtime_config::init_tracing` 同款装配。
    let fmt_layer = tracing_subscriber::fmt::layer().with_writer(writer);
    let otel_layer =
        otel_guard.map(|g| tracing_opentelemetry::layer().with_tracer(g.tracer("reflect")));
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn log_file_creates_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tui-test.log");
        let lf = LogFile::open(&path).unwrap();
        // 第一段:通过 MakeWriter 写。
        {
            let mut w = lf.make_writer();
            w.write_all(b"first line\n").unwrap();
            w.flush().unwrap();
        }
        // 第二段:再次打开同路径应 append(header + first line 仍在)。
        let lf2 = LogFile::open(&path).unwrap();
        {
            let mut w = lf2.make_writer();
            w.write_all(b"second line\n").unwrap();
            w.flush().unwrap();
        }
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("=== zhuye-cli started"));
        assert!(content.contains("first line"));
        assert!(content.contains("second line"));
    }
}
