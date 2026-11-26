#!/bin/sh
# Run from the logged-in user's Terminal when the calling tool cannot access cfprefsd.
set -eu
# 輸入來源清單是每個使用者自己的：用 sudo 會改到 root 的清單
if [ "$(id -u)" -eq 0 ]; then
    echo "請不要用 sudo，直接執行：sh \"$0\" ${1:-}" >&2
    exit 1
fi
case "${1:-}" in
    --dev) NAME="動物園輸入法 開發版" ;;
    --official) NAME="動物園輸入法" ;;
    "")
        if [ -d "$HOME/Library/Input Methods/動物園輸入法 開發版.app" ] && [ ! -d "$HOME/Library/Input Methods/動物園輸入法.app" ]; then
            NAME="動物園輸入法 開發版"
        elif [ -d "$HOME/Library/Input Methods/動物園輸入法.app" ] && [ ! -d "$HOME/Library/Input Methods/動物園輸入法 開發版.app" ]; then
            NAME="動物園輸入法"
        else
            echo "請指定 --dev 或 --official。" >&2; exit 2
        fi ;;
    *) echo "用法：sh repair-macos-registration.command [--dev|--official]" >&2; exit 2 ;;
esac
/usr/bin/python3 - "$HOME/Library/Input Methods/$NAME.app" <<'PY'
import json, pathlib, plistlib, subprocess, sys, tempfile
app = pathlib.Path(sys.argv[1])
metadata = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
bundle = metadata['CFBundleIdentifier']
assert bundle in ('com.zoosewu.inputmethod.ZooType', 'com.zoosewu.inputmethod.ZooType.dev')
root = pathlib.Path.home() / 'Library/Application Support' / ('ZooType-dev' if bundle.endswith('.dev') else 'ZooType') / 'local'
root.mkdir(parents=True, exist_ok=True)
result_path = root / 'registration-repair-result.json'
try:
    raw = subprocess.check_output(['/usr/bin/defaults', 'export', 'com.apple.HIToolbox', '-'])
    preferences = plistlib.loads(raw)
    previous = preferences.get('AppleEnabledInputSources')
    assert isinstance(previous, list) and previous, '原輸入清單無法讀取，未修改設定'
    backup_root = root / 'registration-backups'
    backup_root.mkdir(parents=True, exist_ok=True)
    directory = pathlib.Path(tempfile.mkdtemp(prefix='terminal-', dir=str(backup_root)))
    (directory / 'HIToolbox.plist').write_bytes(raw)
    required = [{'InputSourceKind': 'Keyboard Input Method', 'Bundle ID': bundle}]
    modes = metadata.get('ComponentInputModeDict', {}).get('tsVisibleInputModeOrderedArrayKey', [])
    assert all(mode.startswith(bundle + '.') for mode in modes)
    required += [{'InputSourceKind': 'Input Mode', 'Bundle ID': bundle, 'Input Mode': mode} for mode in modes]
    def contains(entries, required):
        return any(all(row.get(k) == v for k, v in required.items()) for row in entries)
    missing = [row for row in required if not contains(previous, row)]
    if missing:
        fragments = []
        for row in missing:
            xml = plistlib.dumps(row).decode('utf-8')
            fragments.append(xml[xml.index('<dict>'):xml.rindex('</dict>') + len('</dict>')])
        subprocess.run(['/usr/bin/defaults', 'write', 'com.apple.HIToolbox', 'AppleEnabledInputSources', '-array-add'] + fragments, check=True)
    after = plistlib.loads(subprocess.check_output(['/usr/bin/defaults', 'export', 'com.apple.HIToolbox', '-']))['AppleEnabledInputSources']
    assert all(row in after for row in previous), '原有輸入來源有變動，請查看備份'
    assert all(contains(after, row) for row in required), '動物園輸入來源尚未儲存'
    subprocess.run(['/usr/bin/killall', 'TextInputMenuAgent'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    result = {'ok': True, 'bundle': bundle, 'backup': str(directory), 'added': len(missing)}
    result_path.write_text(json.dumps(result, ensure_ascii=False), encoding='utf-8')
    print('修復完成，已保留其他鍵盤並重新載入輸入法選單。')
    print('備份：', directory)
except Exception as error:
    result_path.write_text(json.dumps({'ok': False, 'error': str(error)}, ensure_ascii=False), encoding='utf-8')
    raise
PY
