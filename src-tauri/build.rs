fn native_okim6258_source() -> std::path::PathBuf {
    // Keep the SDK snapshot immutable. Preserve the existing Native-only
    // thread-safe table initialization in the generated compilation input.
    const ORIGINAL: &str = "Oki6258::Oki6258(uint32_t clock, uint8_t flags, uint32_t rate):initial_clock(clock),initial_flags(flags),rate(rate) { compute_tables();reset(); }";
    const NATIVE: &str = "Oki6258::Oki6258(uint32_t clock, uint8_t flags, uint32_t rate):initial_clock(clock),initial_flags(flags),rate(rate) { static const bool initialized = []{compute_tables();return true;}();(void)initialized;reset(); }";
    let source = std::fs::read_to_string(
        "vendor/tetorica-soundchip-native/vendor/mame-okim6258/okim6258.cpp",
    )
    .expect("read pinned SDK OKIM6258 source");
    assert_eq!(
        source.matches(ORIGINAL).count(),
        1,
        "SDK OKIM6258 constructor changed; review the Native table-init adaptation"
    );
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"))
        .join("tetorica_sdk_okim6258.cpp");
    std::fs::write(&output, source.replacen(ORIGINAL, NATIVE, 1))
        .expect("write Native OKIM6258 compilation input");
    output
}

fn native_dx7_sources() -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    const SOURCES: &[&str] = &[
        "dx7note.cc",
        "env.cc",
        "exp2.cc",
        "fm_core.cc",
        "fm_op_kernel.cc",
        "freqlut.cc",
        "lfo.cc",
        "patch.cc",
        "pitchenv.cc",
        "resofilter.cc",
        "ringbuffer.cc",
        "sin.cc",
        "synth_unit.cc",
    ];
    const HEADERS: &[&str] = &[
        "aligned_buf.h",
        "controllers.h",
        "dx7note.h",
        "env.h",
        "exp2.h",
        "fm_core.h",
        "fm_op_kernel.h",
        "freqlut.h",
        "lfo.h",
        "module.h",
        "patch.h",
        "pitchenv.h",
        "resofilter.h",
        "ringbuffer.h",
        "sin.h",
        "synth.h",
        "synth_unit.h",
    ];
    const OVERRIDES: &[&str] = &[
        "dx7note.cc",
        "env.cc",
        "ringbuffer.cc",
        "synth.h",
        "synth_unit.cc",
        "synth_unit.h",
    ];
    let sdk = std::path::Path::new("vendor/tetorica-soundchip-native");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"))
        .join("tetorica_sdk_dx7_msfa");
    std::fs::create_dir_all(&output).expect("create Native DX7 compilation directory");
    // MSFA uses quoted includes: every .cc must sit beside the matching Native
    // headers. Mirror the SDK's copy-only assembly without changing its pin.
    for name in SOURCES.iter().chain(HEADERS.iter()) {
        let directory = if OVERRIDES.contains(name) {
            "native/reference/overrides/msfa"
        } else {
            "vendor/msfa"
        };
        let source = sdk.join(directory).join(name);
        std::fs::copy(&source, output.join(name))
            .unwrap_or_else(|error| panic!("copy pinned SDK DX7 {}: {error}", source.display()));
    }
    let sources = SOURCES.iter().map(|name| output.join(name)).collect();
    (output, sources)
}

