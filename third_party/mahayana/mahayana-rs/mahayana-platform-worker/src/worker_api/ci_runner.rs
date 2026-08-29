use super::*;
use jsonwebtoken::decode_header;
use jsonwebtoken::jwk::JwkSet;

const GITHUB_ACTIONS_ISSUER: &str = "https://token.actions.githubusercontent.com";
const GITHUB_ACTIONS_JWKS_URL: &str =
    "https://token.actions.githubusercontent.com/.well-known/jwks";
const CI_RUNNER_AUDIENCE: &str = "fabushi-ci-runner";
const CI_REPOSITORY: &str = "bhrumom/fabushi";
const CI_REPOSITORY_ID: &str = "1037709914";
const CI_REPOSITORY_OWNER_ID: &str = "281146136";
const CI_WORKFLOW_REF: &str =
    "bhrumom/fabushi/.github/workflows/interactive-runner-mcp.yml@refs/heads/main";
const CI_PROTECTED_REF: &str = "refs/heads/main";
const CI_RUNNER_TOKEN_SECONDS: i64 = 4 * 60 * 60;
const CI_OIDC_MAX_AGE_SECONDS: i64 = 10 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CiRunnerSessionRequest {
    device_id: String,
    source_sha: String,
}

#[derive(Debug, Deserialize)]
struct GitHubActionsClaims {
    iss: String,
    sub: String,
    aud: String,
    jti: String,
    exp: usize,
    iat: usize,
    repository: String,
    repository_id: String,
    repository_owner_id: String,
    repository_visibility: String,
    actor: String,
    actor_id: String,
    workflow_ref: String,
    workflow_sha: String,
    r#ref: String,
    sha: String,
    run_id: String,
    run_attempt: String,
    event_name: String,
    runner_environment: String,
    ref_protected: Value,
}

#[derive(Debug, Deserialize)]
struct CiIdentityRow {
    user_id: String,
}

