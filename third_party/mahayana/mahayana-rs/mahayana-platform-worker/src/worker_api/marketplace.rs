use super::*;

pub(super) async fn marketplace_plugins(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let mut query = "%".to_string();
    let mut platform = None;
    for (key, value) in request.url()?.query_pairs() {
        if key == "q" && !value.trim().is_empty() {
            query = format!("%{}%", value.trim());
        } else if key == "platform" && !value.trim().is_empty() {
            let value = value.trim().to_string();
            if !matches!(value.as_str(), "cli" | "desktop" | "mobile" | "web") {
                return error_response(
                    400,
                    "invalid_marketplace_platform",
                    "platform must be cli, desktop, mobile, or web.",
                );
            }
            platform = Some(value);
        }
    }
    let platform_pattern = platform
        .as_deref()
        .map(|value| format!("%\"{value}\"%"))
        .unwrap_or_else(|| "%".to_string());
    let database = context.env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT mp.plugin_id, mp.display_name, mp.description, mp.latest_version,
                pr.package_sha256, pr.package_size, mp.platforms_json,
                pr.deployment_url, pr.published_at, pr.source_json,
                pr.release_manifest_json, pr.release_manifest_sha256, pr.release_status
         FROM marketplace_plugins mp
         JOIN plugin_releases pr
           ON pr.plugin_id = mp.plugin_id AND pr.version = mp.latest_version
         WHERE mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.release_status = 'approved'
           AND pr.deployment_url <> ''
           AND (mp.display_name LIKE ?1 OR mp.description LIKE ?1 OR mp.plugin_id LIKE ?1)
           AND mp.platforms_json LIKE ?2
         ORDER BY mp.updated_at DESC LIMIT 100",
        &query,
        &platform_pattern
    )?
    .all()
    .await?
    .results::<MarketplacePluginRow>()?;
    let plugins = rows
        .into_iter()
        .map(|row| {
            let platforms =
                serde_json::from_str::<Vec<String>>(&row.platforms_json).unwrap_or_default();
            let source = serde_json::from_str::<Value>(&row.source_json).unwrap_or(Value::Null);
            let release_manifest =
                serde_json::from_str::<Value>(&row.release_manifest_json).unwrap_or(Value::Null);
            json!({
                "pluginId": row.plugin_id,
                "displayName": row.display_name,
                "description": row.description,
                "latestVersion": row.latest_version,
                "packageSha256": row.package_sha256,
                "packageSize": row.package_size.and_then(exact_nonnegative_i64),
                "platforms": platforms,
                "deploymentUrl": row.deployment_url,
                "publishedAt": row.published_at.and_then(exact_nonnegative_i64),
                "source": source,
                "releaseManifest": release_manifest,
                "releaseManifestSha256": row.release_manifest_sha256,
                "releaseStatus": row.release_status,
            })
        })
        .collect::<Vec<_>>();
    Response::from_json(&json!({"plugins": plugins}))
}

fn marketplace_installed_projection(row: &MarketplaceInstalledPluginRow) -> Value {
    let mut projection = row
        .projection_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| {
            json!({
                "id": row.plugin_id,
                "title": row.display_name,
                "description": row.description,
            })
        });
    if let Some(object) = projection.as_object_mut() {
        object.insert("id".into(), Value::String(row.plugin_id.clone()));
        object.insert("pluginId".into(), Value::String(row.plugin_id.clone()));
        object.insert("title".into(), Value::String(row.display_name.clone()));
        object.insert("description".into(), Value::String(row.description.clone()));
        if let Some(version) = row.latest_version.as_ref() {
            object.insert("version".into(), Value::String(version.clone()));
        }
    }
    projection
}

