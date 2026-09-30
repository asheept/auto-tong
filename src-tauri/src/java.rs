use std::fs;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(any(test, not(target_os = "windows")))]
use crate::zip_util::DEFAULT_ZIP_LIMITS as TAR_LIMITS;
#[cfg(target_os = "windows")]
use crate::zip_util::{check_archive_limits, decode_zip_name, DEFAULT_ZIP_LIMITS};

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

fn java_from_metadata(raw: &str, expected_version: &str) -> Result<u32, String> {
    let metadata: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Minecraft 메타데이터 파싱 실패: {e}"))?;
    if metadata["uid"] != "net.minecraft" || metadata["version"] != expected_version {
        return Err("Minecraft 메타데이터의 버전 또는 UID가 요청과 다릅니다".to_string());
    }
    let majors = metadata["compatibleJavaMajors"]
        .as_array()
        .ok_or("Minecraft 메타데이터에 compatibleJavaMajors가 없습니다")?;
    if majors.len() != 1 {
        return Err("Minecraft Java 요구 버전을 하나로 결정할 수 없습니다".to_string());
    }
    let version = majors[0]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| (8..=25).contains(value))
        .ok_or("Minecraft Java 요구 버전이 유효하지 않습니다")?;
    Ok(version)
}

fn known_java_requirement(mc_version: &str) -> Option<u32> {
    let numbers: Vec<u32> = mc_version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match numbers.as_slice() {
        [1, minor, ..] if *minor <= 16 => Some(8),
        [1, 17, ..] => Some(16),
        [1, 18 | 19, ..] => Some(17),
        [1, 20] => Some(17),
        [1, 20, patch] if *patch <= 4 => Some(17),
        [1, 20, patch] if *patch >= 5 => Some(21),
        [1, 21, ..] => Some(21),
        [26, 1, ..] => Some(25),
        _ => None,
    }
}

async fn resolve_java_version_with<F, Fut>(
    mc_version: &str,
    cache_dir: &Path,
    fetch: F,
) -> Result<u32, String>
where
    F: FnOnce(&str) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    if mc_version.is_empty()
        || mc_version.contains("..")
        || !mc_version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(format!("유효하지 않은 Minecraft 버전: {mc_version}"));
    }
    let cache_path = cache_dir.join(format!("{mc_version}.json"));
    if let Ok(raw) = fs::read_to_string(&cache_path) {
        if let Ok(version) = java_from_metadata(&raw, mc_version) {
            return Ok(version);
        }
        log::warn!(
            "Java 요구사항 캐시가 손상되어 다시 조회합니다: {}",
            cache_path.display()
        );
    }

    let url = format!("https://raw.githubusercontent.com/PrismLauncher/meta-launcher/master/net.minecraft/{mc_version}.json");
    match fetch(&url)
        .await
        .and_then(|raw| java_from_metadata(&raw, mc_version).map(|version| (raw, version)))
    {
        Ok((raw, version)) => {
            fs::create_dir_all(cache_dir)
                .map_err(|e| format!("Java 메타데이터 캐시 폴더 생성 실패: {e}"))?;
            let mut temporary = tempfile::Builder::new()
                .prefix(".java-requirement-")
                .tempfile_in(cache_dir)
                .map_err(|e| format!("Java 메타데이터 캐시 생성 실패: {e}"))?;
            temporary
                .write_all(raw.as_bytes())
                .map_err(|e| format!("Java 메타데이터 캐시 쓰기 실패: {e}"))?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|e| format!("Java 메타데이터 캐시 동기화 실패: {e}"))?;
            temporary
                .persist(&cache_path)
                .map_err(|e| format!("Java 메타데이터 캐시 저장 실패: {e}"))?;
            Ok(version)
        }
        Err(error) => {
            if let Some(version) = known_java_requirement(mc_version) {
                log::warn!(
                    "Minecraft {} 메타데이터 조회 실패, 검증된 요구사항 Java {} 사용: {}",
                    mc_version,
                    version,
                    error
                );
                Ok(version)
            } else {
                Err(format!(
                    "Minecraft {}의 Java 요구사항을 확인할 수 없습니다: {}. Minecraft 버전 표기를 확인하고 네트워크 연결 후 재가져오기를 실행하세요",
                    mc_version, error
                ))
            }
        }
    }
}

