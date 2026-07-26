//! Native public-GitHub repository loader.
//!
//! Browser fetch lives in `browser.rs`; this module is the native half of the
//! shell boundary and is compiled only for the desktop binary.

use anyhow::{Context, Result, bail};

use crate::import::{MAX_JSON_BYTES, MAX_TOTAL_BYTES, PickedFile};

pub(crate) fn load_repository(repository_url: &str) -> Result<Vec<PickedFile>> {
    let (owner, repository) = parse_repository(repository_url)?;
    let owner = encode_path_segment(owner);
    let repository = encode_path_segment(repository);
    let api_url =
        format!("https://api.github.com/repos/{owner}/{repository}/git/trees/HEAD?recursive=1");
    let agent = ureq::agent();
    let mut response = agent
        .get(&api_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .with_context(|| {
            format!(
                "loading repository file list; check that https://github.com/{owner}/{repository} is public"
            )
        })?;
    let tree: serde_json::Value = response
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_json()
        .context("decoding GitHub's repository file list")?;
    if tree["truncated"].as_bool().unwrap_or(false) {
        bail!("repository is too large for GitHub to return its complete file list");
    }
    let commit = tree["sha"]
        .as_str()
        .context("GitHub's repository file list has no commit")?;
    let entries = tree["tree"]
        .as_array()
        .context("GitHub's repository file list has no files")?
        .iter()
        .filter_map(|entry| {
            let path = entry["path"].as_str()?;
            (entry["type"].as_str() == Some("blob") && path.to_ascii_lowercase().ends_with(".json"))
                .then_some((path, entry["size"].as_u64().unwrap_or(0)))
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        bail!("repository contains no JSON files");
    }
    if entries.len() > 256 {
        bail!("repository contains more than 256 JSON files");
    }
    entries.iter().try_fold(0_u64, |total, (path, size)| {
        if *size > MAX_JSON_BYTES {
            bail!("{path} is larger than 32 MiB");
        }
        let total = total.saturating_add(*size);
        if total > MAX_TOTAL_BYTES {
            bail!("repository JSON files are larger than 128 MiB in total");
        }
        Ok(total)
    })?;

    let mut files = Vec::with_capacity(entries.len());
    let mut actual_total = 0_u64;
    for (path, _) in entries {
        let encoded_path = path
            .split('/')
            .map(encode_path_segment)
            .collect::<Vec<_>>()
            .join("/");
        let raw_url = format!(
            "https://raw.githubusercontent.com/{owner}/{repository}/{commit}/{encoded_path}"
        );
        let mut response = agent
            .get(&raw_url)
            .call()
            .with_context(|| format!("loading {path}"))?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES + 1)
            .read_to_vec()
            .with_context(|| format!("reading {path}"))?;
        if bytes.len() as u64 > MAX_JSON_BYTES {
            bail!("{path} is larger than 32 MiB");
        }
        actual_total = actual_total.saturating_add(bytes.len() as u64);
        if actual_total > MAX_TOTAL_BYTES {
            bail!("repository JSON files are larger than 128 MiB in total");
        }
        files.push(PickedFile {
            name: path.to_owned(),
            bytes,
        });
    }
    Ok(files)
}

fn parse_repository(url: &str) -> Result<(&str, &str)> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .context("enter a complete GitHub URL, for example https://github.com/owner/repository")?;
    if !scheme.eq_ignore_ascii_case("https") {
        bail!("GitHub repository URL must use https");
    }
    let (host, path) = rest
        .split_once('/')
        .context("GitHub URL must include an owner and repository")?;
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "github.com" | "www.github.com"
    ) {
        bail!("URL must point to github.com");
    }
    if path.contains('?') || path.contains('#') {
        bail!("use the repository's main URL, without a query or fragment");
    }
    let path = path.trim_end_matches('/');
    let mut parts = path.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository_part = parts.next().unwrap_or_default();
    let repository = repository_part
        .strip_suffix(".git")
        .unwrap_or(repository_part);
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        bail!("use the main repository URL: https://github.com/owner/repository");
    }
    if !owner.bytes().all(is_github_name_byte) || !repository.bytes().all(is_github_name_byte) {
        bail!("GitHub owner or repository contains unsupported characters");
    }
    Ok((owner, repository))
}

fn is_github_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

fn encode_path_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{EXAMPLE_REPOSITORIES, decode_packs};

    #[test]
    fn repository_urls_use_the_main_public_form() {
        assert_eq!(
            parse_repository("https://github.com/0x53A/idiosepius-math-2/").unwrap(),
            ("0x53A", "idiosepius-math-2")
        );
        assert_eq!(
            parse_repository("https://www.github.com/owner/repository.git").unwrap(),
            ("owner", "repository")
        );
        assert!(parse_repository("https://github.com/owner/repo/tree/main").is_err());
        assert!(parse_repository("https://example.com/owner/repo").is_err());
    }

    #[test]
    fn paths_are_percent_encoded_one_segment_at_a_time() {
        assert_eq!(encode_path_segment("packs/ä.json"), "packs%2F%C3%A4.json");
    }

    #[test]
    #[ignore = "network smoke test for the public example repositories"]
    fn public_examples_download_and_decode() {
        for (_, url) in EXAMPLE_REPOSITORIES {
            let files = load_repository(url).unwrap();
            let pack = decode_packs(files).unwrap();
            assert!(!pack.questions.is_empty());
        }
    }
}
