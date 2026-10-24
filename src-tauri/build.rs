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
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/wasm");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2612.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2612.h");
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/src/ymfm_opn_provider.cpp"
    );
    println!(
        "cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/src/ymfm_opm_provider.cpp"
    );
    println!("cargo:rerun-if-changed=src/native_genesis_ymfm_opl.cpp");
    println!("cargo:rerun-if-changed=native-nuked");
    println!("cargo:rerun-if-changed=src/native_genesis_nuked.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_nuked_ym2612.inc");
    println!("cargo:rerun-if-changed=native-megacd");
    println!("cargo:rerun-if-changed=src/native_genesis_rf5c164.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_rf5c164.h");
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
    println!("cargo:rerun-if-changed=native-x68000");
    println!("cargo:rerun-if-changed=src/native_genesis_x68000.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.cpp");
    println!("cargo:rerun-if-changed=vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.h");
    println!("cargo:rerun-if-changed=src/native_genesis_x68000.h");
    println!("cargo:rerun-if-changed=native-master-system");
    println!("cargo:rerun-if-changed=src/native_genesis_master_system.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_master_system.h");
    println!("cargo:rerun-if-changed=src/native_genesis_psg.h");
    println!("cargo:rerun-if-changed=native-game-gear");
    println!("cargo:rerun-if-changed=src/native_genesis_game_gear.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_game_gear.h");
    println!("cargo:rerun-if-changed=src/native_genesis_opll.h");
    println!("cargo:rerun-if-changed=native-msx");
    println!("cargo:rerun-if-changed=src/native_genesis_msx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_ym2413.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_ym2413.h");
    println!("cargo:rerun-if-changed=src/native_genesis_ay8910.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_ay8910.h");
    println!("cargo:rerun-if-changed=src/native_genesis_msx.h");
    println!("cargo:rerun-if-changed=native-opl");
    println!("cargo:rerun-if-changed=src/native_genesis_opl.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_okim6258.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_okim6258.h");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_rf5c164.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_rf5c164.h");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_opl.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_extra_opl.h");
    println!("cargo:rerun-if-changed=src/native_genesis_opl.h");
    println!("cargo:rerun-if-changed=src/native_genesis_y8950_memory.h");
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
    // Standalone Game Gear consumes the once-compiled common Sega PSG below.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-genesis")
        .file("src/native_genesis_game_gear.cpp")
        .warnings(false)
        .compile("tetorica_native_game_gear");
    // The selected DOS OPL core references the same ymfm OPL/PCM provider as
    // Master System and MSX. Compile it once and keep consumers first on GNU ld.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-master-system/snapshot/src")
        .include("native-genesis")
        .file("src/native_genesis_opl.cpp")
        .file("src/native_genesis_extra_opl.cpp")
        .file("src/native_genesis_extra_rf5c164.cpp")
        .warnings(false)
        .compile("tetorica_native_opl");
    // MSX references the same OPLL provider as Master System. GNU ld requires
    // this dependent archive before that provider and the common ymfm archive.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-msx/snapshot/third_party/mame-ay8910")
        .include("native-master-system/snapshot/src")
        .include("native-genesis")
        .file("src/native_genesis_msx.cpp")
        .file("src/native_genesis_ay8910.cpp")
        .file("native-msx/snapshot/third_party/mame-ay8910/ay8910.cpp")
        .warnings(false)
        .compile("tetorica_native_msx");
    // The OPLL wrapper needs shared ymfm ADPCM and Sega PSG symbols below.
    // Keep the dependent archive before the common provider for GNU ld.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-master-system/snapshot/src")
        .include("native-genesis")
        .file("src/native_genesis_master_system.cpp")
        .file("src/native_genesis_ym2413.cpp")
        .file("src/native_genesis_ymfm_opl.cpp")
        .file("native-master-system/snapshot/src/ymfm_pcm.cpp")
        .warnings(false)
        .compile("tetorica_native_master_system");
    // X68000 mixing and OKI stay application-owned. Its FM and the independent
    // YM2151 share the SDK's once-compiled OPM provider, before common ymfm on GNU ld.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("native-x68000/snapshot/third_party/mame-okim6258")
        .include("native-genesis")
        .file("src/native_genesis_x68000.cpp")
        .file("src/native_genesis_extra_okim6258.cpp")
        .file("vendor/tetorica-soundchip-native/native/reference/tauri/native_genesis_ym2151.cpp")
        .file("vendor/tetorica-soundchip-native/native/src/ymfm_opm_provider.cpp")
        .file("native-x68000/okim6258.cpp")
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
    // Keep the application's Genesis mixing, DC centering, gains and Sega PSG
    // path. Only its YM2612 adapter and shared OPN/SSG/ADPCM provider come from
    // the pinned SDK. Existing OPN, OPM and OPL consumers use identical ymfm
    // types, so do not add a second provider archive or change GNU ld ordering.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("vendor/tetorica-soundchip-native/vendor/ymfm")
        .include("vendor/tetorica-soundchip-native/wasm")
        .include("native-genesis")
        .file("src/native_genesis_core.cpp")
        .file("src/native_genesis_fx.cpp")
        .file("src/native_genesis_noise.cpp")
        .file("vendor/tetorica-soundchip-native/wasm/ym2612_wasm.cpp")
        .file("native-genesis/segapsg_wasm.cpp")
        .file("vendor/tetorica-soundchip-native/native/src/ymfm_opn_provider.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/ymfm/ymfm_adpcm.cpp")
        .file("vendor/tetorica-soundchip-native/vendor/ymfm/ymfm_ssg.cpp")
        .file("native-genesis/segapsg.cpp")
        .warnings(false)
        .compile("tetorica_native_genesis");
    // GNU ld sees the common-core consumer, then the prefixed adapter, then
    // its one C provider. Keep these in dependency order without duplicate FM.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-nuked/snapshot/third_party/nuked-opn2")
        .file("src/native_genesis_nuked.cpp")
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
        .include("native-nuked/snapshot/third_party/nuked-opn2")
        .file("native-nuked/snapshot/third_party/nuked-opn2/ym3438.c")
        .warnings(false)
        .compile("tetorica_native_nuked_opn2");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-megacd")
        .file("src/native_genesis_rf5c164.cpp")
        .file("native-megacd/rf5c164.cpp")
        .file("native-megacd/rf5c164_wasm.cpp")
        .warnings(false)
        .compile("tetorica_native_megacd");
    println!("cargo:rerun-if-changed=native-dx7");
    println!("cargo:rerun-if-changed=src/native_genesis_dx7.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_dx7.h");
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
        .include("native-dx7")
        .file("src/native_genesis_dx7.cpp");
    for source in [
        "dx7note",
        "env",
        "exp2",
        "fm_core",
        "fm_op_kernel",
        "freqlut",
        "lfo",
        "patch",
        "pitchenv",
        "resofilter",
        "ringbuffer",
        "sin",
        "synth_unit",
    ] {
        dx7.file(format!("native-dx7/{source}.cc"));
    }
    dx7.warnings(false).compile("tetorica_native_dx7");
    println!("cargo:rerun-if-changed=native-nes");
    println!("cargo:rerun-if-changed=src/native_genesis_nes.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_nes.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .file("src/native_genesis_nes.cpp")
        .warnings(false)
        .compile("tetorica_native_nes");
    for name in ["gameboy", "snes", "huc6280"] {
        println!("cargo:rerun-if-changed=native-{name}");
        println!("cargo:rerun-if-changed=src/native_genesis_{name}.cpp");
        println!("cargo:rerun-if-changed=src/native_genesis_{name}.h");
    }
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("native-gameboy/snapshot/third_party/mame-gameboy")
        .file("src/native_genesis_gameboy.cpp")
        .file("native-gameboy/snapshot/third_party/mame-gameboy/gameboy_apu.cpp")
        .warnings(false)
        .compile("tetorica_native_gameboy");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .file("src/native_genesis_huc6280.cpp")
        .warnings(false)
        .compile("tetorica_native_huc6280");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("native-snes/snapshot/third_party/snes-spc/snes_spc")
        .file("src/native_genesis_snes.cpp")
        .file("src/native_genesis_spc.cpp")
        .file("native-snes/snapshot/third_party/snes-spc/snes_spc/SNES_SPC.cpp")
        .file("native-snes/snapshot/third_party/snes-spc/snes_spc/SNES_SPC_misc.cpp")
        .file("native-snes/snapshot/third_party/snes-spc/snes_spc/SNES_SPC_state.cpp")
        .file("native-snes/snapshot/third_party/snes-spc/snes_spc/SPC_DSP.cpp")
        .warnings(false)
        .compile("tetorica_native_snes");
    println!("cargo:rerun-if-changed=src/native_genesis_spc.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_spc.h");
    println!("cargo:rerun-if-changed=native-pwm");
    println!("cargo:rerun-if-changed=src/native_genesis_pwm.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_pwm.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-ffp-contract=off")
        .include("native-pwm/snapshot/wasm")
        .file("src/native_genesis_pwm.cpp")
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
