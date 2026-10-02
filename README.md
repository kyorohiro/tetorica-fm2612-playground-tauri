# Tetorica Playground — Tauri

Tetorica Playground の配布ZIPを取り込み、展開した `dist/` を Git 管理するデスクトップ版です。ローカルと GitHub Actions は同じコミットの本体を組み込みます。
既存の Menu → **Always on Top** で最前面への固定を切り替えます。起動時はOffです。
macOSの非同期ウィンドウ更新に対応し、1クリックで表示を切り替えます。

## 開発

Node.js 22、Rust、各OSのTauriビルド環境が必要です。

```sh
npm ci
npm run dev
```

ビルドは `npm run build`、検証は `npm test` と `cargo test --locked --manifest-path src-tauri/Cargo.toml`。
チェックアウトに `dist/` が含まれるため、起動・ビルド時に本体ZIPをダウンロードしません。
`release.lock.json` は取り込み元の記録と展開内容の整合性検査に使い、dev / build 前と Actions で `python3 scripts/import_release.py --check` を実行します。
`release.source.json` と `fetch_release.py` は廃止しました。

上流の本体を更新する場合:

```sh
python3 scripts/import_release.py /path/to/release.zip --version VERSION
```

取り込み後に動作確認し、`dist/` と `release.lock.json` を同じコミットに含めてください。リリースタグもそのコミットに付けます。
`.gitattributes` で `dist/` の改行変換を無効にし、Windowsでも同じバイト列を保持します。
ラッパーのバージョンと上流ZIPのバージョンは別に管理します。

## 構成

- `dist/`: 上流ZIPそのまま（Git管理対象）
- `desktop/desktop-interface.js`: 既存MenuへTauri専用の最前面固定項目を追加
- `src-tauri/src/main.rs`: ローカルのメインウィンドウに限定した最前面固定コマンド
- `assets/app-icon.png`: 上流Playgroundアイコン。`npm run icons` でデスクトップ用を生成

## MCP

1. アプリを起動してプロジェクトを開きます。
2. **Menu → MCP: Off** を押してOnにします（起動時はOff）。
3. **Menu → MCP connection…** の接続情報を、Streamable HTTP対応のMCPクライアントへ登録します。

接続先は `http://127.0.0.1:39127/mcp`、認証は `Authorization: Bearer <表示されたトークン>` です。
接続画面のJSONは `mcpServers` 形式の例です。クライアントによって設定形式が異なるため、URLとHTTPヘッダーをそれぞれ指定してください。
トークンは初回起動時に生成し、Tauriのアプリデータディレクトリの `mcp-token` に保存します（Unixでは所有者のみ読み書き可能）。MCPのOff/Onやアプリ再起動でも同じトークンを使うため、クライアントへの登録は初回だけです。MCP自体は引き続き起動時Offです。

再発行する場合はMCPをOffにして **Menu → Regenerate MCP token** を選びます。確認後に新しい接続情報が表示され、古いトークンは使えなくなります。この場合のみクライアント設定を更新してください。
別インスタンス等が同じポートを使用中の場合は、Menuに起動エラーを表示します。

| ツール | 内容 |
| --- | --- |
| `list_files` | 開いているプロジェクトのIDとファイル一覧（バイナリは情報のみ） |
| `read_file` | テキストを読み取り。エディターの最新内容を含む |
| `write_file` | テキストファイルを作成・更新（1 MiBまで）。自動実行しない |

更新時は `read_file` の `projectId` と `content` を、それぞれ `projectId` と `expectedContent` に渡します。
人の編集等で内容が変わっていれば更新を拒否します。新規作成は `list_files` の `projectId` と `expectedContent: null` を指定します。
Cassetteの読み込み等でプロジェクトを入れ替えた場合も、古いIDによる更新を拒否します。
対象はメモリー上の仮想ファイルです。**保存は既存のExport Cassetteで行ってください。**
OS上のファイル操作、バイナリ編集、`/sys` の編集、実行・停止のツールは提供しません。

`resources/list` / `resources/read` では、配布版に含まれる型定義 `.d.ts`、`llms.txt`、examplesを読み取り専用で参照できます。
URIは `tetorica://reference/<配布版内のパス>` です。AIにはこれらを参照してからコードを書くよう指示できます。

通信は[MCP Streamable HTTP仕様](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports)のJSON応答方式、プロトコル `2025-06-18` です。
SSE購読は提供せずGETに405を返します。ローカルのネイティブMCPクライアント向けで、ブラウザーOrigin付きリクエストは拒否します。

### エディターとの接続

`desktop/project-interface.js` を、Tauriが配信する `/playground.js` の応答末尾に追加します。
既存モジュールのファイル管理・エディター更新関数を使うため、別のファイルストアへのコピーや同期は不要です。
**ディスク上の `dist` と上流ZIPは変更しませんが、配信時のJavaScriptは拡張します。**
この接続部分は固定した上流版の内部APIに依存します。上流更新時はブラウザーで編集連携を再検証してください。
`build.devUrl` による外部開発サーバーはこの応答拡張の対象外です。`npm run dev` は `tauri dev --no-dev-server` で起動し、アプリ内の配信経路を使います。単独の `tauri dev` は内蔵HTTPサーバーを起動するため、MCPのファイル操作には使わないでください。

「editor bridge is not loaded」は接続処理の未登録を意味します。ウィンドウを前面に出す・HTTPページを再読み込みするだけでは解決しません。開発中はプロセスを終了して `npm run dev` で起動し直してください。


GitHub Actionsは手動実行で各OSのインストーラーを生成し、`v*`タグでドラフトリリースへ添付します。
署名・公証の設定は含みません。秘密情報を含む他アプリの配布スクリプトはコピーしていません。
