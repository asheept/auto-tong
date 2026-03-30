use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Command에 플랫폼별 플래그 적용
fn silent_command(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// zip 파일이 유효한 PrismLauncher 인스턴스인지 검증
/// 바닐라 런처의 .minecraft 폴더를 그대로 압축한 경우를 감지
pub fn validate_zip(zip_path: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("zip 읽기 실패: {}", e))?;

    let mut has_instance_cfg = false;
    let mut has_mmc_pack = false;
    let mut vanilla_launcher_files: Vec<&'static str> = Vec::new();

    // 바닐라 런처 파일 목록 (이것들이 최상위에 있으면 .minecraft 폴더를 통째로 압축한 것)
    const VANILLA_MARKERS: &[&str] = &[
        "launcher_profiles.json",
        "launcher_settings.json",
        "launcher_accounts.json",
        "launcher_accounts_microsoft_store.json",
    ];

    const VANILLA_DIRS: &[&str] = &[
        "assets/objects/",
        "libraries/",
        "versions/",
    ];

    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().replace('\\', "/");

        // instance.cfg / mmc-pack.json 존재 여부 (래핑된 구조 포함)
        if name == "instance.cfg" || name.ends_with("/instance.cfg") {
            has_instance_cfg = true;
        }
        if name == "mmc-pack.json" || name.ends_with("/mmc-pack.json") {
            has_mmc_pack = true;
        }

        // 바닐라 런처 파일 감지
        for &marker in VANILLA_MARKERS {
            if name == marker {
                vanilla_launcher_files.push(marker);
            }
        }
    }

    // instance.cfg 또는 mmc-pack.json이 있으면 유효
    if has_instance_cfg || has_mmc_pack {
        return Ok(());
    }

    // 바닐라 런처 파일이 있으면 → .minecraft 폴더를 압축한 것
    if !vanilla_launcher_files.is_empty() {
        // 추가로 assets/objects/ 또는 libraries/ 디렉토리 존재 확인
        let mut has_vanilla_dirs = false;
        for i in 0..archive.len() {
            let entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry.name().replace('\\', "/");
            for &dir in VANILLA_DIRS {
                if name.starts_with(dir) {
                    has_vanilla_dirs = true;
                    break;
                }
            }
            if has_vanilla_dirs {
                break;
            }
        }

        if has_vanilla_dirs {
            return Err(
                "지원되지 않는 파일 형식입니다. 이 zip은 바닐라 런처의 .minecraft 폴더를 \
                 그대로 압축한 파일입니다. PrismLauncher 인스턴스 형식(instance.cfg 포함)으로 \
                 내보낸 파일만 가져올 수 있습니다."
                    .to_string(),
            );
        }
    }

    // instance.cfg도 없고 바닐라 마커도 없는 경우 → 일단 통과
    // (mods만 묶은 zip 등 다양한 형태가 있을 수 있음)
    Ok(())
}

pub fn import_modpack<F>(exe_path: &str, zip_path: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instances_dir = prism_instances_dir(exe_path)?;
    extract_zip(zip_path, &instances_dir, on_progress)
}

/// PrismLauncher 데이터 루트 폴더 찾기 (표준/portable 지원)
pub fn prism_data_dir(exe_path: &str) -> Result<std::path::PathBuf, String> {
    // 1. 표준 경로
    let standard = dirs::config_dir()
        .ok_or("설정 경로를 찾을 수 없습니다")?
        .join("PrismLauncher");
    if standard.join("instances").exists() {
        log::info!("PrismLauncher 데이터 폴더 (표준): {}", standard.display());
        return Ok(standard);
    }

    // 2. macOS 대체 경로
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let mac_path = home.join("Library/Application Support/PrismLauncher");
            if mac_path.join("instances").exists() {
                log::info!("PrismLauncher 데이터 폴더 (macOS): {}", mac_path.display());
                return Ok(mac_path);
            }
        }
    }

    // 3. 포터블 설치 (exe와 같은 폴더)
    let exe_parent = Path::new(exe_path).parent().ok_or("exe 경로 오류")?;
    if exe_parent.join("instances").exists() {
        log::info!("PrismLauncher 데이터 폴더 (portable): {}", exe_parent.display());
        return Ok(exe_parent.to_path_buf());
    }

    Err(format!(
        "PrismLauncher 데이터 폴더를 찾을 수 없습니다 (표준: {}, portable: {})",
        standard.display(),
        exe_parent.display()
    ))
}

/// PrismLauncher instances 폴더 찾기 (플랫폼별)
pub fn prism_instances_dir(exe_path: &str) -> Result<std::path::PathBuf, String> {
    prism_data_dir(exe_path).map(|d| d.join("instances"))
}

/// 파일이름에서 확장자만 제거하여 인스턴스 이름으로 사용
fn instance_name_from_zip(zip_path: &Path) -> String {
    zip_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .trim()
        .trim_end_matches(&['.', ' '][..])
        .to_string()
}