pub(super) async fn marketplace_added(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to read installed marketplace apps.",
            );
        }
    };
    let database = context.env.d1(DATABASE_BINDING)?;
    let rows = worker::query!(
        &database,
        "SELECT mp.plugin_id, mp.display_name, mp.description, mp.latest_version,
                mpp.projection_json
         FROM account_marketplace_installs ami
         JOIN marketplace_plugins mp ON mp.plugin_id = ami.plugin_id
         JOIN plugin_releases pr
           ON pr.plugin_id = mp.plugin_id AND pr.version = mp.latest_version
         LEFT JOIN marketplace_plugin_projections mpp ON mpp.plugin_id = mp.plugin_id
         WHERE ami.account_user_id = ?1
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.release_status = 'approved' AND pr.deployment_url <> ''
         ORDER BY ami.updated_at DESC, mp.plugin_id ASC",
        &account.user_id
    )?
    .all()
    .await?
    .results::<MarketplaceInstalledPluginRow>()?;
    let apps = rows
        .iter()
        .map(marketplace_installed_projection)
        .collect::<Vec<_>>();
    Response::from_json(&json!({
        "protocol": "fabushi.miniapp.marketplace.v2",
        "accountSynchronized": true,
        "apps": apps,
    }))
}

pub(super) async fn marketplace_plugin_add(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to install marketplace apps.",
            );
        }
    };
    let plugin_id = route_identifier(&context, "plugin_id")?.to_string();
    let body = request.json::<Value>().await.unwrap_or(Value::Null);
    let platform = body
        .get("platform")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    if platform.len() > 64
        || !platform
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return error_response(
            400,
            "invalid_marketplace_platform",
            "platform must be a normalized identifier.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT mp.plugin_id, mp.display_name, mp.description, mp.latest_version,
                mpp.projection_json
         FROM marketplace_plugins mp
         JOIN plugin_releases pr
           ON pr.plugin_id = mp.plugin_id AND pr.version = mp.latest_version
         LEFT JOIN marketplace_plugin_projections mpp ON mpp.plugin_id = mp.plugin_id
         WHERE mp.plugin_id = ?1
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.release_status = 'approved' AND pr.deployment_url <> ''",
        &plugin_id
    )?
    .first::<MarketplaceInstalledPluginRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(
            404,
            "marketplace_plugin_not_found",
            "The approved marketplace plugin does not exist.",
        );
    };
    let now = now_seconds();
    worker::query!(
        &database,
        "INSERT INTO account_marketplace_installs
         (account_user_id, plugin_id, platform, installed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(account_user_id, plugin_id) DO UPDATE SET
           platform = excluded.platform,
           updated_at = excluded.updated_at",
        &account.user_id,
        &plugin_id,
        platform,
        now
    )?
    .run()
    .await?;
    let app = marketplace_installed_projection(&row);
    Response::from_json(&json!({
        "added": true,
        "accountSynchronized": true,
        "pluginId": plugin_id,
        "bot": app.get("bot").cloned().unwrap_or(Value::Null),
        "app": app,
    }))
}

