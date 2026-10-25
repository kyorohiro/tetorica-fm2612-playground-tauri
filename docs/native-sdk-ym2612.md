# Native SDKの取り込み — OPN系／YM2151／OKIM6258／YM2413／OPL系／AY／Sega PSG／RF5C164

Tauriは、固定した`tetorica-soundchip-native`のソース配布をリポジトリ内へ同梱して
YM2612、YM2203、YM2608、YM2610／YM2610B、YM2151、OKIM6258、YM2413、AY-3-8910、Sega PSG、RF5C164と
OPL系4種類（YM3526／YM3812／YMF262／Y8950）をビルドします。通常のcheckoutに必要な音源ソースが含まれ、SDKの別checkoutや
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
| `createSoundChip('ym2413')`の独立Native接続 | `native/reference/tauri/native_genesis_ym2413.cpp` |
| `createSoundChip('ay8910')`の独立Native接続 | `native/reference/tauri/native_genesis_ay8910.cpp` |
| MSXと追加AYに共通のAY-3-8910コア | `vendor/mame-ay8910/ay8910.cpp`と対応ヘッダー |
| Genesisが使うSega PSGの低レベルC ABI | `wasm/segapsg_wasm.cpp`と`vendor/ymfm/chip_state.h` |
| Genesis／Nuked、Master System、Game Gearと追加PSGに共通のコア | `vendor/ymfm/segapsg.cpp`と対応ヘッダー |
| Mega CD／32X系の選択中RF5C164接続 | `native/reference/tauri/native_genesis_rf5c164.cpp`と対応ヘッダー |
| RF5C164の独立追加接続 | `native/reference/tauri/native_genesis_extra_rf5c164.cpp`と対応ヘッダー |
| 選択中RF5C164とraw C ABIに共通のコア | `vendor/mame-rf5c164/rf5c164.cpp`と対応ヘッダー |
| RF5C164のraw C ABI | `wasm/rf5c164_wasm.cpp`と`vendor/ymfm/chip_state.h` |
| DOS PC／MSX Audioの選択中OPL系接続 | `native/reference/tauri/native_genesis_opl.cpp` |
| OPL系4種類の独立追加接続 | `native/reference/tauri/native_genesis_extra_opl.cpp` |
| 選択中／追加Y8950のメモリーinterface | `native/reference/tauri/native_genesis_y8950_memory.h` |
| X68000と追加OKIに共通のdecoderの生成入力 | `vendor/mame-okim6258/okim6258.cpp`と対応ヘッダー |
| 共有OPN provider | `native/src/ymfm_opn_provider.cpp` |
| X68000のFMと追加YM2151に共通のOPM provider | `native/src/ymfm_opm_provider.cpp` |
| YM2413と既存OPL／Y8950接続が共有するOPL provider | `native/src/ymfm_opl_provider.cpp` |
| 共有PCM実装 | `vendor/ymfm/ymfm_pcm.cpp` |
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
従来providerとSDK側ソースの同一性を確認し、共有ライブラリーの依存順序を保持します。

YM2413の独立接続と、共有OPL provider／PCM実装も同じ固定SDKからビルドします。
Master System／MSXの選択中音源は、Tauriの`src-tauri/src/native_genesis_opll.h`にある
`NativeOpllCore`と機種用接続を引き続き使います。独立SDK接続には別のrendererがあり、
この共通ヘッダーを置き換える変更ではありません。
SDKのincludeを優先し、OPL provider／PCMは既存のarchive内で一度だけコンパイルします。
機種用の補助処理はアプリ側に残ります。Sega PSGコアの現在の境界は下記に記載します。
先行移行済みのOPL／PCMとADPCM実装への依存も維持します。

