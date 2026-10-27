"""能力覆盖层的 pty 测试设施(不依赖任何外部 SDK)。

模块名带 `tui_` 前缀:`pty` 是 Python 标准库模块,同名会遮蔽它
(与 submodule `tests/functional/wire.py` 用唯一命名的理由一致)。

**为什么自己起子进程**:能力覆盖层测的是 **本仓库 Rust 代码在真产物上的
行为**(真实渲染、真实按键路由、真实文件落盘)。如果只测内部函数,接线错误
(事件没接到 UI、slash 命令没注册、文件没落)会被遮住 —— 单元测试看不见
「用户真的看到了什么」。

与旧 `test/harness.py` 的差异(本次重构):
  - 隔离 HOME + cwd 由 `Sandbox` 统一管理,会话结束自动清理;
  - 支持任意按键(`key("ctrl+c")` / `key("up")` / `key("f5")`),不再只支持
    字母类 Ctrl 组合;
  - 屏幕断言带等待与超时(回归时**断言失败**而不是挂死套件);
  - 保留原始字节流,用于 panic / 断言失败消息回显;
  - `resize()` 支持改窗口尺寸后重绘。

依赖:`pyte`(pip install pyte)。缺失时 pytest 跳过整层,不算失败。
"""

from __future__ import annotations

import fcntl
import os
import re
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

# 仓库根:tests/functional/pty.py → 上溯两级。
ROOT = Path(__file__).resolve().parents[2]
# 被测二进制:默认 release 产物(与 tests/lib.sh 的约定一致)。
ZHUYE_CLI_BIN = Path(
    os.environ.get("ZHUYE_CLI_BIN", str(ROOT / "target" / "release" / "zhuye-cli"))
)

# 墙钟预算:等待事件/文案的统一上限。回归时断言失败,绝不挂死整个套件。
DEFAULT_TIMEOUT = 30.0
# 与 submodule `tests/functional/wire.py` 的 `DRAIN_TIMEOUT` 同名同义:
# 「等待事件流排空」的墙钟上限。
DRAIN_TIMEOUT = DEFAULT_TIMEOUT
# 启动稳定等待:boot banner 动画 + 首帧渲染。
BOOT_TIMEOUT = 25.0

# HUD 忙碌 spinner(history_render/statusline.rs 的 SPINNER_FRAMES,共 10 帧)。
# 忙碌时状态行渲染成 `<帧> <model> · cwd…`,空闲时无此前缀。判定用
# 「帧字符后跟 ASCII 空格」:boot banner 的 braille 画虽含同款帧字形,
# 但内部以 braille 空白(U+2800,非 Unicode 空白)连接,不会命中。
_BUSY_RE = re.compile("[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] ")

# 按键名 → 字节序列。覆盖 TUI 用到的全部非普通字符输入。
KEY_SEQS: dict[str, bytes] = {
    "enter": b"\r",
    "cr": b"\r",
    "esc": b"\x1b",
    "escape": b"\x1b",
    "tab": b"\t",
    "backtab": b"\x1b[Z",
    "backspace": b"\x7f",
    "space": b" ",
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "home": b"\x1b[H",
    "end": b"\x1b[F",
    "pageup": b"\x1b[5~",
    "pagedown": b"\x1b[6~",
    "delete": b"\x1b[3~",
    "insert": b"\x1b[2~",
    "f1": b"\x1bOP",
    "f2": b"\x1bOQ",
    "f3": b"\x1bOR",
    "f4": b"\x1bOS",
    "f5": b"\x1b[15~",
    "f6": b"\x1b[17~",
    "f10": b"\x1b[21~",
    "f11": b"\x1b[23~",
    "f12": b"\x1b[24~",
    "ctrl+c": b"\x03",
    "ctrl+d": b"\x04",
    "ctrl+a": b"\x01",
    "ctrl+e": b"\x05",
    "ctrl+u": b"\x15",
    "ctrl+k": b"\x0b",
    "ctrl+w": b"\x17",
    "ctrl+r": b"\x12",
    "ctrl+l": b"\x0c",
    "ctrl+z": b"\x1a",
    "ctrl+t": b"\x14",
    "ctrl+o": b"\x0f",
    "ctrl+g": b"\x07",
    "ctrl+b": b"\x02",
    "ctrl+shift+c": b"\x03",
    "ctrl+shift+d": b"\x04",
    "ctrl+left": b"\x1b[1;5D",
    "ctrl+right": b"\x1b[1;5C",
    "ctrl+up": b"\x1b[1;5A",
    "ctrl+down": b"\x1b[1;5B",
    "alt+enter": b"\x1b\r",
    "shift+tab": b"\x1b[Z",
}


def normalize(text: str) -> str:
    """折叠空白为单空格 —— 屏幕折行/markdown 渲染会打断子串匹配。"""
    return " ".join(text.split())


