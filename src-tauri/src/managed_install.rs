use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MANIFEST: &str = ".auto-tong-managed.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ManagedManifest {
    schema_version: u32,
    source_id: String,
    files: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CommitJournal {
    schema_version: u32,
    phase: CommitPhase,
    commit_token: String,
    source_id: String,
    history_key: String,
    modified_secs: u64,
    target: PathBuf,
    backup_root: PathBuf,
    stage_instance: PathBuf,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum CommitPhase {
    Prepared,
    BackupMoved,
    Installed,
    Recorded,
}

pub struct CommitReceipt {
    journal_path: PathBuf,
    journal: CommitJournal,
}

fn write_journal(path: &Path, journal: &CommitJournal) -> Result<(), String> {
    let parent = path.parent().ok_or("설치 journal 폴더 없음")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".auto-tong-journal-write-")
        .tempfile_in(parent)
        .map_err(|e| format!("설치 journal 임시 파일 생성 실패: {e}"))?;
    serde_json::to_writer_pretty(&mut temporary, journal)
        .map_err(|e| format!("설치 journal 쓰기 실패: {e}"))?;
    temporary.write_all(b"\n").map_err(|e| e.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| format!("설치 journal 동기화 실패: {e}"))?;
    temporary
        .persist(path)
        .map_err(|e| format!("설치 journal 저장 실패: {}", e.error))?;
    Ok(())
}