OPL系4種類の選択中接続／追加接続と、対応するヘッダーもSDKからビルドします。
DOS PCはYM3526／YM3812／YMF262、MSX AudioはY8950を選び、追加音源も生成時に4種類から選択します。
SDKのcppは同じSDK内のC ABIヘッダーとY8950メモリーhelperを参照し、アプリ側の転送ヘッダーは作りません。
OPL／PCM／ADPCM providerは既にSDKへ移行済みで、今回重複して追加しません。archiveの順序も維持します。
同じarchiveにある独立RF5C164接続も現在はSDKを使います。PSGの機種用補助処理、Rustの予約・owner制御と混合はそのままです。

AY-3-8910の共通コア／ヘッダーと独立接続もSDKからビルドし、既存のMSX archive内でコアを一度だけ組み込みます。
MSXの選択中音源はローカルの`src-tauri/src/native_genesis_msx.cpp`を使い、AYのレート変換と
`NativeOpllCore`、Rustの予約・owner制御と混合を維持します。
独立SDK接続は別のtick／レート変換を持つため、選択中音源の処理をその接続へ置き換える変更ではありません。
AYの初期clock、tone／noise／envelope、明示的な音量0、Stop／resetの既存動作を引き継ぎます。

Sega PSGの共有コアと低レベルC ABIも、同じ固定SDKへ参照を切り替えます。
`wasm/segapsg_wasm.cpp`はNative C++として使い、SDK側の`chip_state.h`と`segapsg.h`を参照します。
コアcpp／headerと合わせた4入力は旧コピーとbyte一致し、SDKの版や同梱バイト列は変更しません。
既存の`tetorica_native_genesis` archive内で一度だけコンパイルし、Genesis／Nuked、
Master System、Game Gearと`createSoundChip('segapsg')`の追加音源が共有します。
既存のarchive順序、C++14・O2・MSVC `/EHsc`を維持します。
Game GearもSDKのincludeを従来コピーより優先し、同じコア型を使います。

Genesis／Nukedの機種用接続とgain・DC除去・PSG混合はローカルの`src-tauri/src/native_genesis_core.cpp`に残ります。
Master System／Game Gearと追加PSGは、ローカルの機種用接続と`src-tauri/src/native_genesis_psg.h`の
`NativePsgCore`を引き続き使います。PSGのレート変換、clock、Stop／reset、ステレオマスクと試奏の制御を
SDKの機種用接続へ置き換える変更ではありません。Rustの予約・owner管理とMixer／FXも引き継ぎます。

RF5C164の選択中／独立追加接続、コアとraw C ABIも同じ固定SDKからビルドします。
選択中接続は`RF5C164`コアを内包し、raw C ABIとともに既存の`tetorica_native_megacd` archiveへ組み込みます。
コアは一度だけコンパイルします。追加接続は既存の`tetorica_native_opl` archiveに入り、
独自のrendererと8声分の保持出力を持つ既存実装を使います。両方を一つのrendererへ統合する変更ではありません。
接続cpp／headerの2組、コアcpp／header、raw C ABIと`chip_state.h`の8入力は旧コピーとbyte一致します。
SDKの固定版とバイト列、既存のarchive順序・C++14・O2・MSVC `/EHsc`は維持します。

選択中音源と追加音源はそれぞれ64 KiBのRAMを所有し、アップロードされたデータをそのRAMへコピーします。
可変RAMページを借用する方式へ切り替えません。Rust側の不変upload payloadの所有、受入れ・転送予約、
寿命・回収・メモリー予算、既存のStop／resetやKeyboard試奏の制御と混合はホスト側に残ります。
サンプル素材の準備や録音・ファイル入力をSDKへ移す変更でもありません。

Y8950は512個の可変4 KiBページを借用し、最大2 MiBのADPCM-B RAMを扱います。
SDKメモリーhelperはポインター配列とinterfaceを保持し、RAMのバイト列自体は所有しません。
RustのArcによる所有、control側でのCOW準備、寿命・回収・予算管理を引き継ぎます。
公開済みページの読み書きは音声threadが行い、既存のmemory revision／upload revisionの規則を保持します。
この参照切替では新しいRAM転送APIを追加しません。

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

