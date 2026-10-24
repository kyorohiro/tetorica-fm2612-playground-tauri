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
文字入力ごとのプロジェクト全体コピーを行わず、750 ms入力が落ち着いた時にまとめて保存します。連続入力時も最大5秒で保存し、終了・再読み込み時は待機中の最新編集を直ちに保存します。
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

## Native音源SDK

YM2612、YM2203、YM2608、YM2610／YM2610Bは、自己完結した`tetorica-soundchip-native`のソース配布を
`src-tauri/vendor/tetorica-soundchip-native/`へ同梱してビルドします。
取り込み版・配布ZIPのSHA-256・元コミットは`src-tauri/soundchip-sdk.json`に固定し、
`python3 scripts/import_soundchip_sdk.py --check`で同梱ファイルを照合できます。
SDKのcheckout、submodule、ビルド時の音源ソース取得は不要です。

SDKから使うのはYM2612の既存低レベル接続、PC-98／追加生成のYM2203・YM2608接続、
Neo Geo／追加生成のYM2610・YM2610B共通接続と共有OPN providerです。
既に固定しているSDK `0.1.0`を使い、共通providerは一度だけ組み込みます。
PSG、Rustの予約・所有権管理、CPAL、ミキサー／FXとWebViewのAPIはアプリ側に残ります。
他の音源は従来の同梱スナップショットを使います。
固定した版と更新手順、共有providerの範囲は[Native SDKの取り込み](docs/native-sdk-ym2612.md)を参照してください。

## DX7

FILES → `examples/dx7/dx7-init-sine.js` を開いて Run すると、独立した MSFA DX7 音源で演奏できます。
`await useSoundChip('dx7')` は音源を再利用し、`await createSoundChip('dx7', {id:'pad'})` は独立した16音の音源を作ります。
Main／Worker の両方に対応し、音量・mixer・FX・STOP は既存の音声経路を使います。
初期音色は自作の INIT SINE です。音色バンクを FILES に読み込んだ場合は、
`await dx7.loadSysex(await file('/presets/my-bank.syx', {type:'arrayBuffer'}))` の完了後に発音してください。
`dx7.noteOn(60, 100)`／`dx7.noteOff(60)` はチャンネル引数を持たず、MIDI ノート番号を使います。
内部44,100 Hz固定、AudioContext 出力44,100／48,000 Hz対応で、`setClock()` はありません。
実装・上流の制約とライセンスは `dist/js/msfa_dx7/README.md` と `README.vendor.md` に記録しています。

## 音声出力

### 試験用Nativeエンジン

**安定するまではWeb Audioが既定です。** Menu → **Audio engine…** から
**Native (experimental)** を明示的に選べます。切替時は演奏を停止し、
編集中のプロジェクトを引き継いでページを開き直します。選択だけではデバイスを開かず、
RunまたはKeyboardでの試聴時に開始します。通常のアプリ起動はWeb Audioです。
Native選択中も、Genesis／Mega CD／32X／32X Nuked／NES＋FDS／DX7／Game Boy／SNES／PC Engine／PC-98（YM2203／YM2608）／Neo Geo／Neo Geo（YM2610B）／X68000／Game Gear／Master System／MSX／MSX Audio／DOS PC（OPL／OPL2／OPL3）の全標準Modeを選べます。
旧`genesis-nuked` URLでもNativeを選べます。Web Audioでは従来どおり全Modeを選べます。

NativeはGenesis modeのYM2612＋Sega PSG、Main／Worker実行、音色・レジスター操作、Keyboard、
DAC・サンプル・noise、組込みFX・JavaScriptの`liveFx`、Audioモニターと最終出力の録音に対応します。
ミキサー・音量・停止もNative側で処理します。既存のPlayground用C++音源を直接ビルドし、
CPALの音声コールバックで生成・変換・ミックスしてシステム既定の出力機器へ送ります。
AudioWorklet／WebSocketでPCMを転送する現行Audifyとは別の経路です。
Nativeを使う間はWeb Audio用のAudio outputメニューを表示しません。