pub(super) async fn marketplace_release_publish(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    const MAX_PACKAGE_BYTES: usize = 50 * 1024 * 1024;

    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to publish marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_publish_forbidden",
            "The account token does not permit marketplace publishing.",
        );
    }

    let form = request.form_data().await?;
    let field = |name: &str| -> Result<String> {
        match form.get(name) {
            Some(FormEntry::Field(value)) if !value.trim().is_empty() => Ok(value),
            _ => Err(worker::Error::RustError(format!(
                "marketplace release field {name} is required"
            ))),
        }
    };
    let plugin_id = field("pluginId")?;
    let version = field("version")?;
    if !is_identifier(&plugin_id) || !is_version_identifier(&version) {
        return error_response(
            400,
            "invalid_marketplace_release_identifier",
            "pluginId and version must be normalized identifiers.",
        );
    }
    let database = context.env.d1(DATABASE_BINDING)?;
    let existing_plugin = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?;
    if let Some(existing_plugin) = existing_plugin {
        if existing_plugin.publisher_user_id != account.user_id {
            return error_response(
                403,
                "marketplace_plugin_owner_mismatch",
                "The authenticated publisher does not own this plugin ID.",
            );
        }
    }
    let existing_release = worker::query!(
        &database,
        "SELECT package_sha256 FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceExistingReleaseRow>(None)
    .await?;
    if let Some(existing_release) = existing_release {
        return error_response(
            409,
            "version_already_exists",
            &format!(
                "Release {plugin_id}@{version} is immutable and already exists with package SHA-256 {}.",
                existing_release.package_sha256
            ),
        );
    }
    let deployment_url = field("deploymentUrl")?;
    if !is_public_https_url(&deployment_url) {
        return error_response(
            400,
            "invalid_marketplace_deployment_url",
            "deploymentUrl must be a public HTTPS URL.",
        );
    }
    let expected_sha256 = field("packageSha256")?.to_ascii_lowercase();
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return error_response(
            400,
            "invalid_marketplace_package_sha256",
            "packageSha256 must be a 64-character hexadecimal digest.",
        );
    }
    let expected_size = field("packageSize")?
        .parse::<usize>()
        .map_err(|_| worker::Error::RustError("invalid packageSize".into()))?;
    if expected_size == 0 || expected_size > MAX_PACKAGE_BYTES {
        return error_response(
            400,
            "invalid_marketplace_package_size",
            "packageSize must be between 1 byte and 50 MiB.",
        );
    }
    let platforms = serde_json::from_str::<Vec<String>>(&field("platforms")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace platforms".into()))?;
    if platforms.is_empty()
        || platforms
            .iter()
            .any(|platform| !matches!(platform.as_str(), "cli" | "desktop" | "mobile" | "web"))
    {
        return error_response(
            400,
            "invalid_marketplace_platforms",
            "platforms must contain supported Mahayana targets.",
        );
    }
    let source = serde_json::from_str::<Value>(&field("source")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace source".into()))?;
    if let Err(message) = validate_github_source_identity(&source) {
        return error_response(400, "invalid_marketplace_source", &message);
    }
    let release_manifest = serde_json::from_str::<Value>(&field("releaseManifest")?)
        .map_err(|_| worker::Error::RustError("invalid marketplace release manifest".into()))?;
    if let Err(message) = validate_multi_artifact_release_manifest(
        &release_manifest,
        &plugin_id,
        &version,
        &expected_sha256,
        expected_size,
        &platforms,
        &source,
    ) {
        return error_response(400, "invalid_marketplace_release_manifest", &message);
    }
    let source_json = serde_json::to_string(&source)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_bytes = canonical_json_bytes(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_json = String::from_utf8(release_manifest_bytes)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_sha256 = canonical_json_sha256(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let package = match form.get("package") {
        Some(FormEntry::File(file)) => file.bytes().await?,
        _ => {
            return error_response(
                400,
                "marketplace_package_missing",
                "The release package file is required.",
            );
        }
    };
    if package.len() != expected_size {
        return error_response(
            400,
            "marketplace_package_size_mismatch",
            "The uploaded package size does not match release metadata.",
        );
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&package));
    if actual_sha256 != expected_sha256 {
        return error_response(
            400,
            "marketplace_package_sha256_mismatch",
            "The uploaded package digest does not match release metadata.",
        );
    }

    let remote_package = match verified_marketplace_site_package_with_retry(
        &deployment_url,
        &plugin_id,
        &version,
        &actual_sha256,
        expected_size,
        &source,
        &release_manifest,
        &release_manifest_sha256,
    )
    .await
    {
        Ok(package) => package,
        Err(message) => {
            return error_response(400, "marketplace_deployment_verification_failed", &message);
        }
    };
    if remote_package != package {
        return error_response(
            400,
            "marketplace_deployment_package_mismatch",
            "The package served by the Cloudflare plugin site differs from the uploaded release package.",
        );
    }
    let package_key = marketplace_asset_url(&deployment_url, "/mahayana/plugin.tar.gz")?;

    let now = now_seconds();
    let package_size = i64::try_from(expected_size)
        .map_err(|_| worker::Error::RustError("packageSize exceeds D1 integer range".into()))?;
    let platforms_json = serde_json::to_string(&platforms)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_status = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };
    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO marketplace_plugins
                 (plugin_id, display_name, description, publisher_user_id, latest_version,
                  visibility, review_state, created_at, updated_at, platforms_json)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)
                 ON CONFLICT(plugin_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   description = excluded.description,
                   latest_version = excluded.latest_version,
                   visibility = excluded.visibility,
                   review_state = excluded.review_state,
                   updated_at = excluded.updated_at,
                   platforms_json = excluded.platforms_json",
                &plugin_id,
                &format!("Published from {deployment_url}"),
                &account.user_id,
                &version,
                if account.is_test_account {
                    "public"
                } else {
                    "unlisted"
                },
                if account.is_test_account {
                    "approved"
                } else {
                    "pending"
                },
                now,
                &platforms_json
            )?,
            worker::query!(
                &database,
                "INSERT INTO plugin_releases
                 (plugin_id, version, package_key, package_sha256, package_size,
                  tuf_target_path, published_at, deployment_url, source_json,
                  release_manifest_json, release_manifest_sha256, release_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &plugin_id,
                &version,
                &package_key,
                &actual_sha256,
                package_size,
                &format!("plugins/{plugin_id}/{version}.tar.gz"),
                now,
                &deployment_url,
                &source_json,
                &release_manifest_json,
                &release_manifest_sha256,
                release_status
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "published": true,
        "approved": account.is_test_account,
        "pluginId": plugin_id,
        "version": version,
        "deploymentUrl": deployment_url,
        "packageSha256": actual_sha256,
        "packageSize": package_size,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": release_manifest_sha256,
        "releaseStatus": release_status,
    }))
}