上記以外の音源の接続とソースは従来の同梱スナップショットを使います。
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
YM2413の旧接続／providerと`src-tauri/native-master-system/`も由来の記録として保持し、
SDKの接続・OPL／PCM入力を監視します。`native-master-system/`は原典だけになったため監視を外しますが、
機種用接続とローカル`NativeOpllCore`の監視は継続します。
AYの旧接続／コア／ヘッダーと`src-tauri/native-msx/`も原典・ライセンスの記録として保持します。
実際のSDK AYコアのディレクトリと独立接続cpp／headerを監視し、従来の`native-msx/`のソース監視は外します。
既存の同梱ライセンスの参照先は保持し、syncスクリプトだけではSDKの実行ソースを更新しません。
OPL系の旧cpp／header／メモリーhelperと`src-tauri/native-opl/`も由来の記録として保持します。
実際のSDK接続とヘッダー、Y8950メモリーhelperを監視し、原典だけになった`native-opl/`の監視は外します。
Sega PSGの旧コア／C ABIとGenesis／Master System／Game Gearの比較用スナップショットも保持します。
実際の入力は既存のSDK `vendor/ymfm/`と`wasm/`の監視対象です。ローカルの機種用接続と
`NativePsgCore`の監視を継続し、ほかの入力も残る`native-genesis/`の監視は外しません。
旧コピーや各syncスクリプトの更新だけでは、実際のSDK PSGコアは更新されません。
RF5C164の旧接続／コア／raw C ABIと`src-tauri/native-megacd/`も原典・比較用の記録として保持します。
CargoはSDKのRFコアディレクトリと2組の接続cpp／header、既存のSDK共通ヘッダー／wasm入力を監視します。
旧`native-megacd/`は実行ソースや埋め込み音声素材の参照がなくなったためソース監視を外しますが、ライセンスの同梱先は保持します。
旧コピーや`sync_native_megacd.py`の更新だけでは、実際にコンパイルするSDKは変わりません。

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
node scripts/check_native_master_system.mjs
node scripts/check_native_game_gear.mjs
node scripts/check_native_nuked.mjs
node scripts/check_native_megacd.mjs
node scripts/check_native_extra_rf5c164.mjs
node scripts/check_native_msx.mjs
node scripts/check_native_extra_ym2413.mjs
node scripts/check_native_extra_ay8910.mjs
node scripts/check_native_opl.mjs
node scripts/check_native_extra_opl.mjs
node scripts/check_native_msx_audio.mjs
node scripts/check_native_y8950_adpcm.mjs
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
今回のYM2151差分を含みません。このローカル検証時点では各OSの新しいCIは未実行でした。
後続CIの状況は末尾に記録します。実機の音声出力は別の確認です。

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

先行する全OPN移行CIは、今回のOKIM6258差分を含みません。このローカル検証時点では各OSの新しいCIは未実行でした。
後続CIの状況は末尾に記録します。実機の音声出力は別の確認です。

## YM2413移行の検証

今回のYM2413接続と共有OPL／PCMの参照切替は、macOS／Clangで次を確認しました。音声デバイスは開いていません。

15入力が旧コピーとbyte一致し、OPL providerとの差は相対includeパスだけでした。
このYM2413移行時点では、SDK優先で解決される`segapsg.h`も旧ヘッダーと一致し、PSGの実装cppはローカルのままでした。
旧接続とSDK接続を別々にコンパイルして出力のbyte一致を確認し、実WASMとも比較しました。

| 対象 | 比較数 | フレーム |
| --- | ---: | ---: |
| Master System | 60 | 368,640 |
| MSX | 93 | 571,392 |
| 追加YM2413 | 558 | 2,285,568 |
| DOS PC OPLの回帰確認 | 198 | 1,216,512 |
| 追加OPLの回帰確認 | 552 | 3,391,488 |
| MSX Audio／Y8950の回帰確認 | 138 | 847,872 |

