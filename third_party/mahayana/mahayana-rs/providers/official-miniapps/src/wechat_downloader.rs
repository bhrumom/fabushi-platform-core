use html_escape::decode_html_entities;
use regex::Regex;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, COOKIE, REFERER, USER_AGENT};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const PLUGIN_ID: &str = "wechat-article-downloader";
const DESKTOP_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";
const WECHAT_UA: &str = "Mozilla/5.0 (Linux; Android 13; Pixel 7 Build/TQ3A.230805.001; wv) AppleWebKit/537.36 Version/4.0 Chrome/115.0 Mobile Safari/537.36 MicroMessenger/8.0.43.2480(0x28002B37) WeChat/arm64 NetType/WIFI Language/zh_CN ABI/arm64";

#[derive(Debug, Clone)]
struct Config {
    seed_url: String,
    output_dir: PathBuf,
    search_pages: usize,
    delay_ms: u64,
    article_retries: usize,
    max_articles: usize,
    max_albums: usize,
    download_images: bool,
    raw_html: bool,
    allow_sogou: bool,
    strict: bool,
    cookie: Option<String>,
}

impl Config {
    fn from_arguments(arguments: &Value) -> Result<Self, String> {
        let seed_url = string_any(arguments, &["url", "seedUrl", "seed_url", "input"])
            .ok_or_else(|| "url is required".to_string())?;
        validate_seed_url(&seed_url)?;
        let output_dir = string_any(arguments, &["outputDir", "output_dir"])
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("wechat-articles"));
        let cookie = string_any(arguments, &["cookie"])
            .or_else(|| {
                string_any(arguments, &["cookieFile", "cookie_file"]).and_then(read_cookie_file)
            })
            .or_else(|| env::var("WECHAT_COOKIE").ok())
            .filter(|value| !value.trim().is_empty());
        Ok(Self {
            seed_url,
            output_dir,
            search_pages: usize_any(arguments, &["searchPages", "search_pages"])
                .unwrap_or(10)
                .min(10),
            delay_ms: u64_any(arguments, &["delayMs", "delay_ms"])
                .unwrap_or(450)
                .clamp(100, 5_000),
            article_retries: usize_any(arguments, &["articleRetries", "article_retries"])
                .unwrap_or(5)
                .clamp(1, 10),
            max_articles: usize_any(arguments, &["maxArticles", "max_articles"]).unwrap_or(0),
            max_albums: usize_any(arguments, &["maxAlbums", "max_albums"])
                .unwrap_or(250)
                .clamp(1, 2_000),
            download_images: bool_any(
                arguments,
                &[
                    "downloadImages",
                    "download_images",
                    "downloadAssets",
                    "download_assets",
                ],
            )
            .unwrap_or(true),
            raw_html: bool_any(arguments, &["rawHtml", "raw_html"]).unwrap_or(false),
            allow_sogou: bool_any(arguments, &["allowSogou", "allow_sogou"]).unwrap_or(true),
            strict: bool_any(arguments, &["strict"]).unwrap_or(false),
            cookie,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountRecord {
    biz: String,
    nickname: String,
    user_name: String,
    alias: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArticleRecord {
    key: String,
    title: String,
    author: String,
    nickname: String,
    biz: String,
    mid: String,
    idx: String,
    sn: String,
    publish_time: String,
    source_url: String,
    canonical_url: String,
    output_path: Option<String>,
    content_html_path: Option<String>,
    content_text_path: Option<String>,
    body_bytes: usize,
    text_bytes: usize,
    image_count: usize,
    downloaded_images: usize,
    failed_images: usize,
    offline_complete: bool,
    album_ids: Vec<String>,
    discovered_from: String,
}

#[derive(Debug, Clone, Default)]
struct WrittenArticle {
    output_path: Option<String>,
    content_html_path: Option<String>,
    content_text_path: Option<String>,
    body_bytes: usize,
    text_bytes: usize,
    image_count: usize,
    downloaded_images: usize,
    failed_images: usize,
    offline_complete: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumRecord {
    id: String,
    title: String,
    declared_count: Option<usize>,
    discovered_articles: usize,
    pages: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureRecord {
    kind: String,
    target: String,
    error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestStats {
    articles: usize,
    albums: usize,
    failures: usize,
    images: usize,
    downloaded_images: usize,
    failed_images: usize,
    offline_articles: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    generated_at_unix: u64,
    seed_url: String,
    account: AccountRecord,
    stats: ManifestStats,
    articles: Vec<ArticleRecord>,
    albums: Vec<AlbumRecord>,
    failures: Vec<FailureRecord>,
    warnings: Vec<String>,
    discovery_sources: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueuedArticle {
    url: String,
    source: String,
}

#[derive(Debug, Clone, Default)]
struct ParsedArticle {
    account: AccountRecord,
    title: String,
    author: String,
    mid: String,
    idx: String,
    sn: String,
    publish_time: String,
    canonical_url: String,
    body_html: String,
    album_ids: BTreeSet<String>,
    article_urls: BTreeSet<String>,
    article_title_hints: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default)]
struct AlbumPage {
    title: String,
    declared_count: Option<usize>,
    article_urls: Vec<String>,
    continue_flag: bool,
    last_mid: String,
    last_idx: String,
}

struct Downloader {
    config: Config,
    client: Client,
    account: AccountRecord,
    article_queue: VecDeque<QueuedArticle>,
    queued_urls: HashSet<String>,
    article_title_hints: BTreeMap<String, String>,
    processed_keys: HashSet<String>,
    album_queue: VecDeque<String>,
    queued_albums: HashSet<String>,
    processed_albums: HashSet<String>,
    album_members: BTreeMap<String, BTreeSet<String>>,
    articles: BTreeMap<String, ArticleRecord>,
    albums: BTreeMap<String, AlbumRecord>,
    failures: Vec<FailureRecord>,
    warnings: Vec<String>,
    discovery_sources: BTreeSet<String>,
    sogou_done: bool,
    write_files: bool,
}

pub fn run(tool: &str, arguments: &Value) -> Result<Value, String> {
    let config = Config::from_arguments(arguments)?;
    match tool {
        "inspect" => inspect(config),
        "discover" => crawl(config, false),
        "download" => crawl(config, true),
        _ => Err(format!("unsupported {PLUGIN_ID} tool: {tool}")),
    }
}

fn inspect(config: Config) -> Result<Value, String> {
    let client = build_client(&config)?;
    let (_, parsed) = fetch_and_parse_article(&client, &config.seed_url, "seed", &config)?;
    Ok(json!({
        "ok": true,
        "account": parsed.account,
        "article": {
            "title": parsed.title,
            "author": parsed.author,
            "mid": parsed.mid,
            "idx": parsed.idx,
            "canonicalUrl": parsed.canonical_url,
            "albumIds": parsed.album_ids,
            "bodyBytes": parsed.body_html.len()
        }
    }))
}

fn crawl(config: Config, write_files: bool) -> Result<Value, String> {
    if write_files {
        fs::create_dir_all(config.output_dir.join("articles"))
            .map_err(|error| format!("create output directory: {error}"))?;
    }
    let client = build_client(&config)?;
    let seed_url = config.seed_url.clone();
    let mut downloader = Downloader {
        config,
        client,
        account: AccountRecord::default(),
        article_queue: VecDeque::new(),
        queued_urls: HashSet::new(),
        article_title_hints: BTreeMap::new(),
        processed_keys: HashSet::new(),
        album_queue: VecDeque::new(),
        queued_albums: HashSet::new(),
        processed_albums: HashSet::new(),
        album_members: BTreeMap::new(),
        articles: BTreeMap::new(),
        albums: BTreeMap::new(),
        failures: Vec::new(),
        warnings: Vec::new(),
        discovery_sources: BTreeSet::new(),
        sogou_done: false,
        write_files,
    };
    downloader.enqueue_article(seed_url, "seed");
    downloader.run()?;
    let manifest = downloader.manifest();
    let manifest_path = if write_files {
        let path = downloader.config.output_dir.join("manifest.json");
        write_json(&path, &manifest)?;
        write_index(&downloader.config.output_dir, &manifest)?;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };
    if downloader.config.strict && !manifest.failures.is_empty() {
        return Err(format!(
            "download completed with {} failures; see {}",
            manifest.failures.len(),
            manifest_path.as_deref().unwrap_or("result")
        ));
    }
    Ok(json!({
        "ok": manifest.stats.articles > 0,
        "outputDir": write_files.then(|| downloader.config.output_dir.to_string_lossy().to_string()),
        "manifestPath": manifest_path,
        "account": manifest.account,
        "stats": manifest.stats,
        "albums": manifest.albums,
        "failures": manifest.failures,
        "warnings": manifest.warnings,
        "discoverySources": manifest.discovery_sources
    }))
}

impl Downloader {
    fn run(&mut self) -> Result<(), String> {
        loop {
            let mut progressed = false;
            while let Some(article) = self.article_queue.pop_front() {
                if self.limit_reached() {
                    self.warnings.push(format!(
                        "达到 maxArticles={}，已停止继续抓取",
                        self.config.max_articles
                    ));
                    self.article_queue.clear();
                    self.album_queue.clear();
                    return Ok(());
                }
                progressed = true;
                self.process_article(article);
            }
            if !self.sogou_done && self.config.allow_sogou && !self.account.nickname.is_empty() {
                self.sogou_done = true;
                self.discover_sogou();
                continue;
            }
            while let Some(album_id) = self.album_queue.pop_front() {
                if self.processed_albums.len() >= self.config.max_albums {
                    self.warnings.push(format!(
                        "达到 maxAlbums={}，已停止继续展开专辑",
                        self.config.max_albums
                    ));
                    self.album_queue.clear();
                    break;
                }
                progressed = true;
                self.process_album(album_id);
            }
            if self.article_queue.is_empty() && self.album_queue.is_empty() {
                if !progressed {
                    break;
                }
                break;
            }
        }
        Ok(())
    }

    fn limit_reached(&self) -> bool {
        self.config.max_articles > 0 && self.articles.len() >= self.config.max_articles
    }

    fn enqueue_article(&mut self, url: String, source: &str) {
        self.enqueue_article_with_hint(url, source, None);
    }

    fn enqueue_article_with_hint(&mut self, url: String, source: &str, title_hint: Option<String>) {
        let Some(url) = normalize_article_url(&url) else {
            return;
        };
        let identity = article_url_identity(&url);
        if let Some(title_hint) = title_hint
            .map(|value| value.trim().to_string())
            .filter(|value| value.chars().count() >= 4 && !value.starts_with("http"))
        {
            self.article_title_hints
                .entry(identity.clone())
                .or_insert(title_hint);
        }
        if self.queued_urls.insert(identity) {
            self.article_queue.push_back(QueuedArticle {
                url,
                source: source.to_string(),
            });
        }
    }

    fn enqueue_album(&mut self, album_id: String) {
        if album_id.chars().all(|character| character.is_ascii_digit())
            && self.queued_albums.insert(album_id.clone())
        {
            self.album_queue.push_back(album_id);
        }
    }

    fn process_article(&mut self, queued: QueuedArticle) {
        self.discovery_sources.insert(queued.source.clone());
        let identity = article_url_identity(&queued.url);
        let title_hint = self.article_title_hints.get(&identity).cloned();
        let mut recovered_from_title_index = false;
        let mut result =
            fetch_and_parse_article(&self.client, &queued.url, &queued.source, &self.config);
        if let Err(original_error) = &result {
            if self.config.allow_sogou
                && !original_error.starts_with("文章不可公开访问：")
                && title_hint.is_some()
            {
                if let Some(title_hint) = title_hint.as_deref() {
                    if let Ok(recovered) = self.recover_article_via_sogou(title_hint) {
                        recovered_from_title_index = true;
                        result = Ok(recovered);
                    }
                }
            }
        }
        let (html, parsed) = match result {
            Ok(value) => value,
            Err(error) => {
                let kind = if error.starts_with("文章不可公开访问：") {
                    "unavailable-article"
                } else {
                    "article"
                };
                self.failures.push(FailureRecord {
                    kind: kind.into(),
                    target: queued.url,
                    error,
                });
                return;
            }
        };
        if self.account.biz.is_empty() {
            self.account = parsed.account.clone();
        }
        if parsed.account.biz != self.account.biz {
            self.warnings.push(format!(
                "忽略其他公众号文章：{} ({})",
                parsed.title, parsed.account.nickname
            ));
            return;
        }
        let key = article_key(&parsed.mid, &parsed.idx, &queued.url);
        if !self.processed_keys.insert(key.clone()) {
            return;
        }
        for album_id in &parsed.album_ids {
            self.album_members
                .entry(album_id.clone())
                .or_default()
                .insert(key.clone());
            self.enqueue_album(album_id.clone());
        }
        for url in &parsed.article_urls {
            if article_biz(url).as_deref() == Some(self.account.biz.as_str()) {
                self.enqueue_article_with_hint(
                    url.clone(),
                    "article-link",
                    parsed.article_title_hints.get(url).cloned(),
                );
            }
        }
        let written = if self.write_files {
            match write_article(&self.client, &self.config, &parsed, &html) {
                Ok(value) => value,
                Err(error) => {
                    self.failures.push(FailureRecord {
                        kind: "write".into(),
                        target: queued.url.clone(),
                        error,
                    });
                    let image_count = count_images(&parsed.body_html);
                    WrittenArticle {
                        body_bytes: parsed.body_html.len(),
                        text_bytes: html_to_text(&parsed.body_html).len(),
                        image_count,
                        failed_images: image_count,
                        ..WrittenArticle::default()
                    }
                }
            }
        } else {
            WrittenArticle {
                body_bytes: parsed.body_html.len(),
                text_bytes: html_to_text(&parsed.body_html).len(),
                image_count: count_images(&parsed.body_html),
                ..WrittenArticle::default()
            }
        };
        let record = ArticleRecord {
            key: key.clone(),
            title: parsed.title,
            author: parsed.author,
            nickname: parsed.account.nickname,
            biz: parsed.account.biz,
            mid: parsed.mid,
            idx: parsed.idx,
            sn: parsed.sn,
            publish_time: parsed.publish_time,
            source_url: queued.url,
            canonical_url: parsed.canonical_url,
            output_path: written.output_path,
            content_html_path: written.content_html_path,
            content_text_path: written.content_text_path,
            body_bytes: written.body_bytes,
            text_bytes: written.text_bytes,
            image_count: written.image_count,
            downloaded_images: written.downloaded_images,
            failed_images: written.failed_images,
            offline_complete: written.offline_complete,
            album_ids: parsed.album_ids.into_iter().collect(),
            discovered_from: if recovered_from_title_index {
                "sogou-title-recovery".into()
            } else {
                queued.source
            },
        };
        self.articles.insert(key, record);
    }

    fn process_album(&mut self, album_id: String) {
        if !self.processed_albums.insert(album_id.clone()) {
            return;
        }
        self.discovery_sources.insert("public-album-api".into());
        let mut begin_mid = String::new();
        let mut begin_idx = String::new();
        let mut pages = 0usize;
        let mut all_urls = BTreeSet::new();
        let mut title = String::new();
        let mut declared_count = None;
        loop {
            let page = fetch_album_page(
                &self.client,
                &self.config,
                &self.account.biz,
                &album_id,
                &begin_mid,
                &begin_idx,
            );
            let page = match page {
                Ok(page) => page,
                Err(error) => {
                    self.failures.push(FailureRecord {
                        kind: "album".into(),
                        target: album_id.clone(),
                        error,
                    });
                    break;
                }
            };
            pages += 1;
            if title.is_empty() {
                title = page.title.clone();
            }
            if declared_count.is_none() {
                declared_count = page.declared_count;
            }
            let previous_count = all_urls.len();
            for url in page.article_urls {
                if article_biz(&url).as_deref() == Some(self.account.biz.as_str()) {
                    all_urls.insert(url);
                }
            }
            if !page.continue_flag || page.last_mid.is_empty() || all_urls.len() == previous_count {
                break;
            }
            begin_mid = page.last_mid;
            begin_idx = page.last_idx;
            if pages > 1_000 {
                self.warnings
                    .push(format!("专辑 {album_id} 超过 1000 页，已停止"));
                break;
            }
        }
        for url in &all_urls {
            self.album_members
                .entry(album_id.clone())
                .or_default()
                .insert(article_url_identity(url));
            self.enqueue_article(url.clone(), &format!("album:{album_id}"));
        }
        self.albums.insert(
            album_id.clone(),
            AlbumRecord {
                id: album_id,
                title,
                declared_count,
                discovered_articles: all_urls.len(),
                pages,
            },
        );
    }

    fn recover_article_via_sogou(
        &mut self,
        title_hint: &str,
    ) -> Result<(String, ParsedArticle), String> {
        if self.account.nickname.is_empty() || title_hint.chars().count() < 4 {
            return Err("缺少可用于公开索引恢复的标题或公众号名称".into());
        }
        let queries = [
            title_hint.to_string(),
            format!("{title_hint} {}", self.account.nickname),
        ];
        let mut last_error = String::new();
        for query in queries {
            let encoded =
                url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
            let search_url = format!(
                "https://weixin.sogou.com/weixin?type=2&s_from=input&query={encoded}&ie=utf8"
            );
            let html = get_text(&self.client, &search_url, None, false, &self.config)?;
            if is_sogou_verification_page(&html) {
                return Err("搜狗公开索引要求验证码，未尝试绕过".into());
            }
            let links = parse_sogou_exact_links(&html, &self.account.nickname, title_hint);
            if links.is_empty() {
                last_error = format!("公开索引未找到精确标题：{title_hint}");
                continue;
            }
            for link in links {
                match resolve_sogou_link(&self.client, &self.config, &search_url, &link).and_then(
                    |url| {
                        fetch_and_parse_article(
                            &self.client,
                            &url,
                            "sogou-title-recovery",
                            &self.config,
                        )
                    },
                ) {
                    Ok((source, parsed))
                        if parsed.account.biz == self.account.biz
                            && normalized_title(&parsed.title) == normalized_title(title_hint) =>
                    {
                        self.discovery_sources.insert("sogou-title-recovery".into());
                        return Ok((source, parsed));
                    }
                    Ok((_, parsed)) => {
                        last_error = format!(
                            "公开索引候选不匹配：{} / {}",
                            parsed.account.nickname, parsed.title
                        );
                    }
                    Err(error) => last_error = error,
                }
            }
        }
        Err(last_error)
    }

    fn discover_sogou(&mut self) {
        if self.config.search_pages == 0 {
            return;
        }
        self.discovery_sources.insert("sogou-public-index".into());
        let encoded = url::form_urlencoded::byte_serialize(self.account.nickname.as_bytes())
            .collect::<String>();
        for page in 1..=self.config.search_pages {
            let search_url = format!(
                "https://weixin.sogou.com/weixin?type=2&s_from=input&query={encoded}&ie=utf8&page={page}"
            );
            let html = match get_text(&self.client, &search_url, None, false, &self.config) {
                Ok(html) => html,
                Err(error) => {
                    self.failures.push(FailureRecord {
                        kind: "sogou-search".into(),
                        target: search_url,
                        error,
                    });
                    break;
                }
            };
            if is_sogou_verification_page(&html) {
                self.warnings.push(
                    "搜狗公开索引要求验证码；已停止该来源且未尝试绕过，专辑与文章链接抓取仍继续"
                        .into(),
                );
                break;
            }
            for link in parse_sogou_links(&html, &self.account.nickname) {
                match resolve_sogou_link(&self.client, &self.config, &search_url, &link) {
                    Ok(url) => self.enqueue_article(url, "sogou-public-index"),
                    Err(error) => self.failures.push(FailureRecord {
                        kind: "sogou-link".into(),
                        target: link,
                        error,
                    }),
                }
            }
        }
    }

    fn manifest(&self) -> Manifest {
        let articles = self.articles.values().cloned().collect::<Vec<_>>();
        let mut albums = self.albums.values().cloned().collect::<Vec<_>>();
        apply_album_membership_counts(&mut albums, &self.album_members);
        let images = articles.iter().map(|article| article.image_count).sum();
        let downloaded_images = articles
            .iter()
            .map(|article| article.downloaded_images)
            .sum();
        let failed_images = articles.iter().map(|article| article.failed_images).sum();
        let offline_articles = articles
            .iter()
            .filter(|article| article.offline_complete)
            .count();
        Manifest {
            schema_version: 2,
            generated_at_unix: now_unix(),
            seed_url: self.config.seed_url.clone(),
            account: self.account.clone(),
            stats: ManifestStats {
                articles: articles.len(),
                albums: albums.len(),
                failures: self.failures.len(),
                images,
                downloaded_images,
                failed_images,
                offline_articles,
            },
            articles,
            albums,
            failures: self.failures.clone(),
            warnings: self.warnings.clone(),
            discovery_sources: self.discovery_sources.iter().cloned().collect(),
        }
    }
}

fn apply_album_membership_counts(
    albums: &mut [AlbumRecord],
    album_members: &BTreeMap<String, BTreeSet<String>>,
) {
    for album in albums {
        if let Some(members) = album_members.get(&album.id) {
            album.discovered_articles = members.len();
        }
    }
}

fn build_client(config: &Config) -> Result<Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        USER_AGENT,
        reqwest::header::HeaderValue::from_static(DESKTOP_UA),
    );
    if let Some(cookie) = &config.cookie {
        headers.insert(
            COOKIE,
            reqwest::header::HeaderValue::from_str(cookie)
                .map_err(|error| format!("invalid cookie header: {error}"))?,
        );
    }
    Client::builder()
        .default_headers(headers)
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| format!("build HTTP client: {error}"))
}

fn fetch_and_parse_article(
    client: &Client,
    url: &str,
    source: &str,
    config: &Config,
) -> Result<(String, ParsedArticle), String> {
    let mut last_error = String::new();
    for attempt in 1..=config.article_retries {
        match get_text(
            client,
            url,
            Some("https://mp.weixin.qq.com/"),
            false,
            config,
        ) {
            Ok(html) => {
                if let Some(reason) = unavailable_reason(&html) {
                    return Err(format!("文章不可公开访问：{reason}"));
                }
                if is_verification_page(&html) {
                    last_error = "微信返回临时验证页".into();
                } else {
                    match parse_article(&html, url, source) {
                        Ok(parsed) if !parsed.account.biz.is_empty() && !parsed.mid.is_empty() => {
                            return Ok((html, parsed));
                        }
                        Ok(_) => {
                            last_error = "页面没有可验证的公众号 biz/mid 元数据".into();
                        }
                        Err(error) => {
                            last_error = error;
                        }
                    }
                }
            }
            Err(error) => last_error = error,
        }
        if attempt < config.article_retries {
            let backoff_ms = 1_000u64
                .saturating_mul(1u64 << (attempt.saturating_sub(1).min(3) as u32))
                .min(8_000);
            thread::sleep(Duration::from_millis(backoff_ms));
        }
    }
    Err(format!(
        "{last_error}（已重试 {} 次）",
        config.article_retries
    ))
}

fn get_text(
    client: &Client,
    url: &str,
    referer: Option<&str>,
    json_response: bool,
    config: &Config,
) -> Result<String, String> {
    let mut last_error = String::new();
    for attempt in 1..=3 {
        let mut request = client.get(url).header(
            USER_AGENT,
            if json_response { WECHAT_UA } else { DESKTOP_UA },
        );
        if let Some(referer) = referer {
            request = request.header(REFERER, referer);
        }
        request = request.header(
            ACCEPT,
            if json_response {
                "application/json, text/javascript, */*; q=0.01"
            } else {
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            },
        );
        match request.send().and_then(Response::error_for_status) {
            Ok(response) => match response.text() {
                Ok(text) => {
                    thread::sleep(Duration::from_millis(config.delay_ms));
                    return Ok(text);
                }
                Err(error) => last_error = error.to_string(),
            },
            Err(error) => last_error = error.to_string(),
        }
        if attempt < 3 {
            thread::sleep(Duration::from_millis(config.delay_ms * attempt));
        }
    }
    Err(format!("GET {url}: {last_error}"))
}

fn fetch_album_page(
    client: &Client,
    config: &Config,
    biz: &str,
    album_id: &str,
    begin_mid: &str,
    begin_idx: &str,
) -> Result<AlbumPage, String> {
    let mut url =
        Url::parse("https://mp.weixin.qq.com/mp/appmsgalbum").map_err(|error| error.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("action", "getalbum");
        query.append_pair("__biz", biz);
        query.append_pair("album_id", album_id);
        query.append_pair("count", "10");
        query.append_pair("f", "json");
        if !begin_mid.is_empty() {
            query.append_pair("begin_msgid", begin_mid);
            query.append_pair("begin_itemidx", begin_idx);
        }
    }
    let referer = format!(
        "https://mp.weixin.qq.com/mp/appmsgalbum?action=getalbum&__biz={biz}&album_id={album_id}"
    );
    let source = get_text(client, url.as_str(), Some(&referer), true, config)?;
    let payload: Value = serde_json::from_str(&source)
        .map_err(|error| format!("album {album_id} returned non-JSON: {error}"))?;
    let ret = payload
        .pointer("/base_resp/ret")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if ret != 0 {
        return Err(format!("album {album_id} API ret={ret}"));
    }
    let response = payload
        .get("getalbum_resp")
        .ok_or_else(|| format!("album {album_id} missing getalbum_resp"))?;
    let mut article_urls = Vec::new();
    let articles = response
        .get("article_list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut last_mid = String::new();
    let mut last_idx = String::new();
    for article in &articles {
        if let Some(url) = article
            .get("url")
            .and_then(Value::as_str)
            .and_then(normalize_article_url)
        {
            article_urls.push(url);
        }
        last_mid = value_string(article.get("msgid"));
        last_idx = value_string(article.get("itemidx"));
    }
    let base = response.get("base_info").unwrap_or(&Value::Null);
    let title = string_value_any(base, &["title", "msg_title"])
        .or_else(|| string_value_any(response, &["title", "msg_title"]))
        .unwrap_or_default();
    let declared_count = usize_value_any(base, &["article_count"])
        .or_else(|| usize_value_any(response, &["article_count"]));
    Ok(AlbumPage {
        title,
        declared_count,
        article_urls,
        continue_flag: value_truthy(response.get("continue_flag")),
        last_mid,
        last_idx,
    })
}

fn parse_article(html: &str, source_url: &str, _source: &str) -> Result<ParsedArticle, String> {
    let biz = capture_first(
        html,
        &[
            r#"var\s+biz\s*=\s*"([^"]+)""#,
            r#"biz:\s*"([^"]+)""#,
            r#"biz:\s*'([^']+)'"#,
        ],
    );
    let mid = capture_first(
        html,
        &[
            r#"var\s+mid\s*=\s*"([^"]+)""#,
            r#"mid:\s*'([0-9]+)'"#,
            r#"appmsgid\s*=\s*"([0-9]+)""#,
        ],
    );
    let idx = capture_first(
        html,
        &[
            r#"var\s+idx\s*=\s*"([^"]+)""#,
            r#"idx:\s*'([0-9]+)'"#,
            r#"msg_daily_idx\s*=\s*"([0-9]+)""#,
        ],
    );
    let sn = capture_first(
        html,
        &[r#"var\s+sn\s*=\s*"([^"]*)""#, r#"sn:\s*'([0-9a-f]{32})'"#],
    );
    let title = decode_wechat_string(&capture_first(
        html,
        &[
            r#"title:\s*'([^']+)'"#,
            r#"window\.msg_title\s*=\s*'([^']+)'"#,
            r#"var\s+msg_title\s*=\s*"([^"]+)""#,
            r#"<h1[^>]*id="activity-name"[^>]*>(.*?)</h1>"#,
        ],
    ));
    let nickname = decode_wechat_string(&capture_first(
        html,
        &[
            r#"nick_name:\s*'([^']+)'"#,
            r#"window\.nickname\s*=\s*"([^"]+)""#,
            r#"<a[^>]*id="js_name"[^>]*>(.*?)</a>"#,
        ],
    ));
    let user_name = capture_first(
        html,
        &[
            r#"user_name:\s*'([^']+)'"#,
            r#"selfUserName\s*=\s*"([^"]+)""#,
        ],
    );
    let alias = decode_wechat_string(&capture_first(
        html,
        &[r#"window\.alias\s*=\s*"([^"]*)""#, r#"alias:\s*'([^']*)'"#],
    ));
    let author = decode_wechat_string(&capture_first(
        html,
        &[r#"var\s+author\s*=\s*"([^"]*)""#, r#"author:\s*'([^']*)'"#],
    ));
    let publish_time = capture_first(
        html,
        &[
            r#"ct:\s*'([0-9]+)'"#,
            r#"var\s+ct\s*=\s*"([0-9]+)""#,
            r#"create_time:\s*'([0-9]+)'"#,
        ],
    );
    let body_html = extract_js_content(html).unwrap_or_default();
    let album_ids = extract_album_ids(html);
    let article_urls = extract_article_urls(html);
    let article_title_hints = extract_article_title_hints(html);
    let canonical_url = article_urls
        .iter()
        .find(|url| {
            article_mid(url).as_deref() == Some(mid.as_str())
                && article_idx(url).as_deref() == Some(idx.as_str())
        })
        .cloned()
        .or_else(|| canonical_from_parts(&biz, &mid, &idx, &sn))
        .unwrap_or_else(|| source_url.to_string());
    if title.is_empty() && body_html.is_empty() {
        return Err("article title and body are both empty".into());
    }
    Ok(ParsedArticle {
        account: AccountRecord {
            biz,
            nickname,
            user_name,
            alias,
        },
        title,
        author,
        mid,
        idx: if idx.is_empty() { "1".into() } else { idx },
        sn,
        publish_time,
        canonical_url,
        body_html,
        album_ids,
        article_urls,
        article_title_hints,
    })
}

fn extract_js_content(html: &str) -> Option<String> {
    let id_position = html
        .find("id=\"js_content\"")
        .or_else(|| html.find("id='js_content'"))?;
    let start = html[..id_position].rfind("<div")?;
    let tag_regex = Regex::new(r"(?is)</?div\b[^>]*>").ok()?;
    let mut depth = 0i64;
    let mut end = None;
    for found in tag_regex.find_iter(&html[start..]) {
        let tag = found.as_str();
        if tag.starts_with("</") {
            depth -= 1;
            if depth == 0 {
                end = Some(start + found.end());
                break;
            }
        } else {
            depth += 1;
        }
    }
    let end = end?;
    let mut content = html[start..end].to_string();
    content = content.replace("visibility: hidden;", "visibility: visible;");
    content = content.replace("opacity: 0;", "opacity: 1;");
    content = content.replace("data-src=", "src=");
    Some(content)
}

fn extract_album_ids(html: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for pattern in [
        r#"album_id:\s*'([0-9]+)'"#,
        r#"album_id_str:\s*'([0-9]+)'"#,
        r#"[?&]album_id=([0-9]+)"#,
    ] {
        if let Ok(regex) = Regex::new(pattern) {
            for captures in regex.captures_iter(html) {
                if let Some(value) = captures.get(1) {
                    ids.insert(value.as_str().to_string());
                }
            }
        }
    }
    ids
}

fn extract_article_urls(html: &str) -> BTreeSet<String> {
    let mut urls = BTreeSet::new();
    let patterns = [
        r#"https?://mp\.weixin\.qq\.com/s\?[^"'<>\s]+"#,
        r#"https?:\\/\\/mp\.weixin\.qq\.com\\/s\?[^"'<>\s]+"#,
    ];
    for pattern in patterns {
        if let Ok(regex) = Regex::new(pattern) {
            for found in regex.find_iter(html) {
                if let Some(url) = normalize_article_url(found.as_str()) {
                    if article_biz(&url).is_some() && article_mid(&url).is_some() {
                        urls.insert(url);
                    }
                }
            }
        }
    }
    urls
}

fn extract_article_title_hints(html: &str) -> BTreeMap<String, String> {
    let mut hints = BTreeMap::new();
    let Ok(anchor_regex) =
        Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*["']([^"']+)["'][^>]*>(.*?)</a>"#)
    else {
        return hints;
    };
    for captures in anchor_regex.captures_iter(html) {
        let Some(href) = captures.get(1) else {
            continue;
        };
        let Some(url) = normalize_article_url(href.as_str()) else {
            continue;
        };
        if article_biz(&url).is_none() || article_mid(&url).is_none() {
            continue;
        }
        let title = captures
            .get(2)
            .map(|value| strip_tags(value.as_str()))
            .unwrap_or_default();
        if title.chars().count() >= 4 && !title.starts_with("http") {
            hints.entry(url).or_insert(title);
        }
    }
    hints
}

fn parse_sogou_exact_links(html: &str, nickname: &str, title: &str) -> Vec<String> {
    let block_regex = match Regex::new(r#"(?is)<li\s+id="sogou_vr_11002601_box_[0-9]+".*?</li>"#) {
        Ok(regex) => regex,
        Err(_) => return Vec::new(),
    };
    let title_regex = Regex::new(r#"(?is)<h3>\s*<a[^>]+href="([^"]+)"[^>]*>(.*?)</a>"#).ok();
    let author_regex = Regex::new(r#"(?is)<span\s+class="all-time-y2">(.*?)</span>"#).ok();
    let mut result = Vec::new();
    for block in block_regex.find_iter(html) {
        let block = block.as_str();
        let author = author_regex
            .as_ref()
            .and_then(|regex| regex.captures(block))
            .and_then(|captures| captures.get(1))
            .map(|value| strip_tags(value.as_str()))
            .unwrap_or_default();
        let Some(captures) = title_regex.as_ref().and_then(|regex| regex.captures(block)) else {
            continue;
        };
        let result_title = captures
            .get(2)
            .map(|value| strip_tags(value.as_str()))
            .unwrap_or_default();
        if author != nickname || normalized_title(&result_title) != normalized_title(title) {
            continue;
        }
        let href = captures
            .get(1)
            .map(|value| decode_wechat_string(value.as_str()))
            .unwrap_or_default();
        if href.starts_with("/link?") {
            result.push(format!("https://weixin.sogou.com{href}"));
        }
    }
    result
}

fn normalized_title(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .replace('“', "\"")
        .replace('”', "\"")
        .replace('‘', "'")
        .replace('’', "'")
}

fn parse_sogou_links(html: &str, nickname: &str) -> Vec<String> {
    let block_regex = match Regex::new(r#"(?is)<li\s+id="sogou_vr_11002601_box_[0-9]+".*?</li>"#) {
        Ok(regex) => regex,
        Err(_) => return Vec::new(),
    };
    let title_regex = Regex::new(r#"(?is)<h3>\s*<a[^>]+href="([^"]+)"[^>]*>.*?</a>"#).ok();
    let author_regex = Regex::new(r#"(?is)<span\s+class="all-time-y2">(.*?)</span>"#).ok();
    let mut result = Vec::new();
    for block in block_regex.find_iter(html) {
        let block = block.as_str();
        let author = author_regex
            .as_ref()
            .and_then(|regex| regex.captures(block))
            .and_then(|captures| captures.get(1))
            .map(|value| strip_tags(value.as_str()))
            .unwrap_or_default();
        if author != nickname {
            continue;
        }
        let href = title_regex
            .as_ref()
            .and_then(|regex| regex.captures(block))
            .and_then(|captures| captures.get(1))
            .map(|value| decode_wechat_string(value.as_str()))
            .unwrap_or_default();
        if href.starts_with("/link?") {
            result.push(format!("https://weixin.sogou.com{href}"));
        }
    }
    result
}

fn resolve_sogou_link(
    client: &Client,
    config: &Config,
    search_url: &str,
    link: &str,
) -> Result<String, String> {
    let html = get_text(client, link, Some(search_url), false, config)?;
    if is_sogou_verification_page(&html) {
        return Err("搜狗链接要求验证码，未尝试绕过".into());
    }
    let regex = Regex::new(r#"url\s*\+=\s*'([^']*)';"#).map_err(|error| error.to_string())?;
    let mut target = String::new();
    for captures in regex.captures_iter(&html) {
        if let Some(part) = captures.get(1) {
            target.push_str(part.as_str());
        }
    }
    if !target.starts_with("https://mp.weixin.qq.com/s?") {
        return Err("搜狗链接没有返回可用的微信文章地址".into());
    }
    Ok(target)
}

fn write_article(
    client: &Client,
    config: &Config,
    parsed: &ParsedArticle,
    raw_html: &str,
) -> Result<WrittenArticle, String> {
    let slug = slugify(&parsed.title);
    let directory_name = format!(
        "{}-{}-{}",
        if parsed.mid.is_empty() {
            "unknown"
        } else {
            &parsed.mid
        },
        if parsed.idx.is_empty() {
            "1"
        } else {
            &parsed.idx
        },
        slug
    );
    let directory = config.output_dir.join("articles").join(directory_name);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut body = parsed.body_html.clone();
    let image_urls = extract_image_urls(&body);
    let mut downloaded = 0usize;
    let mut failed_image_urls = Vec::new();
    if config.download_images && !image_urls.is_empty() {
        let images_dir = directory.join("images");
        fs::create_dir_all(&images_dir).map_err(|error| error.to_string())?;
        for (index, image_url) in image_urls.iter().enumerate() {
            match download_image(client, config, image_url, &images_dir, index) {
                Ok(local) => {
                    body = localize_media_reference(&body, image_url, &local);
                    downloaded += 1;
                }
                Err(error) => failed_image_urls.push(json!({
                    "url": image_url,
                    "error": error
                })),
            }
        }
    } else if !image_urls.is_empty() {
        failed_image_urls.extend(
            image_urls
                .iter()
                .map(|url| json!({"url": url, "error": "media download disabled"})),
        );
    }
    let remaining_remote_media = extract_image_urls(&body);
    let content_text = html_to_text(&body);
    fs::write(directory.join("content.html"), body.as_bytes())
        .map_err(|error| error.to_string())?;
    fs::write(directory.join("content.txt"), content_text.as_bytes())
        .map_err(|error| error.to_string())?;
    let standalone = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{max-width:860px;margin:32px auto;padding:0 20px;font:17px/1.8 system-ui,sans-serif;color:#222}}img{{max-width:100%;height:auto}}.meta{{color:#666;border-bottom:1px solid #ddd;padding-bottom:16px;margin-bottom:24px}}pre{{white-space:pre-wrap}}</style></head><body><h1>{}</h1><div class=\"meta\">公众号：{}<br>作者：{}<br>发布时间：{}<br>原文：<a href=\"{}\">{}</a><br>本地正文：<a href=\"content.html\">HTML</a> · <a href=\"content.txt\">纯文本</a></div>{}</body></html>",
        escape_html(&parsed.title),
        escape_html(&parsed.title),
        escape_html(&parsed.account.nickname),
        escape_html(&parsed.author),
        escape_html(&parsed.publish_time),
        escape_html(&parsed.canonical_url),
        escape_html(&parsed.canonical_url),
        body
    );
    fs::write(directory.join("index.html"), standalone.as_bytes())
        .map_err(|error| error.to_string())?;
    let offline_complete = remaining_remote_media.is_empty()
        && failed_image_urls.is_empty()
        && !content_text.trim().is_empty();
    let metadata = json!({
        "title": parsed.title,
        "author": parsed.author,
        "account": parsed.account,
        "mid": parsed.mid,
        "idx": parsed.idx,
        "sn": parsed.sn,
        "publishTime": parsed.publish_time,
        "canonicalUrl": parsed.canonical_url,
        "albumIds": parsed.album_ids,
        "contentFiles": {
            "standaloneHtml": "index.html",
            "bodyHtml": "content.html",
            "plainText": "content.txt"
        },
        "bodyBytes": body.len(),
        "textBytes": content_text.len(),
        "imageCount": image_urls.len(),
        "downloadedImages": downloaded,
        "failedImages": failed_image_urls,
        "remainingRemoteMedia": remaining_remote_media,
        "offlineComplete": offline_complete
    });
    write_json(&directory.join("metadata.json"), &metadata)?;
    if config.raw_html {
        fs::write(directory.join("raw.html"), raw_html.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    let article_dir = directory
        .strip_prefix(&config.output_dir)
        .unwrap_or(&directory)
        .to_path_buf();
    Ok(WrittenArticle {
        output_path: Some(article_dir.join("index.html").to_string_lossy().to_string()),
        content_html_path: Some(
            article_dir
                .join("content.html")
                .to_string_lossy()
                .to_string(),
        ),
        content_text_path: Some(
            article_dir
                .join("content.txt")
                .to_string_lossy()
                .to_string(),
        ),
        body_bytes: body.len(),
        text_bytes: content_text.len(),
        image_count: image_urls.len(),
        downloaded_images: downloaded,
        failed_images: failed_image_urls.len().max(remaining_remote_media.len()),
        offline_complete,
    })
}

fn download_image(
    client: &Client,
    config: &Config,
    url: &str,
    directory: &Path,
    index: usize,
) -> Result<String, String> {
    if !is_allowed_media_url(url) {
        return Err("media host not allowed".into());
    }
    let mut last_error = String::new();
    for attempt in 1..=3 {
        match client
            .get(url)
            .header(USER_AGENT, DESKTOP_UA)
            .header(REFERER, "https://mp.weixin.qq.com/")
            .send()
            .and_then(Response::error_for_status)
        {
            Ok(response) => {
                let content_type = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                match response.bytes() {
                    Ok(bytes) => {
                        if bytes.is_empty() || !looks_like_image(&bytes, &content_type) {
                            last_error = format!(
                                "media response is not an image: content-type={content_type} bytes={}",
                                bytes.len()
                            );
                            continue;
                        }
                        let extension = image_extension(url, &content_type);
                        let digest = hex_digest(url.as_bytes());
                        let filename = format!("{:04}-{}.{}", index + 1, &digest[..12], extension);
                        fs::write(directory.join(&filename), &bytes)
                            .map_err(|error| error.to_string())?;
                        thread::sleep(Duration::from_millis(config.delay_ms));
                        return Ok(format!("images/{filename}"));
                    }
                    Err(error) => last_error = error.to_string(),
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        if attempt < 3 {
            thread::sleep(Duration::from_millis(config.delay_ms * attempt));
        }
    }
    Err(last_error)
}

fn extract_image_urls(html: &str) -> Vec<String> {
    let attribute_regex = match Regex::new(
        r#"(?is)(?:src|data-src|data-original|data-croporisrc|data-headimg|poster)\s*=\s*["']([^"']+)["']"#,
    ) {
        Ok(regex) => regex,
        Err(_) => return Vec::new(),
    };
    let css_regex = Regex::new(r#"(?is)url\(\s*["']?([^"')]+)["']?\s*\)"#).ok();
    let mut urls = BTreeSet::new();
    for captures in attribute_regex.captures_iter(html) {
        if let Some(value) = captures
            .get(1)
            .and_then(|value| normalize_media_url(value.as_str()))
        {
            urls.insert(value);
        }
    }
    if let Some(css_regex) = css_regex {
        for captures in css_regex.captures_iter(html) {
            if let Some(value) = captures
                .get(1)
                .and_then(|value| normalize_media_url(value.as_str()))
            {
                urls.insert(value);
            }
        }
    }
    urls.into_iter().collect()
}

fn normalize_media_url(value: &str) -> Option<String> {
    let value = decode_wechat_string(value).trim().to_string();
    let value = if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    };
    is_allowed_media_url(&value).then_some(value)
}

fn localize_media_reference(html: &str, remote: &str, local: &str) -> String {
    let mut localized = html.replace(remote, local);
    localized = localized.replace(&remote.replace('&', "&amp;"), local);
    if let Some(protocol_relative) = remote.strip_prefix("https:") {
        localized = localized.replace(protocol_relative, local);
        localized = localized.replace(&protocol_relative.replace('&', "&amp;"), local);
    }
    localized
}

fn html_to_text(html: &str) -> String {
    let without_hidden = Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>")
        .map(|regex| regex.replace_all(html, "").into_owned())
        .unwrap_or_else(|_| html.to_string());
    let with_breaks =
        Regex::new(r"(?is)<br\s*/?>|</(?:p|div|section|article|li|blockquote|h[1-6]|tr)\s*>")
            .map(|regex| regex.replace_all(&without_hidden, "\n").into_owned())
            .unwrap_or(without_hidden);
    let without_tags = Regex::new(r"(?is)<[^>]+>")
        .map(|regex| regex.replace_all(&with_breaks, "").into_owned())
        .unwrap_or(with_breaks);
    let decoded = decode_wechat_string(&without_tags);
    let mut lines = Vec::new();
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() && lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    lines.join("\n\n")
}

fn count_images(html: &str) -> usize {
    extract_image_urls(html).len()
}

fn is_allowed_media_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    ["qpic.cn", "qlogo.cn", "wx.qq.com", "weixin.qq.com"]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

fn looks_like_image(bytes: &[u8], content_type: &str) -> bool {
    if content_type.to_ascii_lowercase().starts_with("image/") {
        return true;
    }
    bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
        || String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
            .to_ascii_lowercase()
            .contains("<svg")
}

fn image_extension(url: &str, content_type: &str) -> &'static str {
    let lower = content_type.to_ascii_lowercase();
    if lower.contains("png") {
        "png"
    } else if lower.contains("gif") {
        "gif"
    } else if lower.contains("webp") {
        "webp"
    } else if lower.contains("svg") {
        "svg"
    } else if url.to_ascii_lowercase().contains("wx_fmt=png") {
        "png"
    } else if url.to_ascii_lowercase().contains("wx_fmt=gif") {
        "gif"
    } else if url.to_ascii_lowercase().contains("wx_fmt=webp") {
        "webp"
    } else {
        "jpg"
    }
}

fn write_index(output_dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let mut html = String::from(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>微信公众号文章归档</title><style>body{max-width:960px;margin:32px auto;padding:0 20px;font:16px/1.7 system-ui,sans-serif}li{margin:.55em 0}.muted{color:#666}</style></head><body>",
    );
    html.push_str(&format!(
        "<h1>{}</h1><p class=\"muted\">biz: {} · 文章 {} 篇 · 专辑 {} 个 · 失败 {} 项</p><ol>",
        escape_html(&manifest.account.nickname),
        escape_html(&manifest.account.biz),
        manifest.stats.articles,
        manifest.stats.albums,
        manifest.stats.failures
    ));
    for article in &manifest.articles {
        if let Some(path) = &article.output_path {
            html.push_str(&format!(
                "<li><a href=\"{}\">{}</a> <span class=\"muted\">mid={} idx={}</span></li>",
                escape_html(path),
                escape_html(&article.title),
                escape_html(&article.mid),
                escape_html(&article.idx)
            ));
        }
    }
    html.push_str("</ol></body></html>");
    fs::write(output_dir.join("index.html"), html.as_bytes()).map_err(|error| error.to_string())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let mut file = fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())
}

fn normalize_article_url(value: &str) -> Option<String> {
    let mut value = decode_wechat_string(value)
        .trim_matches(|character: char| matches!(character, '"' | '\'' | ')' | '(' | ',' | ';'))
        .to_string();
    if value.starts_with("//") {
        value = format!("https:{value}");
    }
    if value.starts_with("http://mp.weixin.qq.com/") {
        value = value.replacen("http://", "https://", 1);
    }
    let mut url = Url::parse(&value).ok()?;
    let host = url.host_str()?;
    if host != "mp.weixin.qq.com" || !(url.path() == "/s" || url.path().starts_with("/s/")) {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn article_url_identity(url: &str) -> String {
    match (article_mid(url), article_idx(url)) {
        (Some(mid), Some(idx)) => format!("{mid}:{idx}"),
        _ => hex_digest(url.as_bytes()),
    }
}

fn article_key(mid: &str, idx: &str, url: &str) -> String {
    if !mid.is_empty() {
        format!("{}:{}", mid, if idx.is_empty() { "1" } else { idx })
    } else {
        hex_digest(url.as_bytes())
    }
}

fn article_query_value(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .ok()?
        .query_pairs()
        .find_map(|(candidate, value)| (candidate == key).then(|| value.into_owned()))
}

fn article_biz(url: &str) -> Option<String> {
    article_query_value(url, "__biz")
}

fn article_mid(url: &str) -> Option<String> {
    article_query_value(url, "mid")
}

fn article_idx(url: &str) -> Option<String> {
    article_query_value(url, "idx")
}

fn canonical_from_parts(biz: &str, mid: &str, idx: &str, sn: &str) -> Option<String> {
    if biz.is_empty() || mid.is_empty() {
        return None;
    }
    let mut url = Url::parse("https://mp.weixin.qq.com/s").ok()?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("__biz", biz);
        query.append_pair("mid", mid);
        query.append_pair("idx", if idx.is_empty() { "1" } else { idx });
        if !sn.is_empty() {
            query.append_pair("sn", sn);
        }
    }
    Some(url.to_string())
}

fn validate_seed_url(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("invalid url: {error}"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("mp.weixin.qq.com")
        || !(url.path() == "/s" || url.path().starts_with("/s/"))
    {
        return Err("url must be an https://mp.weixin.qq.com/s article link".into());
    }
    Ok(())
}

fn capture_first(source: &str, patterns: &[&str]) -> String {
    for pattern in patterns {
        let Ok(regex) = Regex::new(pattern) else {
            continue;
        };
        let Some(captures) = regex.captures(source) else {
            continue;
        };
        let Some(value) = captures.get(1) else {
            continue;
        };
        let value = strip_tags(value.as_str());
        if !value.is_empty() {
            return value;
        }
    }
    String::new()
}

fn strip_tags(value: &str) -> String {
    let without_tags = Regex::new(r"(?is)<[^>]+>")
        .map(|regex| regex.replace_all(value, "").into_owned())
        .unwrap_or_else(|_| value.to_string());
    decode_wechat_string(without_tags.trim())
}

fn decode_wechat_string(value: &str) -> String {
    let mut value = value
        .replace("\\/", "/")
        .replace("\\x26amp;", "&")
        .replace("\\x26", "&")
        .replace("\\u0026", "&")
        .replace("&amp;", "&")
        .replace("\\'", "'")
        .replace("\\\"", "\"");
    value = decode_html_entities(&value).to_string();
    value.trim().to_string()
}

fn unavailable_reason(html: &str) -> Option<&'static str> {
    [
        ("此内容因违规无法查看", "内容因违规无法查看"),
        ("该内容已被发布者删除", "内容已被发布者删除"),
        ("内容已被删除", "内容已被删除"),
        ("此内容已被删除", "内容已被删除"),
        ("文章不存在", "文章不存在"),
        ("页面不存在", "页面不存在"),
        ("已停止访问该网页", "网页已停止访问"),
        ("链接已过期", "链接已过期"),
    ]
    .into_iter()
    .find_map(|(needle, reason)| html.contains(needle).then_some(reason))
}

fn is_verification_page(html: &str) -> bool {
    html.contains("secitptpage/verify")
        || html.contains("请在微信客户端打开链接")
        || html.contains("环境异常")
        || (html.len() < 100_000 && html.contains("验证"))
}

fn is_sogou_verification_page(html: &str) -> bool {
    html.contains("antispider") || html.contains("请输入验证码") || html.contains("验证码")
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in value.chars().take(60) {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "article".into()
    } else {
        slug.into()
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn hex_digest(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cookie_file(path: String) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn string_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn bool_any(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn usize_any(value: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_u64()
                .map(|value| value as usize)
                .or_else(|| value.as_str()?.parse::<usize>().ok())
        })
    })
}

fn u64_any(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
    })
}

fn string_value_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .map(|candidate| value_string(Some(candidate)))
            .filter(|value| !value.is_empty())
    })
}

fn usize_value_any(value: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_u64()
                .map(|value| value as usize)
                .or_else(|| value.as_str()?.parse::<usize>().ok())
        })
    })
}

fn value_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn value_truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64() == Some(1),
        Some(Value::String(value)) => value == "1" || value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_article_media_by_default() {
        let config = Config::from_arguments(&json!({
            "url": "https://mp.weixin.qq.com/s/test"
        }))
        .unwrap();
        assert!(config.download_images);
    }

    #[test]
    fn extracts_and_localizes_all_wechat_image_attributes() {
        let source = r#"<img src="https://mmbiz.qpic.cn/a.jpg?x=1&amp;y=2" data-croporisrc="//mmbiz.qpic.cn/original.png"><mpprofile data-headimg="http://mmbiz.qpic.cn/avatar.png"></mpprofile><div style="background:url('https://mmbiz.qpic.cn/bg.webp')"></div>"#;
        let urls = extract_image_urls(source);
        assert_eq!(urls.len(), 4);
        assert!(
            urls.iter()
                .any(|url| url == "http://mmbiz.qpic.cn/avatar.png")
        );
        let localized = localize_media_reference(
            source,
            "https://mmbiz.qpic.cn/a.jpg?x=1&y=2",
            "images/a.jpg",
        );
        assert!(localized.contains("src=\"images/a.jpg\""));
        assert!(!localized.contains("a.jpg?x=1"));
        let profile_localized = localize_media_reference(
            source,
            "http://mmbiz.qpic.cn/avatar.png",
            "images/profile-avatar.png",
        );
        assert!(profile_localized.contains("data-headimg=\"images/profile-avatar.png\""));
    }

    #[test]
    fn writes_readable_plain_text_from_article_html() {
        let text = html_to_text("<p>第一段&nbsp;文字</p><p><strong>第二段</strong><br>下一行</p>");
        assert!(text.contains("第一段 文字"));
        assert!(text.contains("第二段"));
        assert!(text.contains("下一行"));
        assert!(!text.contains('<'));
    }

    #[test]
    fn album_manifest_counts_include_the_current_article() {
        let mut albums = vec![AlbumRecord {
            id: "album-1".into(),
            title: "Album".into(),
            declared_count: Some(2),
            discovered_articles: 1,
            pages: 1,
        }];
        let members = BTreeMap::from([(
            "album-1".into(),
            BTreeSet::from(["10:1".into(), "11:1".into()]),
        )]);
        apply_album_membership_counts(&mut albums, &members);
        assert_eq!(albums[0].discovered_articles, 2);
    }

    #[test]
    fn normalizes_escaped_wechat_article_url() {
        let url = normalize_article_url(
            "http://mp.weixin.qq.com/s?__biz=abc==\\x26amp;mid=123\\x26amp;idx=1\\x26amp;sn=deadbeef#rd",
        )
        .unwrap();
        assert!(url.starts_with("https://mp.weixin.qq.com/s?"));
        assert_eq!(article_biz(&url).as_deref(), Some("abc=="));
        assert_eq!(article_mid(&url).as_deref(), Some("123"));
    }

    #[test]
    fn extracts_nested_js_content() {
        let source =
            r#"<html><div class="x" id="js_content"><p>A</p><div>B</div></div><div>C</div></html>"#;
        let content = extract_js_content(source).unwrap();
        assert!(content.contains("<p>A</p>"));
        assert!(content.contains("<div>B</div>"));
        assert!(!content.contains("<div>C</div>"));
    }

    #[test]
    fn filters_sogou_results_by_exact_account_name() {
        let source = r#"<li id="sogou_vr_11002601_box_0"><h3><a href="/link?url=ok">T</a></h3><span class="all-time-y2">目标号</span></li><li id="sogou_vr_11002601_box_1"><h3><a href="/link?url=no">T2</a></h3><span class="all-time-y2">别的号</span></li>"#;
        let links = parse_sogou_links(source, "目标号");
        assert_eq!(links, vec!["https://weixin.sogou.com/link?url=ok"]);
    }

    #[test]
    fn extracts_anchor_text_as_article_recovery_hint() {
        let source = r#"<a href="https://mp.weixin.qq.com/s?__biz=test%3D%3D&amp;mid=99&amp;idx=1&amp;sn=x"><strong>精确文章标题</strong></a>"#;
        let hints = extract_article_title_hints(source);
        assert_eq!(hints.len(), 1);
        assert_eq!(
            hints.values().next().map(String::as_str),
            Some("精确文章标题")
        );
    }

    #[test]
    fn filters_sogou_title_recovery_by_exact_account_and_title() {
        let source = r#"<li id="sogou_vr_11002601_box_0"><h3><a href="/link?url=ok">精确文章标题</a></h3><span class="all-time-y2">目标号</span></li><li id="sogou_vr_11002601_box_1"><h3><a href="/link?url=no">其他标题</a></h3><span class="all-time-y2">目标号</span></li>"#;
        assert_eq!(
            parse_sogou_exact_links(source, "目标号", "精确文章标题"),
            vec!["https://weixin.sogou.com/link?url=ok"]
        );
    }

    #[test]
    fn distinguishes_unavailable_content_from_temporary_verification() {
        assert_eq!(
            unavailable_reason("<p>此内容因违规无法查看</p>"),
            Some("内容因违规无法查看")
        );
        assert_eq!(unavailable_reason("secitptpage/verify"), None);
        assert!(is_verification_page("secitptpage/verify"));
    }

    #[test]
    fn retries_a_temporary_verification_page() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for request_number in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request).unwrap();
                let body = if request_number == 0 {
                    "<html>secitptpage/verify</html>"
                } else {
                    r#"<html><script>var biz = "test-biz"; var mid = "42"; var idx = "1";</script><script>window.__mpVideoTransInfo = {title:'Recovered',nick_name:'Test Account'};</script><div id="js_content"><p>body</p></div></html>"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let url = format!("http://{address}/article");
        let config = Config {
            seed_url: url.clone(),
            output_dir: PathBuf::from("unused"),
            search_pages: 0,
            delay_ms: 100,
            article_retries: 2,
            max_articles: 0,
            max_albums: 1,
            download_images: false,
            raw_html: false,
            allow_sogou: false,
            strict: false,
            cookie: None,
        };
        let client = build_client(&config).unwrap();
        let (_, article) = fetch_and_parse_article(&client, &url, "test", &config).unwrap();
        assert_eq!(article.title, "Recovered");
        assert_eq!(article.mid, "42");
        server.join().unwrap();
    }
}
