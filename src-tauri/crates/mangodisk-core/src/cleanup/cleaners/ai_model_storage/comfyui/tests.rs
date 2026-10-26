use super::*;
use crate::{
    cleanup::{
        source_selection::SourceSelectionPolicy, CleanupSourceSelection, CleanupSourceSelectionMode,
    },
    shared::operation::{test_operation_lock, CoordinatedOperationKind},
};

struct Fixture {
    _directory: tempfile::TempDir,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().canonicalize().unwrap();
        Self {
            _directory: directory,
            home,
        }
    }
    fn file(&self, path: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.home.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
    fn config_directory(&self, name: &str) -> PathBuf {
        #[cfg(target_os = "macos")]
        let root = self.home.join("Library/Application Support");
        #[cfg(windows)]
        let root = self.home.join("AppData/Roaming");
        #[cfg(not(any(windows, target_os = "macos")))]
        let root = self.home.join(".config");
        root.join(name)
    }
    fn json(&self, path: impl AsRef<Path>, value: serde_json::Value) {
        self.file(path, &serde_json::to_vec(&value).unwrap());
    }
    fn installation(&self, path: &str) -> PathBuf {
        for name in [
            "main.py",
            "folder_paths.py",
            "comfy/__init__.py",
            "requirements.txt",
        ] {
            self.file(Path::new(path).join(name), b"fixture");
        }
        self.home.join(path)
    }
    fn scan(&self, projects: Vec<PathBuf>) -> (ScanRuleResult, Option<Preview>) {
        preview_in(&self.home, projects, &|| false, &|_| {}, &|_, _, _| {})
    }
}

#[test]
fn new_desktop_shared_and_per_instance_custom_models_are_discovered_once() {
    let f = Fixture::new();
    let shared = f.file(
        "ComfyUI-Shared/models/checkpoints/base.safetensors",
        b"shared",
    );
    let own = f.file(
        "ComfyUI-Installs/one/ComfyUI/models/loras/own.safetensors",
        b"own",
    );
    let custom = f.file("external models/loras/custom.GGUF", b"custom");
    f.installation("ComfyUI-Installs/one/ComfyUI");
    f.json(f.config_directory("Comfy Desktop").join("settings.json"), serde_json::json!({"modelsDirs":[f.home.join("ComfyUI-Shared/models"), f.home.join("ComfyUI-Shared/models/checkpoints")]}));
    f.json(f.config_directory("Comfy Desktop").join("installations.json"), serde_json::json!([{"installPath":f.home.join("ComfyUI-Installs/one"),"modelDirs":[f.home.join("external models")]}]));
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Found);
    assert!(!rule.default_selected && !rule.recommended_selected);
    let paths = plan
        .unwrap()
        .models
        .into_iter()
        .map(|e| e.model.path)
        .collect::<HashSet<_>>();
    assert_eq!(paths, HashSet::from([shared, own, custom]));
}

#[test]
fn legacy_desktop_base_path_and_multiline_yaml_resolve_relative_to_config() {
    let f = Fixture::new();
    let first = f.file("legacy/models/checkpoints/a.ckpt", b"first");
    let second = f.file("custom/weights/b.safetensors", b"second");
    let third = f.file("custom/with # spaces/c.pth", b"third");
    f.json(
        f.config_directory("ComfyUI").join("config.json"),
        serde_json::json!({"basePath":f.home.join("legacy")}),
    );
    let yaml = format!("external:\n  base_path: '{}'\n  checkpoints: |\n    weights\n    with # spaces\n  custom_nodes: ../private\n  configs: ../private\n", f.home.join("custom").display());
    f.file(
        f.config_directory("ComfyUI")
            .join("extra_models_config.yaml"),
        yaml.as_bytes(),
    );
    f.file("private/never.pt", b"private");
    let (_, plan) = f.scan(vec![]);
    let paths = plan
        .unwrap()
        .models
        .into_iter()
        .map(|e| e.model.path)
        .collect::<HashSet<_>>();
    assert_eq!(paths, HashSet::from([first, second, third]));
}