fn main() {
    // Native C++ wrappers catch allocation failures before returning across Rust FFI.
    // Enable standard exception unwinding for all MSVC C++ translation units.
    println!("cargo:rerun-if-changed=native-genesis");
    println!("cargo:rerun-if-changed=native-fx");
    println!("cargo:rerun-if-changed=native-noise");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.h");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.h");
    println!("cargo:rerun-if-changed=src/native_genesis_core.cpp");
    // This pinned source tree is included in the application repository. Cargo
    // builds it directly, without fetching another checkout or invoking CMake.
    println!("cargo:rerun-if-changed=soundchip-sdk.json");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/ymfm");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/tetorica_sega_psg");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/wasm");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2612.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2612.h");
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/src/ymfm_opn_provider.cpp"
    );
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/src/ymfm_opm_provider.cpp"
    );
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/src/ymfm_opl_provider.cpp"
    );
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/nuked-opn2");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nuked.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nuked_ym2612.inc");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/mame-rf5c164");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_rf5c164.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_rf5c164.h");
    println!("cargo:rerun-if-changed=native-livefx");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.h");
    // Rust still embeds the selected/additional rhythm ROM from this snapshot.
    println!("cargo:rerun-if-changed=native-ym2608");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2608.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2608.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2203.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2203.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2610b.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2610b.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/mame-okim6258");
    println!("cargo:rerun-if-changed=src/native_genesis_x68000.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.h");
    println!("cargo:rerun-if-changed=src/native_genesis_x68000.h");
    println!("cargo:rerun-if-changed=src/native_genesis_master_system.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_master_system.h");
    println!("cargo:rerun-if-changed=src/native_genesis_psg.h");
    println!("cargo:rerun-if-changed=native-game-gear");
    println!("cargo:rerun-if-changed=src/native_genesis_game_gear.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_game_gear.h");
    println!("cargo:rerun-if-changed=src/native_genesis_opll.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/mame-ay8910");
    println!("cargo:rerun-if-changed=src/native_genesis_msx.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2413.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2413.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ay8910.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ay8910.h");
    println!("cargo:rerun-if-changed=src/native_genesis_msx.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_opl.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_okim6258.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_okim6258.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_rf5c164.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_rf5c164.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_opl.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_opl.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_opl.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_y8950_memory.h");
    // Both the independent YM2612 and primary Genesis YM2612 now consume SDK
    // sources. Their common ymfm provider is compiled only once below.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2612.cpp")
        .warnings(false)
        .compile("tetorica_native_ym2612");
    // Standalone Game Gear keeps its application-owned NativePsgCore adapter
    // and consumes the pinned, once-compiled common Sega PSG below.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/tetorica_sega_psg")
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("native-genesis")
        .file("src/native_genesis_game_gear.cpp")
        .warnings(false)
        .compile("tetorica_native_game_gear");
    // Selected DOS/MSX Audio, additional OPL and additional RF5C164 adapters
    // use the pinned SDK. The shared OPL/PCM provider follows with Master
    // System/MSX on GNU ld; the independent RF renderer stays in this archive.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("native-genesis")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_opl.cpp")
        .file(
            "vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_opl.cpp",
        )
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_rf5c164.cpp")
        .warnings(false)
        .compile("tetorica_native_opl");
    // MSX retains its machine adapter and shares the SDK AY core with the
    // independent SDK AY adapter. GNU ld requires this archive before the
    // Master System OPLL provider and the common ymfm archive.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("vendor/tetorica-soundchip-native/vendor/mame-ay8910")
        .include("native-genesis")
        .file("src/native_genesis_msx.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ay8910.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/mame-ay8910/ay8910.cpp")
        .warnings(false)
        .compile("tetorica_native_msx");
    // Machine adapters retain their local NativeOpllCore; independent YM2413
    // and the once-compiled OPL/PCM provider come from the pinned SDK. Shared
    // ADPCM and Sega PSG remain in the following common archive on GNU ld.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/tetorica_sega_psg")
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("native-genesis")
        .file("src/native_genesis_master_system.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2413.cpp")
        .file("vendor/tetorica-soundchip-native/native/src/ymfm_opl_provider.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/ymfm/ymfm_pcm.cpp")
        .warnings(false)
        .compile("tetorica_native_master_system");
    // X68000 mixing stays application-owned. Its FM and the independent YM2151
    // share the SDK's OPM provider. Both OKI adapters share one SDK-derived
    // decoder with the existing Native table-init fix, generated in OUT_DIR.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("vendor/tetorica-soundchip-native/vendor/mame-okim6258")
        .include("native-genesis")
        .file("src/native_genesis_x68000.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_extra_okim6258.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.cpp")
        .file("vendor/tetorica-soundchip-native/native/src/ymfm_opm_provider.cpp")
        .file(native_okim6258_source())
        .warnings(false)
        .compile("tetorica_native_x68000");
    // Selected Neo Geo and independent YM2610/YM2610B share the pinned SDK
    // wrapper, retaining the common OPN provider after this archive on GNU ld.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2610b.cpp")
        .warnings(false)
        .compile("tetorica_native_ym2610b");
    // Selected PC-98 and independent additional YM2203 use the pinned SDK
    // wrapper and the same once-compiled OPN/SSG provider below.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2203.cpp")
        .warnings(false)
        .compile("tetorica_native_ym2203");
    // Selected PC-98 and independent additional YM2608 use the pinned SDK
    // wrapper. GNU ld requires the shared ymfm provider after this consumer.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2608.cpp")
        .warnings(false)
        .compile("tetorica_native_ym2608");
    // Keep the application's Genesis mixing, DC centering and gains. Its
    // YM2612 and shared Sega PSG C ABI/decoder consume the pinned SDK, together
    // with the common OPN/SSG/ADPCM provider. Existing OPN, OPM and OPL consumers
    // use identical ymfm types; preserve this once-compiled archive and GNU ld
    // ordering for Master System, Game Gear and additional PSG consumers.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/tetorica_sega_psg")
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("native-genesis")
        .file("src/native_genesis_core.cpp")
        .file("src/native_genesis_fx.cpp")
        .file("src/native_genesis_noise.cpp")
        .file("vendor/tetorica-soundchip-native/wasm/ym2612_wasm.cpp")
        .file("vendor/tetorica-soundchip-native/wasm/segapsg_wasm.cpp")
        .file("vendor/tetorica-soundchip-native/native/src/ymfm_opn_provider.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/ymfm/ymfm_adpcm.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/ymfm/ymfm_ssg.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/tetorica_sega_psg/segapsg.cpp")
        .warnings(false)
        .compile("tetorica_native_genesis");
    // GNU ld sees the common-core consumer, then the prefixed adapter, then
    // its one C provider. Keep these in dependency order without duplicate FM.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/nuked-opn2")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nuked.cpp")
        .warnings(false)
        .compile("tetorica_native_nuked");
    cc::Build::new()
        .std("c11")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .define("OPN2_Reset", "tetorica_nuked_OPN2_Reset")
        .define("OPN2_SetChipType", "tetorica_nuked_OPN2_SetChipType")
        .define("OPN2_Clock", "tetorica_nuked_OPN2_Clock")
        .define("OPN2_Write", "tetorica_nuked_OPN2_Write")
        .define("OPN2_SetTestPin", "tetorica_nuked_OPN2_SetTestPin")
        .define("OPN2_ReadTestPin", "tetorica_nuked_OPN2_ReadTestPin")
        .define("OPN2_ReadIRQPin", "tetorica_nuked_OPN2_ReadIRQPin")
        .define("OPN2_Read", "tetorica_nuked_OPN2_Read")
        .include("vendor/tetorica-soundchip-native/vendor/nuked-opn2")
        .file("vendor/tetorica-soundchip-native/vendor/nuked-opn2/ym3438.c")
        .warnings(false)
        .compile("tetorica_native_nuked_opn2");
    // Selected Mega-CD RF5C164 and the raw C ABI share one SDK decoder. The
    // independent RF renderer above keeps its own core and voice-cancellation
    // state; both retain their existing copy-into-owned-RAM upload semantics.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/mame-rf5c164")
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_rf5c164.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/mame-rf5c164/rf5c164.cpp")
        .file("vendor/tetorica-soundchip-native/wasm/rf5c164_wasm.cpp")
        .warnings(false)
        .compile("tetorica_native_megacd");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/msfa");
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/overrides/msfa"
    );
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_dx7.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_dx7.h");
    let (msfa_directory, msfa_sources) = native_dx7_sources();
    let mut dx7 = cc::Build::new();
    dx7.cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .define("MSFA_FORCE_SCALAR", None)
        // MSVC exposes M_PI in math.h only when this is defined first.
        .define("_USE_MATH_DEFINES", None)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include(&msfa_directory)
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_dx7.cpp");
    for source in msfa_sources {
        dx7.file(source);
    }
    dx7.warnings(false).compile("tetorica_native_dx7");
    // The SDK adapter contains the ported APU/FDS core. Selected and
    // independent instances share this one compilation unit and archive.
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nes.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nes.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_nes.cpp")
        .warnings(false)
        .compile("tetorica_native_nes");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/snes-spc");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_snes.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_snes.h");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_spc.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_spc.h");
    // Selected and independent Game Boy instances use the same SDK adapter
    // and APU core, compiled once in the existing archive.
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/mame-gameboy");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_gameboy.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_gameboy.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("vendor/tetorica-soundchip-native/vendor/mame-gameboy")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_gameboy.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/mame-gameboy/gameboy_apu.cpp")
        .warnings(false)
        .compile("tetorica_native_gameboy");
    // The SDK wrapper includes huc6280.cpp directly; compile that core only
    // through this translation unit for selected and independent instances.
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/vendor/mame-huc6280");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_huc6280.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_huc6280.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("vendor/tetorica-soundchip-native/vendor/mame-huc6280")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_huc6280.cpp")
        .warnings(false)
        .compile("tetorica_native_huc6280");
    // SNES instruments and complete SPC700 playback share one SDK S-DSP.
    // Keep both adapters and the four core files in the existing archive;
    // compiling two standalone SDK targets here would duplicate the DSP.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("vendor/tetorica-soundchip-native/vendor/snes-spc/snes_spc")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_snes.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_spc.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/snes-spc/snes_spc/SNES_SPC.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/snes-spc/snes_spc/SNES_SPC_misc.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/snes-spc/snes_spc/SNES_SPC_state.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/snes-spc/snes_spc/SPC_DSP.cpp")
        .warnings(false)
        .compile("tetorica_native_snes");
    // The selected 32X and independent extra PWM instances share this SDK
    // wrapper/core. Keep their existing archive and host-side ownership.
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_pwm.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_pwm.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-ffp-contract=off")
        .include("vendor/tetorica-soundchip-native/wasm")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_pwm.cpp")
        .warnings(false)
        .compile("tetorica_native_pwm");
    // LiveFX executes only on its generation/probe owner thread, never the
    // output callback. The host creates a JSRuntime only on first register.
    // GNU ld scans static archives in order: the wrapper must precede QuickJS.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-livefx/quickjs-ng")
        .file("src/native_genesis_livefx.cpp")
        .warnings(false)
        .compile("tetorica_native_livefx_probe");
    cc::Build::new()
        .std("c11")
        // Match QuickJS-NG's MSVC build: C11 alone does not enable <stdatomic.h>.
        .flag_if_supported("/experimental:c11atomics")
        .opt_level(2)
        .define("QUICKJS_NG_BUILD", None)
        // Match QuickJS-NG's build and expose clock_gettime/tm_gmtoff on glibc.
        .define("_GNU_SOURCE", None)
        .include("native-livefx/quickjs-ng")
        .file("native-livefx/quickjs-ng/quickjs-amalgam.c")
        .warnings(false)
        .compile("tetorica_quickjs_ng");
    tauri_build::build()
}