pub(super) async fn marketplace_external_release_publish(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    const MAX_ARTIFACT_BYTES: usize = 100 * 1024 * 1024;

    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to publish marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_publish_forbidden",
            "The account token does not permit marketplace publishing.",
        );
    }

    let body: Value = match request.json().await {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                400,
                "invalid_marketplace_release",
                "The external release request must be valid JSON.",
            );
        }
    };
    let plugin_id = match body.get("pluginId").and_then(Value::as_str) {
        Some(value) if is_identifier(value) => value.to_string(),
        _ => {
            return error_response(
                400,
                "invalid_marketplace_release_identifier",
                "pluginId must be a normalized identifier.",
            );
        }
    };
    let version = match body.get("version").and_then(Value::as_str) {
        Some(value) if is_version_identifier(value) => value.to_string(),
        _ => {
            return error_response(
                400,
                "invalid_marketplace_release_identifier",
                "version must be a normalized version identifier.",
            );
        }
    };
    let display_name = body
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 120)
        .unwrap_or(&plugin_id)
        .to_string();
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.len() <= 2_000)
        .unwrap_or("")
        .to_string();
    let platforms = match body.get("platforms").and_then(Value::as_array) {
        Some(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };
    if platforms.is_empty()
        || platforms.iter().any(|platform| {
            !matches!(
                platform.as_str(),
                "cli" | "desktop" | "mobile" | "web" | "ios" | "android"
            )
        })
    {
        return error_response(
            400,
            "invalid_marketplace_platforms",
            "platforms must contain supported Mahayana targets.",
        );
    }

    let release_manifest = match body.get("releaseManifest") {
        Some(value) => value.clone(),
        None => {
            return error_response(
                400,
                "invalid_marketplace_release_manifest",
                "releaseManifest is required.",
            );
        }
    };
    if let Err(message) = validate_external_release_manifest(
        &release_manifest,
        &plugin_id,
        &version,
        &platforms,
        MAX_ARTIFACT_BYTES,
    ) {
        return error_response(400, "invalid_marketplace_release_manifest", &message);
    }

    let database = context.env.d1(DATABASE_BINDING)?;
    if let Some(existing) = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?
    {
        if existing.publisher_user_id != account.user_id {
            return error_response(
                403,
                "marketplace_plugin_owner_mismatch",
                "The authenticated publisher does not own this plugin ID.",
            );
        }
    }
    if let Some(existing) = worker::query!(
        &database,
        "SELECT package_sha256 FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceExistingReleaseRow>(None)
    .await?
    {
        return error_response(
            409,
            "version_already_exists",
            &format!(
                "Release {plugin_id}@{version} is immutable and already exists with package SHA-256 {}.",
                existing.package_sha256
            ),
        );
    }

    // Admission verifies every external runtime artifact at its publisher URL.
    // The bytes are never persisted by the marketplace.
    let artifacts = release_manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .expect("validated release manifest artifacts");
    let mut resolved_artifacts = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let resolved_url = match resolve_external_artifact_url(artifact).await {
            Ok(url) => url,
            Err(message) => {
                return error_response(400, "marketplace_artifact_resolution_failed", &message);
            }
        };
        let expected_size = artifact
            .get("size")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .expect("validated artifact size");
        let expected_sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .expect("validated artifact sha256");
        if let Err(message) = verify_external_artifact(
            &resolved_url,
            expected_sha256,
            expected_size,
            MAX_ARTIFACT_BYTES,
        )
        .await
        {
            return error_response(400, "marketplace_artifact_verification_failed", &message);
        }
        resolved_artifacts.push(resolved_url);
    }

    let primary = artifacts.first().expect("validated non-empty artifacts");
    let package_sha256 = primary
        .get("sha256")
        .and_then(Value::as_str)
        .expect("validated primary sha")
        .to_ascii_lowercase();
    let package_size = primary
        .get("size")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok())
        .expect("validated primary size");
    let primary_url = resolved_artifacts
        .first()
        .expect("validated primary URL")
        .clone();
    let release_manifest_json = serde_json::to_string(&release_manifest)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let release_manifest_sha256 = format!("{:x}", Sha256::digest(release_manifest_json.as_bytes()));
    let source = body
        .get("source")
        .cloned()
        .unwrap_or_else(|| json!({"provider":"external","artifact": primary.get("source").cloned().unwrap_or(Value::Null)}));
    let source_json = serde_json::to_string(&source)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let platforms_json = serde_json::to_string(&platforms)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let now = now_seconds();
    let release_status = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };
    let visibility = if account.is_test_account {
        "public"
    } else {
        "unlisted"
    };
    let review_state = if account.is_test_account {
        "approved"
    } else {
        "pending"
    };

    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO marketplace_plugins
                 (plugin_id, display_name, description, publisher_user_id, latest_version,
                  visibility, review_state, created_at, updated_at, platforms_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
                 ON CONFLICT(plugin_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   description = excluded.description,
                   latest_version = excluded.latest_version,
                   visibility = excluded.visibility,
                   review_state = excluded.review_state,
                   updated_at = excluded.updated_at,
                   platforms_json = excluded.platforms_json",
                &plugin_id,
                &display_name,
                &description,
                &account.user_id,
                &version,
                visibility,
                review_state,
                now,
                &platforms_json
            )?,
            worker::query!(
                &database,
                "INSERT INTO plugin_releases
                 (plugin_id, version, package_key, package_sha256, package_size,
                  tuf_target_path, published_at, deployment_url, source_json,
                  release_manifest_json, release_manifest_sha256, release_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &plugin_id,
                &version,
                &primary_url,
                &package_sha256,
                package_size,
                &format!("external/{plugin_id}/{version}"),
                now,
                &primary_url,
                &source_json,
                &release_manifest_json,
                &release_manifest_sha256,
                release_status
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "published": true,
        "approved": account.is_test_account,
        "storage": "external",
        "pluginId": plugin_id,
        "version": version,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": release_manifest_sha256,
        "resolvedArtifacts": resolved_artifacts,
        "releaseStatus": release_status,
    }))
}