Mega CDではRF5C164（8声・64 KiB RAM）もNativeで生成します。
`await useSoundChip('rf5c164')`で内蔵PCMを借り、既存の初期コード・サンプラーをMain／Workerで使えます。
KeyboardのChip／CH選択、`loadSample`、ミキサー、RFのclock変更に対応します。
RFの音も共通FX・最終出力monitor・録音へ入ります。StopはRAMと設定を保持して発音を止めます。
音声のファイル選択・マイク入力・編集は既存のWebView側、再生はNative側で行います。
Nativeコアの由来とBSD-3-Clauseライセンスは`src-tauri/native-megacd/`に保存し、ライセンスをアプリにも同梱します。

32Xでは既存のFull構成を引き継ぎ、YM2612＋Sega PSG＋RF5C164に加え、
3段FIFOとタイマーを持つPWMをNativeで生成します。
Browser／npmのWASMと同じC++コアを使い、`pwm`／`await useSoundChip('pwm')`、
Main／Workerのレジスター操作・frame指定の予約書込み・clock変更に対応します。
PWMも共通ミキサー・FX／liveFx・monitor・録音へ入ります。Stopは予約と保持中の出力を消し、
clock・routing・cycleを保持します。resetはレジスターも初期化します。
PWMの由来とBSD-3-Clauseライセンスは`src-tauri/native-pwm/`に保存しています。

32X NukedはYM2612部分をBrowser版と同じNuked OPN2でNative生成し、PSG／RF5C164／PWMは32Xと共通です。
旧Genesis Nuked URLではNuked OPN2＋PSGを使います。通常のGenesis／32Xは引き続きymfmです。
Main／Worker、Keyboard／Operators、DAC、共通ミキサー・FX／liveFx・monitor・録音へ接続します。
レジスターのaddress／data書込み後に24内部clockを進めるWASM版の挙動を維持します。
NukedはLGPL-2.1-or-laterで、由来・対応ソース・ライセンスを`src-tauri/native-nuked/`へ保存します。
配布アプリには`native-sources/application-sources.zip`も同梱し、静的リンクしたNuked／SNESを
変更して再ビルドできるようにします。手順と収録範囲は[NATIVE-SOURCES.md](NATIVE-SOURCES.md)を参照。

PC-98 modeでは、Web版と同じymfmのYM2608をNativeで生成します。
FM 6CH・SSG 3CH・リズム6音・ADPCM-Bを、`fm`／`ssg`／`rhythm`／`adpcm`と
`await useSoundChip('ym2608')`から扱えます。Main／Worker、Keyboard／Operators、
PC98サンプラー、共通FX／liveFx／monitor／録音へ接続します。
標準clockはWeb版と同じ8 MHzで、`await fm.setClock(hz)`で変更できます。
Stopは発音と予約を止め、音色・clock・8 KiBリズムROM／2 MiB ADPCM-B RAMを保持します。
適用済みの状態取得は非同期です。`createSoundChip('ym2608')`で独立した追加音源も使えます。
Native接続と共有OPN／SSG／ADPCMは同梱SDKからビルドします。
Browser比較用の原典・ハッシュと、Rustが読み込むリズムROM素材は`src-tauri/native-ym2608/`に保持します。

Neo GeoではYM2610（FM 4CH）、Neo Geo（YM2610B）では6CH版をNativeで生成します。
両方ともSSG・ADPCM-A 6声・ADPCM-Bに対応し、A/Bそれぞれに16 MiBの独立したROM領域を持ちます。
`fm`／`ssg`／`adpcmA`／`adpcmB`、Main／Worker、Keyboard／Operators、既存Neo Geoサンプラーを使えます。
`await useSoundChip('ym2610')`は選択中の音源を借り、6CH版では`useSoundChip('ym2610b')`も同じ音源を返します。
標準clockは8 MHzで変更可能です。Stopは音色・clock・ROM・サンプル範囲を保持します。
共通ミキサー・FX／liveFx・monitor・録音へ接続し、原典・ハッシュは`src-tauri/native-ym2610b/`に保存しています。
Native接続は両変種とも固定SDKからビルドします。`createSoundChip('ym2610')`／`createSoundChip('ym2610b')`で
独立した追加音源も使え、追加音源のROMはRust側で準備・保持する不変ページを参照します。