YM2413関連は711件・3,225,600フレーム、共有providerの回帰確認は888件・5,455,872フレーム。
合計1,599件・8,681,472フレームで、旧接続とSDK接続はbyte一致、WASMとの最大差分は0でした。
6つの既定checkerも実際のSDKからコンパイルして成功し、使用したパスとソースのハッシュを記録しました。
この再実行分は合計に加えていません。

- 別のNative間比較9ケース・36,864フレームで、追加YM2413の72バイト、Master Systemの100バイト、MSXの104バイトの公開state全体と、PCM／read／status／IRQが一致。ymfmのprivate領域のdumpや、WASMとの全state比較ではない。
- 既存fixtureでcallback中の割当て／解放0を確認。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 再生ライフサイクルは22 mode、604,032フレーム、88回のRun／Stop遷移が成功。混合音源は3構成、7単独音源、追加生成の受入れが成功。16台の制限で17台目を拒否し、Stop後に予約した予算をすべて回収する試験も成功。両runnerのexit codeは0、`deviceOpened: false`。

このYM2413移行はAY／PSG接続自体のSDK移行や、Y8950の新しいADPCM素材転送APIを追加したという意味ではありません。
先行する全OPN移行CIは今回のYM2413差分を含みません。このローカル検証時点では各OSの新しいCIは未実行でした。
後続CIの状況は次節に記録します。実機の音声出力は別の確認です。

## YM2151／OKIM6258／YM2413の5環境CI — 全5構成成功

[Build Desktop — run 40137093552](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40137093552)を、
mainのコミット`634e56f4dcbacd3d204ea5dace454d39fe0fc9ca`から手動実行しました。
先行した全OPN移行に加え、YM2151、OKIM6258とその生成処理、YM2413と共有OPL／PCMの移行を含みます。
run全体が`completed`／`success`で、全5ジョブの自動試験とインストーラービルドが成功しました。
CIに通すためのコード修正は不要でした。

macOS ARM64／x86_64、Windows x86_64、Linux ARM64／x86_64の5構成すべてのNative検証JSONを取得し、内容を確認しました。

| 構成 | Native検証・ソース照合 | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

各環境で次を確認しました。

- SDK374ファイルを含む22件の固定記録と、対応ソースZIP全1,796ファイルの照合成功。
- 22 mode、604,032フレーム、88回のRun／Stop遷移の成功。
- 3混合構成、7単独音源、追加生成の受入れと16台の制限の試験成功。
- runnerのexit codeが0、`deviceOpened: false`。
- 取得した検証アーティファクトZIPのdigestがGitHubの記録と一致。

Native検証レポート5構成分とインストーラー5構成分の計10アーティファクトを確認しました。
すべて有効期限内で、サイズは0より大きいものです。インストーラーのダウンロード・インストール後の起動は未確認です。
アプリ版は`0.90.1`のまま。タグを作らない手動実行で、新しいドラフトリリースや一般公開は行っていません。
先の各移行節の「CI未実行」は、個別のローカル作業を終えた時点の記録です。
OS間のPCM bit一致、各OSでのWASM比較、インストール後の起動、実機の音声出力・ノイズ・UI操作は別の確認です。

## OPL系4種類の接続移行の検証

今回のYM3526／YM3812／YMF262／Y8950接続の参照切替は、macOS ARM64／Clangで次を確認しました。
音声デバイスは開いていません。採用する5ファイルのうち4つは旧コピーとbyte一致し、
追加生成用cppの差はextern-template宣言の直前の説明コメント1行だけです。

旧接続とSDK接続を別々にコンパイルし、共通provider／PCM／ADPCMはどちらも既に採用済みの固定SDKを使用しました。
同じ入力に対する出力をbyte比較してから、実WASMと比較しました。

| 対象 | 比較数 | フレーム |
| --- | ---: | ---: |
| DOS PC OPL／OPL2／OPL3 | 198 | 1,216,512 |
| 追加OPL系4種類 | 552 | 3,391,488 |
| 選択中Y8950 | 138 | 847,872 |
| RAM付きY8950（選択中／追加生成） | 72 | 589,824 |

