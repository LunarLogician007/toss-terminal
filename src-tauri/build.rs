fn main() {
    needle::link();
    tauri_build::build()
}

/// TOSS Terminal: built-in dictation runs Cactus Compute's Whistle on their
/// Needle engine, which ships as a prebuilt static library per platform. It
/// is fetched from a pinned revision, checked against its SHA-256 and linked
/// in; platforms without a build get dictation switched off (no `needle` cfg).
mod needle {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sha2::{Digest, Sha256};

    const REVISION: &str = "2ae11323dc000f5e70c49f7403efa6af12ba9e67";
    const BASE_URL: &str = "https://huggingface.co/Cactus-Compute/needle3/resolve";

    /// (folder on Hugging Face, SHA-256 of its libneedle.a)
    fn build_for(os: &str, arch: &str, target_env: &str) -> Option<(&'static str, &'static str)> {
        match (os, arch, target_env) {
            ("macos", "aarch64", _) => Some((
                "macos-arm64",
                "98da47c15e1065b4cdc7ddc55e825be78414d4586832db3becaeded39a373df4",
            )),
            ("linux", "x86_64", "gnu") => Some((
                "linux-x86_64",
                "8556785e3f3562bb6e402d48770f34c0797f23b9571c07457cdc577928b0b9d3",
            )),
            ("linux", "aarch64", "gnu") => Some((
                "linux-arm64",
                "3c1053149e48f0ae9917f4d336e60f841dddeb3062a8d7b56c7ddf06fdfac33e",
            )),
            // Windows' library is a MinGW build, which the MSVC toolchain
            // can't link; Intel Macs have none.
            _ => None,
        }
    }

    pub fn link() {
        println!("cargo::rustc-check-cfg=cfg(needle)");
        println!("cargo::rerun-if-changed=build.rs");
        println!("cargo::rerun-if-env-changed=NEEDLE_LIB_DIR");
        println!("cargo::rerun-if-env-changed=NEEDLE_LIBCXX_DIR");

        let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
        let Some((folder, sha256)) = build_for(&os, &arch, &target_env) else {
            println!("cargo::warning=no Needle engine for {os}-{arch}: built-in dictation is off");
            return;
        };

        let dir = match env::var_os("NEEDLE_LIB_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("needle"),
        };
        let lib = dir.join("libneedle.a");
        if !matches_sha256(&lib, sha256) {
            if env::var_os("NEEDLE_LIB_DIR").is_some() {
                panic!(
                    "{} isn't the pinned Needle engine for {folder} (SHA-256 {sha256})",
                    lib.display()
                );
            }
            fetch(folder, &dir, &lib);
            if !matches_sha256(&lib, sha256) {
                let _ = fs::remove_file(&lib);
                panic!("the downloaded Needle engine for {folder} failed its SHA-256 check");
            }
        }

        println!("cargo::rustc-link-search=native={}", dir.display());
        println!("cargo::rustc-link-lib=static=needle");
        link_cxx_runtime(&os);
        println!("cargo::rustc-cfg=needle");
    }

    fn matches_sha256(path: &Path, want: &str) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        let got: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        got == want
    }

    fn fetch(folder: &str, dir: &Path, lib: &Path) {
        fs::create_dir_all(dir).expect("create the Needle folder");
        let url = format!("{BASE_URL}/{REVISION}/{folder}/libneedle.a");
        let status = Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error"])
            .args(["--proto", "=https", "--proto-redir", "=https"])
            .args(["--retry", "3"])
            .arg("--output")
            .arg(lib)
            .arg(&url)
            .status()
            .unwrap_or_else(|e| panic!("couldn't run curl to fetch {url}: {e}"));
        if !status.success() {
            panic!("couldn't download {url} (set NEEDLE_LIB_DIR to build offline)");
        }
    }

    /// The engine is built against LLVM's libc++. macOS ships it; on Linux it
    /// is linked statically so the package gains no runtime dependency.
    fn link_cxx_runtime(os: &str) {
        if os == "macos" {
            println!("cargo::rustc-link-lib=c++");
            return;
        }
        let dir = libcxx_dir().unwrap_or_else(|| {
            panic!("libc++.a not found: install libc++-dev and libc++abi-dev, or set NEEDLE_LIBCXX_DIR")
        });
        println!("cargo::rustc-link-search=native={}", dir.display());
        println!("cargo::rustc-link-lib=static=c++");
        println!("cargo::rustc-link-lib=static=c++abi");
    }

    fn libcxx_dir() -> Option<PathBuf> {
        let has = |d: &Path| d.join("libc++.a").is_file() && d.join("libc++abi.a").is_file();
        if let Some(dir) = env::var_os("NEEDLE_LIBCXX_DIR") {
            return Some(PathBuf::from(dir)).filter(|d| has(d));
        }
        // Debian and Ubuntu put them under /usr/lib/llvm-<version>/lib;
        // prefer the newest.
        let mut llvm: Vec<(u32, PathBuf)> = fs::read_dir("/usr/lib")
            .ok()?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let version = name.strip_prefix("llvm-")?.parse().ok()?;
                Some((version, e.path().join("lib")))
            })
            .collect();
        llvm.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
        llvm.into_iter()
            .map(|(_, d)| d)
            .chain(["/usr/lib/x86_64-linux-gnu", "/usr/lib/aarch64-linux-gnu", "/usr/lib"].map(PathBuf::from))
            .find(|d| has(d))
    }
}