X68000ではYM2151（4 MHz・FM 8CH）とOKIM6258（8 MHz・ADPCM 1ストリーム）をNativeで生成します。
既存の`opm`／`adpcm`、`useSoundChip('ym2151')`／`useSoundChip('okim6258')`をMain／Workerから使えます。
Keyboard／Operatorsで音色・LFO・noiseを編集して試奏でき、OKIではサンプルの読込み・波形確認・試奏に対応します。
2チップを別々にミキサーへ登録し、共通FX／liveFx・monitor・録音を通ります。
StopはFM音色・KC/KF・clockとOKIのサンプル・pan／dividerを保持します。由来・ハッシュ・ライセンスは`src-tauri/native-x68000/`に保存します。

Master SystemではSega PSG（3音＋noise）とYM2413 OPLLを、両方3,579,545 HzでNative生成します。
`psg`／`opll`、Main／Worker、Keyboard／Operators、独立ミキサーに対応し、PSGは実機構成に合わせてモノラルです。
OPLLはROM楽器・共有User音色・9 FM声、または6 FM声＋5 rhythm音を使えます。
Stopは音色・PSGのperiod／noise設定・clockを保持し、発音と予約を止めます。
由来・ハッシュは`src-tauri/native-master-system/`に保存し、ymfmとSegaPSGは共有providerを一度だけ組み込みます。

PC-98（YM2203）では4 MHzのYM2203だけを生成し、FM 3CH＋SSG 3CHをモノラルで左右へ出力します。
`fm`／`ssg`／`play`、Main／Worker、6声のKeyboard／音源タブに対応します。
Stopは音色・clock・CH3特殊モード・SSGのtone／noise／envelopeと保持音量を残します。
SSGの明示的な音量0とkeyOffを区別し、Workerから表示へ渡す状態を音声命令として再送しません。
Native接続は固定したSDKからビルドし、Browser比較用の原典・ハッシュは`src-tauri/native-ym2203/`にも保持しています。

MSXではYM2413（3,579,545 Hz）とAY-3-8910（1,789,773 Hz）をNative生成します。
`opll`／`ay`、Main／Worker、Keyboard／Operators、独立ミキサーと共通FX／liveFx・monitor・録音を使えます。
AYのtone／noiseの混合、共有ノイズ・エンベロープ、明示的なエンベロープ再発音に対応します。
Stopは音色・clock・共有設定と発音復元用の音量を保持します。設定音量0もそのまま保持します。
YM2413はMaster Systemと共通の実装を使い、由来・ハッシュ・AYのBSDライセンスは`src-tauri/native-msx/`に保存しています。

MSX Audioでは既存MSX構成にY8950（3,579,545 Hz）のFM 9CH／リズムを加えます。
`opl`＝Y8950、`opll`＝YM2413、`ay`＝AY-3-8910として、Main／Worker、Keyboard／音源タブを使います。
3音源の音量とclockは独立し、Stopは全体を止めて音色・clock・AYの音量／envelope設定を保持します。
Y8950は現行Playgroundと同じFM／リズムを対象とし、ADPCMメモリーAPIは追加していません。
原典・ハッシュは`src-tauri/native-msx-audio/`に保存しています。

Game GearではSega PSGだけをNative生成し、3 tone＋noiseの4チャンネルと左右独立のステレオマスクを使います。
`psg.setStereo()`、Main／Worker、Keyboard／音源タブに対応します。
Stopはperiod・noise・clock・最後の可聴音量・左右マスクを保持し、resetで左右マスクを`0xff`へ戻します。
PSG処理はMaster Systemと共有し、原典・ハッシュは`src-tauri/native-game-gear/`に保存します。

