"""Run diagnostic reproductions against the original Rust modules in a temporary crate.

Passing tests confirm the reviewed defects; they are NOT post-fix regression tests.
Only disposable temp fixtures and a loopback HTTP server are used. No app startup,
launcher processes, user settings, or real instances are touched.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SOURCE = ROOT / "src-tauri" / "src"

MANIFEST = '''[package]
name = "auto-tong-review"
version = "0.0.0"
edition = "2021"
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
dirs = "6"
zip = "2"
log = "0.4"
tokio = { version = "1", features = ["fs", "process", "sync", "time", "rt-multi-thread", "macros"] }
reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }
sha2 = "0.10"
encoding_rs = "0.8"
tempfile = "3"
'''

with tempfile.TemporaryDirectory(prefix="auto-tong-review-") as temp:
    crate = Path(temp)
    (crate / "src").mkdir()
    (crate / "Cargo.toml").write_text(MANIFEST, encoding="utf-8")
    shutil.copy2(ROOT / "src-tauri" / "Cargo.lock", crate / "Cargo.lock")
    source = SOURCE.as_posix()
    lib = "#![allow(dead_code, unused_variables)]\n"
    for name in ("config", "java", "mrpack", "zip_util"):
        lib += f'#[path = "{source}/{name}.rs"] mod {name};\n'
    lib += f'''mod prismlauncher {{
        include!("{source}/prismlauncher.rs");
        pub fn review_extract(zip: &Path, instances: &Path) -> Result<(), String> {{
            extract_zip(zip, instances, |_, _| {{}})
        }}
    }}
    mod tracker {{
        include!("{source}/tracker.rs");
        pub fn review_tracker(path: PathBuf) -> Tracker {{
            Tracker {{ data: Mutex::new(ProcessedFiles::default()), path }}
        }}
    }}
    #[cfg(test)] mod review_tests;
    '''
    (crate / "src" / "lib.rs").write_text(lib, encoding="utf-8")
    shutil.copy2(HERE / "review_tests.rs", crate / "src" / "review_tests.rs")
    result = subprocess.run([
        "cargo", "+stable-x86_64-pc-windows-msvc", "test", "--offline",
        "--manifest-path", str(crate / "Cargo.toml"),
        "--target-dir", str(ROOT / "src-tauri" / "target"),
        "--", "--test-threads=1",
    ], env={**os.environ, "RUST_BACKTRACE": "0"})
    raise SystemExit(result.returncode)
