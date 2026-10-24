# Native SDKの取り込み — YM2612

Tauriは、固定した`tetorica-soundchip-native`のソース配布をリポジトリ内へ同梱して
YM2612をビルドします。通常のcheckoutに必要な音源ソースが含まれ、SDKの別checkoutや
submodule、ビルド中のGitHub取得を必要としません。

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
| 共有OPN provider | `native/src/ymfm_opn_provider.cpp` |
| 共有SSG／ADPCM実装 | `vendor/ymfm/ymfm_ssg.cpp`、`vendor/ymfm/ymfm_adpcm.cpp` |

`wasm/ym2612_wasm.cpp`は名前に`wasm`を含みますが、ここではC++としてNativeにコンパイルします。
SDKのOPN providerがymfmのOPN実装と明示的なテンプレート実体化を一度だけ提供します。
従来のOPN providerと重複してリンクしません。

YM2203／YM2608／YM2610等の既存アプリ接続も同じproviderを参照するため、共有OPN／SSG／ADPCMは
今回の依存範囲に含まれます。共通ADPCM実装は既存のOPM／OPL側からも参照します。
それらの音源接続までSDKへ移したという意味ではありません。
従来providerとSDK側ソースの同一性を確認し、共有ライブラリーの依存順序を保持します。

Genesisのgain、DC除去、PSG混合と出力レート変換は`src-tauri/src/native_genesis_core.cpp`に残ります。
Rustの予約、owner、音源ラックと予算管理、CPAL、ミキサー、FX／liveFx、録音、WebViewとJS APIも
アプリ側の担当です。Sega PSG、他の音源の接続とソースは従来の同梱スナップショットを使います。
ブラウザー／npmのWASM配布やPlaygroundの`dist/`は、このSDK取り込みで更新しません。

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

```sh
node scripts/check_native_genesis_core.mjs /path/to/hello_ymfm --seconds=60 --out=native-validation/web-native.json
```

一致試験はデバイスを開かない生成結果の検証です。各OSのインストーラーや実機での音声出力は
別に確認します。配布アプリの対応ソースと再ビルド手順は[NATIVE-SOURCES.md](../NATIVE-SOURCES.md)を参照してください。

## 今回の検証結果

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

これらはmacOSでの結果です。今回の変更によるWindows／Linuxのビルドと実機での音声出力は未確認です。
