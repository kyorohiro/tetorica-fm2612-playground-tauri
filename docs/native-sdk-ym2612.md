# Native SDKの取り込み — OPN系／YM2151／OKIM6258／YM2413／OPL系／AY／Sega PSG／RF5C164／32X PWM／HuC6280／Game Boy／NES＋FDS／DX7／SNES／SPC／Nuked YM2612

Tauriは、固定した`tetorica-soundchip-native`のソース配布をリポジトリ内へ同梱して
YM2612、YM2203、YM2608、YM2610／YM2610B、YM2151、OKIM6258、YM2413、AY-3-8910、Sega PSG、RF5C164、32X PWM、HuC6280、Game Boy、NES＋FDS、DX7、SNES／SPCと
OPL系4種類（YM3526／YM3812／YMF262／Y8950）、代替Nuked YM2612をビルドします。通常のcheckoutに必要な音源ソースが含まれ、SDKの別checkoutや
submodule、ビルド中のGitHub取得を必要としません。
この手順書は最初のYM2612移行時のファイル名を保持し、その後の音源の移行も記録しています。

## 固定した配布物

| 項目 | 値 |
| --- | --- |
| SDK版 | `0.1.4` |
| ソース配布 | `tetorica-soundchip-native-0.1.4-sources.zip` |
| 配布ZIPのSHA-256 | `f173092241badd27c84d6846a69bd175375f080a57fdaf4983c56751c201f65e` |
| SDKの元コミット | `6f8b872b6f35f5df619b2468d58da57037c96e07` |
| 同梱先 | `src-tauri/vendor/tetorica-soundchip-native/` |
| Tauri側の取り込み記録 | `src-tauri/soundchip-sdk.json` |

取り込み記録は`schemaVersion: 1`で、`name`、`version`、`sourceCommit`、`archiveName`、
`archiveSha256`、`manifestSha256`、`fileCount`を保存します。現在の同梱数は395ファイルです。
`sourceCommit`は取り込み時に指定する由来の記録で、コミット署名や配布物の署名を検証したものではありません。
取り込み時に期待するZIPのSHA-256を検証し、SDKマニフェストと各ファイルのサイズ／SHA-256を照合します。
`--check`は元ZIPやネットワークを使わず、同梱マニフェストのハッシュと全ファイルを記録に照合します。

配布物のソース全体、ライセンス、NOTICE、カタログ、原典とハッシュの記録を保持します。
SDK内のBSD、MIT、Apache、LGPL等の宣言はコンポーネントごとの原文のまま残し、
アプリ側のライセンスで置き換えません。SDK内の`LICENSES.md`と各カタログを参照してください。
同梱している全コンポーネントをアプリでリンクする、という意味ではありません。
SDK `0.1.4`にはWonderSwanのraw C++コアとWASM bridgeも含まれます。
今回の固定版更新ではCargoへWonderSwanをリンクせず、WonderSwan modeは引き続きWeb Audioのみです。

## ビルドで使う部分

`src-tauri/build.rs`は`cc`で、次のSDKソースを既存のアプリ側ライブラリーへ組み込みます。
この段階ではSDKの全CMakeターゲットへ一括して切り替えません。

| 用途 | SDK内のパス |
| --- | --- |
| Genesis主音源のYM2612接続 | `wasm/ym2612_wasm.cpp` |
| Genesis Nuked／32X Nukedと独立追加Nuked YM2612の接続 | `native/reference/tauri/native_genesis_nuked.cpp`と`.inc`、`native_genesis_ym2612.h` |
| NukedのBrowser共通C wrapperとコア | `wasm/nuked_opn2_wasm.c`、`vendor/nuked-opn2/ym3438.c`と対応ヘッダー |
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
| Genesis／Nuked、Master System、Game Gearと追加PSGに共通のコア | `vendor/tetorica_sega_psg/segapsg.cpp`と対応ヘッダー、同フォルダーの`LICENSE` |
| Mega CD／32X系の選択中RF5C164接続 | `native/reference/tauri/native_genesis_rf5c164.cpp`と対応ヘッダー |
| RF5C164の独立追加接続 | `native/reference/tauri/native_genesis_extra_rf5c164.cpp`と対応ヘッダー |
| 選択中RF5C164とraw C ABIに共通のコア | `vendor/mame-rf5c164/rf5c164.cpp`と対応ヘッダー |
| RF5C164のraw C ABI | `wasm/rf5c164_wasm.cpp`と`vendor/ymfm/chip_state.h` |
| 選択中／追加生成32X PWMに共通のNative接続 | `native/reference/tauri/native_genesis_pwm.cpp`と対応ヘッダー |
| 32X PWMのFIFO・routing・timer・PCM生成 | `wasm/pwm32x_core.h` |
| 選択中／追加生成HuC6280の共通Native接続 | `native/reference/tauri/native_genesis_huc6280.cpp`と対応ヘッダー |
| 接続が直接includeするHuC6280コア | `vendor/mame-huc6280/huc6280.cpp`と対応ヘッダー |
| 選択中／追加生成Game Boyの共通Native接続 | `native/reference/tauri/native_genesis_gameboy.cpp`と対応ヘッダー |
| 共通Game Boy APU | `vendor/mame-gameboy/gameboy_apu.cpp`と対応ヘッダー |
| 選択中NES＋FDS／追加生成NESの共通接続とAPU／FDS移植実装 | `native/reference/tauri/native_genesis_nes.cpp`と対応ヘッダー |
| 選択中／追加生成DX7の共通Native接続 | `native/reference/tauri/native_genesis_dx7.cpp`と対応ヘッダー |
| 共通MSFAの24入力と6 Native override | `vendor/msfa/`と`native/reference/overrides/msfa/`からCargoの出力先へコピーして組み立て |
| 選択中／追加生成SNES S-DSPの共通接続 | `native/reference/tauri/native_genesis_snes.cpp`と対応ヘッダー |
| 独立追加SPC700ファイルプレーヤーの接続 | `native/reference/tauri/native_genesis_spc.cpp`と対応ヘッダー |
| SNES／SPC共通S-DSPとSPC700コア | `vendor/snes-spc/snes_spc/`の`SPC_DSP.cpp`、`SNES_SPC.cpp`、`SNES_SPC_misc.cpp`、`SNES_SPC_state.cpp`と対応ヘッダー |
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
これらの先行移行は同梱済みのSDK `0.1.0`へ参照を切り替えました。Nuked単体ターゲットを追加した`0.1.1`、Sega PSGを独立フォルダーへ分離した`0.1.2`を経て、現在はWonderSwanを収録した`0.1.4`を固定し、既存音源の音声計算を保持しています。
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
SDK `0.1.2`では、コアcpp／headerを`vendor/tetorica_sega_psg/`へ分離しています。
Tetorica独自実装のBSD-3-Clause表記とSMS Powerの仕様資料への参照を付け、共有状態helperは`vendor/ymfm/chip_state.h`に維持します。
DSPの計算、C ABI、Native側のレート変換と音量は変更していません。
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