計960ケース・6,045,696フレームで旧／SDKがbyte一致し、WASMとの差分は0でした。
更新した4つのcheckerの既定ビルドも実際のSDKからコンパイルして成功し、使用したcppのパスとハッシュを記録しました。
この再実行分は件数へ二重加算していません。RAM付き試験では2 MiB末尾、CPU書き込み、部分アップロード、
clear、保持したRAMでのStop／reset、FMとの同時再生、KeyboardのADPCM試奏と低clockのholdも比較しています。

- 別のNative間比較6ケース・24,576フレームで、選択中524バイト／追加生成520バイトの公開state全体、PCM／status／IRQ／read、RAMのrevisionと毎フレーム384バイトのRAM観測が一致。終了時には呼び出し側の2 MiBバンクと4 KiBの差し替えバッファ全体も一致し、コア破棄後まで保持されました。
- この状態比較は公開C ABIのsnapshotであり、ymfm非公開メモリーのdumpやWASMとの全内部状態比較ではありません。全2 MiBを毎フレームdumpする試験でもありません。
- 既存の選択中／追加生成OPLと強化Y8950メモリーfixtureで、ページ境界・寿命・revision・Stop／reset保持・FMを維持したADPCM取り消しと、callback中の割当て／解放0を確認しました。
- `cargo test --locked --offline`が723件成功、明示実行用4件ignored。
- 22 mode、604,032フレーム、88回のRun／Stop遷移が成功。3混合構成、7単独音源、受入れ／16台制限とStop後の予算回収も成功。両report runnerはexit code 0、`deviceOpened: false`。

先行するrun `40137093552`は共有OPL／PCM providerまでの移行を含みますが、今回の接続差分は含みません。
このローカル検証時点では新しい各OSのCIは未実行でした。後続CIの結果は末尾に記録します。
実機の音声出力・インストール後の起動は別の確認です。

## AY-3-8910移行の検証

macOS ARM64／Clangで音声デバイスを開かずに検証しました。採用する独立接続cpp／headerと
共通コアcpp／headerの4ファイルは旧コピーとbyte一致し、SDKの固定版も変更していません。

| 対象 | 比較数 | フレーム数 |
| --- | --- | --- |
| 選択中MSXのAY／OPLL出力 | 93 | 571,392 |
| 独立追加AYのPCM／公開状態 | 114 | 466,944 |

44.1／48／96 kHzで、全207比較・1,038,336フレームの旧／SDK fixture出力がbyte一致し、
続く実配布WASM比較も最大差0でした。全3音のtone／noise、16 envelope形状、period境界、
clockのhold／平均化、明示的な音量0、Stop／reset／再試奏と独立したOPLL制御を含みます。
既定ビルドのcheckerもSDK入力から成功し、実際のcppパスとSHA-256を記録しました。
その再実行分は上表に二重加算しません。

- Native間の別比較では6ケース・24,576フレームで、選択中104／独立24バイトの公開状態、PCM、毎フレーム16個のAY readがbyte一致しました。非公開のコア全メモリーやWASM内部状態のdumpではありません。
- 共通fixtureの追加YM2413も558ケース・2,285,568フレーム、実WASM最大差0。AY移行の旧／SDK比較数とは別の回帰確認です。
- 旧／SDKのfixture自己試験が成功し、callback操作中のallocation／freeは0でした。
- Rust全試験は723成功・0失敗・明示実行用4件ignored。
- 22 mode・604,032フレーム・88 Run／Stop、および3混合構成・7単独音源、受入れ／16台制限とStop後の予算回収が成功しました。MSX Audioを含む既存のRust機種構成を使い、両runnerのexit codeは0でした。

このローカル検証時点では、今回のOPL接続／AY移行を含む5構成CIは未実行でした。
後続CIの結果は次節に記録します。先行するrun `40137093552`の成功範囲と区別してください。
実機の音声出力は別の確認です。

## OPL接続／AY移行後の5環境CI — 4構成成功、macOS x86_64失敗

