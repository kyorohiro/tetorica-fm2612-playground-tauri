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
実行ファイルと作業ディレクトリーの両方も、ドライブ／UNCの通常パス形式へ変換します。
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
| `shell_execute` | 仮想shellを実行し、projectId／code／stdout／stderrを返す |

更新時は `read_file` の `projectId` と `content` を、それぞれ `projectId` と `expectedContent` に渡します。
人の編集等で内容が変わっていれば更新を拒否します。新規作成は `list_files` の `projectId` と `expectedContent: null` を指定します。
Cassetteの読み込み等でプロジェクトを入れ替えた場合も、古いIDによる更新を拒否します。
対象はメモリー上の仮想ファイルです。**保存は既存のExport Cassetteで行ってください。**
`shell_execute` はOSのshellではなく、Playground内の仮想shellです。
`list_files` の `projectId` と `command`（文字列または引数配列）を渡します。
`stdin`（最大1 MiB）と `timeoutMs`（100〜30000 ms、既定10000 ms）も指定できます。
commandは最大16 KiB、引数配列は最大128要素です。
`help --json` でコマンド一覧を確認でき、find／rg／read／patch／check／js／result／snapshot／undo、パイプとリダイレクトも利用できます。
`js` はWorkerのJavaScriptを実行し、`play` は演奏を開始できます。
`/sys` の書込みと `/index.js` の削除は引き続き禁止します。
OS上のファイルにはアクセスしません。

例（MCPのtool引数）:

```json
{"projectId":"list_filesのID","command":["read","--json","/index.js"]}
```

`read --json` のrevisionを、editの`--revision`やpatchの`expectedRevision`に指定して更新します。
変更前に `snapshot before`、対象作業直後に `snapshot after` を保存し、
`undo --dry-run --json before after` → `undo --json before after` で競合を確認して取り消せます。
無関係な編集もsnapshot間に入っている場合は、undoの末尾に対象パスを指定してください。
MCPのOff／世代変更／プロジェクト切替／タイムアウトでその要求を中断します。
待ち時間もタイムアウトに含み、期限切れの待機コマンドは後から実行しません。
中断は完了済みの変更を巻き戻さないため、変更コマンドを自動再試行しないでください。
stdoutは32000文字、stderrは8000文字まで返し、切詰めは`truncated`に記録します。
取消履歴はメモリ内のみで、プロジェクト切替や再起動時に破棄します。

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

## 内蔵 Git（開発中）

今の編集内容から始める場合は、Shell で `git start`、または Menu → **Git…** →
**Save current project and start Git…** を選びます。native のフォルダ選択で空フォルダを
作成・選択すると、現在のファイルを保存して Git を初期化します。エディタの内容は保持します。
続けて `git add index.js`、`git commit -m "First commit"` と進めます。
空でないフォルダへの新規保存は拒否します。

Menu → **Git…** から実フォルダを開きます。Virtual Files と Monaco に取り込み、
**Save files** で実フォルダへ保存します。外部で変更されたファイルは無条件に上書きせず、競合として停止します。
`.git`、`.ssh`、`/sys`、node_modules、target は編集対象に取り込みません。
保存からの削除は、このセッションで取り込んだ管理対象ファイルだけに限定します。
フォルダの関連付けはセッション内で保持し、New/Import Cassette や再起動後は開き直します。

初めてのフォルダでは **Initialize Git**、保存後は **Changes** でファイルを選び、
メッセージと Author/Email を指定して **Stage selected and commit** を押します。
Author/Email が両方空の場合は Git 設定の署名情報を利用します。
コミットには既にステージ済みのファイルも含みます。`git diff --cached` で確認できます。
Changes／Diff と Shell の `git status`／`git diff` は、現在の編集内容をフォルダへ保存してから確認します。
Status は untracked／unstaged／staged を表示します。`git diff` は未ステージの差分、
`git diff --cached` はステージ済みの差分です。History は直近50コミットを表示します。

Shell でも `git init`, `git status`, `git diff [--cached]`, `git add PATH...`,
`git add .`, `git commit -m MESSAGE`, `git log`, `git fetch`, `git pull`, `git push` が利用できます。
これらは native API の別名であり、システムの Git/SSH CLI は呼びません。

SSH は **Generate SSH key…** でアプリ専用の Ed25519 鍵を作り、**Show public key** の公開鍵を
Git ホストへ登録します。既存の秘密鍵やシステムの ssh-agent は利用しません。
秘密鍵はパスフレーズ必須で暗号化し、アプリのデータ領域に owner-only で保存します（Unix）。
パスフレーズは macOS の native secure text field で入力し、WebView・IPC に渡しません。
現在、Windows/Linux の secure input は未実装のため、それらの環境では SSH 操作を拒否します。
公開鍵は一つだけ管理し、既存の鍵は上書きしません。

SSH repository URL を入力して **Set origin…** で接続先を設定し、Fetch/Pull/Push を利用します。
Clone は選択した空フォルダへ行います。初回のホスト鍵は native ダイアログで確認し、
登録済みの鍵が変わった場合は接続を拒否します。Push は現在のブランチへの non-force push のみです。
Pull は保存・コミット済みの clean な作業ツリーからの fast-forward のみを扱い、分岐した履歴の
自動 merge/rebase、ブランチ切り替え、LFS、submodule、コミット署名は未対応です。
シンボリックリンク・hard link、linked worktree、外部 workdir、非UTF-8のパスは現在取り扱いません。

**Request cancel** は通信の中断を要求します。チェック地点で中断するため即時停止ではなく、
既にサーバーが受理した push を取り消す機能ではありません。
1ファイル64MiB・プロジェクト合計256MiB・10000ファイル/ディレクトリまで取り込みます。
保存はファイル単位の一時ファイルと rename を使い、失敗時は可能な範囲でロールバックします。
複数ファイル全体のクラッシュ時原子性は保証しません。

Git は `git2`/内蔵 `libgit2`、SSH 通信は `libssh2`、鍵生成と暗号化は Rust の `ssh-key` を使用します。
検証は native の同期・Git・鍵テスト、テスト用 SSH サーバーによる生成鍵の認証、
配布済み Web 本体に追加した Git UI の保存・ステージ・コミット・Shell 連携を対象としています。
実際の Git ホストと各OSでの動作は、配布前に別途確認してください。

Git の追加検証:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run test:git:ssh
# dist/ を 8087 で配信し、Playwright を用意してから:
npm run test:git:browser
```

SSH テストは一時サーバーと一時的な生成鍵だけを使い、ユーザーの鍵・SSH agent・Git ホストには接続しません。
Git UI のブラウザ検証は `GIT_TEST_URL` で配信先を変更できます。
内蔵 Git の依存ライセンスは bundle 内の `git-licenses.txt` に同梱します。
