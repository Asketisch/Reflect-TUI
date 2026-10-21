"""live 组的 pytest fixture。

live 用例复用能力覆盖层的 pty 设施(`tests/functional/tui_pty.py`),
因此把该目录加进 `sys.path` —— 否则从仓库根跑 `pytest tests/live/`
会找不到模块。
"""

from __future__ import annotations

import sys
from pathlib import Path

FUNCTIONAL_DIR = Path(__file__).resolve().parents[1] / "functional"
if str(FUNCTIONAL_DIR) not in sys.path:
    sys.path.insert(0, str(FUNCTIONAL_DIR))
