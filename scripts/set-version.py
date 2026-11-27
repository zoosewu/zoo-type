"""把 Cargo.toml 的 [workspace.package] version 改成指定版號（發布時以 tag 為準）。

用法：python scripts/set-version.py 0.1.11
之後要執行 cargo update --workspace 更新 Cargo.lock。
"""
import re
import sys
from pathlib import Path

# Windows 主控台預設不是 UTF-8
sys.stdout.reconfigure(encoding="utf-8")
sys.stderr.reconfigure(encoding="utf-8")

version = sys.argv[1].removeprefix("v")
if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?", version):
    sys.exit(f"版號格式不對：{version}")
path = Path(__file__).resolve().parent.parent / "Cargo.toml"
text = path.read_text(encoding="utf-8")
new, n = re.subn(
    r'(?ms)(^\[workspace\.package\].*?^version\s*=\s*")[^"]+(")',
    lambda m: m[1] + version + m[2],
    text,
    count=1,
)
if n != 1:
    sys.exit("找不到 [workspace.package] 的 version")
path.write_text(new, encoding="utf-8", newline="\n")
print(f"版號：{version}")
