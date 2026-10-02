fn main() {
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "macos",
        "Sonora requires macOS"
    );
    cc::Build::new()
        .file("native/audio.m")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=13.0")
        .compile("sonora_native");
    for framework in ["Foundation", "AppKit", "AVFoundation", "AudioToolbox"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rerun-if-changed=native/audio.m");
}
