# Native SDKの取り込み — OPN系／YM2151／OKIM6258

Tauriは、固定した`tetorica-soundchip-native`のソース配布をリポジトリ内へ同梱して
YM2612、YM2203、YM2608、YM2610／YM2610B、YM2151、OKIM6258をビルドします。通常のcheckoutに必要な音源ソースが含まれ、SDKの別checkoutや
submodule、ビルド中のGitHub取得を必要としません。
この手順書は最初のYM2612移行時のファイル名を保持し、その後の音源の移行も記録しています。

## 固定した配布物

| 項目 | 値 |
| --- | --- |
| SDK版 | `0.1.0` |
| ソース配布 | `tetorica-soundchip-native-0.1.0-sources.zip` |
| 配布ZIPのSHA-256 | `26db7518a55465b1e4dc58c19eb425fc6ea1ab282bd85920b0249556f5d1f6ce` |
| SDKの元コミット | `affd1059599f3c7598191c14f5a864b44963d4aa` |
| 同梱先 | `src-tauri/vendor/tetorica-soundchip-native/` |
| Tauri側の取り込み記録 | `src-tauri/soundchip-sdk.json` |

取り込み記録は`schemaVersion: 1`で、`name`、`version`、`sourceCommit`、`archiveName`、
`archiveSha256`、`manifestSha256`、`fileCount`を保存します。現在の同梱数は374ファイルです。
`sourceCommit`は取り込み時に指定する由来の記録で、コミット署名や配布物の署名を検証したものではありません。
取り込み時に期待するZIPのSHA-256を検証し、SDKマニフェストと各ファイルのサイズ／SHA-256を照合します。
`--check`は元ZIPやネットワークを使わず、同梱マニフェストのハッシュと全ファイルを記録に照合します。

配布物のソース全体、ライセンス、NOTICE、カタログ、原典とハッシュの記録を保持します。
SDK内のBSD、MIT、Apache、LGPL等の宣言はコンポーネントごとの原文のまま残し、
アプリ側のライセンスで置き換えません。SDK内の`LICENSES.md`と各カタログを参照してください。
同梱している全コンポーネントをアプリでリンクする、という意味ではありません。

## ビルドで使う部分

`src-tauri/build.rs`は`cc`で、次のSDKソースを既存のアプリ側ライブラリーへ組み込みます。
この段階ではSDKの全CMakeターゲットへ一括して切り替えません。

| 用途 | SDK内のパス |
| --- | --- |
| Genesis主音源のYM2612接続 | `wasm/ym2612_wasm.cpp` |
| `createSoundChip('ym2612')`の独立Native接続 | `native/reference/tauri/native_genesis_ym2612.cpp` |
| PC-98（YM2203）と`createSoundChip('ym2203')`のNative接続 | `native/reference/tauri/native_genesis_ym2203.cpp` |
| PC-98（YM2608）と`createSoundChip('ym2608')`のNative接続 | `native/reference/tauri/native_genesis_ym2608.cpp` |
| Neo GeoのYM2610／YM2610Bと、それぞれの追加音源のNative接続 | `native/reference/tauri/native_genesis_ym2610b.cpp` |
| `createSoundChip('ym2151')`の独立Native接続 | `native/reference/tauri/native_genesis_ym2151.cpp` |
| `createSoundChip('okim6258')`の独立Native接続 | `native/reference/tauri/native_genesis_extra_okim6258.cpp` |
| X68000と追加OKIに共通のdecoderの生成入力 | `vendor/mame-okim6258/okim6258.cpp`と対応ヘッダー |
| 共有OPN provider | `native/src/ymfm_opn_provider.cpp` |
| X68000のFMと追加YM2151に共通のOPM provider | `native/src/ymfm_opm_provider.cpp` |
| 共有SSG／ADPCM実装 | `vendor/ymfm/ymfm_ssg.cpp`、`vendor/ymfm/ymfm_adpcm.cpp` |

`wasm/ym2612_wasm.cpp`は名前に`wasm`を含みますが、ここではC++としてNativeにコンパイルします。
SDKのOPN providerがymfmのOPN実装と明示的なテンプレート実体化を一度だけ提供します。
従来のOPN providerと重複してリンクしません。