#[test]
fn indexed_manual_installations_resolve_external_yaml_without_touching_adjacent_data() {
    let f = Fixture::new();
    let install = f.installation("projects/manual");
    let own = f.file("projects/manual/models/vae/own.pt", b"own");
    let extra = f.file("outside/loras/x.safetensors", b"extra");
    f.file(
        "projects/manual/extra_model_paths.yaml",
        b"external:\n  base_path: ../../outside\n  loras: loras\n",
    );
    for name in [
        "output/image.png",
        "user/workflows/keep.json",
        "custom_nodes/plugin.py",
        "models/input/private.pt",
        "models/loras/uncompleted.safetensors.part",
    ] {
        f.file(Path::new("projects/manual").join(name), b"keep");
    }
    let (rule, plan) = f.scan(vec![install]);
    assert_eq!(rule.status, ScanItemStatus::Found);
    assert_eq!(
        plan.unwrap()
            .models
            .into_iter()
            .map(|e| e.model.path)
            .collect::<HashSet<_>>(),
        HashSet::from([own, extra])
    );
}

#[test]
fn diffusers_packages_remain_one_complete_model() {
    let f = Fixture::new();
    f.json(
        "ComfyUI-Shared/models/diffusers/pipeline/model_index.json",
        serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
    );
    f.file(
        "ComfyUI-Shared/models/diffusers/pipeline/unet/model.safetensors",
        b"weights",
    );
    f.file(
        "ComfyUI-Shared/models/diffusers/pipeline/tokenizer/vocab.json",
        b"{}",
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 1);
    assert_eq!(
        plan.unwrap().models[0].model.path,
        f.home.join("ComfyUI-Shared/models/diffusers/pipeline")
    );
}

#[test]
fn explicit_source_selection_dry_run_and_deletion_preserve_other_models_and_workflows() {
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let selected = f.file(
        "ComfyUI-Shared/models/checkpoints/remove.safetensors",
        b"remove",
    );
    let other = f.file(
        "ComfyUI-Shared/models/checkpoints/keep.safetensors",
        b"keep",
    );
    let workflow = f.file("ComfyUI-Shared/output/keep.png", b"output");
    let (_, plan) = f.scan(vec![]);
    let plan = plan.unwrap();
    let policy = SourceSelectionPolicy::from_request(
        &HashSet::from([CLEANER_ID.to_string()]),
        &[CleanupSourceSelection {
            rule_id: CLEANER_ID.into(),
            mode: CleanupSourceSelectionMode::Include,
            paths: vec![selected.to_string_lossy().into()],
        }],
    )
    .unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let dry = execute_preview(&plan, policy.scope(CLEANER_ID), true, &operation);
    assert_eq!(dry.status, CleanupActionStatus::Previewed);
    assert!(selected.exists());
    let result = execute_preview(&plan, policy.scope(CLEANER_ID), false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Completed);
    assert_eq!(result.released_bytes, 6);
    assert!(!selected.exists());
    assert!(other.exists());
    assert!(workflow.exists());
}

#[test]
fn changed_files_and_removed_configuration_revoke_deletion_authority() {
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let model = f.file("custom/a.safetensors", b"initial");
    let settings = f.config_directory("Comfy Desktop").join("settings.json");
    f.json(
        &settings,
        serde_json::json!({"modelsDirs":[f.home.join("custom")]}),
    );
    let (_, plan) = f.scan(vec![]);
    let plan = plan.unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    f.file(&model, b"changed-content");
    assert_eq!(
        execute_preview(&plan, None, false, &operation).status,
        CleanupActionStatus::Failed
    );
    assert!(model.exists());
    let (_, plan) = f.scan(vec![]);
    let plan = plan.unwrap();
    f.json(&settings, serde_json::json!({"modelsDirs":[]}));
    assert_eq!(
        execute_preview(&plan, None, false, &operation).status,
        CleanupActionStatus::Failed
    );
    assert!(model.exists());
}

#[test]
fn models_remain_selectable_and_deletable_without_process_inventory() {
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let model = f.file("ComfyUI-Shared/models/loras/active.safetensors", b"model");
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Found);
    assert!(rule.selectable);
    assert!(!rule.requires_app_close);
    assert!(rule.running_processes.is_empty());
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan.unwrap(), None, false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Completed);
    assert!(result.running_processes.is_empty());
    assert!(!model.exists());
}