DOS PCではYM3526／YM3812（3,579,545 Hz）またはYMF262（14,318,180 Hz）を1チップだけNative生成します。
`opl`、Main／Worker、Keyboard／Operators、9／18 FMチャンネル、OPL3の6組の4オペレーター音色に対応します。
OPL／OPL2はモノラル、OPL3は左右出力を使います。全モデルでコードと音源タブからリズムを操作できます。
Stopは音色・clock・LFO・出力設定・4op構成を保持して発音と予約を止め、通常のkeyOffはリリースを残します。
原典・ハッシュは`src-tauri/native-opl/`に保存し、既存ymfm providerを共有します。

DX7 modeではMSFAのC++音源を直接使い、16音ポリフォニーで生成します。
`dx7`／`useSoundChip('dx7')`、音色・SysEx、オペレーター・feedback編集、Keyboard／Operatorsに対応します。
Main／Workerとも共通FX・liveFx・Audioモニター・最終出力録音を通ります。
Stopは音色バンクと選択を保持して発音を止めます。YM2612／PSGは生成しません。
内部44,100 Hz固定で出力機器のsample rateへ変換します。`createSoundChip('dx7')`による独立音源の追加にも対応します。
由来とApache-2.0ライセンスは`src-tauri/native-dx7/`に保存し、ライセンスをアプリにも同梱します。

NES＋FDS modeでは、現在のJSNES APUとFDSのJS実装をC++へ移植した音源を使います。
Pulse 2声、Triangle、Noise、DMC、FDSの6CH、DMC用64 KiB RAM、FDS波形・変調、clock変更に対応します。
Main／Workerの`nes`／`useSoundChip('nes')`、KeyboardのChip／CH選択、Operatorsの編集・試聴を使えます。
StopはRAM・波形・変調表・設定・clockを保持し、resetはRAMと波形も初期化します。
音は共通FX／liveFx・monitor・録音へ入り、YM2612／PSG／DX7は生成しません。
VGM取込みは基準clockが1,789,773 Hzのときに主音源を借用します。PALや別の基準clockのVGMはWeb Audioを使ってください。
Browser版も同じC++音源のWASMを使います。Nativeコア・原典・ハッシュは`src-tauri/native-nes/`に保存し、
JSNESのApache-2.0、FDSのMITライセンスとJSNES著作者情報をアプリへ同梱します。

Game Boy／PC Engineでは、Browser版と同じMAME由来のC++音源を直接使います。
`gameboy`／`pce`と`useSoundChip('gameboy'/'huc6280')`、Main／Worker、KeyboardのChip／CH選択、
Operatorsの音色・波形編集／試聴、clock変更に対応します。Stopは波形・音色・clockを保持します。

SNESではS-DSPをNativeで動かし、8声のBRR楽器・サンプルRAM・ADSR／GAIN・noise・echoを使えます。
`snes`／`useSoundChip('snes')`、BRRアップロード、Main／Worker、Keyboard／Operators、
キャプチャ済みDSP／RAM timelineの準備・再生・停止・状態取得に対応します。
StopはBRR・RAMと設定を保持して発音を止めます。SPC700 CPUを実行するファイルプレーヤーは、別の`createSoundChip('spc')`で追加できます。
3モードとも共通ミキサー・FX／liveFx・最終出力monitor／録音へ接続し、Genesis音源は生成しません。
Game Boy／HuC6280のBSDライセンス、SNES_SPCのLGPL-2.1とソース・ビルド手順を同梱しています。

Nativeの`createSoundChip()`はPlaygroundの公開21種類に対応し、合計16台・64 MiBの管理予算で独立した音源を追加できます。
Mode内蔵音源を借りる`useSoundChip()`とは別に、個別のMixer ID・Keyboard・音色情報・破棄を扱います。
Genesis／Mega CD／32X系の新しいCassetteにある`examples/genesis/mixed-chip-stability.js`は、
独立したYM2612・RF5C164・DX7を混ぜる40秒の反復曲です。同じコードでWeb Audio／Native、Main／Workerを比較できます。
Keyboardは曲と別のFM CH2–6を使います。Mixerの`scene-fm`／`scene-pcm`／`scene-dx7`でSolo／Muteして試聴できます。