YM2203とYM2608は、それぞれ選択中のPC-98音源と追加生成の両方で、同じSDK側のNative接続を使います。
Neo Geoも選択中／追加音源のYM2610（FM 4CH）とYM2610B（FM 6CH）が一つのSDK接続を共有し、生成時に変種を選択します。
既に同梱済みのSDK `0.1.0`へビルド参照を切り替えるため、SDK配布物や固定値は更新しません。
共有OPN／SSG／ADPCMはYM2612移行時からSDK側のものを一度だけ組み込んでおり、
個別の移行用にもう一つ追加しません。既存のarchive順序を維持します。

YM2151の追加生成用接続と、OPMの明示的なテンプレート実体化を提供するproviderもSDKからビルドします。
X68000の選択中FMはTauriの`src-tauri/src/native_genesis_x68000.cpp`内に残り、同じSDKのOPM providerを使います。
SDKのincludeを従来スナップショットより先に指定し、OPM providerは一度だけコンパイルします。
既存のarchive順序と、先行移行でSDKへ切り替えた共有SSG／ADPCMも維持します。
共通ADPCMを参照するOPL等の接続全体までSDKへ移したという意味ではありません。
従来providerとSDK側ソースの同一性を確認し、共有ライブラリーの依存順序を保持します。

Genesisのgain、DC除去、PSG混合と出力レート変換は`src-tauri/src/native_genesis_core.cpp`に残ります。
Rustの予約、owner、音源ラックと予算管理、CPAL、ミキサー、FX／liveFx、録音、WebViewとJS APIも
アプリ側の担当です。YM2203のFM／SSG制御、Stop、レート変換も既存の接続動作を引き継ぎ、
Rust側の予約・owner・追加生成の管理やJS APIを変更しません。

YM2608のSDK接続は、各インスタンスに8 KiBのリズムROM領域と2 MiBのADPCM-B RAMを確保します。
リズムROMの素材はTauri側の`src-tauri/native-ym2608/snapshot/web/tetorica_ym2608_adpcm_rom.bin`を
Rustの選択中音源／追加音源が引き続き読み込み、Native接続へアップロードします。素材の参照先はSDKへ移しません。
サンプルの準備・転送予約・メモリー予算と寿命管理はホスト側に残り、既存のStop／resetとレート変換を引き継ぎます。

YM2610／YM2610Bの選択中音源は、SDK接続がADPCM-A／B各16 MiBの連続したROMバンクを所有します。
追加音源は各16 MiBの論理アドレス空間へ64 KiBの不変ページを割り当てる疎な構成です。
Rust側で変更ページを複製・準備し、予約命令が音声callbackで適用される再生フレームに、ページのポインターを公開します。
SDK接続はそのページを借用します。
ページの所有権・寿命・予算・回収、転送の予約とサンプル素材の準備は引き続きTauri側の担当です。
既存のROM素材やその読込み元をSDKへ移す変更ではありません。

OKIM6258の追加生成用接続とヘッダーもSDKからビルドし、X68000と追加音源は同じdecoderを共有します。
ただし固定SDKのraw decoderには、Tauriが従来から持つスレッド安全な初期化がありません。
`src-tauri/build.rs`がSDKソースから`OUT_DIR/tetorica_sdk_okim6258.cpp`へコンパイル用decoderを生成し、
constructorのテーブル初期化だけを従来と同じ一度限りの処理へ置き換えます。
置換の目印が一つだけであることを検査し、一致しなければビルドを失敗させます。
ADPCMの計算を変えず、固定SDKのバイト列・版も変更しません。新しいPython／CMakeのビルド依存は追加しません。
生成したdecoderは既存のarchive内で一度だけコンパイルし、SDKのヘッダーを優先して参照します。

X68000の機種用接続、Rust側のサンプル所有・予約・予算管理、音源別の混合はTauri側に残ります。
選択中音源の2つの16 MiBサンプルバンクも機種用接続に残ります。
追加OKI接続はサンプルをコピーせず借用し、Rustがその参照の所有・寿命を管理します。
OKIの素材やサンプル転送の管理をSDKへ移す変更ではありません。