pub(super) async fn marketplace_release_metadata(
    _request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT pr.plugin_id, pr.version, pr.package_sha256, pr.package_size,
                pr.deployment_url, pr.published_at, mp.platforms_json,
                pr.source_json, pr.release_manifest_json, pr.release_manifest_sha256,
                pr.release_status, pr.revoked_at, pr.revocation_reason
         FROM plugin_releases pr
         JOIN marketplace_plugins mp ON mp.plugin_id = pr.plugin_id
         WHERE pr.plugin_id = ?1 AND pr.version = ?2
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.deployment_url <> ''",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseMetadataRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    };
    if row.release_status == "revoked" {
        let revoked_at = row.revoked_at.and_then(exact_nonnegative_i64);
        let reason = row
            .revocation_reason
            .as_deref()
            .unwrap_or("publisher_request");
        return error_response(
            410,
            "release_revoked",
            &format!("Release {plugin_id}@{version} was revoked at {revoked_at:?}: {reason}."),
        );
    }
    if row.release_status != "approved" {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    }
    let platforms = serde_json::from_str::<Vec<String>>(&row.platforms_json).unwrap_or_default();
    let source = serde_json::from_str::<Value>(&row.source_json).unwrap_or(Value::Null);
    let release_manifest =
        serde_json::from_str::<Value>(&row.release_manifest_json).unwrap_or(Value::Null);
    let Some(package_size) = exact_nonnegative_i64(row.package_size) else {
        return error_response(
            503,
            "marketplace_package_size_invalid",
            "The approved release has an invalid package size.",
        );
    };
    let Some(published_at) = exact_nonnegative_i64(row.published_at) else {
        return error_response(
            503,
            "marketplace_published_at_invalid",
            "The approved release has an invalid published timestamp.",
        );
    };
    Response::from_json(&json!({
        "pluginId": row.plugin_id,
        "version": row.version,
        "packageSha256": row.package_sha256,
        "packageSize": package_size,
        "deploymentUrl": row.deployment_url,
        "publishedAt": published_at,
        "platforms": platforms,
        "source": source,
        "releaseManifest": release_manifest,
        "releaseManifestSha256": row.release_manifest_sha256,
        "releaseStatus": row.release_status,
    }))
}