fn extract_zip<F>(zip_path: &Path, instances_dir: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instance_name = instance_name_from_zip(zip_path);
    if instance_name.is_empty() {
        return Err("파일이름에서 인스턴스 이름을 추출할 수 없습니다".to_string());
    }

    let file =
        fs::File::open(zip_path).map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("zip 읽기 실패: {}", e))?;

    // zip 구조 감지
    let mut root_prefix: Option<String> = None;
    {
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                let name = entry.name().replace('\\', "/");
                if name == "instance.cfg" || name == "mmc-pack.json" {
                    root_prefix = None;
                    break;
                }
                if name.ends_with("/instance.cfg") || name.ends_with("/mmc-pack.json") {
                    if let Some(pos) = name.find('/') {
                        root_prefix = Some(format!("{}/", &name[..pos]));
                    }
                    break;
                }
            }
        }
    }

    log::info!(
        "zip 형식: {}, 루트: {:?}",
        if root_prefix.is_some() { "wrapped" } else { "flat" },
        root_prefix
    );

    let total = archive.len();
    let instance_dir = instances_dir.join(&instance_name);
    fs::create_dir_all(&instance_dir)
        .map_err(|e| format!("인스턴스 디렉토리 생성 실패 {}: {}", instance_dir.display(), e))?;

    for i in 0..total {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");
        if raw_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        let relative_name = if let Some(ref prefix) = root_prefix {
            if let Some(rest) = raw_name.strip_prefix(prefix.as_str()) {
                rest.to_string()
            } else if raw_name.trim_end_matches('/') == prefix.trim_end_matches('/') {
                on_progress(i + 1, total);
                continue;
            } else {
                raw_name.clone()
            }
        } else {
            raw_name.clone()
        };

        if relative_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        // 경로 탐색 공격 차단: ".." 또는 절대경로 포함 시 건너뜀
        if relative_name.contains("..") || Path::new(&relative_name).is_absolute() {
            log::warn!("위험한 zip 엔트리 차단: {}", relative_name);
            on_progress(i + 1, total);
            continue;
        }

        let target = instance_dir.join(&relative_name);

        if entry.is_dir() || relative_name.ends_with('/') {
            fs::create_dir_all(&target)
                .map_err(|e| format!("디렉토리 생성 실패 {}: {}", target.display(), e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("디렉토리 생성 실패 {}: {}", parent.display(), e))?;
            }
            let mut outfile = fs::File::create(&target)
                .map_err(|e| format!("파일 생성 실패 {}: {}", target.display(), e))?;
            io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("파일 쓰기 실패: {}", e))?;
        }

        on_progress(i + 1, total);
    }

    // instance.cfg 의 name= 값 업데이트
    let cfg_path = instance_dir.join("instance.cfg");
    if cfg_path.exists() {
        if let Ok(content) = fs::read_to_string(&cfg_path) {
            let eol = if content.contains("\r\n") { "\r\n" } else { "\n" };
            let updated = content
                .lines()
                .map(|line| {
                    if line.starts_with("name=") {
                        format!("name={}", instance_name)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(eol);
            if let Err(e) = fs::write(&cfg_path, updated) {
                log::warn!("instance.cfg 업데이트 실패: {}", e);
            }
        }
    }

    log::info!("인스턴스 '{}' 으로 압축 해제 완료", instance_name);
    Ok(())
}

/// PrismLauncher 프로세스 상태를 확인하고 PID를 반환
fn get_pid_by_path(exe_path: &str) -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        let normalized = exe_path.replace('/', "\\");
        let output = silent_command(Command::new("powershell").args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-Process | Where-Object {{ $_.Path -eq '{}' }} | Select-Object -First 1 -ExpandProperty Id",
                normalized
            ),
        ]))
        .output()
        .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse::<u32>().ok()
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("pgrep")
            .args(["-f", exe_path])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().next()?.trim().parse::<u32>().ok()
    }
}

/// 해당 PID의 자식 프로세스 중 java가 있는지 확인
fn has_java_child(parent_pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        let output = silent_command(Command::new("powershell").args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {} -and $_.Name -match 'java' }} | Select-Object -First 1 ProcessId",
                parent_pid
            ),
        ]))
        .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.lines().any(|line| line.trim().parse::<u32>().is_ok())
            }
            Err(_) => false,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("pgrep")
            .args(["-P", &parent_pid.to_string(), "java"])
            .output();

        match output {
            Ok(o) => !o.stdout.is_empty(),
            Err(_) => false,
        }
    }
}

/// PrismLauncher 종료. 성공하면 true, 게임 실행 중이면 false.
pub async fn kill_prism(exe_path: &str) -> bool {
    match get_pid_by_path(exe_path) {
        Some(pid) => {
            if has_java_child(pid) {
                log::info!(
                    "PrismLauncher(PID:{}) 게임 실행 중 — 대기 (녹화 보호)",
                    pid
                );
                false
            } else {
                log::info!("PrismLauncher(PID:{}) 게임 미실행 — 종료", pid);
                kill_process(pid);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                true
            }
        }
        None => true, // 이미 꺼져있음
    }
}

/// PrismLauncher 시작
pub fn start_prism(exe_path: &str) {
    Command::new(exe_path).spawn().ok();
    log::info!("PrismLauncher 시작됨: {}", exe_path);
}

/// PrismLauncher 새로고침 시도. 성공하면 true, java 대기 중이면 false.
pub async fn try_refresh(exe_path: &str) -> bool {
    if kill_prism(exe_path).await {
        start_prism(exe_path);
        true
    } else {
        false
    }
}

fn kill_process(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        silent_command(Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]))
            .output()
            .ok();
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output()
            .ok();
    }
}
