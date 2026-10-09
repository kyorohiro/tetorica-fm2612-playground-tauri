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

メニュー先頭の New Cassette / Import Cassette / Export Cassette でプロジェクトを操作できます。
デスクトップ版はコード・素材ファイル・編集中のファイルと実行ファイルを WebView の IndexedDB に自動保存し、次回起動時に復元します。保存先はチップ／エンジンごとに分かれます。
メニューに保存状態を表示し、ウィンドウを閉じるときは最後の保存完了を待ちます。New Cassette は確認後に現在の下書きを置き換えるため、残したい曲は先に Export Cassette してください。
ブラウザー版にはこの自動保存を適用しません。
`release.lock.json` は取り込み元の記録と展開内容の整合性検査に使い、dev / build 前と Actions で `python3 scripts/import_release.py --check` を実行します。
`release.source.json` と `fetch_release.py` は廃止しました。

上流の本体を更新する場合:

```sh
npm run import:release -- ./xxx.zip
```

ZIP の置き場所・ファイル名は自由です。version 指定は不要で、ZIP と展開ファイルの SHA-256 を記録します。
取り込み後に動作確認し、`dist/` と `release.lock.json` を同じコミットに含めてください。リリースタグもそのコミットに付けます。
macOS の付随ファイル（`.DS_Store` と `__MACOSX/`）は取り込み・検証対象から除外します。アプリ本体のバイト列と取り込み元ZIPのハッシュは保持します。
`.gitattributes` で `dist/` の改行変換を無効にし、Windowsでも同じバイト列を保持します。
Tauri アプリのバージョンは、この ZIP の取り込みでは変更しません。

## 構成

- `dist/`: 上流ZIPそのまま（Git管理対象）
- `desktop/desktop-interface.js`: 既存MenuへTauri専用の最前面固定項目を追加
- `src-tauri/src/main.rs`: ローカルのメインウィンドウに限定した最前面固定コマンド
- `assets/app-icon.png`: 上流Playgroundアイコン。`npm run icons` でデスクトップ用を生成

## 音声出力

起動時は **WebView (default)** です。Menu → **Audio output** で **Audify** を
選び、Audio output device から機器を選択して **Apply output** を押します。
接続表示を確認して Run で演奏を再開してください。出力の変更・機器一覧の更新は
演奏を停止します。WebView を選ぶと通常の出力へ戻ります。

物理的なスピーカー・USBオーディオ機器に加え、インストール済みの BlackHole などの
仮想オーディオ機器も選べます。別アプリ側で同じ機器を入力元に指定すると、
DAWでの録音やエフェクト処理へ送れます。仮想機器自体のインストールは行いません。
デバイス接続が失敗・切断した場合は演奏を止めて WebView モードへ戻し、原因を表示します。
出力設定は保存せず、次回起動も WebView です。

音源・ミキサー・エフェクトは従来の Web Audio で処理し、最終出力を追加の
AudioWorklet からローカルの Node/Audify プロセスへ渡します。Main／Worker 実行と
STOP のフェードは同じ処理を使います。音声の待ち行列は512フレームの最大4ブロック
（機器が異なるサイズを返す場合はそのサイズ）に制限し、停止後の古いデータを蓄積しません。
機器の推奨サンプルレートを使用し、Web Audio と異なる場合は線形補間で変換します。
追加の転送・バッファによる遅延があるため、本番ライブでの遅延・音切れの評価は別途必要です。
macOS/arm64 で検証済み。Windows/Linux の実機確認は未実施です。
Windowsでは機器列挙と出力の両方でWASAPIを指定し、ASIOドライバーの自動探索を行いません。
Windowsの同梱準備では、Audify 1.10.1のRtAudioにCOMオブジェクトを終了前に解放する
修正を適用し、CMake／Visual Studio C++でネイティブモジュールを再ビルドします。
既成のWindowsバイナリーへ差し替えることはできません。
音声プロセスの応答が途切れた場合は、終了コードとNode／ネイティブ側のエラーを
Consoleへ表示します。メニューにも選択・コピーできるエラー全文を表示します。
失敗してもAudifyの選択とエラーを残すので、メニューを開き直して「Copy error」で
コピーできます。接続失敗時の実際の音声経路はWebViewへ戻し、エラーは「Clear error」まで保持します。