32X PWMのNative接続cpp／headerと、ヘッダー実装の`wasm/pwm32x_core.h`も固定SDKから使います。
3入力は旧コピーとbyte一致し、選択中32X／32X Nukedと独立追加生成が同じ接続を使います。
既存の`tetorica_native_pwm` archive、C++17・O2・MSVC `/EHsc`・`-ffp-contract=off`とリンク順序を維持します。
SDK接続のFIFO・保持出力のsource tagと取り消しを引き継ぎ、Rustの書込み予約、owner・寿命・予算、
Stop／resetと試奏の制御、Mixer／FXや機種APIはアプリ側に残ります。SDKの版や同梱バイト列は変更しません。

HuC6280も選択中PC Engine／独立追加生成の共通Native接続とコアを固定SDKから使います。
接続cppの差はinclude先の2行だけで、従来スナップショットの相対参照をSDKのコアへ切り替えます。
接続ヘッダーとコアcpp／headerは旧コピーとbyte一致します。接続cppが`huc6280.cpp`を直接includeするため、
コアを別にコンパイルせず、既存の`tetorica_native_huc6280`内の1翻訳単位を維持します。
C++17・O2・MSVC `/EHsc`・`-fwrapv`・`-fno-strict-aliasing`・`-ffp-contract=off`とリンク順序も維持します。
Rustの予約・owner・寿命・予算、PCM経路、Stop／reset・試奏と機種API、共通Mixer／FXはアプリ側に残ります。

Game Boyも選択中／独立追加生成の共通Native接続とAPUを固定SDKから使います。
接続cpp／headerと`vendor/mame-gameboy/gameboy_apu.cpp`／headerの4入力は旧コピーとbyte一致します。
APU headerは従来Native版の状態観測用`NativeGameboyAccess` friendを含みます。Browser原本との差は以前からあるもので、今回DSPや観測用接続を追加する変更ではありません。
既存の`tetorica_native_gameboy` archive内で接続とAPUを各1回ずつコンパイルし、
C++17・O2・MSVC `/EHsc`・`-fwrapv`・`-fno-strict-aliasing`・`-ffp-contract=off`とリンク順序を維持します。
Rustの予約・owner・寿命・予算、PCM、Stop／reset・試奏、Mixer／FXとWebView／JS APIはアプリ側に残ります。

選択中NES＋FDSと独立追加NESの共通Native接続も固定SDKを使います。
接続cpp／headerは旧コピーとbyte一致し、cpp内に従来のJSNES／FDSからのC++移植実装を含みます。
既存の`tetorica_native_nes` archive内の1翻訳単位、C++17・O2・MSVC `/EHsc`・`-fwrapv`・`-fno-strict-aliasing`・`-ffp-contract=off`とリンク順序を維持します。
SDKの`vendor/tetorica-nes/nes_core.cpp`を追加でコンパイルする変更ではありません。
各インスタンスの64 KiB DMC RAMとFDS波形・変調表を維持し、アップロードをRAMへコピーします。
RustのDMC転送payload所有・予約、owner・寿命・台数・予算と回収、PCM、Stop／reset・試奏、Mixer／FXと機種APIはアプリ側に残ります。

選択中DX7と独立追加DX7の共通接続cpp／header、およびMSFAも固定SDKからビルドします。
接続cpp／headerは旧コピーとbyte一致します。`src-tauri/build.rs`の`native_dx7_sources()`が、
`vendor/msfa/`の24共有ファイルと`native/reference/overrides/msfa/`の6ファイルを
`OUT_DIR/tetorica_sdk_dx7_msfa/`へ編集せずにコピーし、13個のccと17個のheaderを組み立てます。
overrideは`dx7note.cc`、`env.cc`、`ringbuffer.cc`、`synth.h`、`synth_unit.cc`、`synth_unit.h`です。
引用形式のincludeはソースの隣を先に探すため、共有ソースとoverrideの直接混在ではなく、同じprivateディレクトリを使います。
既存の`tetorica_native_dx7` archive内の14翻訳単位とリンク順序、C++17・O2・`MSFA_FORCE_SCALAR`・`_USE_MATH_DEFINES`・
MSVC `/EHsc`・`-fwrapv`・`-fno-strict-aliasing`・`-ffp-contract=off`を維持し、新しいPython／CMake依存は追加しません。

SDKのNative-only `synth_unit.cc`には、Panic時に16個の事前確保済み`Dx7Note`へ`Dx7Note{}`を値代入し、
note状態とfeedback履歴を初期化する修正があります。オブジェクトのアドレスを変えず、allocation／freeも行いません。
通常のnote-on／release／killは保持しますが、Panic後の再発音は意図した修正のため、旧スナップショットとの全面的なbyte一致は主張しません。
155バイト音色のコピー、16音のslot制御と内部44.1 kHzの生成・device rate変換は既存接続のままです。
Rustの音色所有・転送予約、Keyboard lease・owner、16 slotの受入れ制御、PCM、Stop、Mixer／FXと機種APIはアプリ側に残ります。

SNES S-DSPの選択中／独立追加接続とSPC700ファイルプレーヤーも、同じ固定SDKからビルドします。
2つの接続cppと2 ABI header、SNES-SPCの24ファイルは旧コピーとbyte一致します。
既存の`tetorica_native_snes` archiveに接続2個とコア4個の計6翻訳単位を維持し、
共有`SPC_DSP.cpp`は一度だけコンパイルします。SDK CMakeの2つの独立ライブラリーを追加せず、
`SPC_Filter.cpp`や別C ABIの`spc.cpp`／`dsp.cpp`も組み込みません。
既存のarchive順序、C++17・O2・MSVC `/EHsc`・`-fwrapv`・`-fno-strict-aliasing`・`-ffp-contract=off`を保持します。

SNESのRAMアップロードは各コア所有の64 KiBへ即時コピーします。timelineは初期RAM／DSPとイベント列を借用し、
Rustが検証済みの不変`Arc<Timeline>`を再生・pause・完了後も保持し、置換／global Stop／破棄時にcallback外で回収します。
Native StopはRAM・設定を保持してgate／履歴を消すため、保持したecho RAMが鳴る場合があります。resetはRAMも消します。
SPCは読込みでCPU／RAM／DSP状態をコア内へコピーし、Rustが元ファイル全体を保持してStop／絶対seek用のコアを再構築します。
生成・load・最大16,000 Nativeフレーム単位のseek・破棄はcontrol側で行い、準備済みのコアを単一の音声ownerへ公開・回収します。
pauseは巻き戻さず、mute後も128フレームの既存PCMやechoが残り得る従来の動作です。
観測用snapshotは保存状態ではなく、DSPイベントの借用領域はコア変更を除外して読み出します。
Rustの所有・予約・予算・owner／group・Mixer／FXとJS APIは変更しません。

Y8950は512個の可変4 KiBページを借用し、最大2 MiBのADPCM-B RAMを扱います。
SDKメモリーhelperはポインター配列とinterfaceを保持し、RAMのバイト列自体は所有しません。
RustのArcによる所有、control側でのCOW準備、寿命・回収・予算管理を引き継ぎます。
公開済みページの読み書きは音声threadが行い、既存のmemory revision／upload revisionの規則を保持します。
この参照切替では新しいRAM転送APIを追加しません。

