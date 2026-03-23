use std::fs;
use std::path::{Path, PathBuf};

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
    } else if major == 1 && minor >= 18 && minor <= 20 {
        17
    } else if major == 1 && minor == 20 {
        // 1.20.5+ needs 21, 1.20.0-1.20.4 needs 17
        let patch = parts.get(2).copied().unwrap_or(0);
        if patch >= 5 { 21 } else { 17 }
    } else {
        // 1.21+
        21
    }
}

/// PrismLauncher java 폴더에서 해당 버전의 javaw.exe 경로를 찾기
fn find_java_in_prism(java_version: u32) -> Option<PathBuf> {
    let prism_java = dirs::config_dir()?.join("PrismLauncher").join("java");
    let folder_name = format!("java-{}", java_version);
    let javaw = prism_java.join(&folder_name).join("bin").join("javaw.exe");
    if javaw.exists() {
        Some(javaw)
    } else {
        None
    }
}

/// 시스템에서 Java를 찾기 (JAVA_HOME, PATH 등)
fn find_java_in_system(java_version: u32) -> Option<PathBuf> {
    // 일반적인 설치 경로 확인
    let program_files = std::env::var("ProgramFiles").unwrap_or_default();
    let candidates = vec![
        format!("{}/Eclipse Adoptium/jdk-{}", program_files, java_version),
        format!("{}/Eclipse Adoptium/jre-{}", program_files, java_version),
        format!("{}/Java/jdk-{}", program_files, java_version),
        format!("{}/Java/jdk{}", program_files, java_version),
    ];

    for candidate in candidates {
        let javaw = Path::new(&candidate).join("bin").join("javaw.exe");
        if javaw.exists() {
            return Some(javaw);
        }
    }
    None
}

/// Java가 존재하는지 확인, 없으면 다운로드
pub async fn ensure_java(java_version: u32) -> Result<PathBuf, String> {
    // 1. PrismLauncher java 폴더에서 찾기
    if let Some(path) = find_java_in_prism(java_version) {
        log::info!("Java {} 발견 (PrismLauncher): {}", java_version, path.display());
        return Ok(path);
    }

    // 2. 시스템에서 찾기
    if let Some(path) = find_java_in_system(java_version) {
        log::info!("Java {} 발견 (시스템): {}", java_version, path.display());
        return Ok(path);
    }

    // 3. 없으면 다운로드
    log::info!("Java {} 미설치 — Adoptium에서 다운로드합니다", java_version);
    download_java(java_version).await
}

/// Adoptium Temurin JDK 다운로드 및 설치
async fn download_java(java_version: u32) -> Result<PathBuf, String> {
    let prism_java = dirs::config_dir()
        .ok_or("APPDATA 경로를 찾을 수 없습니다")?
        .join("PrismLauncher")
        .join("java");
    fs::create_dir_all(&prism_java).map_err(|e| format!("java 폴더 생성 실패: {}", e))?;

    let url = format!(
        "https://api.adoptium.net/v3/binary/latest/{}/ga/windows/x64/jre/hotspot/normal/eclipse?project=jdk",
        java_version
    );

    log::info!("Java {} 다운로드 중: {}", java_version, url);

    // reqwest로 다운로드
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

    log::info!("Java {} 다운로드 완료 ({} MB)", java_version, bytes.len() / 1024 / 1024);

    // 임시 파일에 저장
    let temp_zip = prism_java.join(format!("java-{}-temp.zip", java_version));
    fs::write(&temp_zip, &bytes).map_err(|e| format!("임시 파일 저장 실패: {}", e))?;

    // 압축 해제
    let target_dir = prism_java.join(format!("java-{}", java_version));
    fs::create_dir_all(&target_dir).ok();

    let file = fs::File::open(&temp_zip).map_err(|e| format!("zip 열기 실패: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;

    // Adoptium zip은 "jdk-21.0.x+y-jre/" 같은 루트 폴더가 있음 → 제거
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

    // 임시 파일 삭제
    fs::remove_file(&temp_zip).ok();

    let javaw = target_dir.join("bin").join("javaw.exe");
    if javaw.exists() {
        log::info!("Java {} 설치 완료: {}", java_version, javaw.display());
        Ok(javaw)
    } else {
        Err(format!(
            "Java {} 설치 후 javaw.exe를 찾을 수 없습니다: {}",
            java_version,
            target_dir.display()
        ))
    }
}

/// 인스턴스의 mmc-pack.json을 읽어서 Java를 확보하고 instance.cfg에 JavaPath를 설정
pub async fn setup_java_for_instance(instance_dir: &Path) -> Result<(), String> {
    let mmc_pack_path = instance_dir.join("mmc-pack.json");
    if !mmc_pack_path.exists() {
        log::warn!("mmc-pack.json 없음: {}", instance_dir.display());
        return Ok(());
    }

    let content =
        fs::read_to_string(&mmc_pack_path).map_err(|e| format!("mmc-pack.json 읽기 실패: {}", e))?;

    let mc_version = match get_minecraft_version(&content) {
        Some(v) => v,
        None => {
            log::warn!("Minecraft 버전을 감지할 수 없습니다");
            return Ok(());
        }
    };

    let java_ver = required_java_version(&mc_version);
    log::info!("Minecraft {} → Java {} 필요", mc_version, java_ver);

    let javaw_path = ensure_java(java_ver).await?;

    // instance.cfg에 JavaPath 설정
    let cfg_path = instance_dir.join("instance.cfg");
    if cfg_path.exists() {
        let content = fs::read_to_string(&cfg_path).map_err(|e| format!("instance.cfg 읽기 실패: {}", e))?;

        let java_path_str = javaw_path.to_string_lossy().replace('/', "\\");
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

        fs::write(&cfg_path, result).map_err(|e| format!("instance.cfg 쓰기 실패: {}", e))?;
        log::info!("JavaPath 설정 완료: {}", java_path_str);
    }

    Ok(())
}
