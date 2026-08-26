use html_escape::decode_html_entities;
use regex::Regex;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, COOKIE, REFERER, USER_AGENT};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Config {
    urls: Vec<String>,
    output_dir: PathBuf,
    delay_ms: u64,
    retries: usize,
    max_items: usize,
    max_bytes: u64,
    overwrite: bool,
    cookie: Option<String>,
}

impl Config {
    fn from_arguments(arguments: &Value) -> Result<Self, String> {
        let mut urls = Vec::new();
        if let Some(values) = arguments.get("urls").and_then(Value::as_array) {
            urls.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
        }
        for key in ["url", "input"] {
            if let Some(value) = arguments.get(key).and_then(Value::as_str) {
                urls.extend(value.lines().flat_map(extract_urls));
            }
        }
        let mut seen = BTreeSet::new();
        urls.retain(|value| seen.insert(value.clone()));
        if urls.is_empty() {
            return Err(
                "urls, url, or input containing at least one Douyin URL is required".into(),
            );
        }
        for value in &urls {
            validate_douyin_url(value)?;
        }
        let max_items = integer(arguments, &["maxItems", "max_items"])
            .unwrap_or(200)
            .clamp(1, 2_000) as usize;
        if urls.len() > max_items {
            return Err(format!(
                "batch has {} URLs; maxItems is {max_items}",
                urls.len()
            ));
        }
        let cookie = text(arguments, &["cookie"])
            .or_else(|| text(arguments, &["cookieFile", "cookie_file"]).and_then(read_cookie_file))
            .or_else(|| env::var("DOUYIN_COOKIE").ok())
            .filter(|value| !value.trim().is_empty());
        Ok(Self {
            urls,
            output_dir: text(arguments, &["outputDir", "output_dir"])
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("douyin-videos")),
            delay_ms: integer(arguments, &["delayMs", "delay_ms"])
                .unwrap_or(750)
                .clamp(250, 10_000),
            retries: integer(arguments, &["retries"]).unwrap_or(3).clamp(1, 8) as usize,
            max_items,
            max_bytes: integer(arguments, &["maxBytes", "max_bytes"])
                .unwrap_or(DEFAULT_MAX_BYTES)
                .clamp(1_048_576, DEFAULT_MAX_BYTES),
            overwrite: boolean(arguments, &["overwrite"]).unwrap_or(false),
            cookie,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoRecord {
    source_url: String,
    canonical_url: String,
    aweme_id: String,
    title: String,
    watermark_free_url: Option<String>,
    output_path: Option<String>,
    bytes: u64,
    sha256: Option<String>,
    status: String,
    failure: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchManifest {
    schema_version: u32,
    generated_at_unix: u64,
    output_dir: String,
    requested: usize,
    resolved: usize,
    downloaded: usize,
    skipped: usize,
    failed: usize,
    videos: Vec<VideoRecord>,
}

pub fn run(tool: &str, arguments: &Value) -> Result<Value, String> {
    match tool {
        "resolve" | "download" => execute(tool, Config::from_arguments(arguments)?),
        _ => Err(format!("unsupported douyin batch downloader tool: {tool}")),
    }
}

fn execute(tool: &str, config: Config) -> Result<Value, String> {
    let client = build_client()?;
    if tool == "download" {
        fs::create_dir_all(&config.output_dir).map_err(|error| {
            format!("failed to create {}: {error}", config.output_dir.display())
        })?;
    }
    let mut videos = Vec::new();
    for (index, source_url) in config.urls.iter().enumerate() {
        if index > 0 {
            thread::sleep(Duration::from_millis(config.delay_ms));
        }
        let mut record = match resolve_with_retries(&client, source_url, &config) {
            Ok(record) => record,
            Err(error) => failed_record(source_url, error),
        };
        if tool == "download" && record.status == "resolved" {
            if let Err(error) = download_record(&client, &config, &mut record) {
                record.status = "failed".into();
                record.failure = Some(error);
            }
        }
        videos.push(record);
    }
    let manifest = BatchManifest {
        schema_version: 1,
        generated_at_unix: now_unix(),
        output_dir: config.output_dir.to_string_lossy().into_owned(),
        requested: videos.len(),
        resolved: videos
            .iter()
            .filter(|video| matches!(video.status.as_str(), "resolved" | "downloaded" | "skipped"))
            .count(),
        downloaded: videos
            .iter()
            .filter(|video| video.status == "downloaded")
            .count(),
        skipped: videos
            .iter()
            .filter(|video| video.status == "skipped")
            .count(),
        failed: videos
            .iter()
            .filter(|video| video.status == "failed")
            .count(),
        videos,
    };
    if tool == "download" {
        write_json(&config.output_dir.join("manifest.json"), &manifest)?;
    }
    serde_json::to_value(&manifest).map_err(|error| error.to_string())
}

fn resolve_with_retries(
    client: &Client,
    source_url: &str,
    config: &Config,
) -> Result<VideoRecord, String> {
    let mut last_error = String::new();
    for attempt in 1..=config.retries {
        match resolve_video(client, source_url, config.cookie.as_deref()) {
            Ok(record) => return Ok(record),
            Err(error) => last_error = error,
        }
        if attempt < config.retries {
            thread::sleep(Duration::from_millis(
                config.delay_ms.saturating_mul(attempt as u64),
            ));
        }
    }
    Err(last_error)
}

fn resolve_video(
    client: &Client,
    source_url: &str,
    cookie: Option<&str>,
) -> Result<VideoRecord, String> {
    let mut request = client
        .get(source_url)
        .header(USER_AGENT, DESKTOP_UA)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(REFERER, "https://www.douyin.com/");
    if let Some(cookie) = cookie {
        request = request.header(COOKIE, cookie);
    }
    let response = request
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let final_url = response.url().clone();
    validate_douyin_url(final_url.as_str())?;
    if !response.status().is_success() {
        return Err(format!("Douyin returned HTTP {}", response.status()));
    }
    let html = response
        .text()
        .map_err(|error| format!("failed to read page: {error}"))?;
    if is_access_challenge(&html) {
        return Err(
            "Douyin requires login/verification; reauthenticate manually and retry at a lower rate"
                .into(),
        );
    }
    let decoded = decode_page_source(&html);
    let candidates = collect_media_candidates(&decoded);
    let media_url = candidates
        .into_iter()
        .find(|candidate| is_watermark_free_candidate(candidate))
        .ok_or_else(|| {
            "no authorized watermark-free playback URL was exposed by the page".to_string()
        })?;
    let aweme_id =
        extract_aweme_id(final_url.as_str(), &decoded).unwrap_or_else(|| short_hash(source_url));
    Ok(VideoRecord {
        source_url: source_url.into(),
        canonical_url: final_url.to_string(),
        aweme_id,
        title: extract_title(&decoded),
        watermark_free_url: Some(media_url),
        output_path: None,
        bytes: 0,
        sha256: None,
        status: "resolved".into(),
        failure: None,
    })
}

fn download_record(
    client: &Client,
    config: &Config,
    record: &mut VideoRecord,
) -> Result<(), String> {
    let media_url = record
        .watermark_free_url
        .as_deref()
        .ok_or_else(|| "missing media URL".to_string())?;
    validate_media_url(media_url)?;
    let filename = format!("{}-{}.mp4", safe_filename(&record.title), record.aweme_id);
    let path = config.output_dir.join(filename);
    if path.exists() && !config.overwrite {
        record.status = "skipped".into();
        record.output_path = Some(path.to_string_lossy().into_owned());
        return Ok(());
    }
    let temporary = path.with_extension("mp4.part");
    let response = client
        .get(media_url)
        .header(USER_AGENT, DESKTOP_UA)
        .header(REFERER, &record.canonical_url)
        .send()
        .map_err(|error| format!("media request failed: {error}"))?;
    write_media(response, &temporary, config.max_bytes).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })?;
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| format!("failed to replace {}: {error}", path.display()))?;
    }
    fs::rename(&temporary, &path)
        .map_err(|error| format!("failed to finalize {}: {error}", path.display()))?;
    let bytes = fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len();
    let digest = sha256_file(&path)?;
    record.status = "downloaded".into();
    record.output_path = Some(path.to_string_lossy().into_owned());
    record.bytes = bytes;
    record.sha256 = Some(digest);
    Ok(())
}