Nukedは既存のC++接続archiveと、全8個の`OPN2_*`を`tetorica_nuked_OPN2_*`へprefixしたCコアarchiveを使います。
Genesisのconsumer、Nuked接続、C providerの順でリンクし、SDKのCMakeライブラリーを追加して重複させません。
C++14／O2／MSVC `/EHsc`とC11／O2／`-fwrapv`、初回生成時の`std::call_once`によるglobal chip type選択を保持します。
接続cppの差はwrapperのincludeパスだけで、`.inc`・共通state header・C wrapper・コア・ライセンスは旧コピーとbyte一致します。
raw address／dataの各write後24内部clock、生成時24 clockの出力合計×11、clock／144のsource rateを保持します。
選択中Genesis／32X Nukedは自然なreset時のDCを保持し、ymfmのcalibration frameを消費しません。
独立追加Nukedは6CHの個別コアを持ち、機種用PSG・source gain・DC除去を含みません。
周波数ラッチ、通常CH3とCSMの別取消し、DAC、mute、試奏とStop／resetは既存の動作を保持します。

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

通常の22音源IDに加え、代替Nuked YM2612も固定SDKへ参照を切り替えます。機種用の補助処理はアプリ側に残ります。
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
PWMの旧接続と`src-tauri/native-pwm/`も原典・比較用として保持します。
実際のSDK接続cpp／headerとSDK wasmディレクトリを監視し、旧スナップショットのソース監視は外します。
`sources.json`が固定するスナップショットのREADME等の原典・ハッシュ記録は変更せず、旧コピーやsyncスクリプトだけではSDKの入力を更新しません。
HuC6280の旧接続・スナップショット・固定原典も比較用に保持し、READMEや`PINNED`、ライセンスの参照先は変更しません。
実際のSDK接続cpp／headerと`vendor/mame-huc6280/`を監視し、旧スナップショットのソース監視は外します。
Game Boyの旧接続・スナップショット・固定原典とライセンスの同梱先も保持します。
Cargoは実際のSDK接続cpp／headerと`vendor/mame-gameboy/`を監視し、実消費元のない旧スナップショットのソース監視を外します。
旧コピーや`sync_native_gameboy.py`だけでは実際のSDK入力を更新しません。READMEや`PINNED`等の固定原典は変更しません。
NESの旧接続とJS原典スナップショット、`core.json`／`sources.json`、READMEとライセンスの同梱先も保持します。
CargoはSDK接続cpp／headerを監視し、実消費元のない旧スナップショットのソース監視を外します。
旧コピーや`sync_native_nes.py`だけでは実際のSDK入力を更新しません。
DX7の旧接続と`src-tauri/native-dx7/`の32ファイルのスナップショット、原本／修正版のハッシュ、ライセンスの同梱先も保持します。
CargoはSDKの`vendor/msfa/`と6 Native overrideのディレクトリ、接続cpp／headerを監視し、
Rustの埋め込み消費元がない旧`native-dx7/`とローカル接続のソース監視を外します。
旧コピーや`sync_native_dx7.py`だけでは実際のSDK入力を更新しません。SDKの固定配布物へ直接変更を加えません。
SNES／SPCの旧接続と`src-tauri/native-snes/`の34件の固定記録、LGPLライセンス・由来・syncスクリプトも保持します。
CargoはSDKの`vendor/snes-spc/`と2接続cpp／headerを監視し、Rustの埋め込み消費元がない旧`native-snes/`とローカル接続のソース監視を外します。
旧コピーや`sync_native_snes.py`だけでは実際のSDK入力を更新しません。
Nukedの旧接続cpp／`.inc`と`src-tauri/native-nuked/`の9件の固定記録、LGPLライセンス・由来も保持します。
CargoはSDK接続cpp／`.inc`、共通state header、SDKの`vendor/nuked-opn2/`と`wasm/`を監視し、旧コピーのソース監視を外します。
`sync_native_nuked.py`だけでは実際のSDK入力を更新しません。元の`README.md`や固定ハッシュは変更しません。
配布物を改変して再リンクする場合の実入力と手順は[NATIVE-SOURCES.md](../NATIVE-SOURCES.md)を参照してください。

## 照合と更新

通常の検証はTauriのルートで実行します。別のSDKリポジトリや配布サーバーへ接続しません。

```sh
python3 scripts/import_soundchip_sdk.py --check
```

SDKを更新するときだけ、検証したソースZIPを明示して取り込みます。
次の値は現在同梱している配布物の例です。新しい配布ではその配布の期待ハッシュと元コミットを指定してください。

```sh
python3 scripts/import_soundchip_sdk.py \
  --archive /path/to/tetorica-soundchip-native-0.1.4-sources.zip \
  --sha256 f173092241badd27c84d6846a69bd175375f080a57fdaf4983c56751c201f65e \
  --source-commit 6f8b872b6f35f5df619b2468d58da57037c96e07
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

PWMとGame Boyの選択中接続もTauriルートから比較できます。

```sh
node scripts/check_native_pwm.mjs --source /path/to/hello_ymfm
node scripts/check_native_gameboy.mjs --source /path/to/hello_ymfm
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
node scripts/check_native_extra_ym2612_nuked.mjs
node scripts/check_native_megacd.mjs
node scripts/check_native_extra_rf5c164.mjs
node scripts/check_native_extra_pwm.mjs
node scripts/check_native_huc6280.mjs
node scripts/check_native_extra_huc6280.mjs
node scripts/check_native_gameboy.mjs
node scripts/check_native_extra_gameboy.mjs
node scripts/check_native_nes.mjs --wasm
node scripts/check_native_extra_nes.mjs
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

## 32X PWM移行の検証

macOS ARM64／Clang・Nodeで音声デバイスを開かずに検証しました。
Native接続cpp／headerとコアヘッダーの3入力は旧コピーとbyte一致し、SDKの固定版は変更していません。
旧／SDKを別々にコンパイルし、同じ命令列のstdoutのbyte一致を確認してから実配布WASMとJS参照を比較しました。

| 対象 | 固有Native命令列 | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| 選択中PWM | 57 | 466,944 | 57 | 466,944 |
| 追加PWMとコア単体 | 75 | 444,672 | 225 | 1,334,016 |
| 合計 | 132 | 911,616 | 282 | 1,800,960 |

追加側の同じ75命令列を3通りのWorklet block分割で比較しており、282回を固有命令列の数へ加算しません。
132件のうち129件は公開C ABI、3件・18,432フレームは従来コア単体のDAC／mute設定の確認です。
公開Native接続の設定を追加したという意味ではありません。旧／SDKはbyte一致し、実WASM／JSとの差はすべて0でした。
2つの既定SDKコンパイル経路も成功し、使用した入力のパスとハッシュを記録しました。この再確認分も集計に加えていません。