#[test]
fn open_model_file_can_be_deleted_without_closing_its_reader() {
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let model = f.file("ComfyUI-Shared/models/loras/open.safetensors", b"model");
    let mut reader = fs::File::open(&model).unwrap();
    let (_, plan) = f.scan(vec![]);
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan.unwrap(), None, false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Completed);
    assert_eq!(result.released_bytes, 5);
    assert!(!model.exists());
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).unwrap();
    assert_eq!(contents, b"model");
}

#[cfg(unix)]
#[test]
fn deleting_an_empty_model_and_skipping_a_denied_model_reports_partial_success() {
    use std::os::unix::fs::PermissionsExt;
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let empty = f.file("ComfyUI-Shared/models/loras/empty.safetensors", b"");
    let denied = f.file("ComfyUI-Shared/models/locked/denied.safetensors", b"denied");
    let (_, plan) = f.scan(vec![]);
    let directory = denied.parent().unwrap();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan.unwrap(), None, false, &operation);
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!empty.exists());
    assert!(denied.exists());
    assert_eq!(result.affected_item_count, 1);
    assert_eq!(result.failed_item_count, 1);
    assert_eq!(result.released_bytes, 0);
    assert_eq!(result.reason_code, Some(CleanupActionReason::ItemsSkipped));
    assert_eq!(result.status, CleanupActionStatus::Partial);
}

#[cfg(windows)]
#[test]
fn locked_model_is_skipped_while_other_models_are_deleted_and_can_be_retried() {
    use std::os::windows::fs::OpenOptionsExt;
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let blocked = f.file(
        "ComfyUI-Shared/models/loras/locked.safetensors",
        b"locked model",
    );
    let removable = f.file("ComfyUI-Shared/models/loras/remove.safetensors", b"other");
    let (_, plan) = f.scan(vec![]);
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&blocked)
        .unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan.unwrap(), None, false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Partial);
    assert_eq!(result.reason_code, Some(CleanupActionReason::ItemsSkipped));
    assert_eq!(result.failed_item_count, 1);
    assert_eq!(result.affected_item_count, 1);
    assert_eq!(result.released_bytes, 5);
    assert!(blocked.exists());
    assert!(!removable.exists());
    assert_eq!(locked.metadata().unwrap().len(), 12);
    drop(locked);
    let remaining = PREVIEW.get().unwrap().lock().unwrap().clone().unwrap();
    let retry = execute_preview(&remaining, None, false, &operation);
    assert_eq!(retry.status, CleanupActionStatus::Completed);
    assert_eq!(retry.released_bytes, 12);
    assert!(!blocked.exists());
}

#[test]
#[ignore = "requires MANGODISK_COMFYUI_TEST_PYTHON for a real process validation"]
fn live_python_process_does_not_block_model_selection_or_deletion() {
    use std::process::{Child, Command, Stdio};
    struct Probe(Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _lock = test_operation_lock();
    let python = std::env::var_os("MANGODISK_COMFYUI_TEST_PYTHON")
        .expect("provide the validation Python executable");
    let mut probe = Probe(
        Command::new(python)
            .args(["-c", "import time; time.sleep(60)"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(probe.0.try_wait().unwrap().is_none());
    let f = Fixture::new();
    let model = f.file("ComfyUI-Shared/models/loras/live.safetensors", b"model");
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Found);
    assert!(rule.selectable && !rule.requires_app_close);
    assert!(rule.running_processes.is_empty());
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan.unwrap(), None, false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Completed);
    assert_eq!(result.released_bytes, 5);
    assert!(!model.exists());
    assert!(
        probe.0.try_wait().unwrap().is_none(),
        "cleanup must leave the Python process running"
    );
    println!(
        "comfyui_live_process_validation pid={} status={:?} bytes={}",
        probe.0.id(),
        result.status,
        result.released_bytes
    );
}

#[cfg(unix)]
#[test]
fn symbolic_links_never_authorize_external_weights() {
    let f = Fixture::new();
    let private = f.file("private/secret.safetensors", b"private");
    let root = f.home.join("ComfyUI-Shared/models");
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&private, root.join("linked.safetensors")).unwrap();
    std::os::unix::fs::symlink(private.parent().unwrap(), root.join("linked-dir")).unwrap();
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 0);
    assert!(plan.unwrap().models.is_empty());
    assert!(private.exists());
}