pub(super) async fn marketplace_plugin_download(
    _request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let release = worker::query!(
        &database,
        "SELECT pr.deployment_url, pr.package_key, pr.package_sha256, pr.package_size,
                pr.release_status, pr.revoked_at, pr.revocation_reason
         FROM plugin_releases pr
         JOIN marketplace_plugins mp ON mp.plugin_id = pr.plugin_id
         WHERE pr.plugin_id = ?1 AND pr.version = ?2
           AND mp.visibility = 'public' AND mp.review_state = 'approved'
           AND pr.deployment_url <> ''",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseDownloadRow>(None)
    .await?;
    let Some(release) = release else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    };
    if release.release_status == "revoked" {
        let revoked_at = release.revoked_at.and_then(exact_nonnegative_i64);
        let reason = release
            .revocation_reason
            .as_deref()
            .unwrap_or("publisher_request");
        return error_response(
            410,
            "release_revoked",
            &format!("Release {plugin_id}@{version} was revoked at {revoked_at:?}: {reason}."),
        );
    }
    if release.release_status != "approved" {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The approved plugin release does not exist.",
        );
    }

    if !is_public_https_url(&release.deployment_url) {
        return error_response(
            503,
            "marketplace_deployment_url_invalid",
            "The approved release does not point to a valid Cloudflare Pages/Worker site.",
        );
    }
    let package_size = match exact_nonnegative_i64(release.package_size)
        .and_then(|size| usize::try_from(size).ok())
    {
        Some(size) if size > 0 && size <= 50 * 1024 * 1024 => size,
        _ => {
            return error_response(
                503,
                "marketplace_package_size_invalid",
                "The approved release has an invalid package size.",
            );
        }
    };
    // The marketplace is a metadata/control plane, not a binary CDN. Package
    // bytes stay on the publisher's immutable GitHub/npm/HTTPS origin. Older
    // releases already store their externally served package URL in
    // `package_key`, so redirecting is backward compatible while removing the
    // Worker from the plugin data path. Clients MUST continue validating the
    // catalogued size and SHA-256 after following this redirect.
    let package_url = Url::parse(&release.package_key).map_err(|_| {
        worker::Error::RustError("marketplace release package URL is invalid".into())
    })?;
    if package_url.scheme() != "https" || package_url.host_str().is_none() {
        return error_response(
            503,
            "marketplace_package_url_invalid",
            "The approved release does not point to a public HTTPS artifact.",
        );
    }
    let mut response = Response::redirect_with_status(package_url, 307)?;
    response
        .headers_mut()
        .set("X-Mahayana-Package-Sha256", &release.package_sha256)?;
    response
        .headers_mut()
        .set("X-Mahayana-Package-Size", &package_size.to_string())?;
    response
        .headers_mut()
        .set("Cache-Control", "public, max-age=300")?;
    Ok(response)
}