- Tauriの既存fixtureと、境界・tag試験を含むSDK既存の検証fixtureを旧／SDKの両方へ実行し、constructorの境界・初期値、不正FIFO状態のrollback、64ビットsource tagの寿命・取り消し、独立インスタンスと観測read、Stop／resetが成功しました。
- 追加側の公開state ABIは96バイト、coreの確保量は168バイトで、render中のallocation／freeは0。状態・tagの自己試験はPCM比較と別で、毎フレームのprivate全状態dumpやWASM内部状態の比較ではありません。
- `npm test`は31／31成功。Rust全試験は723成功・0失敗・明示実行用4件ignoredでした。初回sandbox内の2件の失敗はMCPのローカルTCP bind制限で、製品を修正せず制限外で再実行しました。
- 再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stopが成功。
- 既存の追加音源混合試験は3混合構成・7単独音源、受入れ／16台制限で17台目の拒否、Stop後の予約予算0への回収が成功。両runnerはexit code 0、`deviceOpened: false`でした。

直前の全5構成成功run `40148943609`は今回のPWM移行を含みません。
このPWMローカル検証終了時点ではcommit／push、新しいCIとsanitizerは未実施でした。
その後、PWM差分は`39a02786...`へcommit済みです。PWMを含む後続CIの結果は末尾に記録します。実機の音声出力は別の確認です。

## HuC6280移行の検証

Native接続cppの差はinclude先の2行だけで、接続headerとコアcpp／headerは旧コピーとbyte一致します。
macOS ARM64／Clang・Nodeで旧／SDKを別々にコンパイルし、同じ入力のstdoutのbyte一致を確認してから実配布WASMと比較しました。

| 対象 | Native入力数（suite内） | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| 選択中HuC6280 | 39 | 239,616 | 39 | 239,616 |
| 追加HuC6280 | 51 | 438,912 | 153 | 1,316,736 |

選択中の39入力は追加側51入力の部分集合で、sample rate・frame数・命令列SHAとPCMのSHAで重複を確認しています。
両suiteの延べ90入力・678,528フレームに対し、重複を除いた固有Native入力は51件・438,912フレームです。
追加側の各入力を3通りのWorklet block分割で比較し、実WASM比較は計192回・1,556,352フレームで差0でした。
比較回数や既定コンパイル経路の再確認を、固有Native入力の数へ加算していません。

- Tauriの既存fixtureとSDK既存の検証fixtureを旧／SDKへ実行し、constructor境界、不正入力、5 rate×4 clock×6 voice、32バイト波形コピー、stereo／LFO／noise、held PCM、Stop／reset、独立インスタンスと観測readが成功しました。
- 公開state ABIは352バイト、Native本体の確保量は704バイトで、render中のallocation／freeは0。状態・callbackの自己試験はPCM比較と別で、毎フレームのprivate全状態dumpやWASM内部状態の比較ではありません。
- 既定のSDKコンパイル経路2本と実行ファイルをinline指定する経路が成功し、不正な`--fixture`の6形式をコンパイル前に拒否しました。入力のパスとハッシュも記録しました。

`npm test`は31／31成功。Rust全試験は723成功・0失敗・明示実行用4件ignoredでした。
MCPのローカルTCP bindのため、今回は初めから制限外で実行し、sandbox内の失敗や製品修正はありません。
再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stopが成功しました。
追加音源混合試験も3混合構成・7単独音源、受入れ／16台制限で17台目の拒否が成功し、
受入れ・台数制限の双方でStop後の予約予算が0へ戻りました。両runnerはexit code 0、`deviceOpened: false`でした。
直近の全5構成成功run `40148943609`は、PWMと今回のHuC6280移行を含みません。
このHuC6280ローカル検証終了時点では、差分のcommit／pushと新しいCIは未実施でした。
その後、HuC6280差分は`2125896b...`へcommit／push済みです。PWMと合わせた後続CIの結果は次節に記録します。実機の音声出力は別の確認です。

## 32X PWM／HuC6280移行後の5環境CI — 全5構成成功

[Build Desktop — run 40162092207](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40162092207)を、
mainのコミット`2125896b7173c5c662c6eaf2f2b69b4a010f12b7`から`workflow_dispatch`で手動実行しました。
32X PWMの共通Native接続／コアヘッダーと、PC Engineの選択中／独立追加HuC6280のNative接続／コアのSDK参照切替を含みます。
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
- 3混合構成、7単独音源、受入れ／16台制限で17台目の拒否と、Stop後の予約予算0への回収が成功。
- 両runnerはexit code 0、`deviceOpened: false`。

検証レポート5件とインストーラー5件の計10アーティファクトをGitHub APIで確認しました。
すべて対象run／commitと一致し、有効期限内で、サイズは0より大きいものです。
全5ジョブのドラフトリリース添付stepはskippedでした。
取得したのは検証レポートZIPだけで、インストーラーや対応ソースZIPのダウンロード・起動・独立照合は行っていません。
今回のCI開始後に製品コードや試験は変更せず、記録だけを更新しています。追加のcommit／pushは行っていません。
OS間のPCM bit一致や各OSでのWASM比較、実機の音声出力・ノイズ・UI操作・インストール後の起動は別の確認です。

## Game Boy移行の検証

共通Native接続cpp／headerとAPU cpp／headerの4入力は旧コピーとbyte一致します。
macOS ARM64／Clang・Nodeで旧／SDKを別々にコンパイルし、同じ入力のstdoutのbyte一致を確認してから実配布WASMと比較しました。

| 対象 | Native入力数（suite内） | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| 選択中Game Boy | 42 | 396,912 | 42 | 396,912 |
| 追加Game Boy | 48 | 439,056 | 144 | 1,317,168 |

両suiteの36入力はsample rate・frame数・命令列SHAが同じで、PCMのSHAも一致しています。
延べ90入力・835,968フレームに対し、重複を除いた固有Native入力は54件・488,208フレームです。
追加側の各入力を3通りのWorklet block分割で比較し、実WASM比較は計186回・1,714,080フレームで差0でした。
比較回数や既定コンパイル経路の再確認を、固有Native入力の数へ加算していません。

- Tauriの既存fixtureとSDK既存の検証fixtureを旧／SDKへ実行し、constructor境界、不正入力のrollback、3 rate×3 clock×4 voiceのStop、NR52／DMG波形RAM・source maskの保持、半clockでのvoice別held PCMと全体Stop、独立インスタンスと観測read・位相が成功しました。
- 公開state ABIの60バイトとoffsetを確認し、render中のallocation／freeは0。状態・位相・callbackの自己試験はPCM比較と別で、毎フレームのprivate全状態dumpやWASM内部状態の比較ではありません。
- 既定のSDKコンパイル経路2本と実行ファイルをinline指定する経路が成功し、不正な`--fixture`の6形式をコンパイル前に拒否しました。入力のパスとハッシュも記録しました。