[Build Desktop — run 40142510765](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40142510765)を、
mainのコミット`276e744ee5ed00513558b1d995e950a3ef91884c`から`workflow_dispatch`で手動実行しました。
`4451195...`のOPL系4種類の接続移行と、AYコア／独立接続の移行を含みます。
アプリ版は`0.90.1`のままで、タグや新しいドラフトリリース・一般公開は作成していません。
attempt 1は`completed`／`failure`で終了し、5ジョブ中4成功・1失敗でした。再実行はしていません。

| 構成 | ソース照合 | Rust試験・再生／混合report | インストーラービルド |
| --- | --- | --- | --- |
| macOS ARM64 | 成功 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | Rust試験1件失敗、再生／混合reportなし | 未完了 |
| Windows x86_64 | 成功 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功 | 成功（DEB／AppImage） |

5構成の検証アーティファクトを取得し、各ZIPのdigestをGitHubの記録と照合しました。
全構成の`sources.json`でSDK374ファイルを含む22件の固定記録と、対応ソースZIP1,796ファイルの照合が成功しました。
macOS x86_64以外の4構成では次のreportを確認しました。

- 22 mode、604,032フレーム、88回のRun／Stop遷移が成功。
- 3混合構成、7単独音源、受入れと16台制限が成功。
- 両runnerはexit code 0、`deviceOpened: false`。

macOS x86_64はRust試験が722成功・1失敗・4件ignoredでexit code 101でした。
失敗は`native_genesis::extra_snes_tests::extra_snes_completion_wait_releases_control_and_history_expires`です。
CI対象の`src/native_genesis_extra_snes_tests.rs:656`で、状態読取が返した
`Native additional SNES state changed while reading; retry`を`unwrap()`してpanicしました。
この構成にはソース照合reportだけがあり、欠けた再生／混合reportを成功とは扱いません。
検証レポート5件と、成功した4構成のインストーラーの計9アーティファクトをGitHub APIで確認しました。
すべて有効期限内で、サイズは0より大きいものです。全5ジョブのドラフトリリース作成stepはskippedでした。
インストーラーのダウンロード・インストール後の起動は確認していません。

この失敗に対して、ローカルでは上記の試験だけを修正しました。
同じretryエラーに限って5秒を上限に再読取し、通知受信待ちも200 msから5秒へ延長します。
ほかのエラーと、完了前の待機・履歴失効・状態値等のassertは保持します。
音源コア・製品のRust実装・ビルド・APIは変更していません。
修正後のローカルRust全試験は723成功・0失敗・4件ignoredで、対象試験の反復も50／50成功しました。
ローカル検証終了時点では修正は未コミットで、最初のCI対象`276e744...`には含まれていません。
その後、ユーザーが修正とガイドを`842245dd...`としてcommit／pushしました。修正を含む別のCIは次節に記録します。
OPL／AY移行との因果関係を確定したという意味ではありません。

このCIはOS間のPCM bit一致や各OSでのWASM比較を確認するものではありません。
音声デバイスを使う実機の音声出力・ノイズ・UI操作・インストール後の起動も別の確認です。

## SNES試験修正後の5環境CI — 全5構成成功

[Build Desktop — run 40143851548](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40143851548)を、
mainのコミット`842245ddf060fedaeb42e6b01d30175b488816b7`から`workflow_dispatch`で手動実行しました。
OPL接続／AY移行と、前節のSNES試験修正を含みます。最初に失敗したrun `40142510765`とは別の実行です。
アプリ版は`0.90.1`のままで、タグや新しいドラフトリリース・一般公開は作成していません。
attempt 1は`completed`／`success`で終了し、全5ジョブの検証とインストーラービルドが成功しました。

| 構成 | ソース照合・Cargo・再生／混合のstep | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

最初のCIで失敗したmacOS x86_64も、今回は検証とビルドを含むジョブ全体が成功しました。
取得した全5構成の検証JSONを監査し、検証アーティファクトZIPのdigestもGitHubの記録と照合しました。