@dataclass
class Sandbox:
    """隔离的 HOME + cwd;子进程 env 由此派生,不污染宿主进程环境。"""

    home: Path
    cwd: Path

    @classmethod
    def create(cls, prefix: str = "zhuye-cli-test-") -> "Sandbox":
        """建隔离目录,并预写空的 `~/.zhuye/config.toml`。

        `reflect_config::load_default()` 对缺文件是「回落默认配置」而非报错,
        但显式写入可让「配置文件存在」这一前提在用例里显式成立,避免依赖
        宿主机残留配置(宿主机若有 provider 配置,会改变启动行为)。
        """
        base = Path(tempfile.mkdtemp(prefix=prefix))
        home = base / "home"
        cwd = base / "cwd"
        (home / ".zhuye").mkdir(parents=True)
        cwd.mkdir(parents=True)
        (home / ".zhuye" / "config.toml").write_text(
            "# 测试用最小配置:空配置(不指定 provider,靠 ZHUYE_MODEL 环境变量)。\n"
        )
        return cls(home=home, cwd=cwd)

    def env(self, extra: dict[str, str] | None = None) -> dict[str, str]:
        """派生子进程 env:HOME 指向隔离目录,其余继承当前进程。"""
        env = dict(os.environ)
        env["HOME"] = str(self.home)
        # 屏蔽启动期 GitHub releases 探测 —— 离线环境不卡启动。
        env.setdefault("REFLECT_TUI_UPDATE_CHECK", "0")
        # crossterm 在 TERM=dumb / 缺省下不渲染(non-ANSI terminal)。
        if not env.get("TERM") or env["TERM"] == "dumb":
            env["TERM"] = "xterm-256color"
        if extra:
            env.update(extra)
        return env

    def mock_env(self, script: Path | None = None, **extra: str) -> dict[str, str]:
        """派生 mock provider 环境:`ZHUYE_MODEL=mock/mock-1` + 可选脚本。"""
        env: dict[str, str] = {"ZHUYE_MODEL": "mock/mock-1"}
        if script is not None:
            env["ZHUYE_MOCK_SCRIPT"] = str(script)
        env.update(extra)
        return env

    def cleanup(self) -> None:
        """删除整个隔离根目录。"""
        base = self.home.parent
        shutil.rmtree(base, ignore_errors=True)

    # ── 会话产物查询(断言落盘用)──────────────────────────────────────────

    def session_files(self) -> list[Path]:
        """会话 JSONL 文件列表(`~/.zhuye/sessions/YYYY/MM/DD/<thread>.jsonl`)。"""
        base = self.home / ".zhuye" / "sessions"
        return sorted(base.glob("*/*/*/*.jsonl"))

    def trace_files(self) -> list[Path]:
        """telemetry 产物(`~/.zhuye/traces/` 下的 jsonl)。"""
        base = self.home / ".zhuye" / "traces"
        return sorted(base.rglob("*.jsonl"))

    def workspace_files(self, pattern: str = "**/*") -> list[Path]:
        """工作区产物(`.zhuye/plan/*.md` 等)。"""
        return sorted(p for p in self.cwd.glob(pattern) if p.is_file())