Sega PSGと、上記以外の音源の接続とソースは従来の同梱スナップショットを使います。
ブラウザー／npmのWASM配布やPlaygroundの`dist/`は、このSDK取り込みで更新しません。

従来の`src-tauri/src/native_genesis_ym2203.cpp`／`.h`と`src-tauri/native-ym2203/`は、
由来やBrowser比較用の固定記録として保持します。実際のコンパイル対象は`src-tauri/build.rs`で確認してください。
従来コピーの編集や`sync_native_ym2203.py`の実行だけでは、SDK側のNative接続は更新されません。
YM2608も従来の`src-tauri/src/native_genesis_ym2608.cpp`／`.h`と`src-tauri/native-ym2608/`を保持します。
Browser比較用のコピーや`sync_native_ym2608.py`の更新と、実際にコンパイルするSDK接続の更新は別です。
YM2610／YM2610Bも従来の`src-tauri/src/native_genesis_ym2610b.cpp`／`.h`と
`src-tauri/native-ym2610b/`を保持します。これらのコピーや`sync_native_ym2610b.py`の更新だけでは、
SDK側の接続は更新されません。
YM2151も従来の`src-tauri/src/native_genesis_ym2151.cpp`／`.h`とローカルOPM provider、
`src-tauri/native-x68000/`のBrowserスナップショットを由来の記録として保持します。
従来のOKI decoderと追加接続も由来の記録として保持します。
実際のビルドはSDK側の接続・decoder入力と`build.rs`の生成処理を使うため、
従来コピーやスナップショットの更新だけでは、生成されるdecoderは変わりません。
CargoはSDKの`vendor/mame-okim6258/`と追加接続の入力を監視します。
機種用のローカルcpp／headerの監視は残し、実際のコンパイル入力がなくなった`native-x68000/`の監視は外します。

## 照合と更新

通常の検証はTauriのルートで実行します。別のSDKリポジトリや配布サーバーへ接続しません。

```sh
python3 scripts/import_soundchip_sdk.py --check
```

SDKを更新するときだけ、検証したソースZIPを明示して取り込みます。
次の値は現在同梱している配布物の例です。新しい配布ではその配布の期待ハッシュと元コミットを指定してください。

```sh
python3 scripts/import_soundchip_sdk.py \
  --archive /path/to/tetorica-soundchip-native-0.1.0-sources.zip \
  --sha256 26db7518a55465b1e4dc58c19eb425fc6ea1ab282bd85920b0249556f5d1f6ce \
  --source-commit affd1059599f3c7598191c14f5a864b44963d4aa
python3 scripts/import_soundchip_sdk.py --check
```

インポーターは配布ZIPと同梱ファイルの検証を行い、記録を同梱先の外に保存します。
アプリの対応ソースZIPにも、同梱SDKの全ソースとこの記録を収録します。
取り込み先のソースを通常の開発先として直接編集しません。SDK由来の変更を行う場合は、
アプリ側の開発元で確認した変更をSDKのソース配布へ反映し、配布物を作り直してから取り込みます。
これにより利用側は以前の版を保持でき、各リポジトリを同時に更新する必要がありません。
利用側だけに必要な差分は、固定した原本へ混ぜず、アプリ側の接続または別パッチとして由来を記録します。

更新後は同梱ソースと記録を同じコミットに含め、少なくとも次を確認します。

```sh
python3 scripts/prepare_native_sources.py
python3 scripts/check_native_sources.py --bundle --out=native-validation/sources.json
cargo test --locked --manifest-path src-tauri/Cargo.toml
node scripts/check_native_stability.mjs --out=native-validation/stability.json
node scripts/check_native_extra_stability.mjs --out=native-validation/extra-stability.json
```

親のBrowser／WASM checkoutがある場合は、実WASMとのPCM比較も実行できます。
親checkoutは比較用であり、通常のTauriビルドの必須依存ではありません。
次はTauriリポジトリのルートで実行する主Genesisの比較です。

```sh
node scripts/check_native_genesis_core.mjs /path/to/hello_ymfm --seconds=60 --out=native-validation/web-native.json
```

