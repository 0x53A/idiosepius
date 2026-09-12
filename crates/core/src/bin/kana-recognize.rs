//! Standalone kana recognizer test bed.

#![recursion_limit = "256"]

use idiosepius_core::kana::VariantThresholds;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{self, Read},
    path::Path,
    process::ExitCode,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use idiosepius_core::kana::{
    ALGORITHM_ID, CharacterInk, InkPoint, KanaRecognizer, KanaScript, Recognition,
    RecognitionConfig, RecognitionState, VARIANT_MAXIMUM_DISTANCE, VARIANT_MINIMUM_MARGIN,
    synthetic::{self, SyntheticConfig, SyntheticPerturbations},
};
use serde::{Deserialize, Serialize};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let recognizer = KanaRecognizer::new()?;

    if args.first().is_some_and(|argument| {
        matches!(
            argument.as_str(),
            "--synthetic" | "--synthetic-scripted" | "--synthetic-ablation"
        )
    }) {
        let scripted = args[0] == "--synthetic-scripted";
        let ablation = args[0] == "--synthetic-ablation";
        let samples_per_character = args
            .get(1)
            .map(|value| value.parse())
            .transpose()
            .context("synthetic sample count must be an integer")?
            .unwrap_or(24);
        let seed = args
            .get(2)
            .map(|value| parse_seed(value))
            .transpose()?
            .unwrap_or(SyntheticConfig::default().seed);
        if args.len() > 3 {
            bail!(usage());
        }
        let config = SyntheticConfig {
            seed,
            samples_per_character,
            invalid_samples: samples_per_character.max(1) * 12,
            ..SyntheticConfig::default()
        };
        if ablation {
            let profiles = [
                ("baseline", SyntheticPerturbations::default()),
                (
                    "independent_strokes",
                    SyntheticPerturbations {
                        independent_strokes: true,
                        ..SyntheticPerturbations::default()
                    },
                ),
                (
                    "shortened_terminals",
                    SyntheticPerturbations {
                        shortened_terminals: true,
                        ..SyntheticPerturbations::default()
                    },
                ),
                (
                    "joined_strokes",
                    SyntheticPerturbations {
                        joined_strokes: true,
                        ..SyntheticPerturbations::default()
                    },
                ),
                ("all", config.perturbations),
            ];
            let reports: Vec<_> = profiles
                .into_iter()
                .map(|(profile, perturbations)| {
                    let started = Instant::now();
                    let report = synthetic::evaluate_scripted(
                        &recognizer,
                        SyntheticConfig {
                            perturbations,
                            ..config
                        },
                    );
                    synthetic_output(profile, seed, samples_per_character, started, report)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&reports)?);
            return Ok(());
        }
        let started = Instant::now();
        let report = if scripted {
            synthetic::evaluate_scripted(&recognizer, config)
        } else {
            synthetic::evaluate(&recognizer, config)
        };
        let elapsed = started.elapsed();
        let sample_count = (report.samples + report.invalid_samples).max(1) as f64;
        let recognizer_metadata = report.recognizer.clone();
        let output = serde_json::json!({
            "generator": synthetic::GENERATOR_ID,
            "recognizer": recognizer_metadata,
            "seed": seed,
            "scope": if scripted { "per_script" } else { "both_scripts" },
            "samples_per_character": samples_per_character,
            "elapsed_ms": elapsed.as_secs_f64() * 1_000.0,
            "mean_recognition_ms": report.recognition_elapsed_ms / sample_count,
            "mean_evaluation_ms_per_sample": elapsed.as_secs_f64() * 1_000.0 / sample_count,
            "top1_rate": report.top1_rate(),
            "top1_rate_wilson_95": wilson_95(report.top1, report.samples),
            "top5_rate": report.top5_rate(),
            "top5_rate_wilson_95": wilson_95(report.top5, report.samples),
            "accepted_correct": report.accepted_correct,
            "accepted_correct_rate": ratio(report.accepted_correct, report.samples),
            "accepted_correct_rate_wilson_95": wilson_95(
                report.accepted_correct,
                report.samples,
            ),
            "accepted_wrong": report.accepted_wrong,
            "accepted_wrong_rate": ratio(report.accepted_wrong, report.samples),
            "accepted_wrong_rate_wilson_95": wilson_95(
                report.accepted_wrong,
                report.samples,
            ),
            "rejection_rate": report.rejection_rate(),
            "rejection_rate_wilson_95": wilson_95(report.rejected, report.samples),
            "invalid_rejection_rate": report.invalid_rejection_rate(),
            "invalid_rejection_rate_wilson_95": wilson_95(
                report.invalid_rejected,
                report.invalid_samples,
            ),
            "report": report,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if args.len() == 1
        && args
            .first()
            .is_some_and(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        println!("{}", usage());
        return Ok(());
    }
    if args.len() > 1 {
        bail!(usage());
    }
    let source = args.first().map(String::as_str).unwrap_or("-");
    if source != "-" && Path::new(source).is_dir() {
        println!(
            "{}",
            serde_json::to_string_pretty(&evaluate_directory(&recognizer, Path::new(source))?)?
        );
        return Ok(());
    }
    let text = if source == "-" {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text)?;
        text
    } else {
        fs::read_to_string(source).with_context(|| format!("reading {source}"))?
    };
    let input = parse_input(&text)?;
    validate_scope(&recognizer, &input)?;
    let result = recognize_input(&recognizer, &input);
    let output = if input.saved {
        let current_fingerprint = format!("{:016x}", recognizer.template_fingerprint());
        serde_json::json!({
            "expected": input.expected,
            "expected_invalid": input.expected_invalid,
            "expected_source": input.expected_source,
            "prompt": input.prompt.as_ref(),
            "top1_matches": input.expected.zip(result.top().map(|candidate| candidate.character))
                .map(|(expected, actual)| expected == actual),
            "accepted_result_matches": input.expected.map(|expected| {
                result.recognized_character() == Some(expected)
            }),
            "invalid_rejected": input.expected_invalid.then_some({
                !matches!(result.state, RecognitionState::Recognized)
            }),
            "changed_since_save": input.stored_result.as_ref()
                .map(|stored| !stored.reproduced_by(&result)),
            "stored_algorithm": input.stored_algorithm.as_deref(),
            "current_algorithm": ALGORITHM_ID,
            "algorithm_changed": input.stored_algorithm.as_deref()
                .map(|stored| stored != ALGORITHM_ID),
            "stored_template_fingerprint": input.stored_template_fingerprint.as_deref(),
            "current_template_fingerprint": current_fingerprint.as_str(),
            "templates_changed": input.stored_template_fingerprint.as_deref()
                .map(|stored| stored != current_fingerprint.as_str()),
            "stored_config": {
                "maximum_distance": input.stored_maximum_distance,
                "maximum_path_length_ratio": input.stored_maximum_path_length_ratio,
                "minimum_margin": input.stored_minimum_margin,
                "variant_maximum_distance": input.stored_variant_thresholds.variant_maximum_distance,
                "variant_minimum_margin": input.stored_variant_thresholds.variant_minimum_margin,
            },
            "current_config": {
                "maximum_distance": recognizer.config().maximum_distance,
                "maximum_path_length_ratio": recognizer.config().maximum_path_length_ratio,
                "minimum_margin": recognizer.config().minimum_margin,
                "variant_maximum_distance": VARIANT_MAXIMUM_DISTANCE,
                "variant_minimum_margin": VARIANT_MINIMUM_MARGIN,
            },
            "config_changed": if input.stored_variant_thresholds.changed() { Some(true) } else {
                stored_config(&input).map(|stored| stored != recognizer.config())
                    .filter(|changed| *changed || input.stored_variant_thresholds.is_complete())
            },
            "result": result,
        })
    } else {
        serde_json::to_value(result)?
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn evaluate_directory(recognizer: &KanaRecognizer, directory: &Path) -> Result<serde_json::Value> {
    let mut paths = Vec::new();
    collect_json_paths(directory, &mut paths)?;
    paths.sort();

    let mut labeled = 0;
    let mut top1_correct = 0;
    let mut accepted_correct = 0;
    let mut accepted_wrong = 0;
    let mut valid_rejected = 0;
    let mut invalid_labeled = 0;
    let mut invalid_rejected = 0;
    let mut unlabeled = 0;
    let mut unlabeled_rejected = 0;
    let mut rejected = 0;
    let mut changed_since_save = 0;
    let mut stored_results = 0;
    let mut stored_result_missing_files = Vec::new();
    let mut recognizer_metadata_changed = 0;
    let mut recognizer_metadata_missing = 0;
    let mut recognizer_metadata_changes = Vec::new();
    let mut recognizer_metadata_missing_files = Vec::new();
    let mut failures = Vec::new();
    let mut invalid_false_acceptances = Vec::new();
    let mut prompted = Cohort::default();
    let mut prompt_runs: BTreeMap<u64, PromptRunAggregate> = BTreeMap::new();
    let mut prompt_run_metadata_missing_files = Vec::new();
    let mut prompt_sequence_metadata_missing_files = Vec::new();
    let mut manual = Cohort::default();
    let mut unspecified = Cohort::default();
    let mut characters: BTreeMap<char, Cohort> = BTreeMap::new();
    let mut confusions = BTreeMap::new();
    let mut accepted_confusions = BTreeMap::new();
    let mut valid_metrics = MetricSet::default();
    let mut invalid_metrics = MetricSet::default();
    let mut capture_metrics = CaptureMetrics::default();
    let mut errors = Vec::new();
    let mut directory_groups: BTreeMap<String, DirectoryGroup> = BTreeMap::new();
    let mut evaluated = 0;
    let mut recognition_elapsed_ms = 0.0;
    let mut corpus_fingerprint = CorpusFingerprint::default();
    let mut ink_groups: BTreeMap<u64, Vec<InkGroup>> = BTreeMap::new();

    for path in &paths {
        let group_name = directory_group(directory, path);
        directory_groups
            .entry(group_name.clone())
            .or_default()
            .files += 1;
        let relative = relative_file(directory, path);
        let bytes = match fs::read(path) {
            Ok(bytes) => {
                corpus_fingerprint.record(&relative, Some(&bytes));
                bytes
            }
            Err(error) => {
                corpus_fingerprint.record(&relative, None);
                directory_groups.get_mut(&group_name).unwrap().errors += 1;
                errors.push(file_error(directory, path, error));
                continue;
            }
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(error) => {
                directory_groups.get_mut(&group_name).unwrap().errors += 1;
                errors.push(file_error(directory, path, error));
                continue;
            }
        };
        let input = match parse_input(text) {
            Ok(input) => input,
            Err(error) => {
                directory_groups.get_mut(&group_name).unwrap().errors += 1;
                errors.push(file_error(directory, path, error));
                continue;
            }
        };
        if let Err(error) = validate_scope(recognizer, &input) {
            directory_groups.get_mut(&group_name).unwrap().errors += 1;
            errors.push(file_error(directory, path, error));
            continue;
        }
        record_ink(
            &mut ink_groups,
            &input.ink,
            ink_label(&input),
            relative.clone(),
        );
        capture_metrics.record(&input.ink);
        evaluated += 1;
        let started = Instant::now();
        let result = recognize_input(recognizer, &input);
        let sample_elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
        recognition_elapsed_ms += sample_elapsed_ms;
        directory_groups
            .get_mut(&group_name)
            .unwrap()
            .record(&input, &result, sample_elapsed_ms);
        let top = result.top();
        let was_rejected = !matches!(result.state, RecognitionState::Recognized);
        if was_rejected {
            rejected += 1;
        }
        if input.saved {
            if let Some(stored) = &input.stored_result {
                stored_results += 1;
                changed_since_save += usize::from(!stored.reproduced_by(&result));
            } else {
                stored_result_missing_files.push(relative_file(directory, path));
            }
        }
        let current_fingerprint = format!("{:016x}", recognizer.template_fingerprint());
        let metadata_missing = input.saved
            && (input.stored_algorithm.is_none()
                || input.stored_template_fingerprint.is_none()
                || stored_config(&input).is_none()
                || !input.stored_variant_thresholds.is_complete());
        if metadata_missing {
            recognizer_metadata_missing += 1;
            recognizer_metadata_missing_files.push(relative_file(directory, path));
        }
        let algorithm_changed = input
            .stored_algorithm
            .as_deref()
            .is_some_and(|stored| stored != ALGORITHM_ID);
        let templates_changed = input
            .stored_template_fingerprint
            .as_deref()
            .is_some_and(|stored| stored != current_fingerprint.as_str());
        let config_changed = stored_config(&input)
            .is_some_and(|stored| stored != recognizer.config())
            || input.stored_variant_thresholds.changed();
        if algorithm_changed || templates_changed || config_changed {
            recognizer_metadata_changed += 1;
            recognizer_metadata_changes.push(serde_json::json!({
                "file": relative_file(directory, path),
                "algorithm_changed": algorithm_changed,
                "templates_changed": templates_changed,
                "config_changed": config_changed,
            }));
        }
        if let Some(expected) = input.expected {
            valid_metrics.record(&result);
            labeled += 1;
            valid_rejected += usize::from(was_rejected);
            let top_matches = top.is_some_and(|candidate| candidate.character == expected);
            let accepted_matches = result.recognized_character() == Some(expected);
            let accepted_mismatch = result
                .recognized_character()
                .is_some_and(|recognized| recognized != expected);
            top1_correct += usize::from(top_matches);
            accepted_correct += usize::from(accepted_matches);
            accepted_wrong += usize::from(accepted_mismatch);
            let cohort = match input.expected_source {
                Some(ExpectedSource::Prompted) => {
                    let file = relative_file(directory, path);
                    if input
                        .prompt
                        .as_ref()
                        .is_none_or(|prompt| prompt.sequence.is_none())
                    {
                        prompt_sequence_metadata_missing_files.push(file.clone());
                    }
                    match input
                        .prompt
                        .as_ref()
                        .and_then(|prompt| prompt.run_started_at_unix_ms)
                    {
                        Some(run_started_at_unix_ms) => prompt_runs
                            .entry(run_started_at_unix_ms)
                            .or_default()
                            .record(
                                input.prompt.as_ref().unwrap(),
                                input.script_scope,
                                expected,
                                file,
                                SampleOutcome {
                                    top1_correct: top_matches,
                                    accepted_correct: accepted_matches,
                                    accepted_wrong: accepted_mismatch,
                                    rejected: was_rejected,
                                },
                            ),
                        None => prompt_run_metadata_missing_files.push(file),
                    }
                    &mut prompted
                }
                Some(ExpectedSource::Manual) => &mut manual,
                None => &mut unspecified,
            };
            cohort.record(top_matches, accepted_matches, accepted_mismatch);
            characters.entry(expected).or_default().record(
                top_matches,
                accepted_matches,
                accepted_mismatch,
            );
            if !top_matches && let Some(top) = top {
                *confusions.entry((expected, top.character)).or_insert(0) += 1;
                if accepted_mismatch {
                    *accepted_confusions
                        .entry((expected, top.character))
                        .or_insert(0) += 1;
                }
            }
            if !accepted_matches {
                failures.push(serde_json::json!({
                    "file": relative_file(directory, path),
                    "expected": expected,
                    "top": top.map(|candidate| candidate.character),
                    "distance": top.map(|candidate| candidate.distance),
                    "margin": candidate_margin(&result),
                    "path_length_ratio": result.path_length_ratio,
                    "state": result.state,
                    "expected_source": input.expected_source,
                }));
            }
        } else if input.expected_invalid {
            invalid_metrics.record(&result);
            invalid_labeled += 1;
            invalid_rejected += usize::from(was_rejected);
            if !was_rejected {
                invalid_false_acceptances.push(serde_json::json!({
                    "file": relative_file(directory, path),
                    "top": top.map(|candidate| candidate.character),
                    "distance": top.map(|candidate| candidate.distance),
                    "margin": candidate_margin(&result),
                    "path_length_ratio": result.path_length_ratio,
                    "expected_source": input.expected_source,
                }));
            }
        } else {
            unlabeled += 1;
            unlabeled_rejected += usize::from(was_rejected);
        }
    }

    let hiragana_coverage = coverage(recognizer, &characters, KanaScript::Hiragana);
    let katakana_coverage = coverage(recognizer, &characters, KanaScript::Katakana);
    let prompt_runs: Vec<_> = prompt_runs
        .into_iter()
        .map(|(run_started_at_unix_ms, run)| run.to_json(run_started_at_unix_ms, recognizer))
        .collect();
    let prompt_runs_complete = prompt_runs
        .iter()
        .filter(|run| run["complete"] == true)
        .count();
    let duplicate_ink_groups: Vec<_> = ink_groups
        .into_iter()
        .flat_map(|(fingerprint, groups)| {
            groups.into_iter().filter_map(move |group| {
                (group.files.len() > 1).then(|| {
                    let label_conflict = group.labels.len() > 1;
                    let labels = group
                        .labels
                        .into_iter()
                        .map(|(label, files)| {
                            serde_json::json!({
                                "label": label,
                                "samples": files.len(),
                                "files": files,
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!({
                        "fingerprint": format!("{fingerprint:016x}"),
                        "samples": group.files.len(),
                        "files": group.files,
                        "label_conflict": label_conflict,
                        "labels": labels,
                    })
                })
            })
        })
        .collect();
    let duplicate_ink_samples = duplicate_ink_groups
        .iter()
        .map(|group| group["samples"].as_u64().unwrap_or(0) as usize)
        .sum::<usize>();
    let duplicate_ink_excess = duplicate_ink_samples.saturating_sub(duplicate_ink_groups.len());
    let duplicate_ink_label_conflicts = duplicate_ink_groups
        .iter()
        .filter(|group| group["label_conflict"] == true)
        .count();
    Ok(serde_json::json!({
        "directory": directory,
        "files": paths.len(),
        "corpus_fingerprint": corpus_fingerprint.to_json(),
        "ink_fingerprint_algorithm": INK_FINGERPRINT_ALGORITHM,
        "duplicate_ink_group_count": duplicate_ink_groups.len(),
        "duplicate_ink_samples": duplicate_ink_samples,
        "duplicate_ink_excess": duplicate_ink_excess,
        "duplicate_ink_label_conflicts": duplicate_ink_label_conflicts,
        "duplicate_ink_groups": duplicate_ink_groups,
        "evaluated": evaluated,
        "recognition_elapsed_ms": recognition_elapsed_ms,
        "mean_recognition_ms": ratio_f64(recognition_elapsed_ms, evaluated),
        "labeled": labeled,
        "top1_correct": top1_correct,
        "top1_rate": ratio(top1_correct, labeled),
        "top1_rate_wilson_95": wilson_95(top1_correct, labeled),
        "accepted_correct": accepted_correct,
        "accepted_correct_rate": ratio(accepted_correct, labeled),
        "accepted_correct_rate_wilson_95": wilson_95(accepted_correct, labeled),
        "accepted_wrong": accepted_wrong,
        "accepted_wrong_rate": ratio(accepted_wrong, labeled),
        "accepted_wrong_rate_wilson_95": wilson_95(accepted_wrong, labeled),
        "valid_rejected": valid_rejected,
        "valid_rejection_rate": ratio(valid_rejected, labeled),
        "valid_rejection_rate_wilson_95": wilson_95(valid_rejected, labeled),
        "invalid_labeled": invalid_labeled,
        "invalid_rejected": invalid_rejected,
        "invalid_rejection_rate": ratio(invalid_rejected, invalid_labeled),
        "invalid_rejection_rate_wilson_95": wilson_95(invalid_rejected, invalid_labeled),
        "unlabeled": unlabeled,
        "unlabeled_rejected": unlabeled_rejected,
        "rejected": rejected,
        "stored_results": stored_results,
        "stored_result_missing": stored_result_missing_files.len(),
        "stored_result_missing_files": stored_result_missing_files,
        "changed_since_save": changed_since_save,
        "changed_since_save_rate": ratio(changed_since_save, stored_results),
        "recognizer_metadata_changed": recognizer_metadata_changed,
        "recognizer_metadata_missing": recognizer_metadata_missing,
        "recognizer_metadata_changes": recognizer_metadata_changes,
        "recognizer_metadata_missing_files": recognizer_metadata_missing_files,
        "current_algorithm": ALGORITHM_ID,
        "current_template_fingerprint": format!("{:016x}", recognizer.template_fingerprint()),
        "current_config": {
            "maximum_distance": recognizer.config().maximum_distance,
            "maximum_path_length_ratio": recognizer.config().maximum_path_length_ratio,
            "minimum_margin": recognizer.config().minimum_margin,
            "variant_maximum_distance": VARIANT_MAXIMUM_DISTANCE,
            "variant_minimum_margin": VARIANT_MINIMUM_MARGIN,
        },
        "cohorts": {
            "prompted": prompted.to_json(),
            "manual": manual.to_json(),
            "unspecified": unspecified.to_json(),
        },
        "directory_groups": directory_groups.into_iter().map(|(name, group)| {
            group.to_json(name, recognizer)
        }).collect::<Vec<_>>(),
        "prompt_run_count": prompt_runs.len(),
        "prompt_runs_complete": prompt_runs_complete,
        "prompt_runs_incomplete": prompt_runs.len() - prompt_runs_complete,
        "prompt_runs": prompt_runs,
        "prompt_run_metadata_missing": prompt_run_metadata_missing_files.len(),
        "prompt_run_metadata_missing_files": prompt_run_metadata_missing_files,
        "prompt_sequence_metadata_missing": prompt_sequence_metadata_missing_files.len(),
        "prompt_sequence_metadata_missing_files": prompt_sequence_metadata_missing_files,
        "metrics": {
            "valid_labeled": valid_metrics.to_json(),
            "invalid_labeled": invalid_metrics.to_json(),
        },
        "capture": capture_metrics.to_json(),
        "coverage": {
            "hiragana": hiragana_coverage,
            "katakana": katakana_coverage,
        },
        "characters": characters.into_iter().map(|(expected, cohort)| {
            cohort.to_json_with("expected", expected)
        }).collect::<Vec<_>>(),
        "confusions": confusions.into_iter().map(|((expected, recognized), count)| {
            serde_json::json!({
                "expected": expected,
                "recognized": recognized,
                "count": count,
            })
        }).collect::<Vec<_>>(),
        "accepted_confusions": accepted_confusions.into_iter().map(|((expected, recognized), count)| {
            serde_json::json!({
                "expected": expected,
                "recognized": recognized,
                "count": count,
            })
        }).collect::<Vec<_>>(),
        "errors": errors,
        "failures": failures,
        "invalid_false_acceptances": invalid_false_acceptances,
    }))
}

fn collect_json_paths(directory: &Path, paths: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in
        fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", directory.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("inspecting {}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_json_paths(&entry.path(), paths)?;
        } else if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn relative_file(directory: &Path, path: &Path) -> String {
    path.strip_prefix(directory)
        .unwrap_or(path)
        .iter()
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

const CORPUS_FINGERPRINT_ALGORITHM: &str = "fnv1a64-path-and-bytes-v1";

struct CorpusFingerprint {
    hash: u64,
    readable_files: usize,
    bytes: usize,
}

impl Default for CorpusFingerprint {
    fn default() -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
            readable_files: 0,
            bytes: 0,
        }
    }
}

impl CorpusFingerprint {
    fn record(&mut self, relative_path: &str, bytes: Option<&[u8]>) {
        fnv1a64_update(&mut self.hash, &[0x01]);
        fnv1a64_update(&mut self.hash, &(relative_path.len() as u64).to_le_bytes());
        fnv1a64_update(&mut self.hash, relative_path.as_bytes());
        match bytes {
            Some(bytes) => {
                fnv1a64_update(&mut self.hash, &[0x01]);
                fnv1a64_update(&mut self.hash, &(bytes.len() as u64).to_le_bytes());
                fnv1a64_update(&mut self.hash, bytes);
                self.readable_files += 1;
                self.bytes += bytes.len();
            }
            None => fnv1a64_update(&mut self.hash, &[0x00]),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "algorithm": CORPUS_FINGERPRINT_ALGORITHM,
            "value": format!("{:016x}", self.hash),
            "readable_files": self.readable_files,
            "bytes": self.bytes,
        })
    }
}

fn fnv1a64_update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

const INK_FINGERPRINT_ALGORITHM: &str = "fnv1a64-structured-ink-v1";

struct InkGroup {
    ink: CharacterInk,
    files: Vec<String>,
    labels: BTreeMap<String, Vec<String>>,
}

fn ink_label(input: &ParsedInput) -> String {
    if let Some(expected) = input.expected {
        format!("kana:{expected}")
    } else if input.expected_invalid {
        "invalid".to_owned()
    } else {
        "unlabeled".to_owned()
    }
}

fn record_ink(
    groups: &mut BTreeMap<u64, Vec<InkGroup>>,
    ink: &CharacterInk,
    label: String,
    file: String,
) {
    let fingerprint = ink_fingerprint(ink);
    let collisions = groups.entry(fingerprint).or_default();
    if let Some(group) = collisions.iter_mut().find(|group| group.ink == *ink) {
        group.files.push(file.clone());
        group.labels.entry(label).or_default().push(file);
    } else {
        let labels = BTreeMap::from([(label, vec![file.clone()])]);
        collisions.push(InkGroup {
            ink: ink.clone(),
            files: vec![file],
            labels,
        });
    }
}

fn ink_fingerprint(ink: &CharacterInk) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    fnv1a64_update(&mut hash, &(ink.0.len() as u64).to_le_bytes());
    for stroke in &ink.0 {
        fnv1a64_update(&mut hash, &(stroke.len() as u64).to_le_bytes());
        for point in stroke {
            fnv1a64_update(&mut hash, &point.x.to_bits().to_le_bytes());
            fnv1a64_update(&mut hash, &point.y.to_bits().to_le_bytes());
            match point.time_ms {
                Some(time_ms) => {
                    fnv1a64_update(&mut hash, &[0x01]);
                    fnv1a64_update(&mut hash, &time_ms.to_bits().to_le_bytes());
                }
                None => fnv1a64_update(&mut hash, &[0x00]),
            }
            match point.pressure {
                Some(pressure) => {
                    fnv1a64_update(&mut hash, &[0x01]);
                    fnv1a64_update(&mut hash, &pressure.to_bits().to_le_bytes());
                }
                None => fnv1a64_update(&mut hash, &[0x00]),
            }
            let pointer = match point.pointer {
                None => 0,
                Some(idiosepius_core::kana::PointerKind::Mouse) => 1,
                Some(idiosepius_core::kana::PointerKind::Pen) => 2,
                Some(idiosepius_core::kana::PointerKind::Touch) => 3,
                Some(idiosepius_core::kana::PointerKind::Unknown) => 4,
            };
            fnv1a64_update(&mut hash, &[pointer]);
        }
    }
    hash
}

fn directory_group(directory: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(directory).unwrap_or(path);
    if relative
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        return ".".to_owned();
    }
    relative
        .iter()
        .next()
        .map(|component| component.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_owned())
}

fn file_error(directory: &Path, path: &Path, error: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({
        "file": relative_file(directory, path),
        "error": error.to_string(),
    })
}

fn coverage(
    recognizer: &KanaRecognizer,
    characters: &BTreeMap<char, Cohort>,
    script: KanaScript,
) -> serde_json::Value {
    let inventory: Vec<_> = recognizer
        .characters()
        .filter_map(|(character, candidate_script)| {
            (candidate_script == script).then_some(character)
        })
        .collect();
    let observations = inventory
        .iter()
        .filter_map(|character| characters.get(character))
        .map(|cohort| cohort.labeled)
        .sum::<usize>();
    let observed_unique = inventory
        .iter()
        .filter(|character| characters.contains_key(character))
        .count();
    let missing: String = inventory
        .iter()
        .filter(|character| !characters.contains_key(character))
        .copied()
        .collect();
    serde_json::json!({
        "inventory": inventory.len(),
        "observations": observations,
        "observed_unique": observed_unique,
        "missing": missing,
    })
}

#[derive(Default)]
struct Cohort {
    labeled: usize,
    top1_correct: usize,
    accepted_correct: usize,
    accepted_wrong: usize,
}

#[derive(Default)]
struct DirectoryGroup {
    files: usize,
    evaluated: usize,
    errors: usize,
    recognition_elapsed_ms: f64,
    valid: Cohort,
    characters: BTreeMap<char, Cohort>,
    valid_rejected: usize,
    invalid_labeled: usize,
    invalid_rejected: usize,
    unlabeled: usize,
    unlabeled_rejected: usize,
    valid_metrics: MetricSet,
    invalid_metrics: MetricSet,
    capture_metrics: CaptureMetrics,
}

impl DirectoryGroup {
    fn record(&mut self, input: &ParsedInput, result: &Recognition, elapsed_ms: f64) {
        self.evaluated += 1;
        self.recognition_elapsed_ms += elapsed_ms;
        self.capture_metrics.record(&input.ink);
        let rejected = !matches!(result.state, RecognitionState::Recognized);
        if let Some(expected) = input.expected {
            let top1_correct = result
                .top()
                .is_some_and(|candidate| candidate.character == expected);
            self.valid.record(
                top1_correct,
                result.recognized_character() == Some(expected),
                result
                    .recognized_character()
                    .is_some_and(|recognized| recognized != expected),
            );
            self.characters.entry(expected).or_default().record(
                top1_correct,
                result.recognized_character() == Some(expected),
                result
                    .recognized_character()
                    .is_some_and(|recognized| recognized != expected),
            );
            self.valid_rejected += usize::from(rejected);
            self.valid_metrics.record(result);
        } else if input.expected_invalid {
            self.invalid_labeled += 1;
            self.invalid_rejected += usize::from(rejected);
            self.invalid_metrics.record(result);
        } else {
            self.unlabeled += 1;
            self.unlabeled_rejected += usize::from(rejected);
        }
    }

    fn to_json(&self, name: String, recognizer: &KanaRecognizer) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "files": self.files,
            "evaluated": self.evaluated,
            "errors": self.errors,
            "recognition_elapsed_ms": self.recognition_elapsed_ms,
            "mean_recognition_ms": ratio_f64(self.recognition_elapsed_ms, self.evaluated),
            "labeled": self.valid.labeled,
            "top1_correct": self.valid.top1_correct,
            "top1_rate": ratio(self.valid.top1_correct, self.valid.labeled),
            "top1_rate_wilson_95": wilson_95(self.valid.top1_correct, self.valid.labeled),
            "accepted_correct": self.valid.accepted_correct,
            "accepted_correct_rate": ratio(self.valid.accepted_correct, self.valid.labeled),
            "accepted_correct_rate_wilson_95": wilson_95(
                self.valid.accepted_correct,
                self.valid.labeled,
            ),
            "accepted_wrong": self.valid.accepted_wrong,
            "accepted_wrong_rate": ratio(self.valid.accepted_wrong, self.valid.labeled),
            "accepted_wrong_rate_wilson_95": wilson_95(
                self.valid.accepted_wrong,
                self.valid.labeled,
            ),
            "valid_rejected": self.valid_rejected,
            "valid_rejection_rate": ratio(self.valid_rejected, self.valid.labeled),
            "valid_rejection_rate_wilson_95": wilson_95(
                self.valid_rejected,
                self.valid.labeled,
            ),
            "invalid_labeled": self.invalid_labeled,
            "invalid_rejected": self.invalid_rejected,
            "invalid_rejection_rate": ratio(self.invalid_rejected, self.invalid_labeled),
            "invalid_rejection_rate_wilson_95": wilson_95(
                self.invalid_rejected,
                self.invalid_labeled,
            ),
            "unlabeled": self.unlabeled,
            "unlabeled_rejected": self.unlabeled_rejected,
            "metrics": {
                "valid_labeled": self.valid_metrics.to_json(),
                "invalid_labeled": self.invalid_metrics.to_json(),
            },
            "coverage": {
                "hiragana": coverage(recognizer, &self.characters, KanaScript::Hiragana),
                "katakana": coverage(recognizer, &self.characters, KanaScript::Katakana),
            },
            "capture": self.capture_metrics.to_json(),
        })
    }
}