pub(super) async fn marketplace_release_revoke(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => {
            return error_response(
                401,
                "unauthorized",
                "A valid Mahayana account token is required to revoke marketplace releases.",
            );
        }
    };
    if !account.is_test_account
        && !account
            .scopes
            .iter()
            .any(|scope| scope == "marketplace.publish")
    {
        return error_response(
            403,
            "marketplace_revoke_forbidden",
            "The account token does not permit marketplace release revocation.",
        );
    }

    let plugin_id = route_identifier(&context, "plugin_id")?;
    let version = route_version(&context)?;
    let database = context.env.d1(DATABASE_BINDING)?;
    let owner = worker::query!(
        &database,
        "SELECT publisher_user_id FROM marketplace_plugins WHERE plugin_id = ?1",
        &plugin_id
    )?
    .first::<MarketplacePluginOwnerRow>(None)
    .await?;
    let Some(owner) = owner else {
        return error_response(
            404,
            "marketplace_plugin_not_found",
            "The marketplace plugin does not exist.",
        );
    };
    if owner.publisher_user_id != account.user_id {
        return error_response(
            403,
            "marketplace_plugin_owner_mismatch",
            "The authenticated publisher does not own this plugin ID.",
        );
    }

    let release = worker::query!(
        &database,
        "SELECT release_status, revoked_at, revocation_reason
         FROM plugin_releases WHERE plugin_id = ?1 AND version = ?2",
        &plugin_id,
        &version
    )?
    .first::<MarketplaceReleaseStatusRow>(None)
    .await?;
    let Some(release) = release else {
        return error_response(
            404,
            "marketplace_release_not_found",
            "The marketplace release does not exist.",
        );
    };
    if release.release_status == "revoked" {
        return Response::from_json(&json!({
            "revoked": true,
            "pluginId": plugin_id,
            "version": version,
            "releaseStatus": release.release_status,
            "revokedAt": release.revoked_at.and_then(exact_nonnegative_i64),
            "reason": release.revocation_reason,
        }));
    }

    let body = request.json::<Value>().await.unwrap_or_else(|_| json!({}));
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("publisher_request")
        .trim()
        .to_string();
    if reason.is_empty() || reason.len() > 256 || reason.chars().any(char::is_control) {
        return error_response(
            400,
            "invalid_revocation_reason",
            "Revocation reason must contain 1 to 256 printable characters.",
        );
    }

    let now = now_seconds();
    database
        .batch(vec![
            worker::query!(
                &database,
                "UPDATE plugin_releases
                 SET release_status = 'revoked', revoked_at = ?1, revocation_reason = ?2
                 WHERE plugin_id = ?3 AND version = ?4",
                now,
                &reason,
                &plugin_id,
                &version
            )?,
            worker::query!(
                &database,
                "UPDATE marketplace_plugins
                 SET latest_version = (
                     SELECT version FROM plugin_releases
                     WHERE plugin_id = ?2 AND release_status = 'approved'
                     ORDER BY published_at DESC LIMIT 1
                 ), updated_at = ?1
                 WHERE plugin_id = ?2 AND latest_version = ?3",
                now,
                &plugin_id,
                &version
            )?,
        ])
        .await?;

    Response::from_json(&json!({
        "revoked": true,
        "pluginId": plugin_id,
        "version": version,
        "releaseStatus": "revoked",
        "revokedAt": now,
        "reason": reason,
    }))
}