pub async fn resolve_java_version(mc_version: &str) -> Result<u32, String> {
    let cache_dir = crate::config::config_dir().join("java-requirements");
    resolve_java_version_with(mc_version, &cache_dir, |url| {
        let url = url.to_string();
        async move {
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?;
            let mut response = client
                .get(url)
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("Minecraft 메타데이터 크기 제한 초과".to_string());
                }
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).map_err(|e| e.to_string())
        }
    })
    .await
}

/// PrismLauncher java 폴더에서 해당 버전의 java 경로를 찾기
fn find_java_in_prism(java_version: u32, prism_data: Option<&Path>) -> Option<PathBuf> {
    let folder_name = format!("java-{}", java_version);

    // prism_data가 제공되면 해당 경로 우선 탐색 (portable 지원)
    if let Some(data_dir) = prism_data {
        let java_bin = data_dir
            .join("java")
            .join(&folder_name)
            .join("bin")
            .join(JAVA_BINARY);
        return java_bin.exists().then_some(java_bin);
    }

    // 표준 경로 탐색
    let prism_java = dirs::config_dir()?.join("PrismLauncher").join("java");
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
            format!(
                "/Library/Java/JavaVirtualMachines/temurin-{}.jre/Contents/Home",
                java_version
            ),
            format!(
                "/Library/Java/JavaVirtualMachines/temurin-{}.jdk/Contents/Home",
                java_version
            ),
            format!(
                "/Library/Java/JavaVirtualMachines/jdk-{}.jdk/Contents/Home",
                java_version
            ),
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
pub async fn ensure_java(java_version: u32, prism_data: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = find_java_in_prism(java_version, prism_data) {
        match crate::java_runtime::validate_runtime(&path, java_version).await {
            Ok(()) => {
                log::info!(
                    "Java {} 검증 완료 (PrismLauncher): {}",
                    java_version,
                    path.display()
                );
                return Ok(path);
            }
            Err(error) => log::warn!("기존 Prism Java 검증 실패 ({}): {}", path.display(), error),
        }
    }

    if let Some(path) = find_java_in_system(java_version) {
        match crate::java_runtime::validate_runtime(&path, java_version).await {
            Ok(()) => {
                log::info!(
                    "Java {} 검증 완료 (시스템): {}",
                    java_version,
                    path.display()
                );
                return Ok(path);
            }
            Err(error) => log::warn!("시스템 Java 검증 실패 ({}): {}", path.display(), error),
        }
    }

    log::info!("Java {} 미설치 — Adoptium에서 다운로드합니다", java_version);
    download_java(java_version, prism_data).await.map_err(|error| {
        format!("Java {java_version} 준비 실패: {error}. 네트워크 연결과 Prism 데이터 폴더의 쓰기 권한을 확인한 뒤 재가져오기를 실행하세요")
    })
}

/// Adoptium Temurin JRE 다운로드 및 설치
async fn download_java(java_version: u32, prism_data: Option<&Path>) -> Result<PathBuf, String> {
    let prism_java = match prism_data {
        Some(data_dir) => data_dir.join("java"),
        None => dirs::config_dir()
            .ok_or("설정 경로를 찾을 수 없습니다")?
            .join("PrismLauncher")
            .join("java"),
    };
    log::info!("Java 설치 경로: {}", prism_java.display());
    fs::create_dir_all(&prism_java).map_err(|e| format!("java 폴더 생성 실패: {}", e))?;

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {}", e))?;
    let package =
        crate::java_download::fetch_package(&client, java_version, ADOPTIUM_OS, ADOPTIUM_ARCH)
            .await?;
    log::info!("Java {} 다운로드 중: {}", java_version, package.url);
    let bytes = crate::java_download::download_verified(&client, &package).await?;

    log::info!(
        "Java {} 다운로드 완료 ({} MB)",
        java_version,
        bytes.len() / 1024 / 1024
    );

    let target_dir = prism_java.join(format!("java-{}", java_version));
    let stage = tempfile::Builder::new()
        .prefix(".java-stage-")
        .tempdir_in(&prism_java)
        .map_err(|e| format!("Java 임시 설치 폴더 생성 실패: {e}"))?;

    #[cfg(target_os = "windows")]
    extract_zip_archive(&bytes, stage.path(), &prism_java)?;

    #[cfg(not(target_os = "windows"))]
    extract_tar_gz(&bytes, stage.path())?;

    let staged_bin = stage.path().join("bin").join(JAVA_BINARY);
    crate::java_runtime::validate_runtime(&staged_bin, java_version).await?;

    let backup = tempfile::Builder::new()
        .prefix(".java-backup-")
        .tempdir_in(&prism_java)
        .map_err(|e| format!("Java 백업 폴더 생성 실패: {e}"))?;
    let old_dir = backup.path().join("old");
    let had_old = target_dir.exists();
    if had_old {
        fs::rename(&target_dir, &old_dir).map_err(|e| format!("기존 Java 백업 실패: {e}"))?;
    }
    if let Err(error) = fs::rename(stage.path(), &target_dir) {
        if had_old {
            if let Err(restore_error) = fs::rename(&old_dir, &target_dir) {
                let preserved = backup.keep();
                return Err(format!(
                    "Java 설치 실패: {error}; 기존 Java 복원 실패: {restore_error}; 백업: {}",
                    preserved.display()
                ));
            }
        }
        return Err(format!("Java 설치 실패: {error}"));
    }
    let java_bin = target_dir.join("bin").join(JAVA_BINARY);
    log::info!(
        "Java {} 검증 및 설치 완료: {}",
        java_version,
        java_bin.display()
    );
    Ok(java_bin)
}

#[cfg(target_os = "windows")]
fn extract_zip_archive(bytes: &[u8], target_dir: &Path, prism_java: &Path) -> Result<(), String> {
    let mut temp_zip = tempfile::Builder::new()
        .prefix(".java-archive-")
        .tempfile_in(prism_java)
        .map_err(|e| format!("Java 임시 ZIP 생성 실패: {e}"))?;
    temp_zip
        .write_all(bytes)
        .map_err(|e| format!("임시 ZIP 저장 실패: {e}"))?;
    let file = temp_zip
        .reopen()
        .map_err(|e| format!("zip 열기 실패: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;
    check_archive_limits(&mut archive, DEFAULT_ZIP_LIMITS)?;

    let root_prefix = {
        if !archive.is_empty() {
            let first_name = archive
                .by_index(0)
                .ok()
                .map(|e| decode_zip_name(e.name_raw(), e.name()));
            first_name.and_then(|n| n.find('/').map(|pos| format!("{}/", &n[..pos])))
        } else {
            None
        }
    };

    let mut names = crate::import_path::OutputNames::default();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 오류: {e}"))?;
        let raw = decode_zip_name(entry.name_raw(), entry.name());
        let relative = if let Some(prefix) = &root_prefix {
            raw.strip_prefix(prefix).unwrap_or("")
        } else {
            &raw
        };
        if !relative.is_empty() {
            names.insert(relative)?;
        }
    }

    let output = crate::confined_output::ConfinedOutput::open(target_dir)?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 오류: {}", e))?;

        let raw_name = decode_zip_name(entry.name_raw(), entry.name());
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

        if entry.is_dir() || relative.ends_with('/') {
            output.create_dir(&relative)?;
        } else {
            let mut outfile = output.create_file(&relative, false)?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }
    }

    Ok(())
}

#[cfg(any(test, not(target_os = "windows")))]
fn extract_tar_gz(bytes: &[u8], target_dir: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let output = crate::confined_output::ConfinedOutput::open(target_dir)?;
    let mut archive = Archive::new(GzDecoder::new(bytes));
    let mut count = 0usize;
    let mut total = 0u64;
    let mut names = crate::import_path::OutputNames::default();
    for entry in archive
        .entries()
        .map_err(|e| format!("tar 목록 읽기 실패: {e}"))?
    {
        let entry = entry.map_err(|e| format!("tar 엔트리 읽기 실패: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar 경로 읽기 실패: {e}"))?;
        let raw = path.to_str().ok_or("tar 경로가 UTF-8이 아닙니다")?;
        let raw = raw.strip_prefix("./").unwrap_or(raw);
        crate::import_path::safe_relative(raw)?;
        if let Some((_, relative)) = raw.split_once('/') {
            if !relative.is_empty() {
                names.insert(relative)?;
            }
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(format!(
                "Java tar에 허용되지 않는 링크 또는 특수 엔트리: {raw}"
            ));
        }
        count += 1;
        if count > TAR_LIMITS.entries || entry.size() > TAR_LIMITS.file_bytes {
            return Err("Java tar 엔트리 수 또는 단일 파일 크기 제한 초과".to_string());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("Java tar 총 크기 계산 범위 초과")?;
        if total > TAR_LIMITS.total_bytes {
            return Err("Java tar 총 해제 크기 제한 초과".to_string());
        }
    }

    let mut archive = Archive::new(GzDecoder::new(bytes));
    for entry in archive
        .entries()
        .map_err(|e| format!("tar 목록 읽기 실패: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("tar 엔트리 읽기 실패: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar 경로 읽기 실패: {e}"))?;
        let raw = path.to_str().ok_or("tar 경로가 UTF-8이 아닙니다")?;
        let raw = raw.strip_prefix("./").unwrap_or(raw);
        let Some((_, relative)) = raw.split_once('/') else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        if entry.header().entry_type().is_dir() {
            output.create_dir(relative)?;
        } else {
            let mut file = output.create_file(relative, false)?;
            std::io::copy(&mut entry, &mut file)
                .map_err(|e| format!("Java 파일 추출 실패: {e}"))?;
        }
    }
    Ok(())
}

/// 인스턴스의 mmc-pack.json을 읽어서 Java를 확보하고 instance.cfg에 JavaPath를 설정
pub async fn setup_java_for_instance(
    instance_dir: &Path,
    prism_data: Option<&Path>,
) -> Result<(), String> {
    let mmc_pack_path = instance_dir.join("mmc-pack.json");
    if !mmc_pack_path.exists() {
        return Err(format!("mmc-pack.json 없음: {}", instance_dir.display()));
    }

    let content = fs::read_to_string(&mmc_pack_path)
        .map_err(|e| format!("mmc-pack.json 읽기 실패: {}", e))?;

    let mc_version = match get_minecraft_version(&content) {
        Some(v) => v,
        None => {
            return Err("Minecraft 버전을 감지할 수 없습니다".to_string());
        }
    };

    let java_ver = resolve_java_version(&mc_version).await?;
    log::info!("Minecraft {} → Java {} 필요", mc_version, java_ver);

    let java_path = ensure_java(java_ver, prism_data).await?;

    // instance.cfg에 JavaPath 설정
    let cfg_path = instance_dir.join("instance.cfg");
    if cfg_path.exists() {
        let content =
            fs::read_to_string(&cfg_path).map_err(|e| format!("instance.cfg 읽기 실패: {}", e))?;

        // 원본 줄바꿈 보존 (PrismLauncher는 Windows에서 \r\n 사용)
        let eol = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };

        let java_path_str = java_path.to_string_lossy().to_string();
        // PrismLauncher는 Qt QSettings(INI 포맷)로 instance.cfg를 읽음.
        // Qt INI 파서가 \U, \P 등을 이스케이프 시퀀스로 해석하여 백슬래시를 제거하므로
        // Windows에서도 포워드 슬래시를 사용 (Java/PrismLauncher 모두 지원)
        #[cfg(target_os = "windows")]
        let java_path_str = java_path_str.replace('\\', "/");

        let java_ver_str = java_ver.to_string();

        let mut has_java_path = false;
        let mut has_override = false;
        let mut has_automatic = false;
        let mut has_java_version = false;
        let updated: Vec<String> = content
            .lines()
            .map(|line| {
                if line.starts_with("JavaPath=") {
                    has_java_path = true;
                    format!("JavaPath={}", java_path_str)
                } else if line.starts_with("OverrideJavaLocation=") {
                    has_override = true;
                    "OverrideJavaLocation=true".to_string()
                } else if line.starts_with("AutomaticJava=") {
                    has_automatic = true;
                    "AutomaticJava=false".to_string()
                } else if line.starts_with("JavaVersion=") {
                    has_java_version = true;
                    format!("JavaVersion={}", java_ver_str)
                } else {
                    line.to_string()
                }
            })
            .collect();

        let mut result = updated.join(eol);
        if !has_java_path {
            result.push_str(eol);
            result.push_str(&format!("JavaPath={}", java_path_str));
        }
        if !has_override {
            result.push_str(eol);
            result.push_str("OverrideJavaLocation=true");
        }
        if !has_automatic {
            result.push_str(eol);
            result.push_str("AutomaticJava=false");
        }
        if !has_java_version {
            result.push_str(eol);
            result.push_str(&format!("JavaVersion={}", java_ver_str));
        }

        fs::write(&cfg_path, result).map_err(|e| format!("instance.cfg 쓰기 실패: {}", e))?;
        log::info!("JavaPath 설정 완료: {} (Java {})", java_path_str, java_ver);
    } else {
        return Err(format!("instance.cfg 없음: {}", instance_dir.display()));
    }

    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod archive_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn java_zip_parent_path_cannot_escape_target() {
        let root = tempfile::tempdir().unwrap();
        let prism_java = root.path().join("java");
        let target = root.path().join("installed");
        fs::create_dir_all(&prism_java).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("sentinel"), "old").unwrap();
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("runtime/../../escape.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"escape").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(extract_zip_archive(&bytes, &target, &prism_java).is_err());
        assert!(!root.path().join("escape.txt").exists());
        assert_eq!(fs::read_to_string(target.join("sentinel")).unwrap(), "old");
    }

    #[test]
    fn java_zip_refuses_junction_to_external_bin() {
        let root = tempfile::tempdir().unwrap();
        let prism_java = root.path().join("java");
        let target = root.path().join("installed");
        let outside = root.path().join("outside");
        fs::create_dir_all(&prism_java).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let link = target.join("bin");
        let linked = std::os::windows::fs::symlink_dir(&outside, &link).is_ok()
            || std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()
                .is_ok_and(|result| result.status.success());
        if !linked {
            eprintln!("symlink/junction 생성 권한이 없어 Java fixture를 건너뜁니다");
            return;
        }
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("runtime/bin/javaw.exe", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"fake").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(extract_zip_archive(&bytes, &target, &prism_java).is_err());
        assert!(!outside.join("javaw.exe").exists());
    }
}

#[cfg(test)]
mod java_requirement_tests {
    use super::*;

    #[tokio::test]
    async fn known_versions_have_confirmed_offline_requirements() {
        let cache = tempfile::tempdir().unwrap();
        for (minecraft, java) in [
            ("1.16.5", 8),
            ("1.17.1", 16),
            ("1.20.4", 17),
            ("1.20.5", 21),
            ("1.21", 21),
            ("26.1", 25),
        ] {
            let actual = resolve_java_version_with(minecraft, cache.path(), |_| async {
                Err("offline".to_string())
            })
            .await
            .unwrap();
            assert_eq!(actual, java, "Minecraft {minecraft}");
        }
        let unknown = resolve_java_version_with("26.2", cache.path(), |_| async {
            Err("offline".to_string())
        })
        .await
        .unwrap_err();
        assert!(unknown.contains("Java 요구사항"));
        assert!(unknown.contains("재가져오기"));
        assert!(
            resolve_java_version_with("unknown/version", cache.path(), |_| async {
                Err("offline".to_string())
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn official_prism_requirement_is_validated_and_cached() {
        let cache = tempfile::tempdir().unwrap();
        let fixture = include_str!("../fixtures/prism-minecraft-26.1-java.json");
        let actual =
            resolve_java_version_with("26.1", cache.path(), |_| async { Ok(fixture.to_string()) })
                .await
                .unwrap();
        assert_eq!(actual, 25);
        assert_eq!(
            fs::read_to_string(cache.path().join("26.1.json")).unwrap(),
            fixture
        );

        let cached = resolve_java_version_with("26.1", cache.path(), |_| async {
            Err("network unavailable".to_string())
        })
        .await
        .unwrap();
        assert_eq!(cached, 25);
        fs::write(
            cache.path().join("26.1.json"),
            r#"{"uid":"net.minecraft","version":"26.2","compatibleJavaMajors":[21]}"#,
        )
        .unwrap();
        let repaired =
            resolve_java_version_with("26.1", cache.path(), |_| async { Ok(fixture.to_string()) })
                .await
                .unwrap();
        assert_eq!(repaired, 25);
    }
}

#[cfg(test)]
mod tar_archive_tests {
    use super::*;

    fn fixture_tar(path: &str, content: &[u8]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        header.set_cksum();
        builder.append(&header, content).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn java_tar_parent_path_cannot_escape_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("installed");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("sentinel"), "old").unwrap();
        let archive = fixture_tar("runtime/../../escape.txt", b"escape");
        assert!(extract_tar_gz(&archive, &target).is_err());
        assert!(!root.path().join("escape.txt").exists());
        assert_eq!(fs::read_to_string(target.join("sentinel")).unwrap(), "old");
    }
}
