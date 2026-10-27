#!/usr/bin/env python3
"""Mock Anthropic Messages API —— 供 zhuye-cli 的**真实 HTTP 链路**测试使用。

监听 127.0.0.1(端口由内核分配),处理 `POST /v1/messages`(stream=true),
按脚本回放 SSE 事件流。零外部依赖(标准库 http.server)。

## 为什么需要它

仓库既有的能力覆盖层用 `ZHUYE_MODEL=mock/mock-1` —— 那是 reflect-llm
`ModelClient` 层的内置 provider,**完全绕过** HTTP 传输、SSE 解析与
Anthropic 协议形态。本 mock server 补上那一层:请求构造(URL / header /
body)、SSE 事件分帧(`content_block_start` / `content_block_delta` /
`content_block_stop` / `message_stop`)、`tool_use` 块都要真的过一遍。

它还提供 `delay`(秒):给「回合中断」这类需要**确定性 busy 窗口**的用例
留出时间 —— 内置 mock provider 瞬时完成,离线无法制造 busy(见
`tests/COVERAGE.md` §6 的相关记录)。

## 进程内运行

服务以 `MockAnthropicServer` 类在**测试进程内的守护线程**里运行:
构造即绑定监听(端口 0 = 内核分配,无占端口竞态),对被测 TUI 子进程
而言与独立进程的 HTTP 服务完全等价。历史上这里起 python 子进程,在
macos runner 上冷启动可卡过 30s 且无任何输出(CI #27/#28 实测),故改
进程内。`__main__` 入口保留独立运行能力(读 MOCK_ANTHROPIC_PORT /
MOCK_ANTHROPIC_DELAY / MOCK_ANTHROPIC_SCRIPT 环境变量)。

脚本规则(按序消费):
  - 脚本文件是 UTF-8 文本,每行一条回复脚本;空行分隔多条。行语法:
      `text: <回复文本>`            —— 普通文本回复(默认)
      `text_file: <路径>`           —— 用文件内容作为回复文本
      `tool: <name>|<json-args>`    —— 回复一个 tool_use 块
      `raw: <json>`                 —— 直接给一段 SSE data(高级)
    未配置脚本时的默认行为:回显最后一条 user 消息:
      `MOCK REPLY: <最后一条用户消息首行>`
"""
from __future__ import annotations

import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

_print_lock = threading.Lock()


def log(msg):
    with _print_lock:
        print(f"[mock] {msg}", flush=True)


def _sse(event, data):
    return [("event", event), ("data", json.dumps(data, ensure_ascii=False))]


def _text_reply_events(text):
    events = []
    events += _sse(
        "message_start",
        {
            "type": "message_start",
            "message": {
                "id": "msg_mock",
                "type": "message",
                "role": "assistant",
                "model": "mock-claude",
                "content": [],
                "usage": {"input_tokens": 10, "output_tokens": 0},
            },
        },
    )
    events += _sse(
        "content_block_start",
        {
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""},
        },
    )
    # 按行切小块,模拟真实流式输出。
    chunks = [text[i : i + 24] for i in range(0, len(text), 24)] or [""]
    for c in chunks:
        events += _sse(
            "content_block_delta",
            {
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": c},
            },
        )
    events += _sse("content_block_stop", {"type": "content_block_stop", "index": 0})
    events += _sse(
        "message_delta",
        {
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 21},
        },
    )
    events += _sse("message_stop", {"type": "message_stop"})
    return events


def _tool_reply_events(name, args_json):
    events = []
    events += _sse(
        "message_start",
        {
            "type": "message_start",
            "message": {
                "id": "msg_mock_tool",
                "type": "message",
                "role": "assistant",
                "model": "mock-claude",
                "content": [],
                "usage": {"input_tokens": 10, "output_tokens": 0},
            },
        },
    )
    events += _sse(
        "content_block_start",
        {
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "tool_use", "id": "toolu_mock_1", "name": name},
        },
    )
    for c in [args_json[i : i + 32] for i in range(0, len(args_json), 32)] or [""]:
        events += _sse(
            "content_block_delta",
            {
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": c},
            },
        )
    events += _sse("content_block_stop", {"type": "content_block_stop", "index": 0})
    events += _sse(
        "message_delta",
        {
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 30},
        },
    )
    events += _sse("message_stop", {"type": "message_stop"})
    return events


class ScriptState:
    """按序消费脚本;耗尽后回退到回显模式(引擎的多轮调用会超额消费)。"""

    def __init__(self, replies):
        self.replies = replies
        self.idx = 0
        self.lock = threading.Lock()

    def next(self, last_user_text):
        with self.lock:
            if self.idx < len(self.replies):
                ev = self.replies[self.idx]
                self.idx += 1
                return ev
        return _text_reply_events(f"MOCK REPLY: {last_user_text}")


