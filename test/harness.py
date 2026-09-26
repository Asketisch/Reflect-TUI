"""TUI pty 测试 harness。

按丢失前原始模块的公开 API 重建(依据 `test_workflow_live.py` 的调用面与
`__pycache__` 残留的符号表):

- `find_binary()` — 定位被测二进制:`REFLECT_TUI_BIN` env > `target/release`
  > `target/debug`;找不到抛 `FileNotFoundError`。
- `TuiSession` — pty 会话(pyte 终端仿真 + 后台读线程),context manager。
  方法面与 live 脚本一致:`type_fast` / `enter` / `esc` / `ctrl` / `exit` /
  `wait` / `wait_stable` / `_pump` / `screen_text` / `display_lines` / `close`。

依赖:`pyte`(pip install pyte);仅 `TuiSession` 需要,`find_binary` 不需要。
"""
from __future__ import annotations

import fcntl
import os
import signal
import struct
import subprocess
import sys
import termios
import threading
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def normalize(text: str) -> str:
    """折叠空白为单空格 —— 屏幕折行/markdown 渲染会打断子串匹配。"""
    return " ".join(text.split())


def find_binary() -> str:
    """定位被测 reflect-tui 二进制(与原始实现同优先级)。"""
    env_bin = os.environ.get("REFLECT_TUI_BIN")
    candidates: list[Path] = []
    if env_bin:
        candidates.append(Path(env_bin))
    for profile in ("release", "debug"):
        candidates.append(REPO_ROOT / "target" / profile / "reflect-tui")
    for c in candidates:
        if c.exists() and os.access(c, os.X_OK):
            return str(c)
    raise FileNotFoundError(
        "reflect-tui binary not found; build it (cargo build --release) "
        "or point REFLECT_TUI_BIN at an existing binary"
    )


class TuiSession:
    """在 pty 里跑一个 reflect-tui 进程,用 pyte 维护屏幕快照。

    用法::

        with TuiSession(cols=120, rows=38) as s:
            s.wait_stable(max_wait=8.0)
            s.type_fast("/status")
            s.enter(settle=1.0)
            print(s.screen_text())
    """

    def __init__(
        self,
        workdir: str | os.PathLike[str] | None = None,
        binary: str | None = None,
        args: list[str] | None = None,
        cols: int = 120,
        rows: int = 40,
        env: dict[str, str] | None = None,
    ):
        try:
            import pyte
        except ImportError as e:  # pragma: no cover
            raise ImportError("TuiSession needs pyte: pip install pyte") from e

        self._pyte = pyte
        self.workdir = str(workdir or REPO_ROOT)
        self.binary = binary or find_binary()
        self.args = list(args or [])

        child_env = dict(os.environ)
        # 屏蔽启动期 GitHub releases 探测,离线环境不卡启动。
        child_env.setdefault("REFLECT_TUI_UPDATE_CHECK", "0")
        # crossterm 在 TERM=dumb/缺省 下不渲染(non-ANSI terminal)。CI / 嵌入
        # shell 常见 dumb,这里强制成可用的 TERM。
        if not child_env.get("TERM") or child_env["TERM"] == "dumb":
            child_env["TERM"] = "xterm-256color"
        if env:
            child_env.update(env)

        master_fd, slave_fd = os.openpty()
        fcntl.ioctl(slave_fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self._master_fd = master_fd
        self._screen = pyte.Screen(cols, rows)
        self._stream = pyte.ByteStream(self._screen)

        self._proc = subprocess.Popen(
            [self.binary, *self.args],
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            cwd=self.workdir,
            env=child_env,
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
    def type_fast(self, text: str) -> None:
        """整段写入 —— 等价真实粘贴/快速输入(slash 弹窗对其已验证)。"""
        self._send(text.encode())

    def enter(self, settle: float = 1.0) -> None:
        self._send(b"\r")
        time.sleep(settle)

    def esc(self, settle: float = 0.5) -> None:
        self._send(b"\x1b")
        time.sleep(settle)

    def ctrl(self, key: str) -> None:
        self._send(bytes([ord(key.upper()) - 0x40]))

    # ── 屏幕读取 ──────────────────────────────────────────────────────
    def display_lines(self) -> list[str]:
        return list(self._screen.display)

    def screen_text(self) -> str:
        return "\n".join(self.display_lines())

    # ── 等待 ──────────────────────────────────────────────────────────
    def _pump(self, settle: float = 0.5, max_wait: float = 2.0) -> None:
        """让后台线程持续喂数据,停留 `max_wait`(先静置 `settle`)。"""
        deadline = time.time() + max(0.0, max_wait)
        time.sleep(settle)
        while time.time() < deadline:
            time.sleep(0.05)

    def wait(self, settle: float = 0.5, max_wait: float = 5.0) -> None:
        self._pump(settle=settle, max_wait=max_wait)

    def wait_stable(self, max_wait: float = 10.0, quiet: float = 0.4) -> bool:
        """等屏幕停止变化(连续 `quiet` 秒无更新)。返回是否在时限内稳定。"""
        deadline = time.time() + max_wait
        last = None
        last_change = time.time()
        while time.time() < deadline:
            snap = self.screen_text()
            if snap != last:
                last = snap
                last_change = time.time()
            elif time.time() - last_change >= quiet:
                return True
            time.sleep(0.1)
        return False

    # ── 生命周期 ──────────────────────────────────────────────────────
    def exit(self, timeout: float = 5.0) -> None:
        """礼貌退出:Ctrl-C → 超时 SIGKILL 整个进程组。"""
        if self._proc.poll() is None:
            self._send(b"\x03")
            try:
                self._proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(os.getpgid(self._proc.pid), signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
                self._proc.wait(timeout=timeout)

    def close(self) -> None:
        if self._closed:
            return
        self.exit()
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