Node実行ファイル、Audifyのネイティブモジュール、必要な実行時依存とライセンスを
ビルド時に `audio-sidecar-bundle/` へ用意し、アプリの resources に同梱します。
利用者のPCにNodeをインストールする必要はありません。ビルドするOS・CPUに合う
NodeとAudifyを使ってください。システムの外部共有ライブラリーに依存するNodeではなく、
単体で起動できる公式Node配布を使います。必要なら `TETORICA_AUDIO_NODE` で実行ファイルを指定できます。

- `audio-sidecar/server.mjs`: デバイス列挙・ストリーム制御・上限付き音声受け渡し
- `audio-sidecar/output_audify.mjs`: 上流 `node/output_audify.mjs` のスナップショット（BSD-3-Clause、`dist/LICENSE`）
- `desktop/audio-interface.js` / `audio-worklet.js`: メニューと出力接続。固定した上流の配信応答へ追加
- `src-tauri/src/audio.rs`: ローカルのメインウィンドウだけに公開する制御コマンド。終了時にプロセスを停止

`npm test` はPCMの上限制御とサンプルレート変換も検証します。
`cargo test` を直接実行する場合は、先に `node scripts/prepare_audio_sidecar.mjs` を
実行してください。Cargo は Tauri CLI の beforeBuildCommand を呼びません。
CIでも、音声リソースの準備と `node scripts/check_audio_sidecar_bundle.mjs` による
ネイティブモジュールの読み込み確認を Cargo テストより前に実行します。
WindowsではWASAPIの列挙・GCによる解放を3回繰り返し、子Nodeの正常終了まで確認します。
別の一時ディレクトリーへ同梱物をコピーし、実際の音声サーバーの起動・応答・終了も検証します。
Windowsではインストール先に使われる`\\?\`形式のパスも検証します。
Nodeへ渡す起動スクリプトは、同梱フォルダーを作業ディレクトリーとする相対パスです
（[Nodeの拡張パス解決の不具合](https://github.com/nodejs/node/issues/62446)を回避）。
macOSの署名なしCI／開発ビルドは、NodeとAudifyをアドホック署名にそろえ、
前回のDeveloper ID署名との混在によるネイティブモジュールの読み込み失敗を防ぎます。
実機の低音量テストは、同梱準備後に `node scripts/check_audify_browser.cjs` で実行できます。
この検証には親の開発リポジトリーの `docs/` と Playwright が必要です。
配布ZIP本体と `release.lock.json` は、このTauri専用機能では変更しません。

### macOS の署名・公証

`sh deploy_mac.sh` は Intel と Apple Silicon を順にビルドします。
各ターゲット用の公式 Node v25.2.1 と Audify 1.10.1 の事前ビルドを取得し、
固定した SHA256 を確認して `audio-mac-cache/` に保存します。次回はキャッシュを利用します。
ホストCPUの Audify を他CPU版へ混ぜず、同梱する全Mach-OのCPUを検査します。

`APPLE_SIGNING_IDENTITY` が設定された場合、同梱 `.node`、全 `.dylib`、Node を
Developer ID・secure timestamp・Hardened Runtime 付きで署名・検証し、その後Tauriが
外側のアプリを署名・公証します。NodeだけにJIT用のentitlementを付けます。
`get-task-allow` や library validation の無効化は付けません。
Appleの公証用パスワードなどは従来どおり実行環境で設定してください。

Apple Silicon だけを再試行する場合:

```sh
sh deploy_mac.sh aarch64-apple-darwin
```

取得元: https://nodejs.org/dist/v25.2.1/SHASUMS256.txt と
https://github.com/almoghamdani/audify/releases/tag/v1.10.1 の配布資材です。

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
