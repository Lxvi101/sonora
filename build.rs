fn main() {
    assert_eq!(
        std::env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "macos",
        "Sonora requires macOS"
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let lame = out.join("lame");
    let status = std::process::Command::new("bash")
        .arg("scripts/build-lame.sh")
        .arg(&lame)
        .status()
        .expect("Build LAME");
    assert!(status.success(), "LAME build failed");
    let lib = lame.join("install/lib");
    let profile = out.ancestors().nth(3).expect("Cargo profile directory");
    std::fs::copy(
        lib.join("libmp3lame.0.dylib"),
        profile.join("libmp3lame.0.dylib"),
    )
    .unwrap();
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=mp3lame");
    for rpath in [
        "@executable_path",
        "@executable_path/..",
        "@executable_path/../Frameworks",
    ] {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{rpath}");
    }
    println!("cargo:rerun-if-changed=scripts/build-lame.sh");
    println!("cargo:rerun-if-changed=vendor/lame-4.0.tar.gz");
    cc::Build::new()
        .include(lame.join("install/include"))
        .file("native/audio.m")
        .file("native/media.m")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=13.0")
        .compile("sonora_native");
    for framework in [
        "Foundation",
        "AppKit",
        "AVFoundation",
        "AudioToolbox",
        "CoreMedia",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rerun-if-changed=native/audio.m");
    println!("cargo:rerun-if-changed=native/media.m");
}