`npm test`は31／31成功。Rust全試験は723成功・0失敗・明示実行用4件ignoredでした。
MCPのローカルTCP bindのため、今回は初めから制限外で実行し、sandbox内の失敗や製品修正はありません。
再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stopが成功し、runnerはexit code 0、`deviceOpened: false`でした。
追加音源混合試験も3混合構成・7単独音源、受入れ／16台制限で17台目の拒否が成功し、
受入れ・台数制限の双方でStop後の予約予算が0へ戻りました。runnerはexit code 0、`deviceOpened: false`でした。
直前の全5構成成功run `40162092207`は、今回のGame Boy参照切替を含みません。
このGame Boyローカル検証終了時点ではcommit／push、新しいCIとsanitizerは未実施でした。
その後、Game Boy差分は`407a75b3...`へcommit済みです。NES＋FDSと合わせた後続CIの結果は末尾に記録します。実機の音声出力は別の確認です。

## NES＋FDS移行の検証

共通Native接続とAPU／FDS移植実装を含むcpp、およびC ABI headerは旧コピーとbyte一致します。
macOS ARM64／Clang・Nodeで旧／SDKを別々にコンパイルし、同じ入力のstdoutのbyte一致を確認してから実配布WASMとJS参照を比較しました。

| 対象 | Native入力数（suite内） | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| 選択中NES＋FDS | 72 | 710,784 | 72 | 710,784 |
| 追加NES | 81 | 784,512 | 243 | 2,353,536 |

選択中の72入力は追加側81入力の部分集合で、output rate・base clock・frame数・FDS設定・命令列SHAとPCMのSHAで重複を確認しています。
延べ153入力・1,495,296フレームに対し、重複を除いた固有Native入力は81件・784,512フレームです。
追加側の各入力を3通りのWorklet block分割で比較し、実WASM比較は計315回・3,064,320フレームで差0でした。JS参照との差も0です。
比較回数や既定コンパイル経路の再確認を、固有Native入力の数へ加算していません。

- Tauriの既存fixtureとSDK既存の検証fixtureを旧／SDKへ実行し、rate／base clock／region境界、64 KiB入力コピーと不正RAM転送のrollback、DMCのwrap／IRQ、他声のgate操作による最終バイトのPCM・寿命保護、FDS波形／変調／I/O gate、6声のmute中の進行と声別gate分離が成功しました。
- 公開state ABIは400バイト、Native本体の確保量は81,584バイト。StopのRAM／波形／clock保持、reset後の64 KiB全RAMの0読出しとclock／mute保持、観測readと独立インスタンスを確認し、render中のallocation／freeは0でした。これらは別の自己試験で、毎フレームのprivate全状態・全RAMやWASM内部状態を比較したものではありません。
- 既定のSDKコンパイル経路2本と実行ファイルをinline指定する経路が成功し、選択中の比較は`--wasm`を明示しました。不正な`--fixture`の6形式をコンパイル前に拒否し、入力のパスとハッシュも記録しました。
- 既存のメモリーコピー診断は旧／SDKの双方で転送条件とallocation／free 0を確認しました。時間値の一致や速度同等は主張せず、PCM比較数へ加算していません。

`npm test`は31／31成功。Rust全試験は723成功・0失敗・明示実行用4件ignoredでした。
MCPのローカルTCP bindのため、今回は初めから制限外で実行し、sandbox内の失敗や製品修正はありません。
再生ライフサイクルは22 mode、604,032フレーム、88 Run／Stopが成功し、runnerはexit code 0、`deviceOpened: false`でした。
追加音源混合試験も3混合構成・7単独音源、受入れ／16台制限で17台目の拒否が成功し、
受入れ・台数制限の双方でStop後の予約予算が0へ戻りました。runnerはexit code 0、`deviceOpened: false`でした。
直前の全5構成成功run `40162092207`は、Game Boyと今回のNES参照切替を含みません。
このNESローカル検証終了時点ではcommit／push、新しいCIとsanitizerは未実施でした。
その後、NES差分は`a70e86d2061349a396de7e4816a5438fea5dcb4f`へcommit／push済みです。Game Boyと合わせた後続CIの結果は次節に記録します。実機の音声出力は別の確認です。

## Game Boy／NES＋FDS移行後の5環境CI — 全5構成成功

[Build Desktop — run 40168462256](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40168462256)を、
mainのコミット`a70e86d2061349a396de7e4816a5438fea5dcb4f`から`workflow_dispatch`で手動実行しました。
Game Boyの共通Native接続／APUと、NES＋FDSの共通接続／APU／FDS実装のSDK参照切替を含みます。
attempt 1は`completed`／`success`で終了し、全5ジョブの検証とインストーラービルドが成功しました。CIのためのコード修正は不要でした。
アプリ版は`0.90.1`のままで、新しいタグやドラフトリリース・一般公開は作成していません。

| 構成 | ソース照合・npm・Cargo・再生／混合の検証 | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

全5構成の検証JSONを取得・監査し、検証アーティファクトZIPのdigestもGitHubの記録と照合しました。

- `npm test`のPython 22件とNode 31件が全5構成で成功。CargoはWindowsが726成功・0失敗・3件ignored、ほか4構成が723成功・0失敗・4件ignoredでした。
- SDK374ファイルを含む22件の固定記録と、対応ソースZIP1,796ファイルの照合が成功しました。
- Game Boy／NESを含む22 mode、604,032フレーム、88回のRun／Stop遷移が成功しました。
- 既存の3混合構成・7単独音源、受入れ／16台制限で17台目の拒否と、Stop後の予約予算0への回収が成功しました。両runnerはexit code 0、`deviceOpened: false`です。
- この混合構成はGame Boy／NES専用の組合せを追加したものではありません。

検証レポート5件とインストーラー5件の計10アーティファクトをGitHub APIで確認しました。
すべて対象run／commitと一致し、有効期限内で、サイズは0より大きいものです。全5ジョブのドラフトリリース添付stepはskippedでした。
取得したのは検証レポートZIPだけで、インストーラーや対応ソースZIPのダウンロード・起動・独立照合は行っていません。
今回のCI開始後に製品コードや試験は変更せず、記録だけを更新しています。追加のcommit／pushは行っていません。
OS間のPCM bit一致や各OSでのWASM比較、実機の音声出力・ノイズ・UI操作・インストール後の起動は別の確認です。先行するローカル比較結果と区別してください。

## DX7移行の検証

2026-10-25。選択中／独立追加の共通Native接続とMSFAを固定SDKへ切り替えました。
接続cpp／headerとMSFA 30入力中29ファイルは旧コピーとbyte一致し、差はNative Panicのfeedback履歴初期化だけです。
MSFAは24共有入力と6 Native overrideをコピーして組み立てます。macOS ARM64／Clang・Nodeで、
通常再生の旧／SDK一致と、意図したPanic修正を分けて検証しました。

| 対象 | 固有Native入力数 | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| 選択中DX7の既存再生比較 | 114 | 792,576 | 114 | 792,576 |
| 独立追加DX7の既存比較 | 21 | 239,616 | 63 | 718,848 |
| 32 algorithm × 3 rateのPanic／再発音 | 96 | 786,432 | 96 | 786,432 |
| 合計 | 231 | 1,818,624 | 273 | 2,297,856 |