fn write_media(mut response: Response, path: &Path, max_bytes: u64) -> Result<(), String> {
    if !response.status().is_success() {
        return Err(format!("media server returned HTTP {}", response.status()));
    }
    if let Some(length) = response.content_length() {
        if length > max_bytes {
            return Err(format!(
                "media is {length} bytes, above maxBytes {max_bytes}"
            ));
        }
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !(content_type.starts_with("video/") || content_type == "application/octet-stream") {
        return Err(format!("unexpected media content type {content_type}"));
    }
    let mut file = fs::File::create(path).map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut written = 0_u64;
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        written = written.saturating_add(count as u64);
        if written > max_bytes {
            return Err(format!("download exceeded maxBytes {max_bytes}"));
        }
        file.write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    if written == 0 {
        return Err("downloaded media is empty".into());
    }
    file.sync_all().map_err(|error| error.to_string())
}

fn build_client() -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::limited(8))
        .timeout(Duration::from_secs(45))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())
}

fn extract_urls(value: &str) -> Vec<String> {
    Regex::new(r#"https?://[^\s\"'<>，。；]+"#)
        .map(|regex| {
            regex
                .find_iter(value)
                .map(|item| item.as_str().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn decode_page_source(value: &str) -> String {
    let mut decoded = decode_html_entities(value).to_string();
    // Douyin can embed media metadata inside JSON strings that are themselves
    // serialized into another page-state string. Normalize a small, bounded
    // number of escaping layers so both `\/` and nested `\\/` URLs become
    // valid HTTPS candidates without attempting unbounded recursive decoding.
    for _ in 0..4 {
        let next = decoded
            .replace("\\u002F", "/")
            .replace("\\u002f", "/")
            .replace("\\/", "/")
            .replace("\\u0026", "&")
            .replace("&amp;", "&");
        if next == decoded {
            break;
        }
        decoded = next;
    }
    decoded
}

fn collect_media_candidates(source: &str) -> Vec<String> {
    let mut candidates = BTreeSet::new();
    if let Ok(regex) = Regex::new(r#"https://[^\s\"'<>\\]+"#) {
        for match_ in regex.find_iter(source) {
            let candidate = match_
                .as_str()
                .trim_end_matches([',', '}', ']'])
                .to_string();
            if looks_like_video_url(&candidate) {
                candidates.insert(candidate);
            }
        }
    }
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by_key(|url| {
        let watermark = url.contains("playwm") || url.contains("watermark=1");
        let quality = if url.contains("origin") || url.contains("source") {
            0
        } else {
            1
        };
        (watermark, quality, std::cmp::Reverse(url.len()))
    });
    candidates
}

fn looks_like_video_url(value: &str) -> bool {
    value.contains("/video/tos/")
        || value.contains("/play/")
        || value.contains("/playwm/")
        || value.contains("douyinvod.com")
        || value.contains("bytecdn.cn") && value.contains("video")
}

fn is_watermark_free_candidate(value: &str) -> bool {
    !value.contains("/playwm/")
        && !value.contains("watermark=1")
        && validate_media_url(value).is_ok()
}

fn validate_douyin_url(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("invalid URL: {error}"))?;
    if url.scheme() != "https" || !host_is(url.host_str(), "douyin.com") {
        return Err("URL must use HTTPS on douyin.com or a Douyin subdomain".into());
    }
    Ok(())
}

fn validate_media_url(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("invalid media URL: {error}"))?;
    let allowed = [
        "douyin.com",
        "douyinvod.com",
        "bytecdn.cn",
        "byteimg.com",
        "bytevcloud.com",
        "ibytedtos.com",
        "snssdk.com",
        "zjcdn.com",
    ];
    if url.scheme() != "https" || !allowed.iter().any(|domain| host_is(url.host_str(), domain)) {
        return Err("media URL is not on an approved HTTPS Douyin/ByteDance host".into());
    }
    Ok(())
}

fn host_is(host: Option<&str>, domain: &str) -> bool {
    host.is_some_and(|host| host == domain || host.ends_with(&format!(".{domain}")))
}

fn extract_aweme_id(url: &str, source: &str) -> Option<String> {
    for pattern in [
        r"/video/(\d{8,})",
        r#"\"aweme_id\"\s*:\s*\"(\d{8,})\""#,
        r"modal_id=(\d{8,})",
    ] {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(value) = regex
                .captures(url)
                .or_else(|| regex.captures(source))
                .and_then(|captures| captures.get(1))
            {
                return Some(value.as_str().into());
            }
        }
    }
    None
}

fn extract_title(source: &str) -> String {
    for pattern in [
        r#"(?is)<meta[^>]+(?:name|property)=[\"'](?:description|og:title)[\"'][^>]+content=[\"']([^\"']+)"#,
        r#"(?is)<title>(.*?)</title>"#,
        r#"\"desc\"\s*:\s*\"([^\"]+)\""#,
    ] {
        if let Ok(regex) = Regex::new(pattern) {
            if let Some(value) = regex.captures(source).and_then(|captures| captures.get(1)) {
                let title = decode_html_entities(value.as_str()).trim().to_string();
                if !title.is_empty() {
                    return title;
                }
            }
        }
    }
    "douyin-video".into()
}

fn safe_filename(value: &str) -> String {
    let mut output = String::new();
    let mut dash = false;
    for character in value.chars().take(80) {
        if character.is_alphanumeric() || matches!(character, '-' | '_' | ' ' | '·') {
            output.push(character);
            dash = false;
        } else if !dash {
            output.push('-');
            dash = true;
        }
    }
    let output = output.trim_matches([' ', '-', '.']);
    if output.is_empty() {
        "douyin-video".into()
    } else {
        output.into()
    }
}

fn failed_record(source_url: &str, failure: String) -> VideoRecord {
    VideoRecord {
        source_url: source_url.into(),
        canonical_url: source_url.into(),
        aweme_id: short_hash(source_url),
        title: "douyin-video".into(),
        watermark_free_url: None,
        output_path: None,
        bytes: 0,
        sha256: None,
        status: "failed".into(),
        failure: Some(failure),
    }
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| error.to_string())
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)?
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn integer(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
    })
}