`liveFx`は既存の`custom_fx.js`をQuickJS-NGで実行します。
使用中だけNativeの専用workerで処理し、2048フレーム（48kHzで約42.7ms）の遅延が加わります。
初回登録までworker・VMを作らず、未使用時は直接出力します。最後の削除でVMを解放します。
音声callbackはJSの完了を待たず、処理が間に合わない場合は同じ時刻のdry音へ戻します。
Audioモニターと録音は処理後の最終出力です。FX MonitorのFX単位表示は未対応です。
processは同期関数に限り、外側の変数やDOM・タイマーを使えません。`sampleRate`は利用できます。
contextには循環参照・TypedArray・Map等も渡せますが、Error・Blob／File・SharedArrayBuffer等は未対応です。
context graphは128 KiB、ソースは192 KiB、命令全体は256 KiB、名前は128 UTF-8 bytesまでです。
通常の例外は該当FXをbypassして表示します。VMの実行制限で継続不能になった場合はStop／Runで再試行します。
由来とMITライセンスは`src-tauri/native-livefx/`に保存し、ライセンスをアプリにも同梱します。
共有ソースとエンジンの照合・デバイスを使わないRustテストは次で実行できます。

```sh
python3 scripts/sync_native_genesis_livefx.py --check --source ../..
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml native_live_fx_probe
```

`noise.create()`は共通C DSPのwhite／pink／brown／gray／clipを32声まで生成します。
filter・ramp・attack／releaseに対応し、loop取消しではrelease後に領域を回収します。
圧縮サンプルは読込み時だけブラウザーのofflineデコーダーを使い、Native側で再生します。
stream、FM／PSGのchip clock変更、独立した音色ファイルの試聴、
PSGの適用済み状態表示は未対応です。
対応するWeb Audioへ設定で切り戻してください。FMの編集情報はJS側の設定値であり、
Native内部のエンベロープ等の観測ではありません。
Keyboardは物理CHを共有し、scriptの未来の書込みが手動演奏を上書きする場合があるため、
演奏中は空いているCHを選んでください。Native開始失敗は表示し、自動で別エンジンを鳴らしません。