pub fn commit_stage(
    stage_instance: &Path,
    target: &Path,
    source_id: &str,
    history_key: &str,
    modified_secs: u64,
) -> Result<CommitReceipt, String> {
    commit_stage_with_hook(
        stage_instance,
        target,
        source_id,
        history_key,
        modified_secs,
        |_| Ok(()),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitCheckpoint {
    InitialJournalWrite,
    InitialJournalPublish,
    Prepared,
    BackupRenamed,
    BackupRecorded,
    StageRenamed,
    InstalledRecorded,
}

fn commit_stage_with_hook(
    stage_instance: &Path,
    target: &Path,
    source_id: &str,
    history_key: &str,
    modified_secs: u64,
    mut checkpoint: impl FnMut(CommitCheckpoint) -> Result<(), String>,
) -> Result<CommitReceipt, String> {
    let instances = target.parent().ok_or("인스턴스 상위 폴더 없음")?;
    let root = instances.parent().ok_or("Prism 데이터 폴더 없음")?;
    let backup = tempfile::Builder::new()
        .prefix(".auto-tong-backup-")
        .tempdir_in(root)
        .map_err(|e| format!("설치 백업 폴더 생성 실패: {e}"))?;
    let mut initial_journal = tempfile::Builder::new()
        .prefix(".auto-tong-journal-write-")
        .tempfile_in(root)
        .map_err(|e| format!("설치 journal 임시 파일 생성 실패: {e}"))?;
    let unique_name = initial_journal
        .path()
        .file_name()
        .ok_or("설치 journal 임시 이름 없음")?
        .to_string_lossy();
    let journal_path = root.join(format!(".auto-tong-journal-{unique_name}.json"));
    let commit_token = journal_path
        .file_name()
        .ok_or("설치 journal 이름 없음")?
        .to_string_lossy()
        .to_string();
    let mut journal = CommitJournal {
        schema_version: 1,
        phase: CommitPhase::Prepared,
        commit_token,
        source_id: source_id.to_string(),
        history_key: history_key.to_string(),
        modified_secs,
        target: target.to_path_buf(),
        backup_root: backup.path().to_path_buf(),
        stage_instance: stage_instance.to_path_buf(),
    };
    checkpoint(CommitCheckpoint::InitialJournalWrite)?;
    serde_json::to_writer_pretty(&mut initial_journal, &journal)
        .map_err(|e| format!("설치 journal 쓰기 실패: {e}"))?;
    initial_journal
        .write_all(b"\n")
        .map_err(|e| format!("설치 journal 쓰기 실패: {e}"))?;
    initial_journal
        .as_file()
        .sync_all()
        .map_err(|e| format!("설치 journal 동기화 실패: {e}"))?;
    checkpoint(CommitCheckpoint::InitialJournalPublish)?;
    initial_journal
        .persist_noclobber(&journal_path)
        .map_err(|e| format!("설치 journal 저장 실패: {}", e.error))?;
    journal.backup_root = backup.keep();
    checkpoint(CommitCheckpoint::Prepared)?;
    let old = journal.backup_root.join("old");
    if target.exists() {
        fs::rename(target, &old).map_err(|e| format!("기존 인스턴스 백업 실패: {e}"))?;
    }
    checkpoint(CommitCheckpoint::BackupRenamed)?;
    journal.phase = CommitPhase::BackupMoved;
    write_journal(&journal_path, &journal)?;
    checkpoint(CommitCheckpoint::BackupRecorded)?;
    if let Err(error) = fs::rename(stage_instance, target) {
        if old.exists() {
            fs::rename(&old, target).map_err(|restore| {
                format!(
                "새 인스턴스 확정 실패: {error}; 기존 인스턴스 복원도 실패: {restore}; 백업: {}",
                journal.backup_root.display())
            })?;
        }
        fs::remove_dir_all(&journal.backup_root).ok();
        fs::remove_file(&journal_path).ok();
        return Err(format!("새 인스턴스 확정 실패: {error}"));
    }
    checkpoint(CommitCheckpoint::StageRenamed)?;
    journal.phase = CommitPhase::Installed;
    write_journal(&journal_path, &journal)?;
    checkpoint(CommitCheckpoint::InstalledRecorded)?;
    Ok(CommitReceipt {
        journal_path,
        journal,
    })
}

impl CommitReceipt {
    pub fn token(&self) -> &str {
        &self.journal.commit_token
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.journal.phase = CommitPhase::Recorded;
        write_journal(&self.journal_path, &self.journal)?;
        fs::remove_dir_all(&self.journal.backup_root)
            .map_err(|e| format!("설치 백업 정리 실패: {e}"))?;
        fs::remove_file(&self.journal_path).map_err(|e| format!("설치 journal 정리 실패: {e}"))?;
        Ok(())
    }
}

pub fn recover_journals(
    root: &Path,
    was_recorded: impl Fn(&str, u64, &str) -> bool,
) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let mut recovered = 0;
    for entry in fs::read_dir(root).map_err(|e| format!("설치 journal 검사 실패: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(".auto-tong-journal-") || !name.ends_with(".json") {
            continue;
        }
        let journal_path = entry.path();
        let journal: CommitJournal = serde_json::from_slice(
            &fs::read(&journal_path).map_err(|e| format!("설치 journal 읽기 실패: {e}"))?,
        )
        .map_err(|e| format!("설치 journal 손상 {}: {e}", journal_path.display()))?;
        if journal.schema_version != 1
            || journal.target.parent().and_then(Path::parent) != Some(root)
            || journal.backup_root.parent() != Some(root)
            || journal
                .stage_instance
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                != Some(root)
        {
            return Err(format!(
                "설치 journal 경로 검증 실패: {}",
                journal_path.display()
            ));
        }
        let old = journal.backup_root.join("old");
        if journal.phase != CommitPhase::Recorded
            && !was_recorded(
                &journal.history_key,
                journal.modified_secs,
                &journal.commit_token,
            )
        {
            if journal.target.exists() && (old.exists() || journal.phase != CommitPhase::Prepared) {
                let quarantine = tempfile::Builder::new()
                    .prefix(".auto-tong-recovery-")
                    .tempdir_in(root)
                    .map_err(|e| format!("복구 보관 폴더 생성 실패: {e}"))?
                    .keep();
                fs::rename(&journal.target, quarantine.join("new"))
                    .map_err(|e| format!("미기록 설치 보관 실패: {e}"))?;
            }
            if old.exists() {
                fs::rename(&old, &journal.target)
                    .map_err(|e| format!("기존 인스턴스 복원 실패: {e}"))?;
            }
        }
        if journal.backup_root.exists() {
            fs::remove_dir_all(&journal.backup_root)
                .map_err(|e| format!("복구 백업 정리 실패: {e}"))?;
        }
        fs::remove_file(&journal_path).map_err(|e| format!("복구 journal 정리 실패: {e}"))?;
        recovered += 1;
    }
    Ok(recovered)
}

fn checked_entries(root: &Path) -> Result<Vec<(String, PathBuf, bool)>, String> {
    let mut entries = Vec::new();
    if !root.exists() {
        return Ok(entries);
    }
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| format!("인스턴스 파일 검사 실패: {e}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("인스턴스 메타데이터 검사 실패: {e}"))?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(format!(
                "링크 또는 junction이 있는 인스턴스는 자동 수정하지 않습니다: {}",
                entry.path().display()
            ));
        }
        if entry.path() == root {
            continue;
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(format!(
                "특수 파일은 가져올 수 없습니다: {}",
                entry.path().display()
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_str()
            .ok_or("인스턴스 파일 이름이 UTF-8이 아닙니다")?
            .replace('\\', "/");
        crate::import_path::safe_relative(&relative)?;
        entries.push((relative, entry.path().to_path_buf(), metadata.is_dir()));
    }
    Ok(entries)
}

#[cfg(target_os = "windows")]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(target_os = "windows"))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("파일 해시 열기 실패: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("파일 해시 읽기 실패: {e}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn read_manifest(target: &Path) -> Result<Option<ManagedManifest>, String> {
    let path = target.join(MANIFEST);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(&path).map_err(|e| format!("관리 파일 목록 읽기 실패: {e}"))?;
    let manifest: ManagedManifest =
        serde_json::from_slice(&raw).map_err(|e| format!("관리 파일 목록 손상: {e}"))?;
    if manifest.schema_version != 1 {
        return Err("지원하지 않는 관리 파일 목록 버전".into());
    }
    for (name, hash) in &manifest.files {
        crate::import_path::safe_relative(name)?;
        if name == MANIFEST || hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("관리 파일 목록의 경로 또는 해시가 잘못되었습니다".into());
        }
    }
    Ok(Some(manifest))
}