fn boolean(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn read_cookie_file(path: String) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_access_challenge(value: &str) -> bool {
    [
        "captcha",
        "verify-center",
        "安全验证",
        "扫码登录",
        "请登录后查看",
    ]
    .iter()
    .any(|needle| value.to_lowercase().contains(&needle.to_lowercase()))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_deduplicates_and_prefers_watermark_free_candidates() {
        let source = r#"{\"video\":{\"play_addr\":{\"url_list\":[\"https:\\/\\/v3-dy-o.zjcdn.com\\/video\\/tos\\/cn\\/source.mp4\"]},\"play_addr_lowbr\":{\"url_list\":[\"https:\\/\\/www.douyin.com\\/aweme\\/v1\\/playwm\\/?video_id=x\"]}}}"#;
        let candidates = collect_media_candidates(&decode_page_source(source));
        assert_eq!(candidates.len(), 2);
        assert!(is_watermark_free_candidate(&candidates[0]));
        assert!(!is_watermark_free_candidate(&candidates[1]));
    }

    #[test]
    fn rejects_non_douyin_and_private_network_sources() {
        assert!(validate_douyin_url("https://www.douyin.com/video/12345678").is_ok());
        assert!(validate_douyin_url("https://example.com/video/12345678").is_err());
        assert!(validate_douyin_url("http://www.douyin.com/video/12345678").is_err());
    }

    #[test]
    fn sanitizes_paths_and_extracts_video_ids() {
        assert_eq!(safe_filename("../危险:*?标题"), "危险-标题");
        assert_eq!(
            extract_aweme_id("https://www.douyin.com/video/729123456789", "").as_deref(),
            Some("729123456789")
        );
    }

    #[test]
    fn accepts_lines_or_arrays_and_deduplicates() {
        let config = Config::from_arguments(&json!({
            "input":"分享 https://v.douyin.com/abc/\nhttps://v.douyin.com/abc/",
            "urls":["https://www.douyin.com/video/729123456789"]
        }))
        .unwrap();
        assert_eq!(config.urls.len(), 2);
        assert_eq!(config.max_items, 200);
    }
}