デバイスを開かない検証:

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml native_genesis
python3 scripts/import_soundchip_sdk.py --check
node scripts/check_native_genesis_core.mjs ../..
node scripts/check_native_genesis_noise.mjs ../..
python3 scripts/sync_native_genesis.py --check --source ../..
python3 scripts/sync_native_genesis_noise.py --check --source ../..
```

全Modeの反復・長時間生成と、同梱ソースの照合も音声デバイスなしで実行できます。
ソースZIPはcheckoutが変わるたびに再生成します。親checkoutなしでも原典の照合は可能です。

```sh
python3 scripts/prepare_native_sources.py
python3 scripts/check_native_sources.py --bundle --out=native-validation/sources.json
node scripts/check_native_stability.mjs --out=native-validation/stability.json
node scripts/check_native_stability.mjs --long --out=native-validation/long.json
node scripts/check_native_extra_stability.mjs --out=native-validation/extra-stability.json
node scripts/check_native_extra_stability.mjs --long --out=native-validation/extra-stability-long.json
```

短時間版は標準21 Mode＋旧Genesis Nukedの生成・破棄、可変callback、所有者取消し、
Stop／再開、未来命令・保持参照の回収を確認します。長時間版は各演奏30秒を3周し、
処理時間と実時間に対する生成速度をJSONへ記録します。固定の速度閾値は設けません。
GitHub Actionsの5環境でも原典・ソースZIPと短時間版を照合し、失敗時もJSONを保存します。
これらの結果は物理音声デバイスの長時間安定性や聴感上のノイズを保証するものではありません。

追加音源の試験は実RustのShared／Rendererと混在ラックを使います。FM／PCM／DX7、SNES／SPC、
YM2608／YM2610の転送を含む3構成（代表7種類）で、4,096フレームの固定／可変callbackのPCM bit一致、
owner取消し、破棄と再生成、Stop後の保持参照・予算の回収を確認します。
16台の制限、トランザクション失敗の巻き戻し、音源別と制限前の混合信号のpeak／RMS／AC-RMSも記録します。
長時間版は各30秒を2 Run×3周で継続生成します。予約bytesはNativeの管理対象であり、プロセスのRSSではありません。
試験用音色の音量値から、共通のデフォルト音量を自動変更することはありません。

親のBrowser/WASM checkoutがある場合、同じ命令列を継続して生成・比較できます。
次はFM単独／FM＋PSGを各60秒、44.1／48／96 kHzで比較します。

```sh
node scripts/check_native_genesis_core.mjs ../.. --seconds=60 --out=native-validation/web-native.json
```

Menu → Audio diagnosticsのNative録音JSONには、開始・終了近くのキュー状態、遅延命令、
overflow、liveFx under／overrunと差分も保存します。通常再生時のpollingや時計の再設定は追加しません。
状態読取りが失敗・timeoutになっても正常な録音は保存できます。これらは機器のdropout数とは区別してください。

機器を無音で開いて時計・停止・解放だけ確認する場合は、明示的に実行します。
音符の発音・録音・出力機器の変更は行いません。

```sh
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml native_genesis::tests::native_device_silent_smoke -- --ignored --exact --nocapture --test-threads=1
```

C++のスナップショットは各`src-tauri/native-*/`と同梱SDKに由来・ハッシュを記録しています。
SDKへ移したOPN系音源の更新は[SDKの取り込み手順](docs/native-sdk-ym2612.md)で行い、
`sync_native_genesis.py`は残している従来スナップショットの照合・更新に使います。
共有UI／Runtimeの変更は上流のPlayground ZIPを通常のimport手順で取り込みます。

### Web Audioの出力先

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

接続先は **MCP connection… に表示されたURL**、認証は `Authorization: Bearer <表示されたトークン>` です。初回は `http://127.0.0.1:39127/mcp` を使い、使用中の場合は空いているポートを自動で割り当てます。割り当てたポートはアプリデータディレクトリの `mcp-port` に保存し、次回のOff/Onや再起動でも優先して使います。保存済みポートが使用中なら別の空きポートへ切り替わるため、そのときは接続画面のURLをクライアント設定へ反映してください。
接続画面のJSONは `mcpServers` 形式の例です。クライアントによって設定形式が異なるため、URLとHTTPヘッダーをそれぞれ指定してください。
トークンは初回起動時に生成し、Tauriのアプリデータディレクトリの `mcp-token` に保存します（Unixでは所有者のみ読み書き可能）。MCPのOff/Onやアプリ再起動でも同じトークンを使います。接続ポートが変わらなければクライアントへ再登録する必要はありません。MCP自体は引き続き起動時Offです。

再発行する場合はMCPをOffにして **Menu → Regenerate MCP token** を選びます。確認後に新しい接続情報が表示され、古いトークンは使えなくなります。トークン再発行・ポート変更の際はクライアント設定を更新してください。
ポート競合以外の起動エラーはMenuに表示します。通信は実際に確保したループバックアドレス・ポートに限定し、異なるHostやブラウザーOrigin付き要求は引き続き拒否します。

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

## Cassette Git（開発中）

ローカル Git は browser と Tauri で同じ JavaScript 実装（isomorphic-git）を使用します。
`/.git` は Virtual Files に保存され、Export Cassette に履歴・ステージ・接続先が含まれます。
Import Cassette はその履歴を復元し、New Cassette は編集ファイルと履歴を消します。
通常の Git 操作で実フォルダを選ぶ必要はありません。

Git 操作は Shell のコマンドから行います。Menu → **Git 設定…** はアプリ共通のトークン設定だけを扱います。

```sh
git init
git config user.name "Your Name"
git config user.email "you@example.com"
git add index.js
git commit -m "First commit"
git status .
git status --short
git diff index.js
git diff --cached
git log
git branch -a
```