#[derive(Default)]
struct MetricSet {
    distance: Vec<f32>,
    margin: Vec<f32>,
    path_length_ratio: Vec<f32>,
}

#[derive(Default)]
struct CaptureMetrics {
    samples: usize,
    stroke_count: Vec<f32>,
    point_count: Vec<f32>,
    duration_ms: Vec<f32>,
    points: usize,
    points_with_finite_time: usize,
    points_with_pressure: usize,
    points_with_pointer: usize,
    mouse_samples: usize,
    pen_samples: usize,
    touch_samples: usize,
    unknown_samples: usize,
    mixed_samples: usize,
    unspecified_samples: usize,
}

impl CaptureMetrics {
    fn record(&mut self, ink: &CharacterInk) {
        self.samples += 1;
        self.stroke_count.push(ink.0.len() as f32);
        let points: Vec<_> = ink.0.iter().flatten().collect();
        self.point_count.push(points.len() as f32);
        self.points += points.len();
        self.points_with_finite_time += points
            .iter()
            .filter(|point| point.time_ms.is_some_and(f64::is_finite))
            .count();
        self.points_with_pressure += points
            .iter()
            .filter(|point| point.pressure.is_some())
            .count();
        self.points_with_pointer += points
            .iter()
            .filter(|point| point.pointer.is_some())
            .count();

        let mut earliest = f64::INFINITY;
        let mut latest = f64::NEG_INFINITY;
        for time in points
            .iter()
            .filter_map(|point| point.time_ms)
            .filter(|time| time.is_finite())
        {
            earliest = earliest.min(time);
            latest = latest.max(time);
        }
        if earliest.is_finite() {
            let duration = (latest - earliest).max(0.0);
            if duration.is_finite() && duration <= f64::from(f32::MAX) {
                self.duration_ms.push(duration as f32);
            }
        }

        let mut pointer_mask = 0_u8;
        for pointer in points.iter().filter_map(|point| point.pointer) {
            pointer_mask |= match pointer {
                idiosepius_core::kana::PointerKind::Mouse => 0b0001,
                idiosepius_core::kana::PointerKind::Pen => 0b0010,
                idiosepius_core::kana::PointerKind::Touch => 0b0100,
                idiosepius_core::kana::PointerKind::Unknown => 0b1000,
            };
        }
        match pointer_mask {
            0 => self.unspecified_samples += 1,
            0b0001 => self.mouse_samples += 1,
            0b0010 => self.pen_samples += 1,
            0b0100 => self.touch_samples += 1,
            0b1000 => self.unknown_samples += 1,
            _ => self.mixed_samples += 1,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "samples": self.samples,
            "stroke_count": metric_summary(&self.stroke_count),
            "point_count": metric_summary(&self.point_count),
            "duration_ms": metric_summary(&self.duration_ms),
            "point_metadata": {
                "points": self.points,
                "finite_time": self.points_with_finite_time,
                "finite_time_rate": ratio(self.points_with_finite_time, self.points),
                "pressure": self.points_with_pressure,
                "pressure_rate": ratio(self.points_with_pressure, self.points),
                "pointer": self.points_with_pointer,
                "pointer_rate": ratio(self.points_with_pointer, self.points),
            },
            "pointer_sample_cohorts": {
                "mouse": self.mouse_samples,
                "pen": self.pen_samples,
                "touch": self.touch_samples,
                "unknown": self.unknown_samples,
                "mixed": self.mixed_samples,
                "unspecified": self.unspecified_samples,
            },
        })
    }
}