- SDK374ファイルを含む22件の固定記録と、対応ソースZIP1,796ファイルの照合が成功。
- 22 mode、604,032フレーム、88回のRun／Stop遷移が成功。
- 3混合構成、7単独音源、受入れ／16台制限とStop後の予約予算回収が成功。
- 両runnerはexit code 0、`deviceOpened: false`。

前回reportが欠けたmacOS x86_64も、今回は3つのJSONが揃って成功しました。
検証レポート5件とインストーラー5件の計10アーティファクトをGitHub APIで確認しました。
すべて有効期限内で、サイズは0より大きいものです。全5ジョブのドラフトリリース作成stepはskippedでした。
インストーラーや対応ソースZIPをダウンロードして起動・独立照合する確認は行っていません。
今回のCI開始後に製品コードや試験は変更せず、記録だけを更新しています。
OS間のPCM bit一致、各OSでのWASM比較、実機の音声出力・ノイズ・UI操作・インストール後の起動は別の確認です。

## Sega PSG移行の検証

macOS ARM64／Clangで音声デバイスを開かずに検証しました。
固定SDKのコアcpp／header、低レベルC ABIと`chip_state.h`の4入力は旧コピーとbyte一致します。
旧入力とSDK入力を別々にコンパイルし、同じ命令列に対するstdoutのbyte一致を確認してから、実配布WASMと比較しました。

| 対象 | 比較数 | フレーム | WASMとの最大PCM差 |
| --- | ---: | ---: | ---: |
| Game Gear／追加PSGの共通接続 | 69 | 454,656 | 0 |
| Master System | 63 | 387,072 | 0 |
| NukedのFM＋PSG | 222 | 1,363,968 | 2.9802322387695312e-8 |
| GenesisのFM＋PSG | 6 | 49,152 | 3.725290298461914e-9 |

計360ケース・2,254,848フレームで旧／SDKの出力がbyte一致しました。
NukedとGenesisのWASMとの差は、参照切替前と同じ微小な浮動小数点の丸め差です。
4つの既定checkerも実際のSDK入力から成功し、コンパイルしたパスとハッシュを記録しました。
その再確認分は上表へ二重加算しません。
PSGのattenuation 0は最大音量、15は消音です。保持したattenuation 0をStop／offで失わず、auditionで復帰するケースも確認しました。

- 別のNative間比較6ケース・36,864フレームで、raw C ABIの40バイトのシリアライズ状態全体とPCMが一致しました。同じローカルビルド条件での比較で、privateオブジェクト全体やWASM内部状態のdumpではありません。この状態APIのvector割当てはcallback外で扱います。
- Game Gear・Master System・Nukedのfixture自己試験で、callback中のallocation／freeが0でした。主Genesisの比較はPCMとFM pitchを対象とし、単独のallocation測定は含みません。
- Rust全試験は723成功・0失敗・明示実行用4件ignored。
- 再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stop遷移が成功。runnerはexit code 0、`deviceOpened: false`。
- 追加音源は3混合構成・7単独音源が成功。受入れ、16台制限での17台目の拒否、Stop後の予約予算0への回収も成功しました。runnerはexit code 0、`deviceOpened: false`。

先行するrun `40143851548`はこのPSG参照切替を含みません。このローカル検証終了時点では各OSのCIは未実行でした。
後続CIの結果は末尾に記録します。実機の音声出力は別の確認です。

## RF5C164移行の検証

macOS ARM64／Clangで音声デバイスを開かずに検証しました。
選択中／独立追加接続、コア、raw C ABIと共通状態ヘッダーの8入力は旧コピーとbyte一致します。
旧入力とSDK入力を別々にコンパイルし、同じ命令列に対するstdoutのbyte一致を確認してから実配布WASMと比較しました。

| 対象 | 比較数 | フレーム | WASMとの最大差 |
| --- | ---: | ---: | ---: |
| 選択中RF5C164（外側のclock変換とgain rampを含む） | 21 | 98,304 | 0 |
| 独立追加RF5C164 | 75 | 448,512 | 0 |