既存135入力・1,032,192 Nativeフレームは旧／SDKのstdoutがbyte一致し、双方とも実WASM最大差0でした。
追加側は1入力を3通りのblock分割へ比較します。追加checker内の選択中比較の再実行、
既定／fixture指定の再確認、自己試験と公開JS adapterの確認は集計へ加算していません。
新しいPanic比較はSDKと実WASMで全96入力の差0、新規インスタンスへ戻る自己試験も96／96成功しました。
旧版は79／96入力でSDKとの差があり、17入力は一致しました。全入力でframe 1,403の再発音直前までは一致し、
差は再発音後だけに現れ、最大差は`0.0002746583195403218`でした。この意図した差を通常再生の回帰結果と分けています。
SDKと実WASMの合計273比較・2,297,856フレームの最大差は0でした。

- Tauri既存fixtureの自己試験は旧／SDKとも成功。SDKの強化fixtureはrate境界、不正slotの無作用、保護slotとrelease tail、16音の確保、held／released音へのlive edit、voice identity分離、即時Panic無音と96回のfeedback再発音が成功しました。強化した新規音源一致判定は旧版で意図どおり失敗し、別に実行した旧版の制御試験は成功しました。
- Native本体と16個の事前確保noteは合計35,320バイト。17通りの部分生成失敗の回収、LFO／filter／resamplerの独立性、保護slot割当て、単一slot killを確認し、render／制御中のallocation／freeは0でした。毎フレームのprivate全状態やWASM内部状態をdump比較したものではありません。
- 2本の既定checkerが固定SDKから成功。1 fixtureにつきSDK32ファイルとfixture側2ファイルの計34入力をSHA-256と照合し、30組立入力の実読み込みと両ABI headerを確認しました。`--fixture`／`--out`のinline指定は成功し、不正な12形式をコンパイル前に拒否し、入力fixtureを保持しました。
- 実Cargo生成物の30入力はSDKとbyte一致し、32ソース入力の監視を確認しました。実archiveの14 object、10個のC ABI exportの各1回の定義と既存リンク順序を保持し、旧ヘッダー混入や新しい動的ライブラリー指示はありません。
- 公開JS adapterは44.1／48 kHzで確認し、96 kHzは実配布WASM exportsへ同じ整数位相変換を適用した比較です。公開adapterの96 kHz対応を追加したという意味ではありません。実WASMのSHA-256は`f5de761e82ea303420fb3aac79a0a59dd053773b81453b955254dd13aac77d6f`で、`web/`と`docs/`が一致しました。

`npm test`はPython 22件・Node 31件が成功。Rust全試験は723成功・0失敗・明示実行用4件ignoredでした。
MCPのローカルTCP bindのため初めから制限外で実行し、sandbox内の失敗や製品修正はありません。
再生ライフサイクルは22 mode・604,032フレーム・88 Run／Stopが成功しました。
追加音源の3混合構成・7単独音源、受入れ／16台制限で17台目の拒否も成功し、両制限試験でStop後の予約予算は0へ戻りました。
再生／混合の両runnerはexit code 0、`deviceOpened: false`です。
直前の全5構成成功run `40168462256`はこのDX7移行を含みません。
このDX7ローカル検証を終えた時点ではcommit／pushと新しいCIは未実施でした。その後のSNES／SPCとまとめたCI結果は末尾へ記録します。
sanitizer、実機の音声出力・ノイズ・UI操作やインストール後の起動は未確認です。
リアルタイム期限や処理速度の保証もこの確認範囲に含まれません。

## SNES／SPC移行の検証

2026-10-25。選択中／独立追加SNES S-DSPと独立追加SPC700ファイルプレーヤーの接続・共通コアを固定SDKへ切り替えました。
着手前のTauriは`469200109298ad3ba80ca1f0dca7ee0063b0eaa5`でclean、直前のDX7移行までコミット済みです。
接続cpp／headerの4ファイルとSNES-SPC 24ファイルはbyte一致し、旧34件のsnapshot pinとライセンスは保持しています。
SDKの版・374ファイル・取り込み記録とBrowser／npmのWASMは変更していません。

macOS ARM64／Clang・Nodeで、音声デバイスを開かずに旧／SDKと実配布WASM／Workletへ比較しました。

| 対象 | 固有Native入力数 | Nativeフレーム | 実WASM比較回数 | 比較フレーム |
| --- | ---: | ---: | ---: | ---: |
| SNES（選択中と追加生成の重複を除く） | 75 | 761,856 | 267 | 2,727,936 |
| 独立追加SPC | 84 | 688,128 | 252 | 2,064,384 |
| 合計 | 159 | 1,449,984 | 519 | 4,792,320 |

選択中SNESの42入力・442,368フレームは追加側75入力の部分集合です。追加側は1入力を3通りのblock分割へ比較し、
Native入力数とWASM比較回数を区別します。全入力の旧／SDK stdout／stderrはbyte一致し、実WASM比較の最大差は0でした。
既定checkerの再実行、fixture指定と自己試験は集計へ加算しません。
実配布WASM SHA-256は`cbbe06819574249217f35da16eebd45a53274e53251f31242e7dd407b03d0d70`です。

- SNESの既存／SDK fixtureを旧／SDKで2種類の自己試験へ実行し、8回とも成功しました。公開state ABIは152バイト、ローカルの確保量は67,280バイト。コピーRAM・全体reset・境界・安定したread-only view、8声と保護gate、保持Stop・独立性と借用timelineのpause／完了／再開を確認し、render／制御中allocation／freeは0でした。
- SPCの28シナリオを3出力rateで確認し、実WASMでPCMと810 snapshot・358,056件の順序付きDSP書込みを照合しました。SDK強化fixtureも旧／SDKで成功し、56バイトのstate ABI・16バイトのevent ABI、ローカルの確保量204,624バイト、コピーload・control側seek・pause・独立mute・非進行観測・bounded observer／overflow・失敗／reloadとrender中allocation／free 0を確認しました。
- 毎フレームのCPU／DSP／resamplerのprivate全状態のdump比較ではありません。合成SPC／BRR素材を使い、外部ROMやゲーム音源を追加していません。
- 3本の既定checkerが固定SDKから成功し、SNESの10入力／SPCの15入力のハッシュをコンパイラー依存列と実ファイルへ照合しました。inline fixture／out指定は成功し、不正18形式をコンパイル前に拒否。指定fixtureを保持しました。
- 実Cargo archiveの6 object、35個のC ABI exportの各1回の定義、SDK側headerの解決と実入力の監視を確認しました。共有S-DSPを追加せず、旧スナップショットの個別license／対応ソースresource監視は保持しました。

`npm test`はPython 22件・Node 31件成功、Rustは723成功・0失敗・4件ignoredでした。
MCPのTCP bindのため初めから制限外で実行し、sandbox内失敗はありません。
22 mode・604,032フレーム・88 Run／Stop、3混合構成・7単独音源と受入れ／16台制限も成功し、
17台目を拒否して両制限試験のStop後予約予算が0へ戻りました。両runnerはexit code 0、`deviceOpened: false`です。
直前の全5構成成功run `40168462256`はDX7も今回のSNES／SPC移行も含みません。
対応ソースZIPの照合後にcommit／pushし、DX7を含む5構成CIでも確認しました。結果は次節に記録します。
新しいタグやreleaseは作成しません。実機聴音・UI操作、インストール後の起動とリアルタイム期限は別の確認です。

