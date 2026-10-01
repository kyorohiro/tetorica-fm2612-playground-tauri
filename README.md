# Tetorica Playground — Tauri

Tetorica Playground の配布ZIPを変更せずに読み込む、独立したデスクトップ版です。
既存の Menu → **Always on Top** で最前面への固定を切り替えます。起動時はOffです。
macOSの非同期ウィンドウ更新に対応し、1クリックで表示を切り替えます。

## 開発

Node.js 22、Rust、各OSのTauriビルド環境が必要です。

```sh
npm ci
python3 scripts/fetch_release.py
npm run dev
```

ビルドは `npm run build`、検証は `npm test` と `cargo test --locked --manifest-path src-tauri/Cargo.toml`。
配布ZIPは `release.source.json` と `release.lock.json` で固定し、SHA256と全展開ファイルを検証します。
ダウンロードの自動再試行はしません。

ローカルZIPを使う場合:

```sh
python3 scripts/import_release.py /path/to/release.zip
```

上流の更新時のみ `--version VERSION` を付け、URLとロックファイルを一緒に更新してください。
ラッパーのバージョンと上流ZIPのバージョンは別に管理します。

## 構成

- `dist/`: 上流ZIPそのまま（Git管理外）
- `desktop/desktop-interface.js`: 既存MenuへTauri専用の最前面固定項目を追加
- `src-tauri/src/main.rs`: ローカルのメインウィンドウに限定した最前面固定コマンド
- `assets/app-icon.png`: 上流Playgroundアイコン。`npm run icons` でデスクトップ用を生成

MCP連携は未実装です。今後、ファイル一覧・読み取り・編集と型/API/examplesの参照を追加する予定です。

GitHub Actionsは手動実行で各OSのインストーラーを生成し、`v*`タグでドラフトリリースへ添付します。
署名・公証の設定は含みません。秘密情報を含む他アプリの配布スクリプトはコピーしていません。
