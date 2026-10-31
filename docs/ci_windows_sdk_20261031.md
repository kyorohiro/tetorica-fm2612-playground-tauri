# Windows Actions: PS1 SPUヘッダー修正の取り込み

2026-10-31、Tauri HEAD `17cda440d59fab2eaf63b858c2aceffd3e43ad41`の
[Build Desktop run](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/41160069616)を確認。
macOS2種類／Linux2種類は成功。WindowsはCargo testのC++コンパイルで停止した。

`vendor/tetorica-soundchip-native/vendor/mame-ps1-spu/spu.cpp(3217)`で
MSVC C2039／C3861：`std::out_of_range`の宣言が見つからない。
SDK原本は`cf1747387eb2b66323124959d7b356b936197784`で`<stdexcept>`追加済みだが、
Tauriが保持していた旧候補版には未反映だった。

## 取り込み

SDKのclean checkoutから`package_release.py`でソースZIPを生成し、
`import_soundchip_sdk.py`で476ファイルを正規取り込みした。snapshotは直接編集していない。
旧snapshotとSDK原本の変更は`vendor/mame-ps1-spu/spu.cpp`と`catalog/source_inventory.json`の2点のみ。
SCSP等の音源処理・API・frontend・WASMは変更していない。

- SDK版：0.1.7
- SDK元コミット：`cf1747387eb2b66323124959d7b356b936197784`
- source ZIP SHA256：`0ca4bbc5fcc76df36f1f0f634ac0f039e8ec94c76b914e36ebbb12d949963c2d`
- manifest SHA256：`3f83a47b633e75524efabc5d8e1686911ddacf92b23fc469e6696776905fde0b`
- 旧`localCandidate`は不要になり、同コミットのソースそのものへpinした。
  このローカルZIP生成はSDKタグ作成／公開release取得を意味しない。

## 検証

- SDK原本の同コミットは[Windows Native CI成功](https://github.com/kyorohiro/tetorica-soundchip-native/actions/runs/41155523323/job/124401362678)。
- `npm test`：Python32件＋Node31件成功。
- `import_release.py --check`：frontend 1,120 files照合成功。
- `import_soundchip_sdk.py --check`：476 files照合成功。
- Tauri全体の`cargo test --locked --offline`：**814成功・0失敗・4 ignored**（ローカルmacOS arm64）。
  初回sandbox実行はローカルHTTPの2テストだけPermissionDeniedだったため、制限外で全件再実行して成功。
  ignoredの物理音声deviceテストは実行していない。

対応するapplication source ZIPは`prepare_native_sources.py`で再生成し、
`check_native_sources.py --bundle`でcheckoutの全source／hashと照合する。
生成ZIP／manifestは従来どおりgitignore対象で、CIではaudio sidecar準備時にも生成される。

Tauri側は未commit／未push。失敗済みタグの再実行は元のソースを使用するため、
この修正を含む新しいcommitのBuild DesktopでWindowsを確認する。