各音源の選択中音源／追加音源の比較は、Browser／WASM側の`hello_ymfm`ルートで実行します。
既定のビルドはTauriへ同梱したSDKの接続実装と共有providerを使用し、旧コピーをコンパイルしません。
X68000の比較は、機種用接続をTauri側に残し、SDKのOPM providerとSDK入力から生成したOKI decoderを組み込んだ構成を使います。
OKIを使う比較は、Browser側の`scripts/native_okim6258_source.mjs`で同じ生成処理を行い、
SDK入力と生成されたdecoderのパス／ハッシュを記録します。

```sh
cd /path/to/hello_ymfm
node scripts/check_native_ym2203.mjs
node scripts/check_native_extra_ym2203.mjs
node scripts/check_native_ym2608.mjs
node scripts/check_native_extra_ym2608.mjs
node scripts/check_native_ym2610b.mjs
node scripts/check_native_extra_ym2610.mjs
node scripts/check_native_x68000.mjs
node scripts/check_native_extra_ym2151.mjs
node scripts/check_native_extra_okim6258.mjs
```

一致試験はデバイスを開かない生成結果の検証です。各OSのインストーラーや実機での音声出力は
別に確認します。配布アプリの対応ソースと再ビルド手順は[NATIVE-SOURCES.md](../NATIVE-SOURCES.md)を参照してください。

## YM2612移行時の検証結果

macOSで次を確認しました。

- 同梱SDKの374ファイルを`--check`でオフライン照合。
- 旧ソースとSDKソースの14組が一致。主YM2612の旧ビルド／SDKビルドは24ケース・240,000フレームでbit一致。
- 追加YM2612と実WASMは198ケース・1,216,512フレームでPCM、status、IRQ、4種類のread結果が一致。
- 主GenesisのWorklet比較は6項目・49,152フレームで最大差`3.725290298e-9`。旧ビルドの比較結果から変化なし。
- Pythonの22テスト、Nodeの31テスト、`cargo test --locked --offline`の723テストが成功。Cargoの4テストは明示実行用としてignored。
- 22 modeの生成・破棄、604,032フレーム、88回のRun／Stop遷移に成功。代表7音源の3つの混合構成と追加生成の受入れ／台数制限の試験も成功。
- 対応ソースZIPの1,796ファイルを照合。同梱SDK全374ファイル、取り込み記録、手順書とツールを含み、22件のNativeソース固定記録もオフライン照合。
- 対応ソースZIPを独立した一時ディレクトリへ展開し、SDK照合とYM2612 fixtureのC++14ビルド／自己試験に成功。元のSDKやアプリのcheckoutへ依存せず、音声デバイスやネットワークは使用しない。
- SDK更新時のCargo再ビルド監視を含め、`cargo check --locked --offline`も成功。

この節の数値はYM2612移行時のmacOSでの結果です。後続の移行や各OSのビルド、実機での音声出力とは
確認範囲を分けて扱います。

## YM2203移行の検証

今回のYM2203参照切替は、macOSで次を確認しました。

- 接続実装・ヘッダーと推移依存を含む9入力が旧コピーと同一。SDK全374ファイルのオフライン照合も成功し、固定版は変更なし。
- 実際に旧接続とSDK接続を別々にコンパイルし、同じ命令列を実WASMとも比較。選択中音源は186ケース・1,142,784フレーム、追加音源は195ケース・1,198,080フレームで、旧接続とSDK接続はbyte一致、WASMとの最大PCM差分も0。
- 合計は重複を除いて381ケース・2,340,864フレーム。既定checkerでも実SDKからビルドして再比較し、使用した5ソースのハッシュを記録した。この再実行分は合計に加えていない。
- 別の全状態比較6ケース・49,152フレームで、280バイトのsnapshot全体、PCM、status、IRQ、4種類のread結果が一致。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 22 modeの604,032フレーム、88回のRun／Stop遷移が成功。3混合構成、7単独音源、追加生成の受入れと台数制限の試験も成功。

このローカル検証を終えた時点では、YM2203移行を含むWindows／LinuxのCIは未実行でした。
後述の全OPN移行CIで5環境のビルドと自動試験を確認しました。実機の音声出力は別の確認です。

## YM2608移行の検証