impl MetricSet {
    fn record(&mut self, recognition: &Recognition) {
        if let Some(top) = recognition.top() {
            self.distance.push(top.distance);
        }
        if let Some(margin) = candidate_margin(recognition) {
            self.margin.push(margin);
        }
        if let Some(path_length_ratio) = recognition.path_length_ratio {
            self.path_length_ratio.push(path_length_ratio);
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "distance": metric_summary(&self.distance),
            "margin": metric_summary(&self.margin),
            "path_length_ratio": metric_summary(&self.path_length_ratio),
        })
    }
}

fn metric_summary(values: &[f32]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let quantile = |fraction: f32| {
        (!sorted.is_empty()).then(|| {
            let rank = (fraction * sorted.len() as f32).ceil() as usize;
            sorted[rank.max(1).min(sorted.len()) - 1]
        })
    };
    serde_json::json!({
        "count": sorted.len(),
        "min": sorted.first(),
        "p05": quantile(0.05),
        "p50": quantile(0.50),
        "p95": quantile(0.95),
        "max": sorted.last(),
    })
}

fn candidate_margin(recognition: &Recognition) -> Option<f32> {
    recognition
        .candidates
        .first()
        .zip(recognition.candidates.get(1))
        .map(|(first, second)| second.distance - first.distance)
}