class TuiSession:
    """在 pty 里跑一个 zhuye-cli 进程,用 pyte 维护屏幕快照。

    用法::

        with TuiSession(sandbox=sb, cols=120, rows=38) as s:
            s.wait_boot()
            s.type_text("/status")
            s.key("enter")
            s.wait_text("Token")
    """

    def __init__(
        self,
        sandbox: Sandbox,
        args: list[str] | None = None,
        env: dict[str, str] | None = None,
        cols: int = 120,
        rows: int = 40,
        binary: Path | None = None,
    ):
        try:
            import pyte
        except ImportError as e:  # pragma: no cover
            raise ImportError("TuiSession needs pyte: pip install pyte") from e

        self._pyte = pyte
        self.sandbox = sandbox
        self.args = list(args or [])
        self.binary = Path(binary or ZHUYE_CLI_BIN)
        self.cols = cols
        self.rows = rows

        master_fd, slave_fd = os.openpty()
        fcntl.ioctl(slave_fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self._master_fd = master_fd
        self._screen = pyte.Screen(cols, rows)
        self._stream = pyte.ByteStream(self._screen)

        self._raw = bytearray()
        self._raw_lock = threading.Lock()

        self._proc = subprocess.Popen(
            [str(self.binary), *self.args],
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            cwd=str(sandbox.cwd),
            env=sandbox.env(env),
            close_fds=True,
            preexec_fn=os.setsid,
        )
        os.close(slave_fd)  # 父进程只留 master

        self._closed = False
        self._reader_thread = threading.Thread(target=self._read_loop, daemon=True)
        self._reader_thread.start()

    # ── 后台读线程:pty 输出 → pyte 屏幕 ──────────────────────────────

    def _read_loop(self) -> None:
        try:
            while True:
                try:
                    data = os.read(self._master_fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                with self._raw_lock:
                    self._raw.extend(data)
                self._stream.feed(data)
                # 应答 DSR 光标位置查询(`ESC[6n`)—— inline 视口锚定依赖它,
                # 真终端会回 `ESC[<row>;<col>R`;哑 pty 不回,应用会一直等。
                if b"\x1b[6n" in data:
                    self._send(b"\x1b[24;1R")
        except Exception:
            pass

    def _send(self, data: bytes) -> None:
        if self._closed:
            raise RuntimeError("TuiSession is closed")
        try:
            os.write(self._master_fd, data)
        except OSError:
            pass

    # ── 输入 ──────────────────────────────────────────────────────────

    def type_text(self, text: str) -> None:
        """整段写入 —— 等价真实粘贴/快速输入(slash 弹窗对其已验证)。"""
        self._send(text.encode())

    def key(self, name: str) -> None:
        """发一个具名按键(`enter` / `esc` / `ctrl+c` / `up` / `f5` …)。"""
        seq = KEY_SEQS.get(name.lower())
        if seq is None:
            raise KeyError(f"未知按键:{name}")
        self._send(seq)

    def ctrl(self, letter: str) -> None:
        """发 Ctrl+字母(等价 `key("ctrl+x")`)。"""
        self._send(bytes([ord(letter.upper()) - 0x40]))

    # ── 屏幕读取 ──────────────────────────────────────────────────────

    def display_lines(self) -> list[str]:
        return list(self._screen.display)

    def screen_text(self) -> str:
        return "\n".join(self.display_lines())

    def normalized(self) -> str:
        """归一化屏幕文本(空白折叠)—— 断言用这个,避免折行打断匹配。"""
        return normalize(self.screen_text())

    def raw_output(self) -> str:
        """原始字节流解码 —— 断言失败时回显用(panic / backtrace 都在这)。"""
        with self._raw_lock:
            return bytes(self._raw).decode("utf-8", errors="replace")

    def has_panic(self) -> bool:
        """原始输出里是否出现 Rust panic 痕迹。"""
        raw = self.raw_output()
        return bool(
            re.search(r"panicked at|thread '[^']+' panicked|stack backtrace:", raw)
        )

    # ── 等待 ──────────────────────────────────────────────────────────

    def _pump(self, settle: float = 0.4, max_wait: float = 0.6) -> None:
        """让后台线程持续喂数据,停留 `max_wait`(先静置 `settle`)。"""
        deadline = time.monotonic() + max(0.0, max_wait)
        time.sleep(settle)
        while time.monotonic() < deadline:
            time.sleep(0.05)

    def wait(
        self,
        predicate: Callable[[str], bool],
        timeout: float = DEFAULT_TIMEOUT,
        label: str = "条件",
    ) -> str:
        """等 `predicate(归一化屏幕文本)` 成立,返回当时的屏幕文本。

        超时抛 `AssertionError` 并附带屏幕尾部 —— 回归时**断言失败**
        而不是拖死整个套件。
        """
        deadline = time.monotonic() + timeout
        last = ""
        while time.monotonic() < deadline:
            last = self.normalized()
            if predicate(last):
                return last
            self._pump()
        tail = "\n".join(self.display_lines()[-12:])
        raise AssertionError(
            f"{timeout:.0f}s 内未等到「{label}」\n--- 屏幕尾部 ---\n{tail}\n"
            f"--- 原始输出尾部 ---\n{self.raw_output()[-1500:]}"
        )

    def wait_text(self, needle: str, timeout: float = DEFAULT_TIMEOUT) -> str:
        """等屏幕上出现 `needle`(归一化后的大小写敏感匹配)。"""
        return self.wait(lambda t: needle in t, timeout, label=needle)

    def wait_text_ci(self, needle: str, timeout: float = DEFAULT_TIMEOUT) -> str:
        """等屏幕上出现 `needle`(大小写不敏感)。"""
        low = needle.lower()
        return self.wait(lambda t: low in t.lower(), timeout, label=needle)

    def wait_idle(self, timeout: float = DEFAULT_TIMEOUT) -> str:
        """等回合收尾:状态行的忙碌 spinner 消失,返回当时屏幕文本。

        `wait_text` 在文本刚上屏(可能还是 `○ ` 未提交活尾)就会返回;
        而历史行定型(`• ` 前缀)、回合分隔线、会话落盘、Ctrl+O 可复制
        等都发生在**回合提交**时。凡要断言提交态、继续交互(下一轮
        输入 / 按键)或读落盘产物的用例,先 `wait_text(marker)` 再
        `wait_idle()`,避免慢机器上的流中竞态(实测:本地快机与
        ubuntu CI 通过,macos CI 偶发 `○` 态断言失败)。

        空闲信号 = `_BUSY_RE`(spinner 帧 + 空格)不再出现。
        """
        return self.wait(
            lambda t: not _BUSY_RE.search(t),
            timeout,
            label="回合收尾(spinner 消失)",
        )

    def wait_absent(self, needle: str, timeout: float = 5.0) -> str:
        """等 `needle` 从屏幕上消失。"""
        return self.wait(lambda t: needle not in t, timeout, label=f"¬{needle}")

    def wait_boot(self, timeout: float = BOOT_TIMEOUT) -> str:
        """等 TUI 启动到稳定首屏(退出提示出现 = 渲染已就绪)。"""
        return self.wait_text_ci("ctrl+c", timeout)

    def wait_stable(self, max_wait: float = 10.0, quiet: float = 0.4) -> bool:
        """等屏幕停止变化(连续 `quiet` 秒无更新)。返回是否在时限内稳定。"""
        deadline = time.monotonic() + max_wait
        last = None
        last_change = time.monotonic()
        while time.monotonic() < deadline:
            snap = self.screen_text()
            if snap != last:
                last = snap
                last_change = time.monotonic()
            elif time.monotonic() - last_change >= quiet:
                return True
            time.sleep(0.1)
        return False

    def resize(self, cols: int, rows: int) -> None:
        """改窗口尺寸,触发应用重绘(覆盖 resize 响应路径)。"""
        self.cols, self.rows = cols, rows
        self._screen.resize(rows, cols)
        fcntl.ioctl(
            self._master_fd,
            termios.TIOCSWINSZ,
            struct.pack("HHHH", rows, cols, 0, 0),
        )
        self._send(f"\x1b[8;{rows};{cols}t".encode())

    # ── 生命周期 ──────────────────────────────────────────────────────

    def alive(self) -> bool:
        return self._proc.poll() is None

    def returncode(self) -> int | None:
        return self._proc.poll()

    def quit(self, timeout: float = 8.0) -> int | None:
        """礼貌退出:双击 Ctrl-C(应用的多 picker 双击退出语义)。

        哑 pty 下进程退出不可靠(真实终端行为正常),超时则 SIGTERM 兜底。
        返回进程退出码(None = 仍在跑)。
        """
        if self._proc.poll() is None:
            self._send(b"\x03")
            time.sleep(0.1)
            self._send(b"\x03")
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self._proc.poll() is not None:
                break
            time.sleep(0.15)
        if self._proc.poll() is None:
            try:
                os.killpg(os.getpgid(self._proc.pid), signal.SIGTERM)
            except (ProcessLookupError, PermissionError, OSError):
                pass
            deadline = time.monotonic() + 3.0
            while time.monotonic() < deadline and self._proc.poll() is None:
                time.sleep(0.15)
        return self._proc.poll()

    def close(self) -> None:
        if self._closed:
            return
        self.quit()
        self._closed = True
        try:
            os.close(self._master_fd)
        except OSError:
            pass
        self._reader_thread.join(timeout=2.0)

    def __enter__(self) -> "TuiSession":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        # 异常路径也要收掉 pty 进程,不留孤儿 TUI。
        if exc is not None:
            try:
                os.killpg(os.getpgid(self._proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError, OSError):
                pass
        self.close()


# ── mock provider 脚本 ────────────────────────────────────────────────────

def write_mock_script(path: Path, lines: list[str]) -> Path:
    """写 mock provider 脚本:JSONL,每行一次模型回复。

    行格式(见 reflect-llm/src/providers/mock.rs):
    `{"type":"text","text":"..."}` / `{"type":"tool_call","name":"t","args":{}}`
    """
    import json

    path.write_text("\n".join(lines) + "\n")
    return path


def text_reply(text: str) -> str:
    """构造一条纯文本 mock 回复。"""
    import json

    return json.dumps({"type": "text", "text": text}, ensure_ascii=False)


def tool_call(name: str, args: dict | None = None) -> str:
    """构造一条工具调用 mock 回复。"""
    import json

    return json.dumps(
        {"type": "tool_call", "name": name, "args": args or {}}, ensure_ascii=False
    )


def tool_call_delta(name: str, chunks: list[str]) -> str:
    """构造一条**分片形态**的工具调用 mock 回复。

    参数不走 `ToolUseStart`,而是由若干 `ToolUseDelta` 分片送来 ——
    engine 侧走 `args_from_start == false` 分支,逐段转发
    `tool_call_args_delta` 事件给客户端(TUI 用来渐进预览工具参数)。
    """
    import json

    return json.dumps(
        {"type": "tool_call_delta", "name": name, "chunks": chunks},
        ensure_ascii=False,
    )