今回のYM2608参照切替は、macOS ARM64／Clangで次を確認しました。音声デバイスは開いていません。

- 接続実装・ヘッダーと推移依存を含む9入力が旧コピーと同一。SDK全374ファイルのオフライン照合も成功し、固定版は変更なし。
- 旧接続とSDK接続を別々にコンパイルし、同じ命令列を実WASMとも比較。選択中音源は30ケース・184,320フレームで左右PCMが一致。追加音源は252ケース・1,585,152フレームで左右PCM、status、statusHi、IRQ、4種類のread結果が一致。どちらも旧接続とSDK接続はbyte一致、WASMとの差分は0。
- 合計は重複を除いて282ケース・1,769,472フレーム。既定の選択中／追加音源checkerも実SDKからビルドして成功し、使用した5つのC++ソースのハッシュを記録した。この再実行分は合計に加えていない。
- 別のNative間比較6ケース・49,152フレームで、524／536バイトの両state形式全体、PCMとread結果、各観測でRAMの3箇所・合計96バイトが旧接続とSDK接続で一致。RAM全領域のdumpや、WASMとの全state比較ではない。
- 拡張fixtureでROM／RAMの所有・境界・保持、両profileのreset、Stop／再発音、他ボイスの継続、callback中の割当て／解放0を確認。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 22 modeの604,032フレーム、88回のRun／Stop遷移が成功。3混合構成、7単独音源、追加生成の受入れと台数制限の試験も成功。

このローカル検証を終えた時点では、YM2608移行を含むWindows／LinuxのCIは未実行でした。
後述の全OPN移行CIで5環境のビルドと自動試験を確認しました。実機の音声出力は別の確認です。

## YM2610／YM2610B移行の検証

今回のYM2610／YM2610B参照切替は、macOS ARM64／Clangで次を確認しました。音声デバイスは開いていません。

- SDKの接続実装は説明コメント1行だけが旧コピーと異なり、8つのヘッダー／interface入力はbyte一致。SDK全374ファイルのオフライン照合も成功し、固定版は変更なし。
- 旧接続とSDK接続を別々にコンパイルし、同じ命令列を実WASMとも比較。選択中音源は72ケース・442,368フレームで左右PCMが一致。疎なROM構成の追加音源は504ケース・3,096,576フレームで左右PCM、status、statusHi、IRQ、4種類のread結果が一致。どちらもFM 4CH／6CHの両変種を含み、旧接続とSDK接続はbyte一致、WASMとの差分は0。
- 合計は重複を除いて576ケース・3,538,944フレーム。既定の選択中／追加音源checkerも実SDKからビルドして成功し、使用した5つのC++ソースのハッシュを記録した。この再実行分は合計に加えていない。
- 別のNative間比較12ケース・98,304フレームで、524／536バイトの両state形式全体と、各観測で範囲を限定したROM 256バイトが旧接続とSDK接続で一致。保持したCOWページは音源破棄の前後でも内容が変化しなかった。32 MiB全体のROM dumpや、WASMとの全state比較ではない。
- 拡張fixtureでROMの所有・境界・保持、独立インスタンスと他ボイスの継続、取消し、Stop／reset／再発音、callback中の割当て／解放0を確認。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 22 modeの604,032フレーム、88回のRun／Stop遷移が成功。3混合構成、7単独音源、追加生成の受入れと台数制限の試験も成功。

このローカル検証を終えた時点では、YM2610／YM2610B移行を含むWindows／LinuxのCIは未実行でした。
その後の結果は次節に記録しています。実機の音声出力は別の確認です。

## 全OPN移行の5環境CI — 2026-10-25

[Build Desktop — run 40130334227](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40130334227)で、
YM2612／YM2203／YM2608／YM2610／YM2610Bの移行を含むコミット
`5280f83adebc542fdba1f49c5ff9bc8b51860734`をmainから手動実行しました。
run全体と全5ジョブが成功し、CIのためのコード修正は不要でした。

| 構成 | 自動試験・ソース照合 | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

5環境それぞれのNative検証JSONを取得して、次を確認しました。

