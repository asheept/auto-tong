use std::fs;
use std::path::{Path, PathBuf};

/// Java 바이너리 이름 (플랫폼별)
#[cfg(target_os = "windows")]
const JAVA_BINARY: &str = "javaw.exe";
#[cfg(not(target_os = "windows"))]
const JAVA_BINARY: &str = "java";

/// Adoptium 다운로드 OS 이름
#[cfg(target_os = "windows")]
const ADOPTIUM_OS: &str = "windows";
#[cfg(target_os = "macos")]
const ADOPTIUM_OS: &str = "mac";
#[cfg(target_os = "linux")]
const ADOPTIUM_OS: &str = "linux";

/// Adoptium 아키텍처
#[cfg(target_arch = "x86_64")]
const ADOPTIUM_ARCH: &str = "x64";
#[cfg(target_arch = "aarch64")]
const ADOPTIUM_ARCH: &str = "aarch64";

/// mmc-pack.json 내용에서 Minecraft 버전을 추출
pub fn get_minecraft_version(mmc_pack_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(mmc_pack_json).ok()?;
    let components = parsed.get("components")?.as_array()?;
    for comp in components {
        if comp.get("uid")?.as_str()? == "net.minecraft" {
            return comp
                .get("cachedVersion")
                .or_else(|| comp.get("version"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

/// Minecraft 버전 → 필요한 Java 메이저 버전
pub fn required_java_version(mc_version: &str) -> u32 {
    let parts: Vec<u32> = mc_version
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();

    let (major, minor) = match parts.as_slice() {
        [maj, min, ..] => (*maj, *min),
        [maj] => (*maj, 0),
        _ => return 21,
    };

    if major < 1 || (major == 1 && minor <= 16) {
        8
    } else if major == 1 && minor == 17 {
        16
    } else if major == 1 && minor >= 18 && minor <= 19 {
        17
    } else if major == 1 && minor == 20 {
        let patch = parts.get(2).copied().unwrap_or(0);
        if patch >= 5 { 21 } else { 17 }
    } else {
        // 1.21+
        21
    }
}

/// PrismLauncher java 폴더에서 해당 버전의 java 경로를 찾기
fn find_java_in_prism(java_version: u32) -> Option<PathBuf> {
    let prism_java = dirs::config_dir()?.join("PrismLauncher").join("java");
    let folder_name = format!("java-{}", java_version);
    let java_bin = prism_java.join(&folder_name).join("bin").join(JAVA_BINARY);
    if java_bin.exists() {
        Some(java_bin)
    } else {
        None
    }
}

/// 시스템에서 Java를 찾기
fn find_java_in_system(java_version: u32) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        let candidates = vec![
            format!("{}/Eclipse Adoptium/jdk-{}", program_files, java_version),
            format!("{}/Eclipse Adoptium/jre-{}", program_files, java_version),
            format!("{}/Java/jdk-{}", program_files, java_version),
            format!("{}/Java/jdk{}", program_files, java_version),
        ];
        for candidate in candidates {
            let java_bin = Path::new(&candidate).join("bin").join(JAVA_BINARY);
            if java_bin.exists() {
                return Some(java_bin);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = vec![
            format!("/Library/Java/JavaVirtualMachines/temurin-{}.jre/Contents/Home", java_version),
            format!("/Library/Java/JavaVirtualMachines/temurin-{}.jdk/Contents/Home", java_version),
            format!("/Library/Java/JavaVirtualMachines/jdk-{}.jdk/Contents/Home", java_version),
        ];
        for candidate in candidates {
            let java_bin = Path::new(&candidate).join("bin").join(JAVA_BINARY);
            if java_bin.exists() {
                return Some(java_bin);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = vec![
            format!("/usr/lib/jvm/temurin-{}-jre", java_version),
            format!("/usr/lib/jvm/temurin-{}-jdk", java_version),
            format!("/usr/lib/jvm/java-{}-openjdk", java_version),
        ];
        for candidate in candidates {
            let java_bin = Path::new(&candidate).join("bin").join(JAVA_BINARY);
            if java_bin.exists() {
                return Some(java_bin);
            }
        }
    }

    None
}

/// Java가 존재하는지 확인, 없으면 다운로드
pub async fn ensure_java(java_version: u32) -> Result<PathBuf, String> {
    if let Some(path) = find_java_in_prism(java_version) {
        log::info!("Java {} 발견 (PrismLauncher): {}", java_version, path.display());
        return Ok(path);
    }

    if let Some(path) = find_java_in_system(java_version) {
        log::info!("Java {} 발견 (시스템): {}", java_version, path.display());
        return Ok(path);
    }

    log::info!("Java {} 미설치 — Adoptium에서 다운로드합니다", java_version);
    download_java(java_version).await
}

/// Adoptium Temurin JRE 다운로드 및 설치
async fn download_java(java_version: u32) -> Result<PathBuf, String> {
    let prism_java = dirs::config_dir()
        .ok_or("설정 경로를 찾을 수 없습니다")?
        .join("PrismLauncher")
        .join("java");
    fs::create_dir_all(&prism_java).map_err(|e| format!("java 폴더 생성 실패: {}", e))?;

    // macOS는 tar.gz, Windows는 zip
    #[cfg(target_os = "windows")]
    let image_type = "zip";
    #[cfg(not(target_os = "windows"))]
    let image_type = "tar.gz";

    let url = format!(
        "https://api.adoptium.net/v3/binary/latest/{}/ga/{}/{}/jre/hotspot/normal/eclipse?project=jdk",
        java_version, ADOPTIUM_OS, ADOPTIUM_ARCH
    );

    log::info!("Java {} 다운로드 중: {}", java_version, url);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("다운로드 요청 실패: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("다운로드 실패: HTTP {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("다운로드 실패: {}", e))?;

    log::info!(
        "Java {} 다운로드 완료 ({} MB)",
        java_version,
        bytes.len() / 1024 / 1024
    );

    let target_dir = prism_java.join(format!("java-{}", java_version));
    fs::create_dir_all(&target_dir).ok();

    #[cfg(target_os = "windows")]
    extract_zip_archive(&bytes, &target_dir, &prism_java, java_version)?;

    #[cfg(not(target_os = "windows"))]
    extract_tar_gz(&bytes, &target_dir, &prism_java, java_version)?;

    let java_bin = target_dir.join("bin").join(JAVA_BINARY);
    if java_bin.exists() {
        log::info!("Java {} 설치 완료: {}", java_version, java_bin.display());
        Ok(java_bin)
    } else {
        Err(format!(
            "Java {} 설치 후 {}를 찾을 수 없습니다: {}",
            java_version, JAVA_BINARY, target_dir.display()
        ))
    }
}

#[cfg(target_os = "windows")]
fn extract_zip_archive(
    bytes: &[u8],
    target_dir: &Path,
    prism_java: &Path,
    java_version: u32,
) -> Result<(), String> {
    let temp_zip = prism_java.join(format!("java-{}-temp.zip", java_version));
    fs::write(&temp_zip, bytes).map_err(|e| format!("임시 파일 저장 실패: {}", e))?;

    let file = fs::File::open(&temp_zip).map_err(|e| format!("zip 열기 실패: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;

    let root_prefix = {
        if archive.len() > 0 {
            let first_name = archive.by_index(0).ok().map(|e| e.name().to_string());
            first_name.and_then(|n| {
                let n = n.replace('\\', "/");
                n.find('/').map(|pos| format!("{}/", &n[..pos]))
            })
        } else {
            None
        }
    };

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 오류: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");
        let relative = if let Some(ref prefix) = root_prefix {
            if let Some(rest) = raw_name.strip_prefix(prefix.as_str()) {
                rest.to_string()
            } else {
                continue;
            }
        } else {
            raw_name
        };

        if relative.is_empty() {
            continue;
        }

        let target = target_dir.join(&relative);

        if entry.is_dir() || relative.ends_with('/') {
            fs::create_dir_all(&target).ok();
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).ok();
            }
            let mut outfile =
                fs::File::create(&target).map_err(|e| format!("파일 생성 실패: {}", e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }
    }

    fs::remove_file(&temp_zip).ok();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn extract_tar_gz(
    bytes: &[u8],
    target_dir: &Path,
    prism_java: &Path,
    java_version: u32,
) -> Result<(), String> {
    let temp_file = prism_java.join(format!("java-{}-temp.tar.gz", java_version));
    fs::write(&temp_file, bytes).map_err(|e| format!("임시 파일 저장 실패: {}", e))?;

    let output = std::process::Command::new("tar")
        .args(["xzf", &temp_file.to_string_lossy(), "--strip-components=1", "-C", &target_dir.to_string_lossy()])
        .output()
        .map_err(|e| format!("tar 실행 실패: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tar 압축 해제 실패: {}", stderr));
    }

    fs::remove_file(&temp_file).ok();
    Ok(())
}

/// 인스턴스의 mmc-pack.json을 읽어서 Java를 확보하고 instance.cfg에 JavaPath를 설정
pub async fn setup_java_for_instance(instance_dir: &Path) -> Result<(), String> {
    let mmc_pack_path = instance_dir.join("mmc-pack.json");
    if !mmc_pack_path.exists() {
        log::warn!("mmc-pack.json 없음: {}", instance_dir.display());
        return Ok(());
    }

    let content = fs::read_to_string(&mmc_pack_path)
        .map_err(|e| format!("mmc-pack.json 읽기 실패: {}", e))?;

    let mc_version = match get_minecraft_version(&content) {
        Some(v) => v,
        None => {
            log::warn!("Minecraft 버전을 감지할 수 없습니다");
            return Ok(());
        }
    };

    let java_ver = required_java_version(&mc_version);
    log::info!("Minecraft {} → Java {} 필요", mc_version, java_ver);

    let java_path = ensure_java(java_ver).await?;

    // instance.cfg에 JavaPath 설정
    let cfg_path = instance_dir.join("instance.cfg");
    if cfg_path.exists() {
        let content = fs::read_to_string(&cfg_path)
            .map_err(|e| format!("instance.cfg 읽기 실패: {}", e))?;

        let java_path_str = java_path.to_string_lossy().to_string();
        // Windows에서는 백슬래시, 나머지는 그대로
        #[cfg(target_os = "windows")]
        let java_path_str = java_path_str.replace('/', "\\");

        let mut has_java_path = false;
        let updated: Vec<String> = content
            .lines()
            .map(|line| {
                if line.starts_with("JavaPath=") {
                    has_java_path = true;
                    format!("JavaPath={}", java_path_str)
                } else {
                    line.to_string()
                }
            })
            .collect();

        let mut result = updated.join("\n");
        if !has_java_path {
            result.push_str(&format!("\nJavaPath={}", java_path_str));
        }

        fs::write(&cfg_path, result)
            .map_err(|e| format!("instance.cfg 쓰기 실패: {}", e))?;
        log::info!("JavaPath 설정 완료: {}", java_path_str);
    }

    Ok(())
}