#[test]
fn configured_pipeline_roots_do_not_expose_individual_package_weights() {
    let f = Fixture::new();
    let package = f.home.join("ComfyUI-Shared/models/diffusers/pipeline");
    f.json(
        package.join("model_index.json"),
        serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
    );
    f.file(package.join("unet/model.safetensors"), b"model");
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":[package]}),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(
        rule.source_count, 1,
        "a package must not overlap with its internal weights"
    );
    assert_eq!(plan.unwrap().models[0].model.path, package);
}

#[test]
fn configured_output_directories_never_become_selectable_models() {
    let f = Fixture::new();
    let output = f.home.join("ComfyUI-Shared/output");
    f.file(
        output.join("private-training-result.safetensors"),
        b"private",
    );
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":[output]}),
    );
    let (rule, _) = f.scan(vec![]);
    assert_eq!(
        rule.source_count, 0,
        "a misconfigured root must not bypass output exclusions"
    );
    assert!(!rule.selectable);
}

#[test]
fn malformed_pipeline_markers_do_not_expose_partial_package_weights() {
    let f = Fixture::new();
    f.file(
        "ComfyUI-Shared/models/diffusers/broken/model_index.json",
        b"broken json",
    );
    f.file(
        "ComfyUI-Shared/models/diffusers/broken/unet/model.safetensors",
        b"model",
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(plan.is_none());
}

#[cfg(unix)]
#[test]
fn package_deletion_reports_only_confirmed_released_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let package = f.home.join("ComfyUI-Shared/models/diffusers/partial");
    f.json(
        package.join("model_index.json"),
        serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
    );
    f.file(package.join("00-model.safetensors"), b"first");
    f.file(package.join("zz-locked/weights.safetensors"), b"locked");
    let (_, plan) = f.scan(vec![]);
    let plan = plan.unwrap();
    let locked = package.join("zz-locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan, None, false, &operation);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        result.status,
        CleanupActionStatus::Failed | CleanupActionStatus::Partial
    ));
    assert!(result.released_bytes <= result.bytes_expected);
    if result.status == CleanupActionStatus::Failed {
        assert_eq!(result.released_bytes, 0);
    } else {
        assert!(result.released_bytes > 0);
    }
}

#[test]
fn diffusers_categories_without_indexes_do_not_offer_weight_fragments() {
    let f = Fixture::new();
    f.file(
        "ComfyUI-Shared/models/diffusers/incomplete/unet/weights.safetensors",
        b"weights",
    );
    let (rule, _) = f.scan(vec![]);
    assert_eq!(
        rule.source_count, 0,
        "incomplete Diffusers packages must not be split into weight-file sources"
    );
}

#[test]
fn explicit_diffusers_policy_survives_duplicate_and_overlapping_general_roots() {
    let _lock = test_operation_lock();
    let mut unexpected = Vec::new();
    for overlapping_parent in [false, true] {
        let f = Fixture::new();
        let pipeline_root = f.home.join("custom-models/pipelines");
        let fragment = f.file(
            pipeline_root.join("incomplete/unet/weights.safetensors"),
            b"fragment",
        );
        let ordinary = f.file("ComfyUI-Shared/models/loras/ordinary.safetensors", b"model");
        let complete = pipeline_root.join("complete");
        f.json(
            complete.join("model_index.json"),
            serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
        );
        f.file(complete.join("unet/weights.safetensors"), b"complete");
        let general_root = if overlapping_parent {
            pipeline_root.parent().unwrap()
        } else {
            &pipeline_root
        };
        f.json(
            f.config_directory("Comfy Desktop").join("settings.json"),
            serde_json::json!({"modelsDirs":[general_root]}),
        );
        f.file(
            f.config_directory("ComfyUI")
                .join("extra_models_config.yaml"),
            format!("external:\n  diffusers: '{}'\n", pipeline_root.display()).as_bytes(),
        );
        let (rule, plan) = f.scan(vec![]);
        assert_eq!(rule.status, ScanItemStatus::Found);
        let plan = plan.unwrap();
        let paths = plan
            .models
            .iter()
            .map(|e| e.model.path.clone())
            .collect::<HashSet<_>>();
        if paths != HashSet::from([ordinary.clone(), complete.clone()]) {
            unexpected.push((overlapping_parent, paths.contains(&fragment)));
            continue;
        }
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let result = execute_preview(&plan, None, false, &operation);
        assert_eq!(result.status, CleanupActionStatus::Completed);
        assert!(!ordinary.exists());
        assert!(!complete.exists());
        assert!(fragment.exists());
    }
    assert!(
        unexpected.is_empty(),
        "incomplete package fragments became selectable: {unexpected:?}"
    );
}

