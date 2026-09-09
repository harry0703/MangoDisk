//! Opt-in engineering evaluation. Native scans are read-only; provider answers
//! are evidence for human review, never instructions to execute on the machine.
use std::{fs, io::Write, path::PathBuf, time::Instant};

use serde::Deserialize;
use serde_json::json;
use tokio::sync::watch;

use super::{explain, AiConfiguration, AiContext, AiDelta, AiRequest, ReasoningMode};

#[test]
#[ignore = "reads real native catalogs into an explicitly selected private artifact directory"]
fn capture_ai_evaluation_catalogs() {
    let directory = PathBuf::from(
        std::env::var_os("MANGODISK_AI_EVAL_DIRECTORY").expect("explicit artifact directory"),
    );
    assert!(directory.is_absolute() && directory.is_dir());
    let paths = crate::ApplicationPaths::from_base_directories(
        directory.join("state"),
        directory.join("cache"),
    )
    .unwrap();
    crate::configure_application_paths(paths).unwrap();
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else {
        "macos"
    };
    // Write each result immediately so one slow scan does not discard completed evidence.
    macro_rules! capture {
        ($name:literal, $scan:expr) => {{
            let value = serde_json::to_vec_pretty(&$scan.expect("native catalog scan")).unwrap();
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(format!("{}-{platform}.json", $name)))
                .unwrap();
            file.write_all(&value).unwrap();
            println!("catalog_captured module={} bytes={}", $name, value.len());
        }};
    }
    capture!("startup", crate::StartupService::scan());
    capture!("systemOptimization", crate::SystemSettingsService::scan());
    capture!("systemMaintenance", crate::SystemMaintenanceService::scan());
    capture!(
        "privacy",
        crate::PrivacyService::scan(crate::PrivacyScanRequest {
            time_range: crate::privacy::PrivacyTimeRange::AllTime
        })
    );
    capture!(
        "cleanup",
        crate::CleanupScanService::scan_with_deep_project_discovery(false, |_| {})
    );
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationCase {
    id: String,
    context: AiContext,
    #[serde(default)]
    language: Option<String>,
}

async fn evaluate(
    case: &EvaluationCase,
    config: &AiConfiguration,
    index: usize,
) -> serde_json::Value {
    let started = Instant::now();
    let (_cancel, receiver) = watch::channel(false);
    let mut answer = String::new();
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        & 0xffff_ffff_ffff;
    // Gateways can validate the UUID version and RFC variant, not just its shape.
    let operation_id = format!("00000000-0000-4000-{:04x}-{suffix:012x}", 0x8000 | index);
    let result = explain(
        config.clone(),
        AiRequest {
            context: Some(case.context.clone()),
            language: case.language.as_deref().unwrap_or("zh-CN").into(),
        },
        &operation_id,
        receiver,
        |delta| {
            if let AiDelta::Text(text) = delta {
                answer.push_str(&text);
            }
            true
        },
    )
    .await;
    let elapsed_ms = started.elapsed().as_millis();
    println!(
        "evaluation_completed case={} success={} error={:?} elapsed_ms={elapsed_ms}",
        case.id,
        result.is_ok(),
        result.as_ref().err()
    );
    match result {
        Ok(usage) => {
            json!({"id":case.id,"language":case.language.as_deref().unwrap_or("zh-CN"),"module":case.context.subject.module_name(),"elapsedMs":elapsed_ms,"answer":answer,"usage":usage,"error":null})
        }
        Err(error) => {
            json!({"id":case.id,"module":case.context.subject.module_name(),"elapsedMs":elapsed_ms,"answer":answer,"error":error})
        }
    }
}

#[tokio::test]
#[ignore = "calls the real provider for an explicit corpus and incurs token usage"]
async fn evaluate_ai_corpus() {
    let input = std::env::var_os("MANGODISK_AI_EVAL_INPUT").expect("explicit corpus file");
    let output = std::env::var_os("MANGODISK_AI_EVAL_OUTPUT").expect("explicit output file");
    let cases: Vec<EvaluationCase> = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    assert!(!cases.is_empty() && cases.len() <= 64);
    for case in &cases {
        assert!(
            case.id.len() <= 80
                && case
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        );
        case.context.validate().unwrap();
        assert!(super::language::valid_language_tag(
            case.language.as_deref().unwrap_or("zh-CN")
        ));
    }
    let config = AiConfiguration {
        schema_version: 1,
        mode: super::AiServiceMode::Custom,
        free_consent: false,
        endpoint: std::env::var("MANGODISK_AI_TEST_ENDPOINT").expect("explicit endpoint"),
        model: std::env::var("MANGODISK_AI_TEST_MODEL").expect("explicit model"),
        api_key: std::env::var("ZENAI_AI_GATEWAY_API_KEY").expect("explicit credential"),
        reasoning: ReasoningMode::Default,
        temperature: None,
        max_tokens: None,
    };
    config.validate().unwrap();
    // Serialize when a provider limits concurrent streams. This evaluation
    // control does not alter production requests or add automatic retries.
    let concurrency = std::env::var("MANGODISK_AI_EVAL_CONCURRENCY")
        .map(|value| value.parse::<usize>().expect("numeric concurrency"))
        .unwrap_or(2);
    assert!((1..=2).contains(&concurrency));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    for (pair_index, pair) in cases.chunks(concurrency).enumerate() {
        let first = evaluate(&pair[0], &config, pair_index * concurrency);
        let values = if pair.len() == 2 {
            let (left, right) = tokio::join!(
                first,
                evaluate(&pair[1], &config, pair_index * concurrency + 1)
            );
            vec![left, right]
        } else {
            vec![first.await]
        };
        for value in values {
            writeln!(file, "{value}").unwrap();
        }
        file.flush().unwrap();
    }
}