def load_script(path):
    """解析脚本文件 → [事件列表, ...](每个元素是一次回复的 SSE 事件序列)。"""
    replies = []
    if path and os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                if line.startswith("text_file:"):
                    fp = line.split(":", 1)[1].strip()
                    with open(fp, encoding="utf-8") as tf:
                        replies.append(_text_reply_events(tf.read()))
                elif line.startswith("text:"):
                    replies.append(_text_reply_events(line.split(":", 1)[1].lstrip()))
                elif line.startswith("tool:"):
                    body = line.split(":", 1)[1].lstrip()
                    name, _, args = body.partition("|")
                    replies.append(_tool_reply_events(name.strip(), args.strip()))
                elif line.startswith("raw:"):
                    replies.append([("data", line.split(":", 1)[1].lstrip())])
                else:
                    replies.append(_text_reply_events(line))
    return replies


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # 安静模式
        pass

    def do_POST(self):
        parsed = urlsplit(self.path)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b""
        if parsed.path != "/v1/messages":
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        try:
            req = json.loads(body or b"{}")
        except Exception:
            req = {}
        last_user = ""
        for m in reversed(req.get("messages", [])):
            if m.get("role") == "user":
                c = m.get("content")
                if isinstance(c, str):
                    last_user = c
                elif isinstance(c, list):
                    for blk in c:
                        if isinstance(blk, dict) and blk.get("type") == "text":
                            last_user = blk.get("text", "")
                            break
                if last_user:
                    break
        last_user = last_user.strip().splitlines()[0][:120] if last_user.strip() else "(empty)"
        log(f"request: model={req.get('model')} last_user={last_user!r}")

        events = self.server.state.next(last_user)
        # 可选人工延迟(秒):给「回合中断」等时序敏感的测试留出窗口。
        # 客户端可能已中断(管道断开),写响应全程吞 BrokenPipe/ConnectionReset。
        try:
            self._send_stream(events)
        except (BrokenPipeError, ConnectionResetError):
            log(f"client disconnected early: {last_user!r}")

    def _send_stream(self, events):
        delay = self.server.delay
        if delay > 0:
            import time as _t

            _t.sleep(delay)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        # SSE 规范:event 与 data 必须在同一个事件块里(`event: X\ndata: Y\n\n`),
        # 分开发送会被解析成两个事件(data-only 的事件名默认为 "message")。
        for kind, payload in events:
            body = f"{kind}: {payload}\n"
            if kind == "data":
                body += "\n"  # 事件块结束
            chunk = body.encode("utf-8")
            self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
            self.wfile.flush()
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        body = b"mock anthropic ok"
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class _Server(ThreadingHTTPServer):
    """携带业务状态的 httpd 实例:`_Handler` 经 `self.server` 读取。"""

    daemon_threads = True

    def __init__(self, addr, delay: float, state: ScriptState):
        # _Handler 在响应阶段读 self.server.delay / self.server.state,
        # 两者必须挂在 httpd 实例上(而非外层包装对象)。
        self.delay = delay
        self.state = state
        super().__init__(addr, _Handler)


class MockAnthropicServer:
    """进程内 mock Anthropic 服务:构造即监听,`stop()` 收尾。

    - `port=0`:内核分配空闲端口(替代「先占后放」的 _free_port,消除
      端口被并发抢占的竞态);真实端口经 `port` 属性读取。
    - `delay`:每个响应的人为延迟秒数(确定性 busy 窗口)。
    - `script_file`:脚本文件路径;空串 = 回显模式。
    - 服务线程是守护线程,进程退出自动回收;显式 `stop()` 立即收尾。
    """

    def __init__(self, port: int = 0, delay: float = 0.0, script_file: str = ""):
        self.state = ScriptState(load_script(script_file))
        self._httpd = _Server(("127.0.0.1", port), delay, self.state)
        self._thread = threading.Thread(
            target=self._httpd.serve_forever, name="mock-anthropic", daemon=True
        )
        self._thread.start()
        log(f"listening on http://127.0.0.1:{self.port} (script={script_file or '(echo mode)'})")

    @property
    def port(self) -> int:
        return self._httpd.server_address[1]

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def stop(self) -> None:
        self._httpd.shutdown()
        self._httpd.server_close()


def serve_standalone():
    """独立运行入口:读环境变量,行为与历史上的子进程模式一致。"""
    server = MockAnthropicServer(
        port=int(os.environ.get("MOCK_ANTHROPIC_PORT", "18431")),
        delay=float(os.environ.get("MOCK_ANTHROPIC_DELAY", "0") or 0),
        script_file=os.environ.get("MOCK_ANTHROPIC_SCRIPT", ""),
    )
    try:
        # 独立模式下阻塞到被 kill;serve_forever 已在守护线程,主线程挂起即可。
        threading.Event().wait()
    except KeyboardInterrupt:
        server.stop()
        sys.exit(0)


if __name__ == "__main__":
    serve_standalone()