#[test]
fn custom_node_model_categories_accept_explicit_weight_paths_but_not_plugin_code() {
    let f = Fixture::new();
    let install = f.installation("manual");
    let model = f.file("external/ipadapter/adapter.safetensors", b"model");
    f.file("external/plugins/private.pth", b"plugin data");
    f.file(
        "manual/extra_model_paths.yaml",
        b"custom:\n  base_path: ../external\n  ipadapter: ipadapter\n  custom_nodes: plugins\n",
    );
    let (_, plan) = f.scan(vec![install]);
    assert_eq!(
        plan.unwrap()
            .models
            .into_iter()
            .map(|e| e.model.path)
            .collect::<Vec<_>>(),
        vec![model]
    );
}

#[test]
fn relative_json_model_roots_are_not_guessed_from_the_user_home() {
    let f = Fixture::new();
    f.file("models/unrelated.safetensors", b"unrelated");
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":["models"]}),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 0);
    assert!(!rule.selectable);
    assert!(plan.is_none());
}

#[test]
fn packages_with_partial_downloads_are_not_selectable() {
    let f = Fixture::new();
    let package = "ComfyUI-Shared/models/diffusers/downloading";
    f.json(
        Path::new(package).join("model_index.json"),
        serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
    );
    f.file(
        Path::new(package).join("unet/weights.safetensors"),
        b"weights",
    );
    f.file(
        Path::new(package).join("unet/new-weights.safetensors.part"),
        b"downloading",
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(!rule.selectable);
    assert!(plan.is_none());
}

#[test]
fn finder_metadata_does_not_block_complete_pipeline_packages() {
    let _lock = test_operation_lock();
    for regular_file in [true, false] {
        let f = Fixture::new();
        let package = f.home.join("ComfyUI-Shared/models/diffusers/complete");
        f.json(
            package.join("model_index.json"),
            serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
        );
        f.file(package.join("unet/weights.safetensors"), b"model");
        let metadata = package.join(".DS_Store");
        if regular_file {
            f.file(&metadata, b"Finder metadata");
        } else {
            f.file(metadata.join("unrelated.txt"), b"keep");
        }
        let (rule, plan) = f.scan(vec![]);
        if !regular_file {
            assert_eq!(rule.status, ScanItemStatus::Limited);
            assert!(plan.is_none());
            assert!(metadata.join("unrelated.txt").exists());
            continue;
        }
        assert_eq!(rule.status, ScanItemStatus::Found);
        let plan = plan.unwrap();
        assert_eq!(plan.models.len(), 1);
        let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
        let result = execute_preview(&plan, None, false, &operation);
        assert_eq!(result.status, CleanupActionStatus::Completed);
        assert!(!package.exists());
    }
}

#[test]
fn cancellation_and_deep_model_trees_revoke_the_preview() {
    let f = Fixture::new();
    f.file(
        "ComfyUI-Shared/models/checkpoints/first.safetensors",
        b"model",
    );
    let (rule, plan) = preview_in(&f.home, vec![], &|| true, &|_| {}, &|_, _, _| {});
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(!rule.selectable);
    assert!(plan.is_none());
    let deep = (0..MAX_DEPTH + 2).fold(PathBuf::from("ComfyUI-Shared/models"), |path, _| {
        path.join("nested")
    });
    f.file(deep.join("deep.safetensors"), b"model");
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(plan.is_none());
}

#[test]
fn equal_size_replacements_with_preserved_dates_cannot_reuse_preview_authority() {
    let _lock = test_operation_lock();
    let f = Fixture::new();
    let model = f.file("ComfyUI-Shared/models/loras/model.safetensors", b"first");
    let (_, plan) = f.scan(vec![]);
    let plan = plan.unwrap();
    let modified = fs::metadata(&model).unwrap().modified().unwrap();
    let replacement = f.file("ComfyUI-Shared/models/replacement.tmp", b"other");
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::remove_file(&model).unwrap();
    fs::rename(replacement, &model).unwrap();
    let operation = OperationGuard::start(CoordinatedOperationKind::Cleanup).unwrap();
    let result = execute_preview(&plan, None, false, &operation);
    assert_eq!(result.status, CleanupActionStatus::Failed);
    assert_eq!(fs::read(model).unwrap(), b"other");
}

#[test]
fn malformed_json_and_excessive_yaml_depth_never_authorize_cleanup() {
    let f = Fixture::new();
    f.file(
        "ComfyUI-Shared/models/checkpoints/model.safetensors",
        b"model",
    );
    let path = f.config_directory("Comfy Desktop").join("settings.json");
    f.file(&path, b"broken");
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(plan.is_none());
    fs::remove_file(path).unwrap();
    let yaml = format!("nested: {}0{}", "[".repeat(64), "]".repeat(64));
    f.file(
        f.config_directory("ComfyUI")
            .join("extra_models_config.yaml"),
        yaml.as_bytes(),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(plan.is_none());
}

#[test]
#[ignore = "measures metadata-only model discovery on an isolated filesystem workload"]
fn comfyui_fixture_performance_probe() {
    let f = Fixture::new();
    for index in 0..1000 {
        f.file(
            format!("ComfyUI-Shared/models/loras/{index:04}.safetensors"),
            b"weights",
        );
    }
    let large = f.file(
        "ComfyUI-Shared/models/checkpoints/large.safetensors",
        b"sparse fixture",
    );
    let large_bytes = if cfg!(unix) {
        4_u64 * 1024 * 1024 * 1024
    } else {
        16 * 1024 * 1024
    };
    fs::File::options()
        .write(true)
        .open(large)
        .unwrap()
        .set_len(large_bytes)
        .unwrap();
    for run in 0..3 {
        let start = Instant::now();
        let (rule, _) = f.scan(vec![]);
        assert_eq!(rule.status, ScanItemStatus::Found);
        assert_eq!(rule.source_count, 1001);
        assert_eq!(rule.bytes, large_bytes + 7000);
        println!(
            "comfyui_fixture_probe run={} models={} logical_bytes={} elapsed_ms={}",
            run,
            rule.source_count,
            rule.bytes,
            start.elapsed().as_millis()
        );
    }
}

#[cfg(windows)]
#[test]
fn junctions_preserve_external_models_and_never_become_model_roots() {
    let f = Fixture::new();
    let private = f.file("private/keep.safetensors", b"private");
    let root = f.home.join("ComfyUI-Shared/models");
    fs::create_dir_all(&root).unwrap();
    let link = root.join("linked-models");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(private.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the junction fixture must be created"
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 0);
    assert!(plan.unwrap().models.is_empty());
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":[link]}),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.status, ScanItemStatus::Limited);
    assert!(plan.is_none());
    assert!(private.exists());
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn case_aliases_of_the_home_root_cannot_expand_model_cleanup_scope() {
    let f = Fixture::new();
    let alias = f
        .home
        .with_file_name(f.home.file_name().unwrap().to_string_lossy().to_uppercase());
    if !alias.exists() || alias == f.home {
        return;
    }
    f.file("unrelated/private.safetensors", b"private");
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":[alias]}),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 0);
    assert!(!rule.selectable);
    assert!(plan.is_none());
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn case_aliases_inside_pipeline_packages_do_not_duplicate_weight_sources() {
    let f = Fixture::new();
    let package = f.home.join("ComfyUI-Shared/models/diffusers/pipeline");
    f.json(
        package.join("model_index.json"),
        serde_json::json!({"_class_name":"StableDiffusionPipeline"}),
    );
    f.file(package.join("unet/weights.safetensors"), b"model");
    let alias = package.with_file_name("PIPELINE").join("unet");
    if !alias.exists() {
        return;
    }
    f.json(
        f.config_directory("Comfy Desktop").join("settings.json"),
        serde_json::json!({"modelsDirs":[alias]}),
    );
    let (rule, plan) = f.scan(vec![]);
    assert_eq!(rule.source_count, 1);
    assert_eq!(plan.unwrap().models[0].model.path, package);
}