## DX7／SNES／SPC移行後の5環境CI — 全5構成成功

[Build Desktop — run 40171879716](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40171879716)で、
DX7とSNES／SPC移行を含む`8b9e922ad6e9c1f8d33cf5719b05330b58311944`をmainから手動実行しました。
attempt 1、2026-10-25 07:12:01 UTC開始、07:26:29 UTC完了。runは`completed`／`success`で、
全5構成の検証・インストーラービルドとアップロードが成功しました。CIのための製品・試験修正は不要でした。

| 構成 | 自動試験・ソース照合 | インストーラービルド |
| --- | --- | --- |
| macOS ARM64 | 成功 | 成功（DMG） |
| macOS x86_64 | 成功 | 成功（DMG） |
| Windows x86_64 | 成功 | 成功（NSIS） |
| Linux ARM64 | 成功 | 成功（DEB／AppImage） |
| Linux x86_64 | 成功 | 成功（DEB／AppImage） |

全5構成のログでPython 22件・Node 31件が成功しました。Rust全試験はmacOS／Linux各構成が723成功・0失敗・4件ignored、Windowsが726成功・0失敗・3件ignoredでした。
全5構成のNative検証JSONを取得し、次を確認しました。

- SDK374ファイルを含む22件の固定記録と、対応ソースZIP全1,796ファイルの照合成功。
- 22 mode・604,032フレーム・88 Run／Stopと、3混合構成・7単独音源の成功。
- 追加生成の受入れ／16台制限・17台目拒否が成功し、両制限試験のStop後予約予算が0へ戻ること。
- 再生／混合runnerがともにexit code 0、`deviceOpened: false`であること。
- 取得した検証レポートZIP全5組のサイズとGitHub artifact digestが一致すること。

検証5組とインストーラー5組の計10成果物は対象run／commitと一致し、有効期限内・非空でした。
全5ジョブのドラフトリリース添付stepはskipped。アプリ版は`0.90.1`のままで、新しいタグ・ドラフトリリース・一般公開は行っていません。
CIの対応ソースZIP SHA-256はmacOS／Linuxが`39cd14b113557665c039d0af3960839ed566c7ad2fdc49286859d8b5c53fa684`、
Windowsが`f3ac24c3a08277ebcdf3651dce8f2620ebae6dc05bc149fac4de48cb89fb19e3`でした。
各CI内の照合は成功しており、ハッシュ差の原因は断定しません。SDK pinとfrontend archiveは全構成で一致しています。

このCIはDX7とSNES／SPCの移行を含みます。先行するrun `40168462256`には含まれません。
CI後の変更は結果記録だけで、製品・試験入力は同じです。
上記のローカル273比較／519比較は各OSのCIで実行した比較ではありません。
検証レポートだけを取得し、インストーラーや対応ソースZIP自体の取得・独立照合・起動、OS間のPCM一致、実機聴音・UI操作は行っていません。

## Nuked YM2612移行の検証

2026-10-25。代替Nukedエンジンも固定SDK参照に切り替えました。
SDK `0.1.1`は`tetorica::nuked_ym2612`と`<tetorica/nuked_ym2612.hpp>`を提供し、
通常ymfm／Sega PSGと同一実行ファイルへリンクできます。TauriはそのCMakeターゲットを追加リンクせず、
上記のSDKソースを既存の2 archiveへ組み込んでいます。
新しい379ファイルの固定配布物を取り込み、ソース・manifest・取り込み記録のハッシュを照合しました。
従来のSDK `0.1.0`の374ファイルを使った検証は、各先行節の当時の結果として残しています。

macOS ARM64／Clang・Nodeで選択中222入力・1,363,968フレーム、独立追加198入力・1,216,512フレームを比較し、
旧／SDK stdoutは全入力でbyte一致しました。機種混合とraw独立接続は異なるAPI profileなので、
同一符号化命令列39件を同一入力へまとめず、合計420入力・2,580,480フレームです。
実配布WASMとの比較も420回で、独立追加の差は0、選択中は既存のFloat32 ramp差
`2.9802322387695312e-8`を保持しました。意図したDSP変更はありません。
既存fixtureの自己試験はStop無音・再発音、6CH・CSM／DAC・mute・公開状態とcallback allocation／free 0を確認し、
prefix済みコア8 exportの各1回の定義と非prefix定義0も確認しました。
毎frameのprivate全状態のdump比較ではありません。

SDKのソースZIPを独立した一時ディレクトリへ展開し、全24 CTest・18 WAV exampleとJavaScript参照10試験が成功しました。
これはローカルの独立ビルドで、音声デバイスや元checkoutを必要としません。
Tauriの`npm test`もPython 22件・Node 31件成功しました。このツール試験はSDK最終取り込み前に実行し、
合成fixtureを使うため実際の同梱receiptには依存しません。
2本の既定checkerも初回取り込み後のSDK入力から成功し、選択中12入力／独立追加8入力のハッシュを記録しました。
比較ツールもSDKのLinux監査用`native/tests/nuked_audit.cpp`を使う形へ揃え、最終記録はshimを含む13／9入力です。
同じ222／198ケースと自己試験の再実行も成功し、合計420ケースへ重複加算していません。
実Cargo archive各1 objectのDWARFはSDK接続cpp／コアCを指し、10 raw C ABI・16独立C ABI・8 prefix済みOPN2 exportは各1回、非prefix定義は0でした。
独立した依存preprocessで6音源入力がSDK内だけに解決され、実入力監視と既存リンク順序も確認しました。

Rust全試験は723成功・0失敗・明示実行用4件ignored。MCPのローカルTCP bindのため初めから制限外で実行し、sandbox内の失敗はない。
再生ライフサイクルは22 mode・604,032フレーム・88 Run／Stopが成功した。
既存の追加音源3混合構成・7単独音源、受入れ／16台制限と17台目拒否も成功し、両制限試験のStop後予約予算は0へ戻った。
再生／混合runnerはともにexit code 0、`deviceOpened: false`。
この既存混合表に独立追加Nukedは含まれず、Nukedは別のPCM比較・同時リンクfixture・Rust試験と選択中modeの再生試験で確認した。
これらのホスト試験はSDK `60f06e629395b65f1777e7e28c80be9eefca2381`／378ファイルの初回取り込み後に実行した。
初回取り込みから最終配布への差は`RELEASE-MANIFEST.json`、`docs/verification.md`、`native/CMakeLists.txt`と新しい試験用`native/tests/nuked_audit.cpp`の4ファイルだけだった。
実際にTauriが使うコア・接続入力の全バイト列が一致することを`final-import-audit.json`で確認したため、完了済みのホスト試験を再実行していない。
最終ソースZIPを独立展開した全24 CTest・18 WAV exampleとJavaScript参照10試験は改めて成功し、検証scriptのexit codeは0だった。

