use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn import_modpack<F>(exe_path: &str, zip_path: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(usize, usize),
{
    let instances_dir = dirs::config_dir()
        .ok_or("APPDATA 경로를 찾을 수 없습니다")?
        .join("PrismLauncher")
        .join("instances");

    if !instances_dir.exists() {
        let exe_parent = Path::new(exe_path).parent().ok_or("exe 경로 오류")?;
        let portable_instances = exe_parent.join("instances");
        if portable_instances.exists() {
            return extract_zip(zip_path, &portable_instances, on_progress);
        }
        return Err(format!(
            "PrismLauncher instances 폴더를 찾을 수 없습니다: {}",
            instances_dir.display()
        ));
    }

    extract_zip(zip_path, &instances_dir, on_progress)
}

/// 파일이름에서 확장자만 제거하여 인스턴스 이름으로 사용
/// 예: "260320-공룡킬-@각별.zip" → "260320-공룡킬-@각별"
/// 예: "@everyone.zip" → "@everyone"
fn instance_name_from_zip(zip_path: &Path) -> String {
    zip_path.file_stem()
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

    let file = fs::File::open(zip_path)
        .map_err(|e| format!("zip 파일 열기 실패: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("zip 읽기 실패: {}", e))?;

    // zip 구조 감지: 엔트리 이름(문자열)으로 판단
    // flat: instance.cfg가 루트에 있음 → 그대로 instance_name/ 에 풀기
    // wrapped: 루트폴더/instance.cfg → 루트폴더를 제거하고 풀기
    let mut root_prefix: Option<String> = None;
    {
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                let name = entry.name().replace('\\', "/");
                if name == "instance.cfg" || name == "mmc-pack.json" {
                    // flat 형식
                    root_prefix = None;
                    break;
                }
                if name.ends_with("/instance.cfg") || name.ends_with("/mmc-pack.json") {
                    // wrapped 형식: 첫 번째 / 앞이 루트 폴더
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
    fs::create_dir_all(&instance_dir).ok();

    for i in 0..total {
        let mut entry = archive.by_index(i)
            .map_err(|e| format!("zip 엔트리 읽기 실패: {}", e))?;

        let raw_name = entry.name().replace('\\', "/");
        if raw_name.is_empty() {
            on_progress(i + 1, total);
            continue;
        }

        // 루트 폴더 제거 (wrapped 형식일 때)
        let relative_name = if let Some(ref prefix) = root_prefix {
            if let Some(rest) = raw_name.strip_prefix(prefix.as_str()) {
                rest.to_string()
            } else if raw_name.trim_end_matches('/') == prefix.trim_end_matches('/') {
                // 루트 폴더 엔트리 자체 → 건너뜀
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

        let target = instance_dir.join(&relative_name);

        if entry.is_dir() || relative_name.ends_with('/') {
            fs::create_dir_all(&target).ok();
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).ok();
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
            let updated = content.lines().map(|line| {
                if line.starts_with("name=") {
                    format!("name={}", instance_name)
                } else {
                    line.to_string()
                }
            }).collect::<Vec<_>>().join("\n");
            fs::write(&cfg_path, updated).ok();
        }
    }

    log::info!("인스턴스 '{}' 으로 압축 해제 완료", instance_name);
    Ok(())
}

/// PrismLauncher 프로세스 상태를 확인하고 PID를 반환
fn get_pid_by_path(exe_path: &str) -> Option<u32> {
    let normalized = exe_path.replace('/', "\\");
    let output = Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!(
                "Get-Process | Where-Object {{ $_.Path -eq '{}' }} | Select-Object -First 1 -ExpandProperty Id",
                normalized
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim().parse::<u32>().ok()
}

/// 해당 PID의 자식 프로세스 중 java가 있는지 확인
fn has_java_child(parent_pid: u32) -> bool {
    let output = Command::new("powershell")
        .args([
            "-NoProfile", "-Command",
            &format!(
                "Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {} -and $_.Name -match 'java' }} | Select-Object -First 1 ProcessId",
                parent_pid
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().any(|line| line.trim().parse::<u32>().is_ok())
        }
        Err(_) => false,
    }
}

/// PrismLauncher 새로고침 시도. 성공하면 true, java 대기 중이면 false.
pub fn try_refresh(exe_path: &str) -> bool {
    match get_pid_by_path(exe_path) {
        Some(pid) => {
            if has_java_child(pid) {
                log::info!("PrismLauncher(PID:{}) 게임 실행 중 — 대기 (녹화 보호)", pid);
                false
            } else {
                log::info!("PrismLauncher(PID:{}) 게임 미실행 — 재시작", pid);
                Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .ok();
                std::thread::sleep(std::time::Duration::from_secs(1));
                Command::new(exe_path).spawn().ok();
                true
            }
        }
        None => {
            Command::new(exe_path).spawn().ok();
            log::info!("PrismLauncher 시작됨: {}", exe_path);
            true
        }
    }
}
