use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub size: u64,
    pub modified_nanos: u128,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct SourceIdentity {
    pub id: String,
    pub root: String,
    pub relative_path: String,
    pub suggested_instance_id: String,
}

fn normalized_root(root: &Path) -> Result<String, String> {
    let path = fs::canonicalize(root).map_err(|e| format!("원본 루트 확인 실패: {e}"))?;
    let text = path
        .to_str()
        .ok_or("원본 루트 이름이 UTF-8이 아닙니다")?
        .replace('\\', "/");
    #[cfg(target_os = "windows")]
    let text = text.to_lowercase();
    Ok(text.trim_end_matches('/').to_string())
}

pub fn identify(root: &Path, relative: &str, stem: &str) -> Result<SourceIdentity, String> {
    crate::import_path::safe_relative(relative)?;
    let root = normalized_root(root)?;
    let relative_path = relative.replace('\\', "/");
    #[cfg(target_os = "windows")]
    let relative_path = relative_path.to_lowercase();
    let digest = Sha256::digest(format!("{root}\0{relative_path}").as_bytes());
    let digest_hex = format!("{:x}", digest);
    let id = format!("source-{digest_hex}");
    let display: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    let display = display.trim_matches('-');
    let display = if display.is_empty() { "pack" } else { display };
    let suggested_instance_id = format!("{}-{}", display, &digest_hex[..12]);
    crate::import_path::safe_relative(&suggested_instance_id)?;
    Ok(SourceIdentity {
        id,
        root,
        relative_path,
        suggested_instance_id,
    })
}

fn observed_metadata(path: &Path) -> Result<(u64, u128), String> {
    let metadata = fs::metadata(path).map_err(|e| format!("원본 파일 정보 읽기 실패: {e}"))?;
    if !metadata.is_file() {
        return Err("원본이 일반 파일이 아닙니다".into());
    }
    let modified = metadata
        .modified()
        .map_err(|e| format!("원본 수정 시간 읽기 실패: {e}"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("원본 수정 시간이 잘못되었습니다: {e}"))?
        .as_nanos();
    Ok((metadata.len(), modified))
}

pub struct SourceSnapshot {
    _directory: tempfile::TempDir,
    pub archive: PathBuf,
    pub fingerprint: Fingerprint,
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut input = fs::File::open(path).map_err(|e| format!("원본 해시 열기 실패: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| format!("원본 해시 읽기 실패: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// A cheap identity check for already imported archives. A changed archive is
/// still copied and validated by `snapshot_when_stable` before installation.
pub fn probe_fingerprint(path: &Path) -> Result<Option<Fingerprint>, String> {
    let (size, modified_nanos) = observed_metadata(path)?;
    if size == 0 {
        return Ok(None);
    }
    let sha256 = sha256_file(path)?;
    if observed_metadata(path)? != (size, modified_nanos) {
        return Ok(None);
    }
    Ok(Some(Fingerprint {
        size,
        modified_nanos,
        sha256,
    }))
}

fn copy_snapshot_with(
    path: &Path,
    mut after_chunk: impl FnMut(),
) -> Result<SourceSnapshot, String> {
    let (size, modified_nanos) = observed_metadata(path)?;
    if size == 0 {
        return Err("빈 원본 파일은 가져오지 않습니다".into());
    }
    let name = path.file_name().ok_or("원본 파일 이름 없음")?;
    let directory = tempfile::Builder::new()
        .prefix("auto-tong-source-")
        .tempdir()
        .map_err(|e| format!("원본 snapshot 폴더 생성 실패: {e}"))?;
    let archive = directory.path().join(name);
    let mut input = fs::File::open(path).map_err(|e| format!("원본 열기 실패: {e}"))?;
    let mut output =
        fs::File::create(&archive).map_err(|e| format!("원본 snapshot 생성 실패: {e}"))?;
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| format!("원본 복사 읽기 실패: {e}"))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| format!("원본 snapshot 쓰기 실패: {e}"))?;
        hash.update(&buffer[..count]);
        copied += count as u64;
        after_chunk();
    }
    output
        .sync_all()
        .map_err(|e| format!("원본 snapshot 동기화 실패: {e}"))?;
    let copied_hash = format!("{:x}", hash.finalize());
    let (after_size, after_modified) = observed_metadata(path)?;
    if copied != size
        || after_size != size
        || after_modified != modified_nanos
        || sha256_file(path)? != copied_hash
    {
        return Err("복사 중 원본이 바뀌었습니다. 동기화 완료 후 다시 시도합니다".into());
    }
    Ok(SourceSnapshot {
        _directory: directory,
        archive,
        fingerprint: Fingerprint {
            size,
            modified_nanos,
            sha256: copied_hash,
        },
    })
}

pub async fn snapshot_when_stable(path: &Path) -> Result<Option<SourceSnapshot>, String> {
    let before = observed_metadata(path)?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    if observed_metadata(path)? != before {
        return Ok(None);
    }
    copy_snapshot_with(path, || {}).map(Some)
}

pub fn observed_seconds(fingerprint: &Fingerprint) -> u64 {
    (fingerprint.modified_nanos / 1_000_000_000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_names_root_changes_and_windows_spelling_have_stable_distinct_ids() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let a = identify(root.path(), "a/pack.zip", "pack").unwrap();
        let b = identify(root.path(), "b/pack.zip", "pack").unwrap();
        let moved = identify(other.path(), "a/pack.zip", "pack").unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.suggested_instance_id, b.suggested_instance_id);
        assert_ne!(a.id, moved.id);
        #[cfg(target_os = "windows")]
        {
            let spelling = identify(root.path(), "A\\PACK.ZIP", "pack").unwrap();
            assert_eq!(a.id, spelling.id);
        }
    }

    #[test]
    fn same_size_same_time_change_and_copy_race_are_detected() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("pack.zip");
        fs::write(&source, b"first").unwrap();
        let first = copy_snapshot_with(&source, || {}).unwrap();
        let modified: std::time::SystemTime = fs::metadata(&source).unwrap().modified().unwrap();
        fs::write(&source, b"other").unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let second = copy_snapshot_with(&source, || {}).unwrap();
        assert_eq!(first.fingerprint.size, second.fingerprint.size);
        assert_eq!(
            first.fingerprint.modified_nanos,
            second.fingerprint.modified_nanos
        );
        assert_ne!(first.fingerprint.sha256, second.fingerprint.sha256);

        let mut changed = false;
        let error = copy_snapshot_with(&source, || {
            if !changed {
                fs::write(&source, b"third").unwrap();
                fs::File::options()
                    .write(true)
                    .open(&source)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                changed = true;
            }
        })
        .err()
        .unwrap();
        assert!(error.contains("복사 중 원본이 바뀌었습니다"));
    }

    #[test]
    fn probe_detects_same_size_same_time_change_without_copying() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pack.zip");
        fs::write(&path, b"first").unwrap();
        let first = probe_fingerprint(&path).unwrap().unwrap();
        fs::write(&path, b"other").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_nanos(first.modified_nanos as u64)),
            )
            .unwrap();
        let second = probe_fingerprint(&path).unwrap().unwrap();
        assert_eq!(first.size, second.size);
        assert_eq!(first.modified_nanos, second.modified_nanos);
        assert_ne!(first.sha256, second.sha256);
    }
}
