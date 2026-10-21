fn main() {
    println!("cargo:rerun-if-changed=native-genesis");
    println!("cargo:rerun-if-changed=native-fx");
    println!("cargo:rerun-if-changed=native-noise");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.h");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.h");
    println!("cargo:rerun-if-changed=src/native_genesis_core.cpp");
    println!("cargo:rerun-if-changed=native-livefx");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.h");
    cc::Build::new()
        .cpp(true)
        .std("c++14")
        .opt_level(2)
        .include("native-genesis")
        .file("src/native_genesis_core.cpp")
        .file("src/native_genesis_fx.cpp")
        .file("src/native_genesis_noise.cpp")
        .file("native-genesis/ym2612_wasm.cpp")
        .file("native-genesis/segapsg_wasm.cpp")
        .file("native-genesis/ymfm_opn.cpp")
        .file("native-genesis/ymfm_adpcm.cpp")
        .file("native-genesis/ymfm_ssg.cpp")
        .file("native-genesis/segapsg.cpp")
        .warnings(false)
        .compile("tetorica_native_genesis");
    // Offline LiveFX probe only. The application has no route from its audio
    // callback to this host, and the host creates a JSRuntime only on register.
    cc::Build::new()
        .std("c11")
        .opt_level(2)
        .define("QUICKJS_NG_BUILD", None)
        .include("native-livefx/quickjs-ng")
        .file("native-livefx/quickjs-ng/quickjs-amalgam.c")
        .warnings(false)
        .compile("tetorica_quickjs_ng");
    cc::Build::new()
        .cpp(true)
        .std("c++14")
        .opt_level(2)
        .include("native-livefx/quickjs-ng")
        .file("src/native_genesis_livefx.cpp")
        .warnings(false)
        .compile("tetorica_native_livefx_probe");
    tauri_build::build()
}