エディタの変更は Git 操作前に Virtual Files へ反映します。`git diff` は未ステージの変更、
`git diff --cached` はステージ済みの変更です。`git add .` はプロジェクト全体を対象にします。
`/.git` と `/sys` はステージ対象に含みません。
名前・メールはリポジトリ設定として Cassette に保存します。

Tauri は HTTPS の remote 操作を native Rust に任せます。Node.js や外部 CORS proxy は不要です。
旧 Rust Git／SSH 実装は削除済みで、libgit2／libssh2 はビルド・同梱しません。

```sh
git remote add origin https://github.com/owner/repository.git
git remote -v
git push origin main
git fetch
git pull
```

Menu → **Git 設定…** で名前付きのトークンを管理します。

1. **登録名**（例 `GitHub用`）と **接続先ホスト**（例 `https://github.com`）を入力します。
2. **トークンを登録…** を押し、別の native secure input に PAT を貼り付けます。
3. Shell から `git push` を実行し、native 確認画面で保存済みトークンを選びます。
   前回使ったトークンを初期値にし、接続先ホストが一致するものだけ表示します。

登録と前回の選択はアプリ共通で、New/Import Cassette や再起動でも保持します。
設定画面は登録と削除のみです。PAT を入れ替える場合は削除して登録し直します。
削除は GitHub 側での失効操作ではありません。削除時は該当 token の active session を破棄します。
トークンは JavaScript／IPC／Cassette へ返さず、ファイルには名前・ID・ホストとアプリの選択 ID だけ保存します。
PAT 本文は macOS Keychain、Windows Credential Manager、Linux Secret Service に保存します。
入力・選択画面は AppKit／Win32／GTK で開き、PAT を WebView に渡しません。
Linux は GNOME Keyring や Secret Service 対応の KDE Wallet が動作し、保管庫が解除されている必要があります。
保管庫が使えない場合はエラーを表示します。平文ファイルへの保存には切り替えません。
Browser 版はローカル操作のみ対応します。Git 設定画面には Git の操作ボタンはありません。
以前の repository selection 形式は、トークンを保持したまま app selection へ移行します。

通信先を native で確認したリポジトリと Git HTTPS endpoint に限定し、redirect は拒否します。
トークンを登録したホストとは異なるホストへ送信しません。
Push の送信先・branch・commit・トークン名は native ダイアログで確認します。

Cassette は ZIP／展開後とも 16 MiB 上限です。Git のメモリ FS は通常ファイルのみ対応し、
symlink、実行ビットの保存、submodule、Git LFS は未対応です。
Pull は変更のない作業状態での fast-forward のみ対応します。
Git 操作は隔離した snapshot 上で実行し、キャンセルや編集中の変更があれば古い内容を反映しません。
リモートで完了した push はキャンセルしても取り消されません。

検証: root repository の `npm run test:virtual-files` と
`node scripts/check_virtual_git_browser.cjs`、native の `cargo test`。
HTTPS＋トークンでの GitHub push はユーザー環境で確認済みです。
保存済みトークン方式は macOS Keychain の実 API と protocol fixture で検証しています。
Windows は資格情報ストア・Win32 ダイアログの対象モジュールをクロスコンパイルで確認していますが、実機確認は未実施です。
Linux は Docker 内の GNOME Keyring／GTK で、保存・読み出し・削除、伏字入力、選択、キャンセル、画面の破棄と保管庫未接続時のエラーを検証しています。
新しい方式での実 GitHub push の再確認は別途必要です。

保管庫へ一時的なダミー項目を書いて削除するテスト：

```sh
cargo test --manifest-path src-tauri/Cargo.toml os_credential_round_trip -- --ignored
```

Linux の native 入力・選択・キャンセル画面テスト（GTK display が必要）：

```sh
xvfb-run -a cargo test --manifest-path src-tauri/Cargo.toml native_input_selection_cancel_and_cleanup -- --ignored --test-threads=1
```

Git／HTTPS の依存ライセンスはアプリの `git-licenses.txt`、`git-js-licenses.txt` に同梱します。