SDKの初回CI run `40174420834`はLinux 2構成で試験用allocation macroと標準ヘッダーのinclude順によりコンパイルが失敗しました。
SDKの試験専用shimで標準ヘッダーを先に読み、製品コア・接続を変えずに修正しました。
修正コミット`e4b7d7ecef60e3927ac04815cd9f985f62c82f04`の[SDK CI run 40175263453](https://github.com/kyorohiro/tetorica-soundchip-native/actions/runs/40175263453)は全6ジョブ成功しました。
Native 5構成それぞれで19ライブラリー・24 CTest・18 WAV example・Python 51件・JS参照が成功し、診断ZIP 5個のGitHub digestと元ソースZIPのSHA一致を確認しました。WASMは通常ymfm YM2612のビルドと抽出ソース再ビルドの同期ABI検証で、新規Nuked WASMのビルドではありません。

Nuked移行を含む[Tauri CI run 40175615215](https://github.com/kyorohiro/tetorica-fm2612-playground-tauri/actions/runs/40175615215)はattempt 1で`completed`／`success`になりました。
対象は`main`の`367fd7a851ffadd3a01c3cfb985972112156ca63`。Windows x64、Linux x64／ARM64、macOS Intel／ARM64の全5構成で試験とインストーラー生成・アップロードが成功しました。
各ジョブログでPython 22件・Node 31件、CargoはWindows 726成功／3 ignored、ほか4構成723成功／4 ignoredを確認しました。
全5構成の検証JSONでSDK379ファイル・22件の固定記録・対応ソースZIP1,801ファイル、22 mode・604,032フレーム・88 Run／Stopと、3混合構成・7単独音源・受入れ／16台制限・Stop後予算0が成功しました。両runnerはexit code 0、`deviceOpened: false`です。
計10アーティファクトのrun／commit一致・非空・有効期限内と検証ZIP 5個のGitHub digestを確認し、タグ用のドラフトリリース添付stepは全5構成skippedでした。
CI内のソースZIP SHAはWindowsとほか4構成で異なりますが、各CIの内容照合は成功しています。原因は断定せず、ZIP自己ハッシュと検証summaryのハッシュは本体側のNuked作業記録へ残します。
CI対象からのTauri差分はこの結果記録ガイドだけで、製品・試験入力は変更していません。インストーラー／ソースZIPそのものの取得・独立検証・起動は、このレポート監査には含めません。
直前の全5構成成功run `40171879716`は今回のNuked移行を含みません。
実機聴音・UI操作、インストール後の起動、OS間PCM一致、各OSのWASM比較は別の確認です。

## SDK 0.1.2 — Sega PSGの独立配置

2026-10-26。固定ソースZIPをSDK `0.1.2`へ更新し、Sega PSGコアを
`vendor/tetorica_sega_psg/`から一度だけコンパイルする形へ切り替えました。
Game Gear／Master System／Genesisのinclude探索を同じフォルダーへ揃えています。
共有状態helperは`vendor/ymfm/chip_state.h`を維持します。
SDK内の独立したBSD-3-Clauseライセンスをインストーラーの
`tetorica-segapsg-LICENSE.txt`にも収録します。従来のymfmライセンスも保持します。

取り込みはソースZIPの期待SHA-256と宣言した元コミットを指定する正式インポーターで行い、
同梱381ファイルとmanifestのハッシュをオフラインで照合しました。
旧SDKのC/C++ソース・ヘッダー268ファイルを比較し、移動したPSGの2ファイルに追加した
4行のBSD／SPDX表記以外は全バイト列が一致しました。DSP計算・C ABIの変更はありません。
古いNativeスナップショットと由来の記録は更新していません。

macOS ARM64で次の検証が成功しました。

- `npm test`: Python 22件、Node 31件。
- `cargo test --locked --offline`: 726成功、0失敗、明示実行用4件ignored。
- 既定のGenesis実Native／配布WASM比較: 3レート・6ケース・49,152ステレオフレーム。
  最大差は既存のFloat32丸めの範囲の`3.725290298461914e-9`でした。
- Native安定性: 22 mode、604,032フレーム、88 Run／Stop。
  固定／可変ブロックのPCM一致とStop後pending 0を確認しました。
- 追加音源の実Shared／Renderer: 3混合構成、7単独音源、受入れ／16台制限、
  Stop後予約予算0。両runnerはexit code 0、`deviceOpened: false`です。

対応ソースZIPは同梱SDKの新PSGソースとライセンスを再帰的に収録します。
既存の収録・照合試験に新配置のcpp／header／LICENSEを加えています。
これらはローカルのビルドとオフライン試験で、OSの音声出力や実機聴音を確認したものではありません。
Native 5構成のCI結果は別途記録します。


## SDK 0.1.4 — WonderSwanを収録した固定配布への更新

2026-10-27。公開済み`v0.1.4`のソースZIPをSHA-256照合後に取り込み、395ファイルをオフラインで検証しました。
配布manifest本体を除く比較では、旧`0.1.2`から14ファイルを追加し、既存の360ファイルはbyte一致、変更された20ファイルはカタログ・文書・ビルド設定・配布／検証スクリプトです。
既存の音源コア・Native接続・WASM bridge、アプリの`build.rs`・Native接続・Rust制御・Mixer値は変更していません。
追加されたWonderSwanのrawコア・公開header・fixture・WAV example・WASM bridgeをソース配布に保持しますが、
Cargoの接続、Native backendの対応mode、Stop・clock・ownerの制御は今回追加していません。
WonderSwanは引き続きWeb Audioで再生します。

ローカルmacOS ARM64で次を確認しました。

- `import_soundchip_sdk.py --check`と全22種類のNativeソース固定記録が成功。
- `npm test`のPython 22試験・Node 31試験が成功。
- Rust全試験は726成功・0失敗・明示実行用4件ignored。
  最初のsandbox内実行ではMCPのlocalhost TCP待受け2試験だけが権限拒否となり、
  ローカルTCPを許可した制限外の同一`--locked --offline`試験で全件成功しました。
- Genesisの実配布WASMとの比較は44.1／48／96kHz、6ケース・49,152ステレオフレームで成功。
  最大差は既存のFloat32丸めの範囲の`3.725290298461914e-9`。
- Native安定性は22 mode・604,032フレーム・88 Run／Stopが成功。
  固定／可変ブロックのPCM一致、Stop後pending 0、非有限PCM 0を確認しました。
- 追加音源の実Shared／Rendererは3混合構成・7単独音源、受入れ／16台制限・17台目拒否が成功。
  両制限試験のStop後予約予算は0、両runnerのexit codeは0、`deviceOpened: false`です。
- 全28種類のWASM再構築と生成記録を含むPlayground frontendを取り込み、918ファイルの照合と取り込み後の`npm test`も成功。
  frontend ZIPのSHA-256は`fd73d44799dfe7819dba99967331bb3a0d0fab4d545fae8e6ee20fc6b1a0cc54`です。

これらは音声デバイスを開かないローカル試験です。Windows／Linux／Intel MacのTauriビルドと実機聴音は今回の記録に含みません。