- SDK374ファイルを含む22件の固定記録と、対応ソースZIP全1,796ファイルの照合成功。
- 22 mode、604,032フレーム、88回のRun／Stop遷移の成功。
- 3混合構成、7単独音源と、追加生成の受入れ・台数制限の成功。
- runnerのexit codeが0、`deviceOpened: false`。

検証レポート5構成分とインストーラー5構成分の計10アーティファクトが、有効期限内・サイズ0より大きいことも確認しました。
タグを作らない手動実行で、アプリ版は`0.90.1`のまま。新しいドラフトリリースや一般公開は行っていません。
先行する各移行節の「CI未実行」は、そのローカル作業を終えた時点の記録です。
このCIはOS間のPCM bit一致や各OSでのWASM比較、インストール後の起動、実機の音声出力・ノイズ・UI操作を確認したものではありません。

## YM2151移行の検証

今回のYM2151参照切替は、macOSで次を確認しました。音声デバイスは開いていません。

- 接続実装・機種用接続・コア・ヘッダーの11入力が旧コピーとbyte一致。OPM providerとの差は相対includeパスだけ。
- X68000のFM／OKI構成は42ケース・258,048フレーム、追加YM2151は252ケース・1,548,288フレームで比較。旧接続と旧OPM provider、SDK接続とSDK OPM providerをそれぞれ別ビルドし、出力のbyte一致を確認してから実WASMと比較した。最大差分は0。
- 合計は重複を除いて294ケース・1,806,336フレーム。既定checkerも実SDKからビルドして成功し、使用したソースのハッシュを記録した。この再実行分は合計に加えていない。
- 別のNative間比較6ケース・49,152フレームで、272／292バイトの両snapshot全体、PCM、status、IRQ、read結果がbit一致。選択中のFMとOKIの両出力が無音でないことも確認した。WASM内部state全体の一致を確認したものではない。
- 自己試験でcallback中の割当て／解放0を確認。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 再生ライフサイクルは22 mode、604,032フレーム、88回のRun／Stop遷移が成功。混合音源は3構成、7単独音源、追加生成の受入れと16台の制限の試験が成功。両runnerのexit codeは0、`deviceOpened: false`。

上の全OPN移行CIはコミット`5280f83adebc542fdba1f49c5ff9bc8b51860734`に対するもので、
今回のYM2151差分を検証した結果ではありません。新しい各OSのCIと実機の音声出力は未確認です。

## OKIM6258移行の検証

今回のOKIM6258参照切替と生成処理は、macOS／Clangで次を確認しました。音声デバイスは開いていません。

- SDKのraw decoderを変更せずに生成したコンパイル用decoderが、従来のTauri decoderとbyte一致。接続・ヘッダー・機種用接続の6入力もbyte一致。
- 旧Nativeと生成SDK接続を別々にコンパイルして比較。X68000の選択中音源は42入力・258,048フレーム、追加OKIは45入力・264,192フレームでbyte一致。固有の入力は合計87通り・522,240フレーム。
- 実WASMは選択中音源42件に加え、追加OKIの45入力を3種類のブロック分割で135回比較。合計177比較・延べ1,050,624フレームで最大差分0。177件を固有の入力数としては数えない。
- 既定の両checkerも同じ生成処理で成功し、SDK入力・生成decoder・実際にコンパイルしたソースのハッシュを記録。この再実行分は上の件数に加えていない。
- 別のNative間比較6ケース・49,152フレームで、選択中音源292バイト／追加OKI 28バイトのstate全体とPCMが一致。借用サンプルの内容は音源破棄後も変化しなかった。WASM内部state全体の比較ではない。
- 旧Nativeと生成decoderを別プロセスでそれぞれ16スレッド・513回生成し、49段のテーブル計算が一度だけ行われることを確認。
- 既存fixtureでcallback中の割当て／解放0を確認。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 再生ライフサイクルは22 mode、604,032フレーム、88回のRun／Stop遷移が成功。混合音源は3構成、7単独音源、追加生成の受入れが成功。16台の制限で17台目を拒否し、Stop後に予約した予算をすべて回収する試験も成功。両runnerのexit codeは0、`deviceOpened: false`。

先行する全OPN移行CIは、今回のOKIM6258差分を含みません。新しい各OSのCIと実機の音声出力は未確認です。