計96ケース・546,816フレームで旧／SDKの出力がbyte一致し、実WASMとの差も0でした。
追加接続は毎フレーム95値を比較しました。左右PCM、56個のレジスターshadow値、control／selected／mask／clock／bank、
16個のチャンネル位置readと16箇所のRAM観測を含みます。RAM全64 KiBの毎フレーム比較ではありません。
2つの既定checkerも実際のSDK入力から成功し、使用したファイルとディスク上のSHA-256を確認しました。
その再実行分は比較数へ二重加算していません。

- 別のNative間比較6ケース・24,576フレームで、raw C ABIと独立追加接続を44.1／48／96 kHzで確認しました。PCMと16個のbus readを毎フレーム比較し、各ケース27箇所のcheckpointでRAM全65,536バイトを独立した期待値と旧／SDK間で照合しました。
- rawの65,660バイトのシリアライズ状態全体、追加接続の100バイトの公開状態と全RAMも一致しました。ロード後に呼び出し側のバッファを変更してもRAMが変わらず、コピーする所有方式を維持します。範囲外の拒否、reset後のRAM保持、clearとbank切替後の書込みも確認しました。
- これはNative間の公開状態・RAM比較で、privateオブジェクトやWASM内部状態のdumpではありません。raw状態APIのvector割当てはcallback外で扱います。
- 追加接続のfixture自己試験では、9通りのrate／clock条件での音源分離、8声を飽和させた低clockの保持PCM、RAM範囲・保持・状態read、source取り消しとStop／resetが成功。callback中のallocation／freeは0でした。
- Rust全試験は723成功・0失敗・明示実行用4件ignored。
- 再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stop遷移が成功しました。
- 追加音源は3混合構成・7単独音源、受入れ／16台制限とStop後の予約予算0への回収が成功。再生／混合の両runnerはexit code 0、`deviceOpened: false`でした。

先行するrun `40143851548`は今回のSega PSG／RF5C164参照切替を含みません。
このローカル検証終了時点では両移行を含む各OSのCIは未実行でした。後続CIの結果は次節に記録します。
実機の音声出力は別の確認です。

## Sega PSG／RF5C164移行後の5環境CI — 全5構成成功

[Build Desktop — run 40148943609](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40148943609)を、
mainのコミット`4939ddc7f7125f47d969e29c975264eaa528d81c`から`workflow_dispatch`で手動実行しました。
Sega PSGの共通コア／raw C ABIと、RF5C164の選択中／独立追加接続・コア／raw C ABIの移行を含みます。
attempt 1は`completed`／`success`で終了し、全5ジョブの検証とインストーラービルドが成功しました。
アプリ版は`0.90.1`のままで、タグや新しいドラフトリリース・一般公開は作成していません。

| 構成 | ソース照合・Cargo・再生／混合の検証 | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

全5構成の検証JSONを取得・監査し、検証アーティファクトZIPのdigestもGitHubの記録と照合しました。

- SDK374ファイルを含む22件の固定記録と、対応ソースZIP1,796ファイルの照合が成功。
- 22 mode、604,032フレーム、88回のRun／Stop遷移が成功。
- 3混合構成、7単独音源、受入れ／16台制限とStop後の予約予算回収が成功。
- 両runnerはexit code 0、`deviceOpened: false`。

検証レポート5件とインストーラー5件の計10アーティファクトをGitHub APIで確認しました。
すべて対象run／commitと一致し、有効期限内で、サイズは0より大きいものです。
全5ジョブのドラフトリリース添付stepはskippedでした。
インストーラーや対応ソースZIPのダウンロード・起動・独立照合は行っていません。
今回のCI開始後に製品コードや試験は変更せず、記録だけを更新しています。
OS間のPCM bit一致や各OSでのWASM比較、実機の音声出力・ノイズ・UI操作・インストール後の起動は別の確認です。