/// Caller owns a unique stage outside Prism's instances directory.
/// Existing files are read only. Conflicts leave both stage and target intact.
pub fn merge_into_stage(stage: &Path, target: &Path, source_id: &str) -> Result<(), String> {
    merge_into_stage_with_available(stage, target, source_id, |path| {
        fs2::available_space(path).map_err(|e| format!("남은 저장 공간 확인 실패: {e}"))
    })
}

fn merge_into_stage_with_available(
    stage: &Path,
    target: &Path,
    source_id: &str,
    available: impl Fn(&Path) -> Result<u64, String>,
) -> Result<(), String> {
    if !stage.is_dir() {
        return Err("준비한 인스턴스가 없습니다".into());
    }
    if stage.join(MANIFEST).exists() {
        return Err("팩에 앱 관리 파일 목록과 충돌하는 파일이 있습니다".into());
    }
    let stage_entries = checked_entries(stage)?;
    let mut new_files = BTreeMap::new();
    for (relative, path, is_dir) in stage_entries {
        if !is_dir {
            new_files.insert(relative, sha256_file(&path)?);
        }
    }

    let old_entries = checked_entries(target)?;
    let old_manifest = read_manifest(target)?;
    if let Some(old) = &old_manifest {
        if old.source_id != source_id {
            return Err("기존 인스턴스가 다른 원본에 속합니다. 자동 덮어쓰기를 보류합니다".into());
        }
    }
    let old_files = old_manifest.as_ref().map(|m| &m.files);
    // Validate every collision before copying any old file into the stage.
    for (relative, old_path, is_dir) in &old_entries {
        if *is_dir || relative == MANIFEST {
            continue;
        }
        if let Some(new_hash) = new_files.get(relative) {
            let actual = sha256_file(old_path)?;
            let owned_unchanged = old_files
                .and_then(|files| files.get(relative))
                .is_some_and(|hash| hash.eq_ignore_ascii_case(&actual));
            if !owned_unchanged && !actual.eq_ignore_ascii_case(new_hash) {
                return Err(format!(
                    "사용자가 수정한 파일과 새 팩이 충돌합니다: {relative}"
                ));
            }
        }
    }
    let mut preserve_bytes = 0u64;
    for (relative, old_path, is_dir) in &old_entries {
        if *is_dir || relative == MANIFEST || new_files.contains_key(relative) {
            continue;
        }
        let actual = sha256_file(old_path)?;
        let obsolete_unchanged = old_files
            .and_then(|files| files.get(relative))
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&actual));
        if !obsolete_unchanged {
            preserve_bytes = preserve_bytes
                .checked_add(
                    fs::metadata(old_path)
                        .map_err(|e| format!("보존 파일 크기 검사 실패: {e}"))?
                        .len(),
                )
                .ok_or("보존 파일 크기 계산 범위 초과")?;
        }
    }
    let reserve = 1024 * 1024u64;
    if available(stage)? < preserve_bytes.saturating_add(reserve) {
        return Err(format!(
            "재가져오기 준비 공간 부족: 사용자 파일 {} bytes 보존에 필요한 여유 공간을 확보하세요",
            preserve_bytes
        ));
    }
    let stage_output = crate::confined_output::ConfinedOutput::open(stage)?;
    let previous_output = if target.is_dir() {
        Some(crate::confined_output::ConfinedOutput::open(target)?)
    } else {
        None
    };
    for (relative, old_path, is_dir) in old_entries {
        if relative == MANIFEST {
            continue;
        }
        if is_dir {
            stage_output.create_dir(&relative)?;
            continue;
        }
        if new_files.contains_key(&relative) {
            continue;
        }
        let actual = sha256_file(&old_path)?;
        let obsolete_unchanged = old_files
            .and_then(|files| files.get(&relative))
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&actual));
        if obsolete_unchanged {
            continue;
        }
        let mut source = previous_output
            .as_ref()
            .ok_or("기존 인스턴스가 사라졌습니다")?
            .open_file(&relative)?
            .ok_or("기존 사용자 파일이 사라졌습니다")?;
        let mut destination = stage_output.create_file(&relative, false)?;
        std::io::copy(&mut source, &mut destination)
            .map_err(|e| format!("사용자 파일 보존 실패 {relative}: {e}"))?;
    }
    let manifest = ManagedManifest {
        schema_version: 1,
        source_id: source_id.to_string(),
        files: new_files,
    };
    let mut file = stage_output.create_file(MANIFEST, false)?;
    serde_json::to_writer_pretty(&mut file, &manifest)
        .map_err(|e| format!("관리 파일 목록 쓰기 실패: {e}"))?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.sync_all()
        .map_err(|e| format!("관리 파일 목록 동기화 실패: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_initial_journal_failure_is_clean(stop: CommitCheckpoint) {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("PrismData");
        let target = data.join("instances/pack");
        let stage = data.join(".auto-tong-stage-test/instances/pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();

        let existing_journal = data.join(".auto-tong-journal-existing.json");
        let existing_backup = data.join(".auto-tong-backup-existing");
        fs::write(&existing_journal, b"existing journal").unwrap();
        fs::create_dir(&existing_backup).unwrap();
        fs::write(existing_backup.join("marker"), b"existing backup").unwrap();

        assert!(
            commit_stage_with_hook(&stage, &target, "source", "pack.zip", 1, |point| {
                if point == stop {
                    Err("injected initial journal failure".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );

        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
        assert_eq!(fs::read(stage.join("marker")).unwrap(), b"new");
        assert_eq!(fs::read(&existing_journal).unwrap(), b"existing journal");
        assert_eq!(
            fs::read(existing_backup.join("marker")).unwrap(),
            b"existing backup"
        );
        let mut artifacts: Vec<_> = fs::read_dir(&data)
            .unwrap()
            .filter_map(|entry| {
                let name = entry.unwrap().file_name().to_string_lossy().to_string();
                (name.starts_with(".auto-tong-journal-") || name.starts_with(".auto-tong-backup-"))
                    .then_some(name)
            })
            .collect();
        artifacts.sort();
        assert_eq!(
            artifacts,
            vec![
                ".auto-tong-backup-existing".to_string(),
                ".auto-tong-journal-existing.json".to_string()
            ]
        );
    }

    #[test]
    fn reimport_preserves_user_files_and_removes_only_unchanged_managed_files() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("installed");
        let first = root.path().join("first");
        fs::create_dir_all(first.join("mods")).unwrap();
        fs::write(first.join("mods/old.jar"), b"old").unwrap();
        fs::write(first.join("instance.cfg"), b"cfg1").unwrap();
        merge_into_stage(&first, &target, "source-a").unwrap();
        fs::rename(&first, &target).unwrap();
        fs::write(target.join("options.txt"), b"user choices").unwrap();
        fs::create_dir_all(target.join("saves/world")).unwrap();
        fs::write(target.join("saves/world/level.dat"), b"world").unwrap();
        fs::create_dir_all(target.join("screenshots")).unwrap();
        fs::write(target.join("screenshots/capture.png"), b"picture").unwrap();
        fs::create_dir_all(target.join("config")).unwrap();
        fs::write(target.join("config/user.toml"), b"user config").unwrap();

        let next = root.path().join("next");
        fs::create_dir_all(next.join("mods")).unwrap();
        fs::write(next.join("mods/new.jar"), b"new").unwrap();
        fs::write(next.join("instance.cfg"), b"cfg2").unwrap();
        merge_into_stage(&next, &target, "source-a").unwrap();
        assert!(!next.join("mods/old.jar").exists());
        assert_eq!(fs::read(next.join("mods/new.jar")).unwrap(), b"new");
        assert_eq!(fs::read(next.join("options.txt")).unwrap(), b"user choices");
        assert_eq!(
            fs::read(next.join("saves/world/level.dat")).unwrap(),
            b"world"
        );
        assert_eq!(
            fs::read(next.join("screenshots/capture.png")).unwrap(),
            b"picture"
        );
        assert_eq!(
            fs::read(next.join("config/user.toml")).unwrap(),
            b"user config"
        );
        assert_eq!(fs::read(next.join("instance.cfg")).unwrap(), b"cfg2");
    }

    #[test]
    fn edited_owned_file_and_unknown_collision_block_reimport() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("installed");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("options.txt"), b"user").unwrap();
        let next = root.path().join("next");
        fs::create_dir_all(&next).unwrap();
        fs::write(next.join("options.txt"), b"pack").unwrap();
        assert!(merge_into_stage(&next, &target, "source-a").is_err());
        assert_eq!(fs::read(target.join("options.txt")).unwrap(), b"user");

        fs::remove_file(next.join("options.txt")).unwrap();
        fs::write(next.join("mods.jar"), b"original").unwrap();
        merge_into_stage(&next, &target, "source-a").unwrap();
        fs::rename(&target, root.path().join("legacy")).unwrap();
        fs::rename(&next, &target).unwrap();
        fs::write(target.join("mods.jar"), b"user edit").unwrap();
        let third = root.path().join("third");
        fs::create_dir_all(&third).unwrap();
        fs::write(third.join("mods.jar"), b"update").unwrap();
        assert!(merge_into_stage(&third, &target, "source-a").is_err());
        assert_eq!(fs::read(target.join("mods.jar")).unwrap(), b"user edit");
    }

    #[test]
    fn insufficient_space_blocks_copy_before_changing_installed_instance() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("installed");
        let stage = root.path().join("stage");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("options.txt"), b"user data").unwrap();
        fs::write(stage.join("new.jar"), b"pack data").unwrap();
        let error =
            merge_into_stage_with_available(&stage, &target, "source", |_| Ok(0)).unwrap_err();
        assert!(error.contains("공간 부족"));
        assert_eq!(fs::read(target.join("options.txt")).unwrap(), b"user data");
        assert!(!stage.join(MANIFEST).exists());
    }

    #[test]
    fn initial_journal_write_failure_leaves_no_artifacts_or_changes() {
        assert_initial_journal_failure_is_clean(CommitCheckpoint::InitialJournalWrite);
    }

    #[test]
    fn initial_journal_publish_failure_leaves_no_artifacts_or_changes() {
        assert_initial_journal_failure_is_clean(CommitCheckpoint::InitialJournalPublish);
    }

    #[test]
    fn initial_journal_collision_preserves_existing_file_and_instances() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("PrismData");
        let target = data.join("instances/pack");
        let stage = data.join(".auto-tong-stage-test/instances/pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();

        let mut collision_path = None;
        let result = commit_stage_with_hook(&stage, &target, "source", "pack.zip", 1, |point| {
            if point == CommitCheckpoint::InitialJournalPublish {
                let pending_name = fs::read_dir(&data)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
                    .find(|name| name.starts_with(".auto-tong-journal-write-"))
                    .unwrap();
                let path = data.join(format!(".auto-tong-journal-{pending_name}.json"));
                fs::write(&path, b"existing collision").unwrap();
                collision_path = Some(path);
            }
            Ok(())
        });

        assert!(result.is_err());
        let collision_path = collision_path.unwrap();
        assert_eq!(fs::read(&collision_path).unwrap(), b"existing collision");
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
        assert_eq!(fs::read(stage.join("marker")).unwrap(), b"new");
        assert!(!fs::read_dir(&data).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".auto-tong-backup-")));
        assert!(!fs::read_dir(&data).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".auto-tong-journal-write-")));
    }

    #[test]
    fn interrupted_commit_restores_old_instance_at_each_checkpoint() {
        for stop in [
            CommitCheckpoint::Prepared,
            CommitCheckpoint::BackupRenamed,
            CommitCheckpoint::BackupRecorded,
            CommitCheckpoint::StageRenamed,
            CommitCheckpoint::InstalledRecorded,
        ] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("PrismData");
            let instances = data.join("instances");
            let target = instances.join("pack");
            let stage = data
                .join(".auto-tong-stage-test")
                .join("instances")
                .join("pack");
            fs::create_dir_all(&target).unwrap();
            fs::create_dir_all(&stage).unwrap();
            fs::write(target.join("marker"), b"old").unwrap();
            fs::write(stage.join("marker"), b"new").unwrap();
            assert!(
                commit_stage_with_hook(&stage, &target, "source", "pack.zip", 1, |point| {
                    if point == stop {
                        Err("simulated crash".into())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            recover_journals(&data, |_, _, _| false).unwrap();
            assert_eq!(fs::read(target.join("marker")).unwrap(), b"old", "{stop:?}");
            assert!(!fs::read_dir(&data).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".auto-tong-journal-")));
        }
    }

    #[test]
    fn recorded_commit_is_kept_and_backup_cleaned_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("PrismData");
        let instances = data.join("instances");
        let target = instances.join("pack");
        let stage = data
            .join(".auto-tong-stage-test")
            .join("instances")
            .join("pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();
        let receipt = commit_stage(&stage, &target, "source", "pack.zip", 2).unwrap();
        drop(receipt);
        recover_journals(&data, |key, modified, _| key == "pack.zip" && modified == 2).unwrap();
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"new");
        assert!(!fs::read_dir(&data).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".auto-tong-backup-")));
    }

    #[test]
    fn damaged_journal_blocks_automatic_recovery() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".auto-tong-journal-bad.json"), b"{").unwrap();
        assert!(recover_journals(root.path(), |_, _, _| false).is_err());
        assert!(root.path().join(".auto-tong-journal-bad.json").exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn locked_existing_file_defers_commit_and_keeps_old_instance() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("PrismData");
        let target = data.join("instances/pack");
        let stage = data.join(".auto-tong-stage-test/instances/pack");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("marker"))
            .unwrap();
        assert!(commit_stage(&stage, &target, "source", "pack.zip", 1).is_err());
        assert!(target.exists());
        drop(lock);
        recover_journals(&data, |_, _, _| false).unwrap();
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
    }
}