fn claim_truthy(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
        || value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn valid_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn claims_allowed(claims: &GitHubActionsClaims, source_sha: &str, now: i64) -> bool {
    let expected_subject = format!("repo:{CI_REPOSITORY}:ref:{CI_PROTECTED_REF}");
    let expected_device = format!("gha-{}-{}-interactive", claims.run_id, claims.run_attempt);
    let issued_at = i64::try_from(claims.iat).unwrap_or(i64::MAX);
    let expires_at = i64::try_from(claims.exp).unwrap_or_default();
    claims.iss == GITHUB_ACTIONS_ISSUER
        && claims.sub == expected_subject
        && claims.aud == CI_RUNNER_AUDIENCE
        && claims.repository == CI_REPOSITORY
        && claims.repository_id == CI_REPOSITORY_ID
        && claims.repository_owner_id == CI_REPOSITORY_OWNER_ID
        && claims.repository_visibility == "public"
        && claims.workflow_ref == CI_WORKFLOW_REF
        && claims.workflow_sha == claims.sha
        && claims.r#ref == CI_PROTECTED_REF
        && claims.sha == source_sha
        && claims.event_name == "workflow_dispatch"
        && claims.runner_environment == "github-hosted"
        && claim_truthy(&claims.ref_protected)
        && valid_sha(&claims.sha)
        && !claims.actor.trim().is_empty()
        && !claims.actor_id.trim().is_empty()
        && claims.actor_id.bytes().all(|byte| byte.is_ascii_digit())
        && !claims.run_id.trim().is_empty()
        && claims.run_id.bytes().all(|byte| byte.is_ascii_digit())
        && !claims.run_attempt.trim().is_empty()
        && claims.run_attempt.bytes().all(|byte| byte.is_ascii_digit())
        && !claims.jti.trim().is_empty()
        && issued_at <= now.saturating_add(60)
        && now.saturating_sub(issued_at) <= CI_OIDC_MAX_AGE_SECONDS
        && expires_at > now
        && expected_device.len() <= 128
}

async fn verify_github_actions_oidc(
    request: &Request,
    source_sha: &str,
    now: i64,
) -> Result<GitHubActionsClaims> {
    let authorization = request
        .headers()
        .get("Authorization")?
        .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| worker::Error::RustError("invalid Authorization scheme".into()))?;
    let header = decode_header(token).map_err(jwt_error)?;
    if header.alg != Algorithm::RS256 {
        return Err(worker::Error::RustError(
            "GitHub Actions OIDC algorithm rejected".into(),
        ));
    }
    let kid = header
        .kid
        .as_deref()
        .ok_or_else(|| worker::Error::RustError("GitHub Actions OIDC key id missing".into()))?;
    let mut key_request = Request::new(GITHUB_ACTIONS_JWKS_URL, Method::Get)?;
    key_request
        .headers_mut()?
        .set("Accept", "application/json")?;
    let mut response = Fetch::Request(key_request).send().await?;
    if !(200..300).contains(&response.status_code()) {
        return Err(worker::Error::RustError(
            "GitHub Actions public keys unavailable".into(),
        ));
    }
    let jwks: JwkSet = response.json().await?;
    let jwk = jwks
        .find(kid)
        .ok_or_else(|| worker::Error::RustError("GitHub Actions OIDC key not found".into()))?;
    let key = DecodingKey::from_jwk(jwk).map_err(jwt_error)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[GITHUB_ACTIONS_ISSUER]);
    validation.set_audience(&[CI_RUNNER_AUDIENCE]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = decode::<GitHubActionsClaims>(token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    if !claims_allowed(&claims, source_sha, now) {
        return Err(worker::Error::RustError(
            "GitHub Actions OIDC trust policy rejected this job".into(),
        ));
    }
    Ok(claims)
}

pub(super) async fn ci_runner_session_create(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let input: CiRunnerSessionRequest = match request.json().await {
        Ok(input) => input,
        Err(_) => {
            return error_response(
                400,
                "invalid_ci_runner_session",
                "CI Runner session request must be valid JSON.",
            );
        }
    };
    if !valid_relay_identifier(&input.device_id, 128)
        || !input.device_id.starts_with("gha-")
        || !input.device_id.ends_with("-interactive")
        || !valid_sha(&input.source_sha)
    {
        return error_response(
            400,
            "invalid_ci_runner_identity",
            "CI Runner device or source identity is invalid.",
        );
    }
    let now = now_seconds();
    let claims = match verify_github_actions_oidc(&request, &input.source_sha, now).await {
        Ok(claims) => claims,
        Err(_) => {
            return error_response(
                401,
                "github_actions_oidc_rejected",
                "Only the protected Fabushi interactive Runner workflow on main may request this session.",
            );
        }
    };
    let expected_device = format!("gha-{}-{}-interactive", claims.run_id, claims.run_attempt);
    if input.device_id != expected_device {
        return error_response(
            403,
            "ci_runner_device_mismatch",
            "CI Runner device id does not match the authenticated workflow run.",
        );
    }

    let account_database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let identity = worker::query!(
        &account_database,
        "SELECT user_id FROM account_identities
         WHERE provider = 'github' AND issuer = 'https://github.com' AND subject = ?1 LIMIT 1",
        &claims.actor_id
    )?
    .first::<CiIdentityRow>(None)
    .await?;
    let Some(identity) = identity else {
        return error_response(
            403,
            "github_account_not_linked",
            "The workflow actor must first sign in to Fabushi with GitHub. Then ChatGPT and this Runner can share that Fabushi account.",
        );
    };
    let Some(user) = lookup_account_user_by_id(&account_database, &identity.user_id).await? else {
        return error_response(
            404,
            "account_missing",
            "The linked Fabushi account no longer exists.",
        );
    };

    let expires_at = now.saturating_add(CI_RUNNER_TOKEN_SECONDS);
    let session_id = format!("ci-runner:{}:{}", claims.run_id, claims.run_attempt);
    let (access_token, access_jti) = issue_scoped_account_access_token(
        &context.env,
        &identity.user_id,
        &input.device_id,
        &session_id,
        now,
        expires_at,
        vec![
            "account.read".to_string(),
            "marketplace.read".to_string(),
            "marketplace.publish".to_string(),
            "wallet.read".to_string(),
            "commerce.purchase".to_string(),
            "model.invoke".to_string(),
            "remote.computer".to_string(),
            "ci.runner".to_string(),
        ],
        "access",
    )?;
    record_auth_event(
        &account_database,
        Some(&identity.user_id),
        None,
        "ci_runner_session_issued",
        now,
    )
    .await?;
    Ok(Response::from_json(&json!({
        "accessToken": access_token,
        "tokenType": "Bearer",
        "expiresIn": CI_RUNNER_TOKEN_SECONDS,
        "accessTokenExpiresAt": expires_at,
        "sessionId": session_id,
        "deviceId": input.device_id,
        "username": user.username,
        "userId": user.id,
        "user": serialize_account_user(&user),
        "provider": "github-actions",
        "ciRunner": true,
        "repository": claims.repository,
        "runId": claims.run_id,
        "runAttempt": claims.run_attempt,
        "actor": claims.actor,
        "accessJti": access_jti,
    }))?
    .with_headers(auth_headers()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(now: i64) -> GitHubActionsClaims {
        GitHubActionsClaims {
            iss: GITHUB_ACTIONS_ISSUER.into(),
            sub: format!("repo:{CI_REPOSITORY}:ref:{CI_PROTECTED_REF}"),
            aud: CI_RUNNER_AUDIENCE.into(),
            jti: "oidc-jti".into(),
            exp: usize::try_from(now + 300).unwrap(),
            iat: usize::try_from(now - 10).unwrap(),
            repository: CI_REPOSITORY.into(),
            repository_id: CI_REPOSITORY_ID.into(),
            repository_owner_id: CI_REPOSITORY_OWNER_ID.into(),
            repository_visibility: "public".into(),
            actor: "linked-user".into(),
            actor_id: "12345".into(),
            workflow_ref: CI_WORKFLOW_REF.into(),
            workflow_sha: "a".repeat(40),
            r#ref: CI_PROTECTED_REF.into(),
            sha: "a".repeat(40),
            run_id: "101".into(),
            run_attempt: "2".into(),
            event_name: "workflow_dispatch".into(),
            runner_environment: "github-hosted".into(),
            ref_protected: Value::Bool(true),
        }
    }

    #[test]
    fn exact_protected_workflow_claims_are_required() {
        let now = 2_000_000_000;
        let valid = claims(now);
        assert!(claims_allowed(&valid, &"a".repeat(40), now));
        let mut fork = claims(now);
        fork.repository_id = "other".into();
        assert!(!claims_allowed(&fork, &"a".repeat(40), now));
        let mut stale = claims(now);
        stale.iat = usize::try_from(now - CI_OIDC_MAX_AGE_SECONDS - 1).unwrap();
        assert!(!claims_allowed(&stale, &"a".repeat(40), now));
        let mut branch = claims(now);
        branch.r#ref = "refs/heads/feature".into();
        assert!(!claims_allowed(&branch, &"a".repeat(40), now));
    }
}