impl Cohort {
    fn record(&mut self, top1_correct: bool, accepted_correct: bool, accepted_wrong: bool) {
        debug_assert!(!(accepted_correct && accepted_wrong));
        self.labeled += 1;
        self.top1_correct += usize::from(top1_correct);
        self.accepted_correct += usize::from(accepted_correct);
        self.accepted_wrong += usize::from(accepted_wrong);
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "labeled": self.labeled,
            "top1_correct": self.top1_correct,
            "top1_rate": ratio(self.top1_correct, self.labeled),
            "top1_rate_wilson_95": wilson_95(self.top1_correct, self.labeled),
            "accepted_correct": self.accepted_correct,
            "accepted_correct_rate": ratio(self.accepted_correct, self.labeled),
            "accepted_correct_rate_wilson_95": wilson_95(
                self.accepted_correct,
                self.labeled,
            ),
            "accepted_wrong": self.accepted_wrong,
            "accepted_wrong_rate": ratio(self.accepted_wrong, self.labeled),
            "accepted_wrong_rate_wilson_95": wilson_95(
                self.accepted_wrong,
                self.labeled,
            ),
        })
    }

    fn to_json_with(&self, label: &str, value: char) -> serde_json::Value {
        let mut object = self.to_json();
        object.as_object_mut().unwrap().insert(
            label.to_owned(),
            serde_json::Value::String(value.to_string()),
        );
        object
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn ratio_f64(numerator: f64, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator / denominator as f64)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct ConfidenceInterval {
    lower: f64,
    upper: f64,
}

/// Two-sided 95% Wilson score interval for a binomial proportion.
fn wilson_95(successes: usize, total: usize) -> Option<ConfidenceInterval> {
    if total == 0 || successes > total {
        return None;
    }
    const Z: f64 = 1.959_963_984_540_054;
    let n = total as f64;
    let proportion = successes as f64 / n;
    let z_squared = Z * Z;
    let denominator = 1.0 + z_squared / n;
    let center = (proportion + z_squared / (2.0 * n)) / denominator;
    let half_width =
        Z * (proportion * (1.0 - proportion) / n + z_squared / (4.0 * n * n)).sqrt() / denominator;
    Some(ConfidenceInterval {
        lower: (center - half_width).max(0.0),
        upper: (center + half_width).min(1.0),
    })
}

fn synthetic_output(
    profile: &str,
    seed: u64,
    samples_per_character: usize,
    started: Instant,
    report: synthetic::SyntheticReport,
) -> serde_json::Value {
    let elapsed = started.elapsed();
    let sample_count = (report.samples + report.invalid_samples).max(1) as f64;
    let recognizer_metadata = report.recognizer.clone();
    serde_json::json!({
        "profile": profile,
        "generator": synthetic::GENERATOR_ID,
        "recognizer": recognizer_metadata,
        "seed": seed,
        "scope": "per_script",
        "samples_per_character": samples_per_character,
        "elapsed_ms": elapsed.as_secs_f64() * 1_000.0,
        "mean_recognition_ms": report.recognition_elapsed_ms / sample_count,
        "mean_evaluation_ms_per_sample": elapsed.as_secs_f64() * 1_000.0 / sample_count,
        "top1_rate": report.top1_rate(),
        "top1_rate_wilson_95": wilson_95(report.top1, report.samples),
        "top5_rate": report.top5_rate(),
        "top5_rate_wilson_95": wilson_95(report.top5, report.samples),
        "accepted_correct": report.accepted_correct,
        "accepted_correct_rate": ratio(report.accepted_correct, report.samples),
        "accepted_correct_rate_wilson_95": wilson_95(
            report.accepted_correct,
            report.samples,
        ),
        "accepted_wrong": report.accepted_wrong,
        "accepted_wrong_rate": ratio(report.accepted_wrong, report.samples),
        "accepted_wrong_rate_wilson_95": wilson_95(
            report.accepted_wrong,
            report.samples,
        ),
        "rejection_rate": report.rejection_rate(),
        "rejection_rate_wilson_95": wilson_95(report.rejected, report.samples),
        "invalid_rejection_rate": report.invalid_rejection_rate(),
        "invalid_rejection_rate_wilson_95": wilson_95(
            report.invalid_rejected,
            report.invalid_samples,
        ),
        "report": report,
    })
}

fn recognize_input(recognizer: &KanaRecognizer, input: &ParsedInput) -> Recognition {
    match input.script_scope {
        Some(script) => recognizer.recognize_script(&input.ink, script, 10),
        None => recognizer.recognize(&input.ink, 10),
    }
}

fn validate_scope(recognizer: &KanaRecognizer, input: &ParsedInput) -> Result<()> {
    if input.expected_source.is_some() && input.expected.is_none() && !input.expected_invalid {
        bail!("expected_source requires a kana or invalid-ink label");
    }
    if input.expected_invalid && matches!(input.expected_source, Some(ExpectedSource::Prompted)) {
        bail!("invalid ink cannot carry prompted provenance");
    }
    if let Some(prompt) = &input.prompt {
        if !matches!(input.expected_source, Some(ExpectedSource::Prompted)) {
            bail!("prompt metadata requires expected_source prompted");
        }
        if input.expected.is_none() {
            bail!("prompt metadata requires an expected kana label");
        }
        let Some(prompt_scope) = input.script_scope else {
            bail!("prompt metadata requires a hiragana or katakana script scope");
        };
        let inventory = recognizer
            .characters()
            .filter(|(_, script)| *script == prompt_scope)
            .count();
        if prompt.total != inventory {
            bail!(
                "prompt total {} does not match the {inventory}-kana script inventory",
                prompt.total
            );
        }
        if prompt.total == 0 || prompt.position == 0 || prompt.position > prompt.total {
            bail!(
                "prompt position {} is outside 1..={}",
                prompt.position,
                prompt.total
            );
        }
        if prompt.saved_before >= prompt.position {
            bail!(
                "prompt saved_before {} must be smaller than position {}",
                prompt.saved_before,
                prompt.position
            );
        }
        if let Some(sequence) = &prompt.sequence {
            let sequence: Vec<_> = sequence.chars().collect();
            if sequence.len() != prompt.total {
                bail!(
                    "prompt sequence contains {} kana but total is {}",
                    sequence.len(),
                    prompt.total
                );
            }
            let sequence_set: BTreeSet<_> = sequence.iter().copied().collect();
            let inventory_set: BTreeSet<_> = recognizer
                .characters()
                .filter_map(|(character, script)| (script == prompt_scope).then_some(character))
                .collect();
            if sequence_set != inventory_set || sequence_set.len() != prompt.total {
                bail!("prompt sequence is not one permutation of the selected script inventory");
            }
            if input.expected != sequence.get(prompt.position - 1).copied() {
                bail!("expected kana does not match its position in the prompt sequence");
            }
        }
    }
    let Some(expected) = input.expected else {
        return Ok(());
    };
    let expected_scope = recognizer
        .characters()
        .find_map(|(character, script)| (character == expected).then_some(script))
        .with_context(|| format!("expected character {expected} is outside the basic kana set"))?;
    if input
        .script_scope
        .is_some_and(|scope| expected_scope != scope)
    {
        bail!("expected kana {expected} is outside the saved script scope");
    }
    Ok(())
}

struct PromptRunAggregate {
    script_scope: Option<KanaScript>,
    total: usize,
    samples: usize,
    positions: BTreeMap<usize, usize>,
    labels: BTreeMap<char, usize>,
    sequence: Option<String>,
    sequence_metadata_complete: bool,
    records: Vec<(usize, usize)>,
    files: Vec<String>,
    top1_correct: usize,
    accepted_correct: usize,
    accepted_wrong: usize,
    rejected: usize,
    metadata_consistent: bool,
}

#[derive(Clone, Copy)]
struct SampleOutcome {
    top1_correct: bool,
    accepted_correct: bool,
    accepted_wrong: bool,
    rejected: bool,
}

impl Default for PromptRunAggregate {
    fn default() -> Self {
        Self {
            script_scope: None,
            total: 0,
            samples: 0,
            positions: BTreeMap::new(),
            labels: BTreeMap::new(),
            sequence: None,
            sequence_metadata_complete: true,
            records: Vec::new(),
            files: Vec::new(),
            top1_correct: 0,
            accepted_correct: 0,
            accepted_wrong: 0,
            rejected: 0,
            metadata_consistent: true,
        }
    }
}

impl PromptRunAggregate {
    fn record(
        &mut self,
        prompt: &SavedPrompt,
        script_scope: Option<KanaScript>,
        expected: char,
        file: String,
        outcome: SampleOutcome,
    ) {
        if self.samples == 0 {
            self.script_scope = script_scope;
            self.total = prompt.total;
            self.sequence = prompt.sequence.clone();
        } else if self.script_scope != script_scope || self.total != prompt.total {
            self.metadata_consistent = false;
        }
        match (&self.sequence, &prompt.sequence) {
            (Some(existing), Some(current)) if existing != current => {
                self.metadata_consistent = false;
            }
            (None, Some(current)) => self.sequence = Some(current.clone()),
            _ => {}
        }
        self.sequence_metadata_complete &= prompt.sequence.is_some();
        self.samples += 1;
        *self.positions.entry(prompt.position).or_insert(0) += 1;
        *self.labels.entry(expected).or_insert(0) += 1;
        self.records.push((prompt.position, prompt.saved_before));
        self.files.push(file);
        self.top1_correct += usize::from(outcome.top1_correct);
        self.accepted_correct += usize::from(outcome.accepted_correct);
        self.accepted_wrong += usize::from(outcome.accepted_wrong);
        self.rejected += usize::from(outcome.rejected);
    }

    fn to_json(
        &self,
        run_started_at_unix_ms: u64,
        recognizer: &KanaRecognizer,
    ) -> serde_json::Value {
        let duplicate_positions: Vec<_> = self
            .positions
            .iter()
            .filter_map(|(&position, &count)| (count > 1).then_some(position))
            .collect();
        let missing_positions: Vec<_> = (1..=self.total)
            .filter(|position| !self.positions.contains_key(position))
            .collect();
        let positions: Vec<_> = self.positions.keys().copied().collect();
        let inventory: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| {
                (self.script_scope == Some(script)).then_some(character)
            })
            .collect();
        let labels: String = self.labels.keys().collect();
        let duplicate_labels: String = self
            .labels
            .iter()
            .filter_map(|(&label, &count)| (count > 1).then_some(label))
            .collect();
        let missing_labels: String = inventory
            .iter()
            .filter(|label| !self.labels.contains_key(label))
            .collect();
        let unexpected_labels: String = self
            .labels
            .keys()
            .filter(|label| !inventory.contains(label))
            .collect();
        let mut records = self.records.clone();
        records.sort_unstable();
        let saved_before_consistent = duplicate_positions.is_empty()
            && records
                .iter()
                .enumerate()
                .all(|(saved_before, (_, recorded))| saved_before == *recorded);
        let missing_position_labels: Vec<_> = self
            .sequence
            .as_ref()
            .map(|sequence| {
                let sequence: Vec<_> = sequence.chars().collect();
                missing_positions
                    .iter()
                    .filter_map(|position| {
                        sequence.get(position - 1).map(|expected| {
                            serde_json::json!({
                                "position": position,
                                "expected": expected,
                            })
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let complete = self.metadata_consistent
            && self.sequence_metadata_complete
            && saved_before_consistent
            && missing_positions.is_empty()
            && duplicate_positions.is_empty()
            && self.samples == self.total
            && self.labels.len() == self.total
            && missing_labels.is_empty()
            && unexpected_labels.is_empty()
            && duplicate_labels.is_empty();
        serde_json::json!({
            "run_started_at_unix_ms": run_started_at_unix_ms,
            "script_scope": script_scope_name(self.script_scope),
            "total": self.total,
            "samples": self.samples,
            "top1_correct": self.top1_correct,
            "top1_rate": ratio(self.top1_correct, self.samples),
            "top1_rate_wilson_95": wilson_95(self.top1_correct, self.samples),
            "accepted_correct": self.accepted_correct,
            "accepted_correct_rate": ratio(self.accepted_correct, self.samples),
            "accepted_correct_rate_wilson_95": wilson_95(
                self.accepted_correct,
                self.samples,
            ),
            "accepted_wrong": self.accepted_wrong,
            "accepted_wrong_rate": ratio(self.accepted_wrong, self.samples),
            "accepted_wrong_rate_wilson_95": wilson_95(
                self.accepted_wrong,
                self.samples,
            ),
            "rejected": self.rejected,
            "rejection_rate": ratio(self.rejected, self.samples),
            "rejection_rate_wilson_95": wilson_95(self.rejected, self.samples),
            "unique_positions": self.positions.len(),
            "unique_labels": self.labels.len(),
            "positions": positions,
            "labels": labels,
            "sequence": self.sequence,
            "sequence_metadata_complete": self.sequence_metadata_complete,
            "saved_before_consistent": saved_before_consistent,
            "duplicate_labels": duplicate_labels,
            "missing_labels": missing_labels,
            "unexpected_labels": unexpected_labels,
            "duplicate_positions": duplicate_positions,
            "missing_positions": missing_positions,
            "missing_position_labels": missing_position_labels,
            "metadata_consistent": self.metadata_consistent,
            "complete": complete,
            "files": self.files,
        })
    }
}

fn script_scope_name(script_scope: Option<KanaScript>) -> &'static str {
    match script_scope {
        Some(KanaScript::Hiragana) => "hiragana",
        Some(KanaScript::Katakana) => "katakana",
        None => "both",
    }
}

fn parse_input(text: &str) -> Result<ParsedInput> {
    let document: InputDocument = serde_json::from_str(text)
        .context("input must be raw strokes or an idiosepius kana sample")?;
    let (
        strokes,
        expected,
        expected_invalid,
        expected_source,
        prompt,
        script_scope,
        stored_result,
        stored_algorithm,
        stored_template_fingerprint,
        stored_maximum_distance,
        stored_maximum_path_length_ratio,
        stored_minimum_margin,
        stored_variant_thresholds,
        saved,
    ) = match document {
        InputDocument::Ink(strokes) => (
            strokes,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            VariantThresholds::default(),
            false,
        ),
        InputDocument::Saved(sample) => {
            let sample = *sample;
            if sample.format != "idiosepius-kana-sample" {
                bail!("unsupported kana sample format {:?}", sample.format);
            }
            if sample.format_version != 1 {
                bail!(
                    "unsupported kana sample format version {}",
                    sample.format_version
                );
            }
            if sample.expected.is_some() && sample.expected_invalid {
                bail!("saved sample cannot be labeled as both kana and invalid ink");
            }
            let (
                script_scope,
                stored_algorithm,
                stored_template_fingerprint,
                stored_maximum_distance,
                stored_maximum_path_length_ratio,
                stored_minimum_margin,
                stored_variant_thresholds,
            ) = sample
                .recognizer
                .map(|metadata| {
                    (
                        metadata.script_scope,
                        metadata.algorithm,
                        metadata.template_fingerprint,
                        metadata.maximum_distance,
                        metadata.maximum_path_length_ratio,
                        metadata.minimum_margin,
                        metadata.variant_thresholds,
                    )
                })
                .unwrap_or((
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    VariantThresholds::default(),
                ));
            (
                sample.strokes,
                sample.expected,
                sample.expected_invalid,
                sample.expected_source,
                sample.prompt,
                script_scope,
                sample.result,
                stored_algorithm,
                stored_template_fingerprint,
                stored_maximum_distance,
                stored_maximum_path_length_ratio,
                stored_minimum_margin,
                stored_variant_thresholds,
                true,
            )
        }
    };
    let ink = CharacterInk(
        strokes
            .into_iter()
            .map(|stroke| stroke.into_iter().map(JsonPoint::into_ink).collect())
            .collect(),
    );
    Ok(ParsedInput {
        ink,
        expected,
        expected_invalid,
        expected_source,
        prompt,
        script_scope,
        stored_result,
        stored_algorithm,
        stored_template_fingerprint,
        stored_maximum_distance,
        stored_maximum_path_length_ratio,
        stored_minimum_margin,
        stored_variant_thresholds,
        saved,
    })
}

fn parse_seed(text: &str) -> Result<u64> {
    if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).context("synthetic seed is not valid hexadecimal")
    } else {
        text.parse().context("synthetic seed must be an integer")
    }
}

fn usage() -> &'static str {
    "usage: kana-recognize [INK.json|SAMPLE_DIR|-]\n       kana-recognize --synthetic [SAMPLES_PER_CHARACTER] [SEED]\n       kana-recognize --synthetic-scripted [SAMPLES_PER_CHARACTER] [SEED]\n       kana-recognize --synthetic-ablation [SAMPLES_PER_CHARACTER] [SEED]\n\nSAMPLE_DIR is searched recursively for JSON samples."
}

struct ParsedInput {
    ink: CharacterInk,
    expected: Option<char>,
    expected_invalid: bool,
    expected_source: Option<ExpectedSource>,
    prompt: Option<SavedPrompt>,
    script_scope: Option<KanaScript>,
    stored_result: Option<Recognition>,
    stored_algorithm: Option<String>,
    stored_template_fingerprint: Option<String>,
    stored_maximum_distance: Option<f32>,
    stored_maximum_path_length_ratio: Option<f32>,
    stored_minimum_margin: Option<f32>,
    stored_variant_thresholds: VariantThresholds,
    saved: bool,
}

fn stored_config(input: &ParsedInput) -> Option<RecognitionConfig> {
    Some(RecognitionConfig {
        maximum_distance: input.stored_maximum_distance?,
        maximum_path_length_ratio: input.stored_maximum_path_length_ratio?,
        minimum_margin: input.stored_minimum_margin?,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InputDocument {
    Ink(Vec<Vec<JsonPoint>>),
    Saved(Box<SavedSample>),
}

#[derive(Deserialize)]
struct SavedSample {
    format: String,
    format_version: u32,
    strokes: Vec<Vec<JsonPoint>>,
    #[serde(default)]
    expected: Option<char>,
    #[serde(default)]
    expected_invalid: bool,
    #[serde(default)]
    expected_source: Option<ExpectedSource>,
    #[serde(default)]
    prompt: Option<SavedPrompt>,
    #[serde(default)]
    recognizer: Option<SavedRecognizer>,
    #[serde(default)]
    result: Option<Recognition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedSource {
    Manual,
    Prompted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SavedPrompt {
    #[serde(default)]
    run_started_at_unix_ms: Option<u64>,
    position: usize,
    total: usize,
    #[serde(default)]
    saved_before: usize,
    #[serde(default)]
    sequence: Option<String>,
}

#[derive(Deserialize)]
struct SavedRecognizer {
    #[serde(default, deserialize_with = "deserialize_script_scope")]
    script_scope: Option<KanaScript>,
    #[serde(default)]
    algorithm: Option<String>,
    #[serde(default)]
    template_fingerprint: Option<String>,
    #[serde(default)]
    maximum_distance: Option<f32>,
    #[serde(default)]
    maximum_path_length_ratio: Option<f32>,
    #[serde(default)]
    minimum_margin: Option<f32>,
    #[serde(flatten)]
    variant_thresholds: VariantThresholds,
}

fn deserialize_script_scope<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<KanaScript>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    match value.as_deref() {
        None | Some("both") => Ok(None),
        Some("hiragana") => Ok(Some(KanaScript::Hiragana)),
        Some("katakana") => Ok(Some(KanaScript::Katakana)),
        Some(other) => Err(serde::de::Error::unknown_variant(
            other,
            &["both", "hiragana", "katakana"],
        )),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonPoint {
    Pair([f32; 2]),
    Timed((f32, f32, f64)),
    Full(InkPoint),
}

impl JsonPoint {
    fn into_ink(self) -> InkPoint {
        match self {
            Self::Pair([x, y]) => InkPoint::at(x, y),
            Self::Timed((x, y, time_ms)) => InkPoint {
                time_ms: Some(time_ms),
                ..InkPoint::at(x, y)
            },
            Self::Full(point) => point,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_sample_restores_label_scope_and_point_metadata() {
        let input = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "あ",
                "expected_source": "prompted",
                "prompt": {
                    "run_started_at_unix_ms": 1724999000000,
                    "position": 7,
                    "total": 46,
                    "saved_before": 6
                },
                "recognizer": {
                    "script_scope": "hiragana",
                    "algorithm": "multi-prototype-symmetric-directional-chamfer-v2",
                    "template_fingerprint": "0123456789abcdef",
                    "maximum_distance": 0.065,
                    "maximum_path_length_ratio": 1.7,
                    "minimum_margin": 0.012
                },
                "strokes": [[{
                    "x": 12.5,
                    "y": 30.0,
                    "time_ms": 44.0,
                    "pressure": 0.7,
                    "pointer": "pen"
                }]],
                "result": {
                    "state": "recognized",
                    "candidates": [{
                        "character": "あ",
                        "script": "hiragana",
                        "distance": 0.01
                    }]
                }
            }"#,
        )
        .unwrap();

        assert!(input.saved);
        assert_eq!(input.expected, Some('あ'));
        assert!(matches!(
            input.expected_source,
            Some(ExpectedSource::Prompted)
        ));
        let prompt = input.prompt.as_ref().unwrap();
        assert_eq!(prompt.run_started_at_unix_ms, Some(1_724_999_000_000));
        assert_eq!(prompt.position, 7);
        assert_eq!(prompt.total, 46);
        assert_eq!(prompt.saved_before, 6);
        assert_eq!(input.script_scope, Some(KanaScript::Hiragana));
        assert_eq!(
            input.stored_algorithm.as_deref(),
            Some("multi-prototype-symmetric-directional-chamfer-v2")
        );
        assert!(!input.stored_variant_thresholds.is_complete());
        assert_eq!(
            input.stored_template_fingerprint.as_deref(),
            Some("0123456789abcdef")
        );
        assert_eq!(stored_config(&input), Some(RecognitionConfig::default()));
        assert_eq!(input.ink.0[0][0].pressure, Some(0.7));
        assert_eq!(input.stored_result.unwrap().top().unwrap().character, 'あ');
    }

    #[test]
    fn directory_provenance_checks_variant_thresholds_and_legacy_absence() {
        let directory = std::env::temp_dir().join(format!(
            "kana-variant-provenance-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let recognizer = KanaRecognizer::new().unwrap();
        let config = recognizer.config();
        let current = serde_json::json!({
            "format": "idiosepius-kana-sample", "format_version": 1, "strokes": [],
            "recognizer": {
                "algorithm": ALGORITHM_ID,
                "template_fingerprint": format!("{:016x}", recognizer.template_fingerprint()),
                "maximum_distance": config.maximum_distance,
                "maximum_path_length_ratio": config.maximum_path_length_ratio,
                "minimum_margin": config.minimum_margin,
                "variant_maximum_distance": VARIANT_MAXIMUM_DISTANCE,
                "variant_minimum_margin": VARIANT_MINIMUM_MARGIN,
            },
        });
        for (name, field) in [
            ("distance", "variant_maximum_distance"),
            ("margin", "variant_minimum_margin"),
        ] {
            let mut changed = current.clone();
            changed["recognizer"][field] = 0.9.into();
            fs::write(directory.join(format!("{name}.json")), changed.to_string()).unwrap();
        }
        fs::write(directory.join("current.json"), current.to_string()).unwrap();
        let mut legacy = current;
        legacy["recognizer"]
            .as_object_mut()
            .unwrap()
            .remove("variant_maximum_distance");
        legacy["recognizer"]
            .as_object_mut()
            .unwrap()
            .remove("variant_minimum_margin");
        fs::write(directory.join("legacy.json"), legacy.to_string()).unwrap();
        let report = evaluate_directory(&recognizer, &directory).unwrap();
        assert_eq!(report["recognizer_metadata_changed"], 2);
        assert_eq!(report["recognizer_metadata_missing"], 1);
        for changed in report["recognizer_metadata_changes"].as_array().unwrap() {
            assert_eq!(changed["config_changed"], true);
            assert_eq!(changed["algorithm_changed"], false);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_report_counts_labeled_samples() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "idiosepius-kana-corpus-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = recognizer.canonical('あ').unwrap();
        let prompt_sequence: String = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let fingerprint = format!("{:016x}", recognizer.template_fingerprint());
        let config = recognizer.config();
        let result = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
        let sample = serde_json::json!({
            "format": "idiosepius-kana-sample",
            "format_version": 1,
            "expected": "あ",
            "expected_source": "prompted",
            "prompt": {
                "run_started_at_unix_ms": 1700000000000_u64,
                "position": 1,
                "total": 46,
                "saved_before": 0,
                "sequence": prompt_sequence,
            },
            "recognizer": {
                "script_scope": "hiragana",
                "algorithm": ALGORITHM_ID,
                "template_fingerprint": fingerprint,
                "maximum_distance": config.maximum_distance,
                "maximum_path_length_ratio": config.maximum_path_length_ratio,
                "minimum_margin": config.minimum_margin,
            },
            "strokes": ink,
            "result": result,
        });
        fs::write(
            directory.join("sample.json"),
            serde_json::to_vec(&sample).unwrap(),
        )
        .unwrap();
        let mut legacy_prompt = sample.clone();
        legacy_prompt.as_object_mut().unwrap().remove("prompt");
        fs::create_dir(directory.join("writer-b")).unwrap();
        fs::write(
            directory.join("writer-b/legacy-prompt.json"),
            serde_json::to_vec(&legacy_prompt).unwrap(),
        )
        .unwrap();
        let mut mismatch = sample.clone();
        mismatch["recognizer"]["template_fingerprint"] = "0000000000000000".into();
        mismatch["result"]["candidates"][0]["distance"] = 0.123.into();
        fs::write(
            directory.join("mismatch.json"),
            serde_json::to_vec(&mismatch).unwrap(),
        )
        .unwrap();
        let mut config_mismatch = sample;
        config_mismatch["recognizer"]["maximum_distance"] = 0.5.into();
        fs::write(
            directory.join("config-mismatch.json"),
            serde_json::to_vec(&config_mismatch).unwrap(),
        )
        .unwrap();
        let invalid = serde_json::json!({
            "format": "idiosepius-kana-sample",
            "format_version": 1,
            "expected_invalid": true,
            "expected_source": "manual",
            "recognizer": {
                "script_scope": "both",
                "algorithm": ALGORITHM_ID,
                "template_fingerprint": fingerprint,
                "maximum_distance": config.maximum_distance,
                "maximum_path_length_ratio": config.maximum_path_length_ratio,
                "minimum_margin": config.minimum_margin,
            },
            "strokes": [],
        });
        fs::write(
            directory.join("invalid.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        let false_acceptance = serde_json::json!({
            "format": "idiosepius-kana-sample",
            "format_version": 1,
            "expected_invalid": true,
            "expected_source": "manual",
            "recognizer": {
                "script_scope": "hiragana",
                "algorithm": ALGORITHM_ID,
                "template_fingerprint": fingerprint,
                "maximum_distance": config.maximum_distance,
                "maximum_path_length_ratio": config.maximum_path_length_ratio,
                "minimum_margin": config.minimum_margin,
            },
            "strokes": ink,
        });
        fs::write(
            directory.join("invalid-false-acceptance.json"),
            serde_json::to_vec(&false_acceptance).unwrap(),
        )
        .unwrap();
        let truncated_path = directory.join("truncated.json");
        fs::write(&truncated_path, b"{").unwrap();

        let report = evaluate_directory(&recognizer, &directory).unwrap();
        assert_eq!(report["files"], 7);
        assert_eq!(
            report["ink_fingerprint_algorithm"],
            INK_FINGERPRINT_ALGORITHM
        );
        assert_eq!(report["duplicate_ink_group_count"], 1);
        assert_eq!(report["duplicate_ink_samples"], 5);
        assert_eq!(report["duplicate_ink_excess"], 4);
        assert_eq!(report["duplicate_ink_label_conflicts"], 1);
        assert_eq!(report["duplicate_ink_groups"][0]["samples"], 5);
        assert_eq!(report["duplicate_ink_groups"][0]["label_conflict"], true);
        assert_eq!(
            report["duplicate_ink_groups"][0]["labels"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            report["corpus_fingerprint"]["algorithm"],
            CORPUS_FINGERPRINT_ALGORITHM
        );
        assert_eq!(report["corpus_fingerprint"]["readable_files"], 7);
        assert!(report["corpus_fingerprint"]["bytes"].as_u64().unwrap() > 0);
        assert_eq!(
            report["corpus_fingerprint"]["value"]
                .as_str()
                .unwrap()
                .len(),
            16
        );
        assert_eq!(report["evaluated"], 6);
        assert!(report["recognition_elapsed_ms"].as_f64().unwrap() > 0.0);
        assert!(report["mean_recognition_ms"].as_f64().unwrap() > 0.0);
        assert_eq!(report["labeled"], 4);
        assert_eq!(report["top1_correct"], 4);
        assert_eq!(report["accepted_correct"], 4);
        assert_eq!(report["valid_rejected"], 0);
        assert_eq!(report["valid_rejection_rate"], 0.0);
        assert_eq!(report["invalid_labeled"], 2);
        assert_eq!(report["invalid_rejected"], 1);
        assert_eq!(report["invalid_rejection_rate"], 0.5);
        assert_eq!(report["unlabeled"], 0);
        assert_eq!(report["unlabeled_rejected"], 0);
        assert_eq!(report["stored_results"], 4);
        assert_eq!(report["stored_result_missing"], 2);
        assert_eq!(
            report["stored_result_missing_files"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(report["changed_since_save"], 1);
        assert_eq!(report["changed_since_save_rate"], 0.25);
        assert_eq!(
            report["invalid_false_acceptances"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(report["invalid_false_acceptances"][0]["margin"].is_number());
        assert!(report["invalid_false_acceptances"][0]["path_length_ratio"].is_number());
        assert_eq!(report["recognizer_metadata_changed"], 2);
        // These legacy fixtures predate the variant-threshold provenance fields.
        assert_eq!(report["recognizer_metadata_missing"], 6);
        assert_eq!(
            report["recognizer_metadata_changes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            report["recognizer_metadata_changes"][0]["file"],
            "config-mismatch.json"
        );
        assert_eq!(
            report["recognizer_metadata_changes"][0]["config_changed"],
            true
        );
        assert_eq!(
            report["recognizer_metadata_changes"][1]["templates_changed"],
            true
        );
        assert_eq!(report["cohorts"]["prompted"]["labeled"], 4);
        assert_eq!(report["cohorts"]["prompted"]["top1_rate"], 1.0);
        assert_eq!(report["cohorts"]["manual"]["labeled"], 0);
        assert_eq!(report["directory_groups"].as_array().unwrap().len(), 2);
        let root_group = &report["directory_groups"][0];
        assert_eq!(root_group["name"], ".");
        assert_eq!(root_group["files"], 6);
        assert_eq!(root_group["evaluated"], 5);
        assert_eq!(root_group["errors"], 1);
        assert_eq!(root_group["labeled"], 3);
        assert_eq!(root_group["top1_rate"], 1.0);
        assert_eq!(root_group["invalid_labeled"], 2);
        assert_eq!(root_group["invalid_rejection_rate"], 0.5);
        assert_eq!(
            root_group["metrics"]["valid_labeled"]["distance"]["count"],
            3
        );
        assert_eq!(root_group["coverage"]["hiragana"]["observations"], 3);
        assert_eq!(root_group["coverage"]["hiragana"]["observed_unique"], 1);
        assert_eq!(
            root_group["coverage"]["hiragana"]["missing"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            45
        );
        let writer_group = &report["directory_groups"][1];
        assert_eq!(writer_group["name"], "writer-b");
        assert_eq!(writer_group["files"], 1);
        assert_eq!(writer_group["evaluated"], 1);
        assert_eq!(writer_group["errors"], 0);
        assert_eq!(writer_group["labeled"], 1);
        assert_eq!(writer_group["top1_rate"], 1.0);
        assert_eq!(writer_group["coverage"]["hiragana"]["observations"], 1);
        assert_eq!(report["prompt_run_count"], 1);
        assert_eq!(report["prompt_runs_complete"], 0);
        assert_eq!(report["prompt_runs_incomplete"], 1);
        assert_eq!(report["prompt_runs"].as_array().unwrap().len(), 1);
        assert_eq!(
            report["prompt_runs"][0]["run_started_at_unix_ms"],
            1_700_000_000_000_u64
        );
        assert_eq!(report["prompt_runs"][0]["samples"], 3);
        assert_eq!(report["prompt_runs"][0]["top1_correct"], 3);
        assert_eq!(report["prompt_runs"][0]["top1_rate"], 1.0);
        assert_eq!(report["prompt_runs"][0]["accepted_correct_rate"], 1.0);
        assert_eq!(report["prompt_runs"][0]["rejection_rate"], 0.0);
        assert_eq!(report["prompt_runs"][0]["unique_positions"], 1);
        assert_eq!(report["prompt_runs"][0]["duplicate_positions"][0], 1);
        assert_eq!(report["prompt_runs"][0]["duplicate_labels"], "あ");
        assert_eq!(
            report["prompt_runs"][0]["missing_positions"]
                .as_array()
                .unwrap()
                .len(),
            45
        );
        assert_eq!(
            report["prompt_runs"][0]["missing_labels"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            45
        );
        assert_eq!(report["prompt_runs"][0]["metadata_consistent"], true);
        assert_eq!(report["prompt_runs"][0]["complete"], false);
        assert_eq!(report["prompt_run_metadata_missing"], 1);
        assert_eq!(
            report["prompt_run_metadata_missing_files"][0],
            "writer-b/legacy-prompt.json"
        );
        assert_eq!(report["prompt_sequence_metadata_missing"], 1);
        assert_eq!(
            report["prompt_sequence_metadata_missing_files"][0],
            "writer-b/legacy-prompt.json"
        );
        assert_eq!(report["metrics"]["valid_labeled"]["distance"]["count"], 4);
        assert_eq!(report["metrics"]["invalid_labeled"]["distance"]["count"], 1);
        assert_eq!(report["capture"]["samples"], 6);
        assert_eq!(report["capture"]["stroke_count"]["count"], 6);
        assert_eq!(
            report["capture"]["pointer_sample_cohorts"]["unspecified"],
            6
        );
        assert_eq!(root_group["capture"]["samples"], 5);
        assert_eq!(writer_group["capture"]["samples"], 1);
        assert_eq!(report["characters"][0]["expected"], "あ");
        assert_eq!(report["characters"][0]["top1_rate"], 1.0);
        assert_eq!(report["coverage"]["hiragana"]["inventory"], 46);
        assert_eq!(report["coverage"]["hiragana"]["observations"], 4);
        assert_eq!(report["coverage"]["hiragana"]["observed_unique"], 1);
        assert_eq!(
            report["coverage"]["hiragana"]["missing"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            45
        );
        assert_eq!(report["coverage"]["katakana"]["observed_unique"], 0);
        assert_eq!(report["confusions"].as_array().unwrap().len(), 0);
        assert_eq!(report["errors"].as_array().unwrap().len(), 1);
        assert_eq!(report["errors"][0]["file"], "truncated.json");
        assert_eq!(report["failures"].as_array().unwrap().len(), 0);

        let fingerprint = report["corpus_fingerprint"]["value"].clone();
        let repeated = evaluate_directory(&recognizer, &directory).unwrap();
        assert_eq!(repeated["corpus_fingerprint"]["value"], fingerprint);
        fs::write(&truncated_path, b"{{").unwrap();
        let changed = evaluate_directory(&recognizer, &directory).unwrap();
        assert_ne!(changed["corpus_fingerprint"]["value"], fingerprint);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_report_separates_accepted_errors_from_rejected_nearest_matches() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "idiosepius-kana-accepted-error-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let recognizer = KanaRecognizer::new().unwrap();
        let ink = recognizer.canonical('あ').unwrap();
        let sample = serde_json::json!({
            "format": "idiosepius-kana-sample",
            "format_version": 1,
            "expected": "い",
            "expected_source": "manual",
            "recognizer": { "script_scope": "hiragana" },
            "strokes": ink,
        });
        fs::write(
            directory.join("accepted-wrong.json"),
            serde_json::to_vec(&sample).unwrap(),
        )
        .unwrap();

        let report = evaluate_directory(&recognizer, &directory).unwrap();
        assert_eq!(report["labeled"], 1);
        assert_eq!(report["top1_correct"], 0);
        assert_eq!(report["accepted_correct"], 0);
        assert_eq!(report["accepted_wrong"], 1);
        assert_eq!(report["accepted_wrong_rate"], 1.0);
        assert_eq!(report["valid_rejected"], 0);
        assert_eq!(report["accepted_confusions"][0]["expected"], "い");
        assert_eq!(report["accepted_confusions"][0]["recognized"], "あ");
        assert_eq!(report["cohorts"]["manual"]["accepted_wrong"], 1);
        assert_eq!(report["characters"][0]["accepted_wrong"], 1);
        assert_eq!(report["directory_groups"][0]["accepted_wrong"], 1);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn directory_report_recognizes_a_complete_nested_prompt_run() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "idiosepius-kana-complete-run-test-{}-{unique}",
            std::process::id()
        ));
        let writer_directory = directory.join("writer-a/run-1");
        fs::create_dir_all(&writer_directory).unwrap();
        let recognizer = KanaRecognizer::new().unwrap();
        let fingerprint = format!("{:016x}", recognizer.template_fingerprint());
        let config = recognizer.config();
        let hiragana: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let prompt_sequence: String = hiragana.iter().collect();

        for (index, expected) in hiragana.iter().copied().enumerate() {
            let position = index + 1;
            let ink = recognizer.canonical(expected).unwrap();
            let result = recognizer.recognize_script(&ink, KanaScript::Hiragana, 10);
            let sample = serde_json::json!({
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": expected,
                "expected_source": "prompted",
                "prompt": {
                    "run_started_at_unix_ms": 1_700_000_000_000_u64,
                    "position": position,
                    "total": hiragana.len(),
                    "saved_before": index,
                    "sequence": prompt_sequence,
                },
                "recognizer": {
                    "script_scope": "hiragana",
                    "algorithm": ALGORITHM_ID,
                    "template_fingerprint": fingerprint,
                    "maximum_distance": config.maximum_distance,
                    "maximum_path_length_ratio": config.maximum_path_length_ratio,
                    "minimum_margin": config.minimum_margin,
                },
                "strokes": ink,
                "result": result,
            });
            fs::write(
                writer_directory.join(format!("{position:02}-{expected}.json")),
                serde_json::to_vec(&sample).unwrap(),
            )
            .unwrap();
        }

        let report = evaluate_directory(&recognizer, &directory).unwrap();
        assert_eq!(report["files"], 46);
        assert_eq!(report["duplicate_ink_group_count"], 0);
        assert_eq!(report["duplicate_ink_samples"], 0);
        assert_eq!(report["duplicate_ink_excess"], 0);
        assert_eq!(report["duplicate_ink_label_conflicts"], 0);
        assert_eq!(report["evaluated"], 46);
        assert_eq!(report["top1_rate"], 1.0);
        assert_eq!(report["accepted_correct_rate"], 1.0);
        assert_eq!(report["valid_rejection_rate"], 0.0);
        assert_eq!(report["stored_results"], 46);
        assert_eq!(report["stored_result_missing"], 0);
        assert_eq!(report["changed_since_save"], 0);
        assert_eq!(report["changed_since_save_rate"], 0.0);
        assert_eq!(report["coverage"]["hiragana"]["observed_unique"], 46);
        assert_eq!(report["coverage"]["hiragana"]["missing"], "");
        assert_eq!(report["directory_groups"].as_array().unwrap().len(), 1);
        assert_eq!(report["directory_groups"][0]["name"], "writer-a");
        assert_eq!(report["directory_groups"][0]["files"], 46);
        assert_eq!(report["directory_groups"][0]["evaluated"], 46);
        assert_eq!(report["directory_groups"][0]["top1_rate"], 1.0);
        assert_eq!(
            report["directory_groups"][0]["coverage"]["hiragana"]["observed_unique"],
            46
        );
        assert_eq!(
            report["directory_groups"][0]["coverage"]["hiragana"]["missing"],
            ""
        );
        assert_eq!(report["directory_groups"][0]["accepted_correct_rate"], 1.0);
        assert_eq!(report["directory_groups"][0]["valid_rejection_rate"], 0.0);
        assert_eq!(report["prompt_run_count"], 1);
        assert_eq!(report["prompt_runs_complete"], 1);
        assert_eq!(report["prompt_runs_incomplete"], 0);
        assert_eq!(report["prompt_runs"].as_array().unwrap().len(), 1);
        let run = &report["prompt_runs"][0];
        assert_eq!(run["samples"], 46);
        assert_eq!(run["unique_positions"], 46);
        assert_eq!(run["unique_labels"], 46);
        assert_eq!(run["missing_positions"].as_array().unwrap().len(), 0);
        assert_eq!(run["duplicate_positions"].as_array().unwrap().len(), 0);
        assert_eq!(run["missing_labels"], "");
        assert_eq!(run["duplicate_labels"], "");
        assert_eq!(run["sequence_metadata_complete"], true);
        assert_eq!(run["saved_before_consistent"], true);
        assert_eq!(run["missing_position_labels"].as_array().unwrap().len(), 0);
        assert_eq!(run["top1_rate"], 1.0);
        assert_eq!(run["accepted_correct_rate"], 1.0);
        assert_eq!(run["rejection_rate"], 0.0);
        assert_eq!(run["complete"], true);
        assert!(
            run["files"][0]
                .as_str()
                .unwrap()
                .starts_with("writer-a/run-1/")
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn future_sample_versions_are_rejected_explicitly() {
        let error = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 2,
                "strokes": []
            }"#,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("format version 2"));
    }

    #[test]
    fn sample_cannot_claim_both_a_kana_and_invalid_ink() {
        let error = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "あ",
                "expected_invalid": true,
                "strokes": []
            }"#,
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("both kana and invalid"));
    }

    #[test]
    fn labeled_kana_must_belong_to_the_saved_script_scope() {
        let input = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "あ",
                "recognizer": {"script_scope": "katakana"},
                "strokes": []
            }"#,
        )
        .unwrap();
        let error = validate_scope(&KanaRecognizer::new().unwrap(), &input)
            .err()
            .unwrap();
        assert!(error.to_string().contains("outside the saved script scope"));
    }

    #[test]
    fn label_must_be_a_basic_kana_even_without_script_scope() {
        let input = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "漢",
                "strokes": []
            }"#,
        )
        .unwrap();
        let error = validate_scope(&KanaRecognizer::new().unwrap(), &input)
            .err()
            .unwrap();
        assert!(error.to_string().contains("outside the basic kana set"));
    }

    #[test]
    fn prompt_metadata_is_validated_but_legacy_prompted_samples_are_allowed() {
        let recognizer = KanaRecognizer::new().unwrap();
        let legacy = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "あ",
                "expected_source": "prompted",
                "strokes": []
            }"#,
        )
        .unwrap();
        validate_scope(&recognizer, &legacy).unwrap();

        let invalid = parse_input(
            r#"{
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": "あ",
                "expected_source": "prompted",
                "prompt": {"position": 2, "total": 46, "saved_before": 2},
                "recognizer": {"script_scope": "hiragana"},
                "strokes": []
            }"#,
        )
        .unwrap();
        let error = validate_scope(&recognizer, &invalid).unwrap_err();
        assert!(error.to_string().contains("saved_before 2"));
    }

    #[test]
    fn provenance_requires_a_compatible_label() {
        let recognizer = KanaRecognizer::new().unwrap();
        for (json, message) in [
            (
                r#"{
                    "format": "idiosepius-kana-sample",
                    "format_version": 1,
                    "expected_source": "manual",
                    "strokes": []
                }"#,
                "requires a kana or invalid-ink label",
            ),
            (
                r#"{
                    "format": "idiosepius-kana-sample",
                    "format_version": 1,
                    "expected_invalid": true,
                    "expected_source": "prompted",
                    "strokes": []
                }"#,
                "invalid ink cannot carry prompted provenance",
            ),
        ] {
            let input = parse_input(json).unwrap();
            let error = validate_scope(&recognizer, &input).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
    }

    #[test]
    fn prompt_run_is_complete_only_with_one_distinct_label_per_position() {
        let recognizer = KanaRecognizer::new().unwrap();
        let mut run = PromptRunAggregate::default();
        let hiragana: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let sequence: String = hiragana.iter().collect();
        for (index, expected) in hiragana.iter().copied().enumerate() {
            let position = index + 1;
            run.record(
                &SavedPrompt {
                    run_started_at_unix_ms: Some(123),
                    position,
                    total: hiragana.len(),
                    saved_before: position - 1,
                    sequence: Some(sequence.clone()),
                },
                Some(KanaScript::Hiragana),
                expected,
                format!("{position}.json"),
                SampleOutcome {
                    top1_correct: true,
                    accepted_correct: true,
                    accepted_wrong: false,
                    rejected: false,
                },
            );
        }
        let report = run.to_json(123, &recognizer);
        assert_eq!(report["complete"], true);
        assert_eq!(report["missing_positions"].as_array().unwrap().len(), 0);
        assert_eq!(report["missing_labels"], "");
        assert_eq!(report["sequence_metadata_complete"], true);
        assert_eq!(report["saved_before_consistent"], true);

        run.record(
            &SavedPrompt {
                run_started_at_unix_ms: Some(123),
                position: hiragana.len(),
                total: hiragana.len(),
                saved_before: hiragana.len() - 1,
                sequence: Some(sequence.clone()),
            },
            Some(KanaScript::Hiragana),
            *hiragana.last().unwrap(),
            "duplicate.json".into(),
            SampleOutcome {
                top1_correct: false,
                accepted_correct: false,
                accepted_wrong: false,
                rejected: true,
            },
        );
        let report = run.to_json(123, &recognizer);
        assert_eq!(report["complete"], false);
        assert_eq!(report["duplicate_positions"][0], hiragana.len());
        assert_eq!(
            report["duplicate_labels"],
            hiragana.last().unwrap().to_string()
        );
        assert_eq!(report["top1_correct"], hiragana.len());
        assert_eq!(report["accepted_correct"], hiragana.len());
        assert_eq!(report["rejected"], 1);

        let mut legacy = PromptRunAggregate::default();
        for (index, expected) in hiragana.iter().copied().enumerate() {
            let position = index + 1;
            legacy.record(
                &SavedPrompt {
                    run_started_at_unix_ms: Some(456),
                    position,
                    total: hiragana.len(),
                    saved_before: index,
                    sequence: None,
                },
                Some(KanaScript::Hiragana),
                expected,
                format!("legacy-{position}.json"),
                SampleOutcome {
                    top1_correct: true,
                    accepted_correct: true,
                    accepted_wrong: false,
                    rejected: false,
                },
            );
        }
        let legacy_report = legacy.to_json(456, &recognizer);
        assert_eq!(legacy_report["sequence_metadata_complete"], false);
        assert_eq!(legacy_report["saved_before_consistent"], true);
        assert_eq!(legacy_report["complete"], false);
    }

    #[test]
    fn prompt_sequence_and_saved_count_expose_skips_and_missing_files() {
        let recognizer = KanaRecognizer::new().unwrap();
        let hiragana: Vec<_> = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let sequence: String = hiragana.iter().collect();
        let outcome = SampleOutcome {
            top1_correct: true,
            accepted_correct: true,
            accepted_wrong: false,
            rejected: false,
        };

        let mut skipped = PromptRunAggregate::default();
        for (position, saved_before) in [(1, 0), (3, 1)] {
            skipped.record(
                &SavedPrompt {
                    run_started_at_unix_ms: Some(123),
                    position,
                    total: hiragana.len(),
                    saved_before,
                    sequence: Some(sequence.clone()),
                },
                Some(KanaScript::Hiragana),
                hiragana[position - 1],
                format!("{position}.json"),
                outcome,
            );
        }
        let report = skipped.to_json(123, &recognizer);
        assert_eq!(report["saved_before_consistent"], true);
        assert_eq!(report["missing_position_labels"][0]["position"], 2);
        assert_eq!(
            report["missing_position_labels"][0]["expected"],
            hiragana[1].to_string()
        );

        let mut missing_file = PromptRunAggregate::default();
        missing_file.record(
            &SavedPrompt {
                run_started_at_unix_ms: Some(123),
                position: 2,
                total: hiragana.len(),
                saved_before: 1,
                sequence: Some(sequence),
            },
            Some(KanaScript::Hiragana),
            hiragana[1],
            "2.json".into(),
            outcome,
        );
        assert_eq!(
            missing_file.to_json(123, &recognizer)["saved_before_consistent"],
            false
        );
    }

    #[test]
    fn prompt_label_must_match_the_saved_sequence_position() {
        let recognizer = KanaRecognizer::new().unwrap();
        let sequence: String = recognizer
            .characters()
            .filter_map(|(character, script)| (script == KanaScript::Hiragana).then_some(character))
            .collect();
        let first = sequence.chars().next().unwrap();
        let input = parse_input(
            &serde_json::json!({
                "format": "idiosepius-kana-sample",
                "format_version": 1,
                "expected": first,
                "expected_source": "prompted",
                "prompt": {
                    "run_started_at_unix_ms": 123,
                    "position": 2,
                    "total": 46,
                    "saved_before": 0,
                    "sequence": sequence,
                },
                "recognizer": {"script_scope": "hiragana"},
                "strokes": [],
            })
            .to_string(),
        )
        .unwrap();
        let error = validate_scope(&recognizer, &input).unwrap_err();
        assert!(error.to_string().contains("prompt sequence"), "{error}");
    }

    #[test]
    fn metric_summary_uses_nearest_rank_quantiles_and_handles_no_observations() {
        let summary = metric_summary(&[5.0, 1.0, 4.0, 2.0, 3.0]);
        assert_eq!(summary["min"], 1.0);
        assert_eq!(summary["p05"], 1.0);
        assert_eq!(summary["p50"], 3.0);
        assert_eq!(summary["p95"], 5.0);
        assert_eq!(summary["max"], 5.0);

        let empty = metric_summary(&[]);
        assert_eq!(empty["count"], 0);
        assert!(empty["p50"].is_null());
    }

    #[test]
    fn capture_metrics_expose_device_and_sampling_metadata() {
        use idiosepius_core::kana::PointerKind;

        let pen = CharacterInk(vec![vec![
            InkPoint {
                time_ms: Some(10.0),
                pressure: Some(0.5),
                pointer: Some(PointerKind::Pen),
                ..InkPoint::at(1.0, 2.0)
            },
            InkPoint {
                time_ms: Some(60.0),
                pointer: Some(PointerKind::Pen),
                ..InkPoint::at(3.0, 4.0)
            },
        ]]);
        let mixed = CharacterInk(vec![vec![
            InkPoint {
                pointer: Some(PointerKind::Mouse),
                ..InkPoint::at(5.0, 6.0)
            },
            InkPoint {
                pointer: Some(PointerKind::Touch),
                ..InkPoint::at(7.0, 8.0)
            },
        ]]);
        let mut metrics = CaptureMetrics::default();
        metrics.record(&pen);
        metrics.record(&mixed);
        metrics.record(&CharacterInk::default());
        let report = metrics.to_json();

        assert_eq!(report["samples"], 3);
        assert_eq!(report["stroke_count"]["p50"], 1.0);
        assert_eq!(report["point_count"]["min"], 0.0);
        assert_eq!(report["point_count"]["max"], 2.0);
        assert_eq!(report["duration_ms"]["count"], 1);
        assert_eq!(report["duration_ms"]["p50"], 50.0);
        assert_eq!(report["point_metadata"]["points"], 4);
        assert_eq!(report["point_metadata"]["finite_time"], 2);
        assert_eq!(report["point_metadata"]["finite_time_rate"], 0.5);
        assert_eq!(report["point_metadata"]["pressure"], 1);
        assert_eq!(report["point_metadata"]["pressure_rate"], 0.25);
        assert_eq!(report["point_metadata"]["pointer_rate"], 1.0);
        assert_eq!(report["pointer_sample_cohorts"]["pen"], 1);
        assert_eq!(report["pointer_sample_cohorts"]["mixed"], 1);
        assert_eq!(report["pointer_sample_cohorts"]["unspecified"], 1);
    }

    #[test]
    fn structured_ink_fingerprint_includes_strokes_and_point_metadata() {
        use idiosepius_core::kana::PointerKind;

        let ink = CharacterInk(vec![vec![InkPoint {
            x: 1.0,
            y: 2.0,
            time_ms: Some(3.0),
            pressure: Some(0.5),
            pointer: Some(PointerKind::Pen),
        }]]);
        assert_eq!(ink_fingerprint(&ink), ink_fingerprint(&ink.clone()));

        let mut changed_time = ink.clone();
        changed_time.0[0][0].time_ms = Some(4.0);
        assert_ne!(ink_fingerprint(&ink), ink_fingerprint(&changed_time));

        let split = CharacterInk(vec![Vec::new(), ink.0[0].clone()]);
        assert_ne!(ink_fingerprint(&ink), ink_fingerprint(&split));

        let mut groups = BTreeMap::new();
        record_ink(&mut groups, &ink, "kana:あ".into(), "first.json".into());
        record_ink(&mut groups, &ink, "invalid".into(), "copy.json".into());
        record_ink(
            &mut groups,
            &changed_time,
            "kana:あ".into(),
            "changed.json".into(),
        );
        assert_eq!(groups.values().map(Vec::len).sum::<usize>(), 2);
        assert_eq!(
            groups
                .values()
                .flatten()
                .map(|group| group.files.len())
                .sum::<usize>(),
            3
        );
        let duplicate = groups
            .values()
            .flatten()
            .find(|group| group.files.len() == 2)
            .unwrap();
        assert_eq!(duplicate.labels.len(), 2);
    }

    #[test]
    fn wilson_interval_exposes_small_sample_uncertainty() {
        assert_eq!(wilson_95(0, 0), None);
        assert_eq!(wilson_95(2, 1), None);

        let perfect_one = wilson_95(1, 1).unwrap();
        assert!((perfect_one.lower - 0.206_549_314_377_237_45).abs() < 1e-12);
        assert_eq!(perfect_one.upper, 1.0);

        let half = wilson_95(50, 100).unwrap();
        assert!((half.lower - 0.403_831_530_365_995_6).abs() < 1e-12);
        assert!((half.upper - 0.596_168_469_634_004_4).abs() < 1e-12);
    }
}
