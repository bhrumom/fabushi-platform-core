use super::*;

pub(super) async fn password_session_value(
    env: &Env,
    login: &PasswordLoginRequest,
) -> Result<std::result::Result<Value, PasswordAuthRejection>> {
    let identifier = login.username.trim();
    if identifier.is_empty() || login.password.is_empty() || login.password.len() > 1024 {
        return Ok(Err(PasswordAuthRejection {
            status: 400,
            code: "invalid_login_request",
            message: "用户名或邮箱、手机号和密码不能为空",
        }));
    }
    let device_id = match normalize_device_id(login.device_id.as_deref()) {
        Ok(device_id) => device_id,
        Err(_) => {
            return Ok(Err(PasswordAuthRejection {
                status: 400,
                code: "invalid_device_id",
                message: "invalid device id",
            }));
        }
    };
    let database = env.d1(ACCOUNT_DATABASE_BINDING)?;
    let user = lookup_login_user(&database, identifier).await?;
    let Some(user) = user else {
        return Ok(Err(PasswordAuthRejection {
            status: 401,
            code: "invalid_credentials",
            message: "账号或密码错误",
        }));
    };
    let now = now_seconds();
    if account_login_is_rate_limited(&database, &user.id.to_string(), now).await? {
        return Ok(Err(PasswordAuthRejection {
            status: 429,
            code: "login_rate_limited",
            message: "登录尝试过多，请稍后再试",
        }));
    }
    let password_valid = if let Some(upgraded) = user.upgraded_password_phc.as_deref() {
        verify_argon2id(&login.password, upgraded)
    } else {
        let Some(password_hash) = user.password_hash.as_deref() else {
            return Ok(Err(PasswordAuthRejection {
                status: 401,
                code: "password_not_configured",
                message: "当前账号尚未设置密码",
            }));
        };
        let Some(salt) = user.salt.as_deref() else {
            return Ok(Err(PasswordAuthRejection {
                status: 401,
                code: "password_not_configured",
                message: "当前账号尚未设置密码",
            }));
        };
        verify_pbkdf2_sha256(
            &login.password,
            salt,
            password_hash,
            user.iterations,
            user.algo.as_deref(),
        )
        .unwrap_or(false)
    };
    if !password_valid {
        record_auth_event(
            &database,
            Some(&user.id.to_string()),
            None,
            "login_failed",
            now_seconds(),
        )
        .await?;
        return Ok(Err(PasswordAuthRejection {
            status: 401,
            code: "invalid_credentials",
            message: "账号或密码错误",
        }));
    }
    if user.upgraded_password_phc.is_none() {
        let upgraded = hash_password_argon2id(&login.password, &new_password_salt())
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        worker::query!(
            &database,
            "INSERT OR IGNORE INTO account_password_credentials
             (user_id, password_phc, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            user.id.to_string(),
            upgraded,
            now
        )?
        .run()
        .await?;
    }
    let session =
        create_account_session_value(&database, env, &user, &device_id, "login_succeeded").await?;
    Ok(Ok(session))
}

pub(super) async fn password_login(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let login: PasswordLoginRequest = match request.json().await {
        Ok(login) => login,
        Err(_) => {
            return error_response(
                400,
                "invalid_login_request",
                "用户名或邮箱、手机号和密码不能为空",
            );
        }
    };
    match password_session_value(&context.env, &login).await? {
        Ok(session) => Ok(Response::from_json(&session)?.with_headers(auth_headers())),
        Err(rejection) => error_response(rejection.status, rejection.code, rejection.message),
    }
}

fn browser_ticket_hash(ticket: &str) -> String {
    format!("{:x}", Sha256::digest(ticket.as_bytes()))
}

fn browser_poll_secret(env: &Env, attempt_id: &str) -> Result<String> {
    let key = env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let material = format!("fabushi-browser-poll:v1:{attempt_id}:{key}");
    Ok(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(material.as_bytes())),
    )
}

fn constant_time_text_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn auth_public_base_url(env: &Env) -> Result<String> {
    Ok(env
        .var("AUTH_PUBLIC_BASE_URL")?
        .to_string()
        .trim_end_matches('/')
        .to_string())
}

fn browser_portal_url(env: &Env, attempt_id: &str, ticket: &str) -> Result<Url> {
    let mut url = Url::parse(&format!(
        "{}/api/auth/browser/portal",
        auth_public_base_url(env)?
    ))
    .map_err(|error| worker::Error::RustError(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("attemptId", attempt_id)
        .append_pair("ticket", ticket);
    Ok(url)
}

fn browser_authorize_url(env: &Env, attempt_id: &str, ticket: &str, provider: &str) -> Result<Url> {
    let mut url = Url::parse(&format!(
        "{}/api/auth/browser/authorize",
        auth_public_base_url(env)?
    ))
    .map_err(|error| worker::Error::RustError(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("attemptId", attempt_id)
        .append_pair("ticket", ticket)
        .append_pair("provider", provider);
    Ok(url)
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn browser_html_response(html: String) -> Result<Response> {
    let mut response = Response::from_html(html)?;
    let headers = response.headers_mut();
    headers.set("Cache-Control", "no-store")?;
    headers.set("Pragma", "no-cache")?;
    headers.set("Referrer-Policy", "no-referrer")?;
    headers.set("X-Frame-Options", "DENY")?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    headers.set(
        "Content-Security-Policy",
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    )?;
    Ok(response)
}

pub(super) async fn browser_attempt_for_ticket(
    database: &worker::D1Database,
    attempt_id: &str,
    ticket: &str,
) -> Result<Option<BrowserAttemptRow>> {
    if attempt_id.len() > 80 || ticket.len() > 160 || ticket.is_empty() {
        return Ok(None);
    }
    let row = worker::query!(
        database,
        "SELECT attempt_id, provider, device_id, code_verifier, state_hash, status, expires_at
         FROM account_oauth_attempts WHERE attempt_id = ?1 LIMIT 1",
        attempt_id
    )?
    .first::<BrowserAttemptRow>(None)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.provider != "portal"
        || row.status != "pending"
        || row.expires_at <= now_seconds()
        || row.state_hash != browser_ticket_hash(ticket)
    {
        return Ok(None);
    }
    Ok(Some(row))
}

pub(super) async fn browser_provider_buttons(
    env: &Env,
    attempt_id: &str,
    ticket: &str,
    mode: &str,
) -> Result<String> {
    let mut output = String::new();
    for provider_id in PROVIDER_ORDER {
        let Some(provider) = oauth_provider(env, provider_id) else {
            continue;
        };
        if !provider_available(env, provider_id).await {
            continue;
        }
        let url = browser_authorize_url(env, attempt_id, ticket, provider.id)?;
        let glyph = match provider.id {
            "apple" => "A",
            "alipay" => "支",
            "google" => "G",
            "microsoft" => "M",
            "github" => "GH",
            "cloudflare" => "CF",
            _ => "•",
        };
        let verb = if mode == "register" {
            "注册"
        } else {
            "登录"
        };
        output.push_str(&format!(
            r#"<a class="provider" href="{}" data-provider="{}"><span class="provider-icon">{}</span><span>使用 {} {}</span><span class="arrow">›</span></a>"#,
            html_escape(url.as_str()),
            html_escape(provider.id),
            glyph,
            html_escape(provider.display_name),
            verb,
        ));
    }
    Ok(output)
}

pub(super) async fn browser_portal_page(
    env: &Env,
    attempt_id: &str,
    ticket: &str,
    mode: &str,
    message: Option<&str>,
) -> Result<Response> {
    let mode = if mode == "register" {
        "register"
    } else {
        "login"
    };
    let providers = browser_provider_buttons(env, attempt_id, ticket, mode).await?;
    let registration_enabled = registration_email_available(env).await;
    let message = message
        .filter(|message| !message.trim().is_empty())
        .map(|message| {
            format!(
                r#"<p class="form-message error" role="alert">{}</p>"#,
                html_escape(message)
            )
        })
        .unwrap_or_default();
    let login_href = format!(
        "/api/auth/browser/portal?attemptId={}&ticket={}&mode=login",
        html_escape(attempt_id),
        html_escape(ticket),
    );
    let register_href = format!(
        "/api/auth/browser/portal?attemptId={}&ticket={}&mode=register",
        html_escape(attempt_id),
        html_escape(ticket),
    );
    let account_form = if mode == "register" {
        if registration_enabled {
            format!(
                r#"<form id="register-form" method="post" action="/api/auth/browser/register" autocomplete="on">
<input type="hidden" name="attemptId" value="{attempt_id}"><input type="hidden" name="ticket" value="{ticket}">{message}
<label>用户名<input name="username" autocomplete="username" required minlength="3" maxlength="32" pattern="[A-Za-z0-9_-]+" placeholder="3–32 位字母、数字、_ 或 -"></label>
<label>邮箱<div class="code-row"><input id="register-email" name="email" type="email" autocomplete="email" required maxlength="254" placeholder="you@example.com"><button id="send-code" class="code-button" type="button">发送验证码</button></div></label>
<p id="code-status" class="form-message" aria-live="polite"></p>
<label>验证码<input name="verificationCode" inputmode="numeric" autocomplete="one-time-code" required minlength="6" maxlength="6" pattern="[0-9]{{6}}" placeholder="6 位验证码"></label>
<label>密码<input name="password" type="password" autocomplete="new-password" required minlength="8" maxlength="1024" placeholder="至少 8 位"></label>
<label>确认密码<input name="confirmPassword" type="password" autocomplete="new-password" required minlength="8" maxlength="1024" placeholder="再次输入密码"></label>
<button class="primary" type="submit">创建 Fabushi 账号</button></form>"#,
                attempt_id = html_escape(attempt_id),
                ticket = html_escape(ticket),
                message = message,
            )
        } else {
            format!(
                r#"{message}<div class="disabled-note">邮箱注册当前未配置邮件服务。你仍可使用上方已启用的身份提供方创建账号。</div>"#,
                message = message,
            )
        }
    } else {
        format!(
            r#"<form method="post" action="/api/auth/browser/password" autocomplete="on"><input type="hidden" name="attemptId" value="{attempt_id}"><input type="hidden" name="ticket" value="{ticket}">{message}<label>账号、邮箱或手机号<input name="username" autocomplete="username" required maxlength="160" placeholder="you@example.com"></label><label>密码<input name="password" type="password" autocomplete="current-password" required maxlength="1024" placeholder="输入密码"></label><button class="primary" type="submit">登录</button></form>"#,
            attempt_id = html_escape(attempt_id),
            ticket = html_escape(ticket),
            message = message,
        )
    };
    let heading = if mode == "register" {
        "创建 Fabushi 账号"
    } else {
        "登录 Fabushi"
    };
    let sub = if mode == "register" {
        "一个账号即可在桌面、移动端和浏览器之间保持同一身份。"
    } else {
        "使用 Fabushi 账号，或选择你已经在使用的身份提供方。"
    };
    let script = if mode == "register" && registration_enabled {
        r#"<script>
const button=document.getElementById('send-code');
const status=document.getElementById('code-status');
const form=document.getElementById('register-form');
button?.addEventListener('click',async()=>{
 const email=document.getElementById('register-email')?.value?.trim();
 if(!email){status.textContent='请先填写邮箱';return;}
 button.disabled=true;status.textContent='正在发送…';
 const body=new URLSearchParams({attemptId:form.elements.attemptId.value,ticket:form.elements.ticket.value,email});
 try{
  const response=await fetch('/api/auth/browser/register/code',{method:'POST',headers:{'content-type':'application/x-www-form-urlencoded;charset=UTF-8'},body});
  const result=await response.json();
  if(!response.ok) throw new Error(result?.error?.message||result?.message||'验证码发送失败');
  status.textContent='验证码已发送，请检查邮箱';
  let seconds=60;button.textContent=`${seconds}s 后重发`;
  const timer=setInterval(()=>{seconds-=1;if(seconds<=0){clearInterval(timer);button.disabled=false;button.textContent='重新发送';}else{button.textContent=`${seconds}s 后重发`; }},1000);
 }catch(error){button.disabled=false;button.textContent='发送验证码';status.textContent=error.message||'验证码发送失败';}
});
form?.addEventListener('submit',(event)=>{if(form.elements.password.value!==form.elements.confirmPassword.value){event.preventDefault();status.textContent='两次输入的密码不一致';form.elements.confirmPassword.focus();}});
</script>"#
    } else {
        ""
    };
    let html = format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{heading}</title>
<style>
*{{box-sizing:border-box}}html,body{{margin:0;min-height:100%;background:#08080a;color:#f6f6f4;font-family:Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}body{{min-height:100vh;display:grid;place-items:center;padding:32px 18px;background:radial-gradient(circle at 50% -12%,rgba(127,103,255,.14),transparent 34%),#08080a}}.auth{{width:min(430px,100%);padding:32px;border:1px solid rgba(255,255,255,.09);border-radius:24px;background:rgba(18,18,21,.96);box-shadow:0 28px 90px rgba(0,0,0,.44)}}.brand{{display:flex;justify-content:center;align-items:center;gap:10px;margin-bottom:28px;font-size:12px;font-weight:800;letter-spacing:.16em}}.mark{{position:relative;width:34px;height:38px;border-radius:52% 48% 56% 44% / 48% 56% 44% 52%;background:#f5f5f1;animation:breathe 4.8s ease-in-out infinite}}.mark:before,.mark:after{{content:"";position:absolute;top:16px;width:4px;height:6px;border-radius:99px;background:#111;animation:blink 6s ease-in-out infinite}}.mark:before{{left:10px}}.mark:after{{right:10px}}h1{{margin:0;text-align:center;font-size:25px;font-weight:620;letter-spacing:-.035em}}.sub{{margin:10px auto 24px;max-width:330px;color:#8f8f96;text-align:center;font-size:13px;line-height:1.55}}.tabs{{display:grid;grid-template-columns:1fr 1fr;gap:4px;margin-bottom:22px;padding:4px;border-radius:12px;background:#0d0d10}}.tab{{height:36px;display:grid;place-items:center;border-radius:9px;color:#818188;text-decoration:none;font-size:12px;font-weight:700}}.tab.active{{background:#202024;color:#f5f5f3;box-shadow:0 1px 0 rgba(255,255,255,.06) inset}}form{{display:grid;gap:13px}}label{{display:grid;gap:7px;color:#a1a1a7;font-size:11px;font-weight:650}}input{{width:100%;height:48px;padding:0 13px;border:1px solid rgba(255,255,255,.1);border-radius:11px;outline:none;background:#0c0c0f;color:#f7f7f5;font:inherit;font-size:13px}}input:focus{{border-color:rgba(151,134,255,.7);box-shadow:0 0 0 3px rgba(121,99,244,.12)}}button,.provider{{font:inherit}}.primary{{height:49px;border:0;border-radius:11px;background:#f2f2ef;color:#111;font-size:13px;font-weight:800;cursor:pointer}}.primary:hover{{background:#fff}}.divider{{display:flex;align-items:center;gap:12px;margin:22px 0;color:#5f5f67;font-size:10px;text-transform:uppercase;letter-spacing:.08em}}.divider:before,.divider:after{{content:"";height:1px;flex:1;background:rgba(255,255,255,.08)}}.providers{{display:grid;gap:9px}}.provider{{height:50px;display:grid;grid-template-columns:34px 1fr 16px;align-items:center;gap:10px;padding:0 13px;border:1px solid rgba(255,255,255,.1);border-radius:11px;background:#17171a;color:#efefed;text-decoration:none;font-size:13px;font-weight:650;transition:border-color .15s ease,background .15s ease,transform .15s ease}}.provider:hover{{transform:translateY(-1px);border-color:rgba(150,134,255,.42);background:#1c1c20}}.provider-icon{{width:28px;height:28px;display:grid;place-items:center;border-radius:8px;background:#f2f2ef;color:#111;font-size:10px;font-weight:900}}.provider[data-provider="alipay"] .provider-icon{{color:#1677ff}}.provider[data-provider="google"] .provider-icon{{color:#4285f4}}.provider[data-provider="microsoft"] .provider-icon{{color:#2563eb}}.provider[data-provider="github"] .provider-icon{{font-size:8px}}.provider[data-provider="cloudflare"] .provider-icon{{color:#f48120;font-size:8px}}.arrow{{color:#616169;font-size:20px;font-weight:300}}.code-row{{display:grid;grid-template-columns:1fr auto;gap:8px}}.code-button{{min-width:98px;height:48px;border:1px solid rgba(255,255,255,.11);border-radius:11px;background:#1a1a1e;color:#e8e8e5;font-size:11px;font-weight:750;cursor:pointer}}.code-button:disabled{{opacity:.55;cursor:default}}.form-message{{min-height:0;margin:0;color:#8e8e95;font-size:11px;line-height:1.45}}.form-message:empty{{display:none}}.form-message.error{{padding:9px 11px;border:1px solid rgba(255,103,120,.24);border-radius:9px;background:rgba(255,103,120,.07);color:#ff9ca8}}.disabled-note{{padding:13px;border:1px solid rgba(255,255,255,.08);border-radius:11px;background:#111114;color:#85858c;font-size:12px;line-height:1.55}}.fine{{margin:22px 0 0;color:#5d5d64;font-size:10px;line-height:1.6;text-align:center}}.security{{display:flex;justify-content:center;gap:6px;margin-top:11px;color:#686870;font-size:10px}}.security i{{width:6px;height:6px;margin-top:4px;border-radius:50%;background:#70c8a5}}@keyframes breathe{{0%,100%{{transform:rotate(-2deg) scale(1)}}50%{{transform:rotate(2deg) scale(1.04)}}}}@keyframes blink{{0%,46%,49%,100%{{transform:scaleY(1)}}47%,48%{{transform:scaleY(.12)}}}}.mobile-heading{{display:none}}@media(max-width:640px){{html,body{{background:#fafaf7;color:#151515}}body{{display:block;min-height:100svh;padding:0;background:#fafaf7}}.auth{{min-height:100svh;width:100%;padding:64px 24px max(30px,env(safe-area-inset-bottom));border:0;border-radius:0;background:#fafaf7;box-shadow:none;display:flex;flex-direction:column}}.brand{{order:0;margin:0 0 28px}}.brand-name{{display:none}}.mark{{width:78px;height:84px;background:#111;animation:breathe 4.8s ease-in-out infinite}}.mark:before,.mark:after{{top:34px;width:9px;height:22px;background:#fff}}.mark:before{{left:23px}}.mark:after{{right:23px}}h1{{order:1;font-size:32px;font-weight:650;letter-spacing:-.04em}}.desktop-heading{{display:none}}.mobile-heading{{display:inline}}.sub{{order:2;display:none}}.providers{{order:3;display:grid;gap:10px;margin-top:34px}}.provider{{height:60px;grid-template-columns:36px 1fr 18px;padding:0 15px;border:0;border-radius:12px;background:#e9e9e6;color:#242422;font-size:16px;font-weight:560}}.provider:hover{{transform:none;border-color:transparent;background:#e3e3df}}.provider-icon{{width:30px;height:30px;border-radius:9px;background:transparent;color:#242422;font-size:12px}}.arrow{{color:#8a8a85}}.divider{{order:4;margin:22px 0;color:#a0a09a;font-size:10px}}.divider:before,.divider:after{{background:#deded8}}form{{order:5;display:grid;gap:14px}}label{{color:#62625e;font-size:12px}}input{{height:54px;border:0;border-radius:12px;background:#ecece8;color:#181817;font-size:15px}}input:focus{{border:0;box-shadow:0 0 0 3px rgba(17,17,17,.08)}}input::placeholder{{color:#9a9a94}}.primary{{height:56px;border-radius:12px;background:#171717;color:#fff;font-size:16px}}.primary:hover{{background:#000}}.tabs{{order:6;display:block;margin:12px 0 0;padding:0;background:transparent}}.tab{{height:56px;border-radius:12px;background:#e9e9e6;color:#2b2b29;font-size:16px;font-weight:560}}.tab.active{{display:none}}.code-row{{grid-template-columns:1fr auto}}.code-button{{height:54px;border:0;background:#deded9;color:#252523}}.form-message{{color:#6f6f6a;font-size:12px}}.form-message.error{{border:0;background:#ffe9e9;color:#a53737}}.disabled-note{{border:0;background:#ecece8;color:#656560;font-size:13px}}.fine{{order:7;margin:30px auto 0;max-width:350px;color:#9b9b95;font-size:12px;line-height:1.6}}.security{{display:none}}}}@media(prefers-reduced-motion:reduce){{*,*:before,*:after{{animation:none!important;transition:none!important}}}}
</style></head><body><main class="auth"><div class="brand"><span class="mark" aria-hidden="true"></span><span class="brand-name">FABUSHI</span></div><h1><span class="desktop-heading">{heading}</span><span class="mobile-heading">开始使用 Fabushi</span></h1><p class="sub">{sub}</p><nav class="tabs" aria-label="账号模式"><a class="tab {login_active}" href="{login_href}">登录</a><a class="tab {register_active}" href="{register_href}">创建账户</a></nav>{account_form}<div class="divider"><span>或使用 Fabushi 账号</span></div><div class="providers">{providers}</div><p class="fine">继续即表示你同意服务条款与隐私政策。身份提供方仅用于登录所需的最小权限；连接其它服务能力时会再次单独请求授权。</p><div class="security"><i></i><span>一次性 state · PKCE / nonce · 无 token deep link</span></div></main>{script}</body></html>"#,
        heading = html_escape(heading),
        sub = html_escape(sub),
        login_active = if mode == "login" { "active" } else { "" },
        register_active = if mode == "register" { "active" } else { "" },
        login_href = login_href,
        register_href = register_href,
        account_form = account_form,
        providers = providers,
        script = script,
    );
    browser_html_response(html)
}

pub(super) async fn browser_login_start(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let start: BrowserLoginStartRequest = match request.json().await {
        Ok(start) => start,
        Err(_) => BrowserLoginStartRequest::default(),
    };
    if let Some(platform) = start.platform.as_deref()
        && !matches!(
            platform,
            "desktop" | "macos" | "windows" | "linux" | "web" | "mobile"
        )
    {
        return error_response(400, "invalid_auth_platform", "unsupported auth platform");
    }
    let device_id = match normalize_device_id(start.device_id.as_deref()) {
        Ok(device_id) => device_id,
        Err(_) => return error_response(400, "invalid_device_id", "invalid device id"),
    };
    let attempt_id = Uuid::new_v4().to_string();
    let ticket = format!("fbt_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let state_hash = browser_ticket_hash(&ticket);
    let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let now = now_seconds();
    let expires_at = now + OAUTH_ATTEMPT_SECONDS;
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(
        &database,
        "INSERT INTO account_oauth_attempts
         (attempt_id, state_hash, code_verifier, provider, device_id, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, 'portal', ?4, 'pending', ?5, ?6)",
        &attempt_id,
        &state_hash,
        &code_verifier,
        &device_id,
        now,
        expires_at
    )?
    .run()
    .await?;
    let login_url = browser_portal_url(&context.env, &attempt_id, &ticket)?;
    let poll_secret = browser_poll_secret(&context.env, &attempt_id)?;
    Ok(Response::from_json(&json!({
        "attemptId": attempt_id,
        "loginUrl": login_url.as_str(),
        "pollSecret": poll_secret,
        "expiresAt": expires_at,
        "pollAfterMs": 750,
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn browser_login_portal(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let url = request.url()?;
    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let attempt_id = query
        .get("attemptId")
        .map(|value| value.as_ref())
        .unwrap_or_default();
    let ticket = query
        .get("ticket")
        .map(|value| value.as_ref())
        .unwrap_or_default();
    let mode = query
        .get("mode")
        .map(|value| value.as_ref())
        .filter(|value| *value == "register")
        .unwrap_or("login");
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    if browser_attempt_for_ticket(&database, attempt_id, ticket)
        .await?
        .is_none()
    {
        return browser_result_page(false, "登录页面已失效，请返回 Fabushi 重试", None);
    }
    browser_portal_page(&context.env, attempt_id, ticket, mode, None).await
}

pub(super) async fn browser_login_authorize(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let url = request.url()?;
    let query = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let attempt_id = query
        .get("attemptId")
        .map(|value| value.as_ref())
        .unwrap_or_default();
    let ticket = query
        .get("ticket")
        .map(|value| value.as_ref())
        .unwrap_or_default();
    let provider_id = query
        .get("provider")
        .map(|value| value.as_ref())
        .unwrap_or_default();
    let Some(provider) = oauth_provider(&context.env, provider_id) else {
        return browser_portal_page(
            &context.env,
            attempt_id,
            ticket,
            "login",
            Some("该登录方式当前不可用，请选择其他方式"),
        )
        .await;
    };
    if !provider_available(&context.env, provider.id).await {
        return browser_portal_page(
            &context.env,
            attempt_id,
            ticket,
            "login",
            Some("该登录方式尚未完成服务端配置，请选择其他方式"),
        )
        .await;
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let Some(attempt) = browser_attempt_for_ticket(&database, attempt_id, ticket).await? else {
        return browser_result_page(false, "登录页面已失效，请返回 Fabushi 重试", None);
    };
    let state = format!("fbs_{}", Uuid::new_v4().simple());
    let state_hash = format!("{:x}", Sha256::digest(state.as_bytes()));
    let callback = format!(
        "{}/api/auth/oauth/callback",
        auth_public_base_url(&context.env)?
    );
    let authorization_url = build_authorization_url(
        &context.env,
        &provider,
        &state,
        &callback,
        &attempt.code_verifier,
    )
    .await?;
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts SET provider = ?1, state_hash = ?2
         WHERE attempt_id = ?3 AND provider = 'portal' AND status = 'pending'",
        provider.id,
        &state_hash,
        &attempt.attempt_id
    )?
    .run()
    .await?;
    Response::redirect_with_status(authorization_url, 302)
}

pub(super) async fn browser_login_password(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let form = request.form_data().await?;
    let field = |name: &str| -> String {
        match form.get(name) {
            Some(FormEntry::Field(value)) => value,
            _ => String::new(),
        }
    };
    let attempt_id = field("attemptId");
    let ticket = field("ticket");
    let username = field("username");
    let password = field("password");
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let Some(attempt) = browser_attempt_for_ticket(&database, &attempt_id, &ticket).await? else {
        return browser_result_page(false, "登录页面已失效，请返回 Fabushi 重试", None);
    };
    let login = PasswordLoginRequest {
        username,
        password,
        device_id: Some(attempt.device_id.clone()),
    };
    let session = match password_session_value(&context.env, &login).await? {
        Ok(session) => session,
        Err(rejection) => {
            return browser_portal_page(
                &context.env,
                &attempt_id,
                &ticket,
                "login",
                Some(rejection.message),
            )
            .await;
        }
    };
    let now = now_seconds();
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts
         SET provider = 'password', status = 'completed', session_json = ?1, completed_at = ?2
         WHERE attempt_id = ?3 AND provider = 'portal' AND status = 'pending'",
        session.to_string(),
        now,
        &attempt_id
    )?
    .run()
    .await?;
    browser_result_page_for_device(
        true,
        "登录完成，正在返回 Fabushi",
        Some(&attempt_id),
        &attempt.device_id,
    )
}

fn normalize_registration_email(value: &str) -> Option<String> {
    let email = value.trim().to_ascii_lowercase();
    if email.is_empty() || email.len() > 254 || email.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    Some(email)
}

fn valid_registration_username(value: &str) -> bool {
    (3..=32).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn registration_code_hash(env: &Env, attempt_id: &str, email: &str, code: &str) -> Result<String> {
    let key = env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let material = format!("fabushi-registration-code:v1:{attempt_id}:{email}:{code}:{key}");
    Ok(format!("{:x}", Sha256::digest(material.as_bytes())))
}

fn new_registration_code() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{value:06}")
}

pub(super) async fn browser_registration_code(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let form = request.form_data().await?;
    let field = |name: &str| -> String {
        match form.get(name) {
            Some(FormEntry::Field(value)) => value,
            _ => String::new(),
        }
    };
    let attempt_id = field("attemptId");
    let ticket = field("ticket");
    let Some(email) = normalize_registration_email(&field("email")) else {
        return error_response(400, "invalid_registration_email", "请输入有效邮箱");
    };
    if !registration_email_available(&context.env).await {
        return error_response(503, "registration_email_unavailable", "邮箱注册暂不可用");
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    if browser_attempt_for_ticket(&database, &attempt_id, &ticket)
        .await?
        .is_none()
    {
        return error_response(410, "browser_attempt_expired", "注册页面已失效，请重新开始");
    }
    if lookup_login_user(&database, &email).await?.is_some() {
        return error_response(409, "registration_email_exists", "该邮箱已注册，请直接登录");
    }
    let existing = worker::query!(
        &database,
        "SELECT attempt_id, code_hash, sent_at, expires_at, failed_attempts, consumed_at
         FROM account_email_challenges WHERE email = ?1 AND purpose = 'register' LIMIT 1",
        &email
    )?
    .first::<RegistrationChallengeRow>(None)
    .await?;
    let now = now_seconds();
    if let Some(existing) = existing
        && existing.sent_at + 60 > now
    {
        return error_response(
            429,
            "registration_code_rate_limited",
            "验证码发送过于频繁，请稍后再试",
        );
    }
    let code = new_registration_code();
    let code_hash = registration_code_hash(&context.env, &attempt_id, &email, &code)?;
    let challenge_id = Uuid::new_v4().to_string();
    let expires_at = now + 10 * 60;
    worker::query!(
        &database,
        "INSERT INTO account_email_challenges
         (challenge_id, attempt_id, email, purpose, code_hash, sent_at, expires_at, failed_attempts, consumed_at)
         VALUES (?1, ?2, ?3, 'register', ?4, ?5, ?6, 0, NULL)
         ON CONFLICT(email, purpose) DO UPDATE SET
           challenge_id = excluded.challenge_id,
           attempt_id = excluded.attempt_id,
           code_hash = excluded.code_hash,
           sent_at = excluded.sent_at,
           expires_at = excluded.expires_at,
           failed_attempts = 0,
           consumed_at = NULL",
        &challenge_id,
        &attempt_id,
        &email,
        &code_hash,
        now,
        expires_at
    )?
    .run()
    .await?;
    if send_registration_code(&context.env, &email, &code)
        .await
        .is_err()
    {
        worker::query!(
            &database,
            "DELETE FROM account_email_challenges WHERE challenge_id = ?1",
            &challenge_id
        )?
        .run()
        .await?;
        return error_response(
            502,
            "registration_email_failed",
            "验证码发送失败，请稍后重试",
        );
    }
    Ok(Response::from_json(&json!({
        "ok": true,
        "expiresAt": expires_at,
        "resendAfter": now + 60,
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn browser_registration_complete(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let form = request.form_data().await?;
    let field = |name: &str| -> String {
        match form.get(name) {
            Some(FormEntry::Field(value)) => value,
            _ => String::new(),
        }
    };
    let attempt_id = field("attemptId");
    let ticket = field("ticket");
    let username = field("username").trim().to_string();
    let Some(email) = normalize_registration_email(&field("email")) else {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("请输入有效邮箱"),
        )
        .await;
    };
    let verification_code = field("verificationCode");
    let password = field("password");
    let confirm_password = field("confirmPassword");
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let Some(attempt) = browser_attempt_for_ticket(&database, &attempt_id, &ticket).await? else {
        return browser_result_page(false, "注册页面已失效，请返回 Fabushi 重试", None);
    };
    if !valid_registration_username(&username) {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("用户名需为 3–32 位字母、数字、下划线或连字符"),
        )
        .await;
    }
    if password.len() < 8 || password.len() > 1024 {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("密码至少 8 位"),
        )
        .await;
    }
    if password != confirm_password {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("两次输入的密码不一致"),
        )
        .await;
    }
    if verification_code.len() != 6 || !verification_code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("验证码格式无效"),
        )
        .await;
    }
    let challenge = worker::query!(
        &database,
        "SELECT attempt_id, code_hash, sent_at, expires_at, failed_attempts, consumed_at
         FROM account_email_challenges WHERE email = ?1 AND purpose = 'register' LIMIT 1",
        &email
    )?
    .first::<RegistrationChallengeRow>(None)
    .await?;
    let now = now_seconds();
    let Some(challenge) = challenge else {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("请先发送邮箱验证码"),
        )
        .await;
    };
    if challenge.attempt_id != attempt_id
        || challenge.consumed_at.is_some()
        || challenge.expires_at <= now
        || challenge.failed_attempts >= 5
    {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("验证码已失效，请重新发送"),
        )
        .await;
    }
    let expected_hash =
        registration_code_hash(&context.env, &attempt_id, &email, &verification_code)?;
    if !constant_time_text_eq(&expected_hash, &challenge.code_hash) {
        worker::query!(
            &database,
            "UPDATE account_email_challenges SET failed_attempts = failed_attempts + 1
             WHERE email = ?1 AND purpose = 'register' AND attempt_id = ?2",
            &email,
            &attempt_id
        )?
        .run()
        .await?;
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("验证码不正确"),
        )
        .await;
    }
    if lookup_login_user(&database, &username).await?.is_some() {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("该用户名已被使用"),
        )
        .await;
    }
    if lookup_login_user(&database, &email).await?.is_some() {
        return browser_portal_page(
            &context.env,
            &attempt_id,
            &ticket,
            "register",
            Some("该邮箱已注册，请直接登录"),
        )
        .await;
    }
    let max = worker::query!(&database, "SELECT MAX(id) AS max_id FROM users")
        .first::<MaxUserIdRow>(None)
        .await?
        .and_then(|row| row.max_id)
        .unwrap_or(10_000);
    let id = max + 1;
    let password_phc = hash_password_argon2id(&password, &new_password_salt())
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let created_at = Date::now().to_string();
    database
        .batch(vec![
            worker::query!(
                &database,
                "INSERT INTO users
                 (id, user_no, username, email, password_hash, salt, iterations, algo,
                  email_verified, membership_type, created_at)
                 VALUES (?1, ?1, ?2, ?3, '', '', 0, 'argon2id', 1, 'trial', ?4)",
                id,
                &username,
                &email,
                &created_at
            )?,
            worker::query!(
                &database,
                "INSERT INTO account_password_credentials (user_id, password_phc, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                id.to_string(),
                &password_phc,
                now
            )?,
            worker::query!(
                &database,
                "INSERT INTO email_username_mapping (email, username, user_id) VALUES (?1, ?2, ?3)",
                &email,
                &username,
                id
            )?,
            worker::query!(
                &database,
                "UPDATE account_email_challenges SET consumed_at = ?1
                 WHERE email = ?2 AND purpose = 'register' AND attempt_id = ?3 AND consumed_at IS NULL",
                now,
                &email,
                &attempt_id
            )?,
        ])
        .await?;
    let user = lookup_account_user_by_id(&database, &id.to_string())
        .await?
        .ok_or_else(|| worker::Error::RustError("registered account missing".into()))?;
    let session = create_account_session_value(
        &database,
        &context.env,
        &user,
        &attempt.device_id,
        "registration_succeeded",
    )
    .await?;
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts
         SET provider = 'password', status = 'completed', session_json = ?1, completed_at = ?2
         WHERE attempt_id = ?3 AND provider = 'portal' AND status = 'pending'",
        session.to_string(),
        now,
        &attempt_id
    )?
    .run()
    .await?;
    browser_result_page_for_device(
        true,
        "注册完成，正在返回 Fabushi",
        Some(&attempt_id),
        &attempt.device_id,
    )
}

pub(super) async fn browser_login_reopen(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let attempt_id = route_identifier(&context, "attempt_id")?;
    let reopen: BrowserLoginProofRequest = match request.json().await {
        Ok(reopen) => reopen,
        Err(_) => return error_response(400, "invalid_browser_reopen", "登录恢复请求无效"),
    };
    let expected = browser_poll_secret(&context.env, attempt_id)?;
    if reopen.poll_secret.is_empty() || !constant_time_text_eq(&reopen.poll_secret, &expected) {
        return error_response(
            403,
            "browser_reopen_forbidden",
            "登录会话验证失败，请重新开始",
        );
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let current = worker::query!(
        &database,
        "SELECT status, expires_at FROM account_oauth_attempts WHERE attempt_id = ?1 LIMIT 1",
        attempt_id
    )?
    .first::<BrowserAttemptStatusRow>(None)
    .await?;
    let Some(current) = current else {
        return error_response(404, "oauth_attempt_missing", "登录链接不存在或已失效");
    };
    if current.status == "pending" && current.expires_at <= now_seconds() {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'expired', session_json = NULL WHERE attempt_id = ?1 AND status = 'pending'",
            attempt_id
        )?
        .run()
        .await?;
        return Response::from_json(&json!({"status": "expired"}));
    }
    if current.status != "pending" {
        return Response::from_json(&json!({"status": current.status}));
    }
    let ticket = format!("fbt_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let state_hash = browser_ticket_hash(&ticket);
    let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts
         SET provider = 'portal', state_hash = ?1, code_verifier = ?2, session_json = NULL
         WHERE attempt_id = ?3 AND status = 'pending'",
        &state_hash,
        &code_verifier,
        attempt_id
    )?
    .run()
    .await?;
    let login_url = browser_portal_url(&context.env, attempt_id, &ticket)?;
    Response::from_json(&json!({
        "status": "pending",
        "attemptId": attempt_id,
        "loginUrl": login_url.as_str(),
        "pollAfterMs": 750,
    }))
}

pub(super) async fn browser_login_cancel(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let attempt_id = route_identifier(&context, "attempt_id")?;
    let cancel: BrowserLoginProofRequest = match request.json().await {
        Ok(cancel) => cancel,
        Err(_) => return error_response(400, "invalid_browser_cancel", "登录取消请求无效"),
    };
    let expected = browser_poll_secret(&context.env, attempt_id)?;
    if cancel.poll_secret.is_empty() || !constant_time_text_eq(&cancel.poll_secret, &expected) {
        return error_response(
            403,
            "browser_cancel_forbidden",
            "登录会话验证失败，请重新开始",
        );
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let current = worker::query!(
        &database,
        "SELECT status, expires_at FROM account_oauth_attempts WHERE attempt_id = ?1 LIMIT 1",
        attempt_id
    )?
    .first::<BrowserAttemptStatusRow>(None)
    .await?;
    let status = current
        .as_ref()
        .map(|row| row.status.as_str())
        .unwrap_or("missing");
    if status == "pending"
        && current
            .as_ref()
            .is_some_and(|row| row.expires_at <= now_seconds())
    {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'expired', session_json = NULL WHERE attempt_id = ?1 AND status = 'pending'",
            attempt_id
        )?
        .run()
        .await?;
        return Response::from_json(&json!({"status": "expired"}));
    }
    if status == "pending" {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts
             SET status = 'cancelled', session_json = NULL
             WHERE attempt_id = ?1 AND status = 'pending'",
            attempt_id
        )?
        .run()
        .await?;
        return Response::from_json(&json!({"status": "cancelled"}));
    }
    if status == "missing" {
        return error_response(404, "oauth_attempt_missing", "登录链接不存在或已失效");
    }
    Response::from_json(&json!({"status": status}))
}

pub(super) async fn browser_login_poll(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let attempt_id = route_identifier(&context, "attempt_id")?;
    let poll: BrowserLoginProofRequest = match request.json().await {
        Ok(poll) => poll,
        Err(_) => return error_response(400, "invalid_browser_poll", "登录轮询请求无效"),
    };
    let expected = browser_poll_secret(&context.env, attempt_id)?;
    if poll.poll_secret.is_empty() || !constant_time_text_eq(&poll.poll_secret, &expected) {
        return error_response(
            403,
            "browser_poll_forbidden",
            "登录会话验证失败，请重新开始",
        );
    }
    oauth_poll(request, context).await
}

fn oauth_provider(env: &Env, provider: &str) -> Option<OAuthProviderConfig> {
    configured_provider(env, provider)
}

pub(super) async fn oauth_providers(
    _request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let mut providers = Vec::new();
    for id in PROVIDER_ORDER {
        let Some(provider) = oauth_provider(&context.env, id) else {
            continue;
        };
        if !provider_available(&context.env, id).await {
            continue;
        }
        providers.push(json!({
            "id": provider.id,
            "displayName": provider.display_name,
            "enabled": true,
        }));
    }
    Ok(Response::from_json(&json!({"providers": providers}))?.with_headers(auth_headers()))
}

pub(super) async fn oauth_start(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let start: OAuthStartRequest = match request.json().await {
        Ok(start) => start,
        Err(_) => return error_response(400, "invalid_oauth_request", "provider is required"),
    };
    let provider_id = start.provider.trim();
    let Some(provider) = oauth_provider(&context.env, provider_id) else {
        return error_response(400, "oauth_provider_unavailable", "该登录方式尚未配置");
    };
    if !provider_available(&context.env, provider.id).await {
        return error_response(503, "oauth_provider_unavailable", "该登录方式当前不可用");
    }
    if let Some(platform) = start.platform.as_deref()
        && !matches!(
            platform,
            "desktop" | "macos" | "windows" | "linux" | "web" | "mobile"
        )
    {
        return error_response(400, "invalid_oauth_platform", "unsupported OAuth platform");
    }
    let device_id = normalize_device_id(start.device_id.as_deref())?;
    let attempt_id = Uuid::new_v4().to_string();
    let state = format!("fbs_{}", Uuid::new_v4().simple());
    let state_hash = format!("{:x}", Sha256::digest(state.as_bytes()));
    let code_verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let now = now_seconds();
    let expires_at = now + OAUTH_ATTEMPT_SECONDS;
    let callback = format!(
        "{}/api/auth/oauth/callback",
        auth_public_base_url(&context.env)?
    );
    let authorization_url =
        build_authorization_url(&context.env, &provider, &state, &callback, &code_verifier).await?;
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(
        &database,
        "INSERT INTO account_oauth_attempts
         (attempt_id, state_hash, code_verifier, provider, device_id, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
        &attempt_id,
        &state_hash,
        &code_verifier,
        provider.id,
        &device_id,
        now,
        expires_at
    )?
    .run()
    .await?;
    Ok(Response::from_json(&json!({
        "attemptId": attempt_id,
        "provider": provider.id,
        "authorizationUrl": authorization_url.as_str(),
        "expiresAt": expires_at,
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn oauth_poll(_request: Request, context: RouteContext<()>) -> Result<Response> {
    let attempt_id = route_identifier(&context, "attempt_id")?;
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT attempt_id, provider, device_id, code_verifier, status, session_json, expires_at, delivered_at
         FROM account_oauth_attempts WHERE attempt_id = ?1 LIMIT 1",
        attempt_id
    )?
    .first::<OAuthAttemptRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(404, "oauth_attempt_missing", "登录链接不存在或已失效");
    };
    let now = now_seconds();
    if row.expires_at <= now && row.status == "pending" {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'expired' WHERE attempt_id = ?1",
            &row.attempt_id
        )?
        .run()
        .await?;
        return Response::from_json(&json!({"status": "expired", "provider": row.provider}));
    }
    if row.status != "completed" {
        return Response::from_json(&json!({"status": row.status, "provider": row.provider}));
    }
    if row.delivered_at.is_some() {
        return error_response(410, "oauth_session_delivered", "登录结果已经领取");
    }
    let Some(session_json) = row.session_json else {
        return error_response(410, "oauth_session_missing", "登录结果已经失效");
    };
    let session: Value = serde_json::from_str(&session_json)
        .map_err(|error| worker::Error::RustError(error.to_string()))?;
    let delivery = worker::query!(
        &database,
        "UPDATE account_oauth_attempts SET session_json = NULL, delivered_at = ?1
         WHERE attempt_id = ?2 AND delivered_at IS NULL AND session_json IS NOT NULL",
        now,
        &row.attempt_id
    )?
    .run()
    .await?;
    if delivery.meta()?.and_then(|meta| meta.changes).unwrap_or(0) == 0 {
        return error_response(410, "oauth_session_delivered", "登录结果已经领取");
    }
    Response::from_json(&json!({
        "status": "completed",
        "provider": row.provider,
        "session": session,
    }))
}

pub(super) async fn oauth_callback(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let mut fields = std::collections::HashMap::<String, String>::new();
    if request.method() == Method::Post {
        let form = request.form_data().await?;
        for name in [
            "state",
            "code",
            "auth_code",
            "error",
            "error_description",
            "id_token",
            "user",
        ] {
            if let Some(FormEntry::Field(value)) = form.get(name) {
                fields.insert(name.to_string(), value);
            }
        }
    } else {
        for (key, value) in request.url()?.query_pairs() {
            fields.insert(key.into_owned(), value.into_owned());
        }
    }
    let Some(state) = fields.get("state").map(String::as_str) else {
        return browser_result_page(false, "登录状态缺失，请返回应用重试", None);
    };
    let state_hash = format!("{:x}", Sha256::digest(state.as_bytes()));
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let attempt = worker::query!(
        &database,
        "SELECT attempt_id, provider, device_id, code_verifier, status, session_json, expires_at, delivered_at
         FROM account_oauth_attempts WHERE state_hash = ?1 LIMIT 1",
        &state_hash
    )?
    .first::<OAuthAttemptRow>(None)
    .await?;
    let Some(attempt) = attempt else {
        return browser_result_page(false, "登录状态无效，请返回应用重试", None);
    };
    if attempt.status != "pending" || attempt.expires_at <= now_seconds() {
        return browser_result_page(
            false,
            "登录链接已失效，请返回应用重试",
            Some(&attempt.attempt_id),
        );
    }
    if fields.contains_key("error") {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'cancelled' WHERE attempt_id = ?1",
            &attempt.attempt_id
        )?
        .run()
        .await?;
        return browser_cancelled_page("登录已取消，正在返回 Fabushi", Some(&attempt.attempt_id));
    }
    let code = fields
        .get("code")
        .or_else(|| fields.get("auth_code"))
        .map(String::as_str);
    let Some(code) = code else {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'failed' WHERE attempt_id = ?1 AND status = 'pending'",
            &attempt.attempt_id
        )?
        .run()
        .await?;
        return browser_result_page(
            false,
            "授权码缺失，请返回应用重试",
            Some(&attempt.attempt_id),
        );
    };
    let Some(provider) = oauth_provider(&context.env, &attempt.provider) else {
        worker::query!(
            &database,
            "UPDATE account_oauth_attempts SET status = 'failed' WHERE attempt_id = ?1 AND status = 'pending'",
            &attempt.attempt_id
        )?
        .run()
        .await?;
        return browser_result_page(false, "该登录方式当前不可用", Some(&attempt.attempt_id));
    };
    let callback = format!(
        "{}/api/auth/oauth/callback",
        auth_public_base_url(&context.env)?
    );
    let profile = match complete_provider(
        &context.env,
        &provider,
        code,
        &callback,
        &attempt.code_verifier,
        fields.get("id_token").map(String::as_str),
        fields.get("user").map(String::as_str),
    )
    .await
    {
        Ok(profile) => profile,
        Err(_) => {
            worker::query!(
                &database,
                "UPDATE account_oauth_attempts SET status = 'failed' WHERE attempt_id = ?1 AND status = 'pending'",
                &attempt.attempt_id
            )?
            .run()
            .await?;
            return browser_result_page(
                false,
                "身份验证失败，请返回 Fabushi 重试",
                Some(&attempt.attempt_id),
            );
        }
    };
    let user = oauth_resolve_user(&database, &provider, &profile).await?;
    let session = create_account_session_value(
        &database,
        &context.env,
        &user,
        &attempt.device_id,
        &format!("oauth_{}", provider.id),
    )
    .await?;
    let now = now_seconds();
    worker::query!(
        &database,
        "UPDATE account_oauth_attempts
         SET status = 'completed', session_json = ?1, completed_at = ?2
         WHERE attempt_id = ?3 AND status = 'pending'",
        session.to_string(),
        now,
        &attempt.attempt_id
    )?
    .run()
    .await?;
    browser_result_page_for_device(
        true,
        "登录完成，正在返回 Fabushi",
        Some(&attempt.attempt_id),
        &attempt.device_id,
    )
}

pub(super) async fn oauth_resolve_user(
    database: &worker::D1Database,
    provider: &OAuthProviderConfig,
    profile: &OAuthIdentityProfile,
) -> Result<AccountUserRow> {
    let identity = worker::query!(
        database,
        "SELECT user_id FROM account_identities WHERE issuer = ?1 AND subject = ?2 LIMIT 1",
        &profile.issuer,
        &profile.subject
    )?
    .first::<OAuthIdentityRow>(None)
    .await?;
    let now = now_seconds();
    let user_id = if let Some(identity) = identity {
        worker::query!(
            database,
            "UPDATE account_identities SET email = ?1, email_verified = ?2,
             display_name = ?3, avatar_url = ?4, last_login_at = ?5
             WHERE issuer = ?6 AND subject = ?7",
            &profile.email,
            i64::from(profile.email_verified),
            &profile.display_name,
            &profile.avatar_url,
            now,
            &profile.issuer,
            &profile.subject
        )?
        .run()
        .await?;
        identity.user_id
    } else {
        let legacy_user_id = lookup_legacy_provider_user(database, provider, profile).await?;
        let verified_email = profile
            .email_verified
            .then(|| profile.email.as_deref())
            .flatten()
            .filter(|email| !email.trim().is_empty());
        let existing_by_email = if let Some(email) = verified_email {
            lookup_login_user(database, email).await?
        } else {
            None
        };
        let user_id = if let Some(user_id) = legacy_user_id {
            user_id
        } else if let Some(user) = existing_by_email {
            user.id.to_string()
        } else {
            let subject_only_allowed = matches!(provider.id, "alipay" | "microsoft");
            if verified_email.is_none() && !subject_only_allowed {
                return Err(worker::Error::RustError(
                    "identity provider did not return a verified email".into(),
                ));
            }
            let max = worker::query!(database, "SELECT MAX(id) AS max_id FROM users")
                .first::<MaxUserIdRow>(None)
                .await?
                .and_then(|row| row.max_id)
                .unwrap_or(10_000);
            let id = max + 1;
            let subject_slug = profile
                .subject
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .take(12)
                .collect::<String>();
            let username = format!(
                "{}_{}_{}",
                provider.id,
                subject_slug,
                &Uuid::new_v4().simple().to_string()[..6]
            );
            let synthetic_email = synthetic_identity_email(provider.id, &profile.subject);
            let account_email = verified_email.unwrap_or(&synthetic_email);
            let created_at = Date::now().to_string();
            worker::query!(
                database,
                "INSERT INTO users
                 (id, user_no, username, email, nickname, avatar, password_hash, salt,
                  iterations, algo, email_verified, membership_type, created_at)
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, '', '', 0, '', ?6, 'trial', ?7)",
                id,
                &username,
                account_email,
                &profile.display_name,
                &profile.avatar_url,
                i64::from(verified_email.is_some()),
                &created_at
            )?
            .run()
            .await?;
            if let Some(email) = verified_email {
                worker::query!(
                    database,
                    "INSERT OR IGNORE INTO email_username_mapping (email, username, user_id)
                     VALUES (?1, ?2, ?3)",
                    email,
                    &username,
                    id
                )?
                .run()
                .await?;
            }
            id.to_string()
        };
        worker::query!(
            database,
            "INSERT INTO account_identities
             (identity_id, user_id, provider, issuer, subject, email, email_verified,
              display_name, avatar_url, created_at, last_login_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            Uuid::new_v4().to_string(),
            &user_id,
            provider.id,
            &profile.issuer,
            &profile.subject,
            &profile.email,
            i64::from(profile.email_verified),
            &profile.display_name,
            &profile.avatar_url,
            now
        )?
        .run()
        .await?;
        user_id
    };
    sync_legacy_provider_identity(database, &user_id, provider, profile).await?;
    lookup_account_user_by_id(database, &user_id)
        .await?
        .ok_or_else(|| worker::Error::RustError("OAuth account missing".into()))
}

pub(super) async fn lookup_legacy_provider_user(
    database: &worker::D1Database,
    provider: &OAuthProviderConfig,
    profile: &OAuthIdentityProfile,
) -> Result<Option<String>> {
    if provider.id == "apple" {
        return worker::query!(
            database,
            "SELECT CAST(id AS TEXT) AS user_id FROM users WHERE apple_user_id = ?1 LIMIT 1",
            &profile.subject
        )?
        .first::<OAuthIdentityRow>(None)
        .await
        .map(|row| row.map(|row| row.user_id));
    }
    if provider.id != "alipay" {
        return Ok(None);
    }
    let mut subjects = vec![profile.subject.as_str()];
    if let Some(legacy) = profile.legacy_subject.as_deref()
        && !legacy.is_empty()
        && legacy != profile.subject
    {
        subjects.push(legacy);
    }
    for subject in subjects {
        if let Some(row) = worker::query!(
            database,
            "SELECT CAST(id AS TEXT) AS user_id FROM users WHERE alipay_user_id = ?1 LIMIT 1",
            subject
        )?
        .first::<OAuthIdentityRow>(None)
        .await?
        {
            return Ok(Some(row.user_id));
        }
        if let Some(row) = worker::query!(
            database,
            "SELECT CAST(user_id AS TEXT) AS user_id FROM alipay_bindings
             WHERE alipay_user_id = ?1 AND user_id IS NOT NULL LIMIT 1",
            subject
        )?
        .first::<OAuthIdentityRow>(None)
        .await?
        {
            return Ok(Some(row.user_id));
        }
    }
    Ok(None)
}

pub(super) async fn sync_legacy_provider_identity(
    database: &worker::D1Database,
    user_id: &str,
    provider: &OAuthProviderConfig,
    profile: &OAuthIdentityProfile,
) -> Result<()> {
    if provider.id == "apple" {
        worker::query!(
            database,
            "UPDATE users SET apple_user_id = COALESCE(apple_user_id, ?1) WHERE CAST(id AS TEXT) = ?2",
            &profile.subject,
            user_id
        )?
        .run()
        .await?;
    } else if provider.id == "alipay" {
        worker::query!(
            database,
            "UPDATE users SET alipay_user_id = COALESCE(alipay_user_id, ?1),
             alipay_nickname = COALESCE(?2, alipay_nickname),
             alipay_avatar = COALESCE(?3, alipay_avatar),
             alipay_bound_at = COALESCE(alipay_bound_at, ?4)
             WHERE CAST(id AS TEXT) = ?5",
            &profile.subject,
            &profile.display_name,
            &profile.avatar_url,
            Date::now().to_string(),
            user_id
        )?
        .run()
        .await?;
        worker::query!(
            database,
            "INSERT OR IGNORE INTO alipay_bindings (alipay_user_id, user_id, bound_at)
             VALUES (?1, CAST(?2 AS INTEGER), ?3)",
            &profile.subject,
            user_id,
            Date::now().to_string()
        )?
        .run()
        .await?;
    }
    Ok(())
}

fn synthetic_identity_email(provider: &str, subject: &str) -> String {
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{provider}:{subject}").as_bytes())
    );
    format!("{provider}+{}@identity.fabushi.invalid", &digest[..32])
}

pub(super) async fn create_account_session_value(
    database: &worker::D1Database,
    env: &Env,
    user: &AccountUserRow,
    device_id: &str,
    event_type: &str,
) -> Result<Value> {
    let now = now_seconds();
    let session_id = Uuid::new_v4().to_string();
    let family_id = Uuid::new_v4().to_string();
    let refresh_token = new_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token);
    let refresh_expires_at = now + REFRESH_TOKEN_SECONDS;
    let (access_token, access_expires_at, access_jti) =
        issue_account_access_token(env, &user.id.to_string(), device_id, &session_id, now)?;
    database
        .batch(vec![
            worker::query!(
                database,
                "INSERT INTO account_sessions
                 (session_id, refresh_family_id, user_id, device_id, current_refresh_token_hash,
                  created_at, last_used_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
                &session_id,
                &family_id,
                user.id.to_string(),
                device_id,
                &refresh_hash,
                now,
                refresh_expires_at
            )?,
            worker::query!(
                database,
                "INSERT INTO account_refresh_tokens
                 (token_hash, session_id, generation, state, issued_at, expires_at)
                 VALUES (?1, ?2, 0, 'active', ?3, ?4)",
                &refresh_hash,
                &session_id,
                now,
                refresh_expires_at
            )?,
            worker::query!(
                database,
                "INSERT INTO account_auth_events
                 (event_id, user_id, session_id, event_type, occurred_at, details_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                Uuid::new_v4().to_string(),
                user.id.to_string(),
                &session_id,
                event_type,
                now,
                json!({"accessJti": access_jti}).to_string()
            )?,
        ])
        .await?;
    Ok(json!({
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "tokenType": "Bearer",
        "expiresIn": ACCESS_TOKEN_SECONDS,
        "accessTokenExpiresAt": access_expires_at,
        "refreshTokenExpiresAt": refresh_expires_at,
        "sessionId": session_id,
        "deviceId": device_id,
        "username": user.username,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "user": serialize_account_user(user),
    }))
}

fn browser_cancelled_page(message: &str, attempt_id: Option<&str>) -> Result<Response> {
    browser_result_page_with_status("cancelled", false, message, attempt_id)
}

fn browser_result_page_for_device(
    success: bool,
    message: &str,
    attempt_id: Option<&str>,
    device_id: &str,
) -> Result<Response> {
    if success {
        if let Some(request_id) = device_id.strip_prefix("mcp-oauth-") {
            let valid = (32..=128).contains(&request_id.len())
                && request_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
            if valid {
                let return_url = format!(
                    "https://fabushi-mcp.ombhrum.com/oauth/fabushi/complete?request_id={request_id}"
                );
                let literal = serde_json::to_string(&return_url).unwrap_or_else(|_| "null".into());
                return browser_html_response(format!(
                    r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Fabushi 登录</title></head><body><main>登录成功，正在返回 AI 客户端…</main><script>location.replace({literal});</script></body></html>"#
                ));
            }
        }
    }
    browser_result_page(success, message, attempt_id)
}

fn browser_result_page(success: bool, message: &str, attempt_id: Option<&str>) -> Result<Response> {
    browser_result_page_with_status(
        if success { "completed" } else { "failed" },
        success,
        message,
        attempt_id,
    )
}

fn browser_result_page_with_status(
    status: &str,
    success: bool,
    message: &str,
    attempt_id: Option<&str>,
) -> Result<Response> {
    let deep_link = attempt_id.map(|attempt_id| {
        format!("fabushi://auth/complete?attemptId={attempt_id}&status={status}")
    });
    let link_markup = deep_link
        .as_deref()
        .map(|link| {
            format!(
                r#"<a class="return" href="{}">返回 Fabushi</a>"#,
                html_escape(link)
            )
        })
        .unwrap_or_default();
    let wake_script = deep_link
        .as_deref()
        .map(|link| {
            let literal = serde_json::to_string(link).unwrap_or_else(|_| "null".into());
            format!("setTimeout(()=>{{try{{window.location.href={literal}}}catch{{}}}},350);")
        })
        .unwrap_or_default();
    let tone = if success { "ok" } else { "warn" };
    let eyebrow = if success {
        "AUTHENTICATED"
    } else {
        "LOGIN INTERRUPTED"
    };
    browser_html_response(format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Fabushi 登录</title><style>*{{box-sizing:border-box}}html,body{{margin:0;min-height:100%;background:#080808;color:#f6f6f2;font-family:Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}body{{min-height:100vh;display:grid;place-items:center;padding:24px;background:radial-gradient(circle at 50% 35%,rgba(121,98,255,.12),transparent 29%),#080808}}main{{width:min(440px,100%);padding:42px;border:1px solid rgba(255,255,255,.1);border-radius:28px;background:rgba(18,18,18,.94);box-shadow:0 40px 100px rgba(0,0,0,.55);text-align:center}}.mark{{position:relative;width:72px;height:78px;margin:0 auto 28px;border-radius:52% 48% 57% 43% / 46% 58% 42% 54%;background:#f4f4f0;animation:float 4.6s ease-in-out infinite}}.mark:before,.mark:after{{content:"";position:absolute;top:34px;width:8px;height:10px;border-radius:999px;background:#101010}}.mark:before{{left:23px}}.mark:after{{right:23px}}.ring{{position:absolute;inset:-10px;border:1px solid rgba(255,255,255,.1);border-radius:47% 53% 50% 50%;animation:orbit 8s linear infinite}}.eyebrow{{margin:0 0 10px;color:#8d7ee8;font-size:10px;font-weight:850;letter-spacing:.17em}}h1{{margin:0;font-size:26px;font-weight:580;letter-spacing:-.035em}}p{{margin:14px auto 0;color:#8f8f8f;font-size:13px;line-height:1.65}}.state{{width:9px;height:9px;display:inline-block;margin-right:7px;border-radius:50%;background:#72d8ad;box-shadow:0 0 0 6px rgba(114,216,173,.08)}}main[data-tone="warn"] .state{{background:#ff9b7f;box-shadow:0 0 0 6px rgba(255,155,127,.08)}}.return{{height:46px;margin-top:26px;display:flex;align-items:center;justify-content:center;border-radius:13px;background:#f0f0ec;color:#101010;text-decoration:none;font-size:13px;font-weight:780}}small{{display:block;margin-top:18px;color:#5f5f5f;font-size:10px;line-height:1.6}}@keyframes float{{0%,100%{{transform:translateY(0) rotate(-2deg)}}50%{{transform:translateY(-5px) rotate(2deg)}}}}@keyframes orbit{{to{{transform:rotate(360deg)}}}}@media(max-width:640px){{html,body{{background:#fafaf7;color:#171717}}body{{display:block;min-height:100svh;padding:0;background:#fafaf7}}main{{min-height:100svh;width:100%;padding:90px 28px max(34px,env(safe-area-inset-bottom));border:0;border-radius:0;background:#fafaf7;box-shadow:none;display:flex;flex-direction:column;justify-content:center}}.mark{{background:#171717}}.mark:before,.mark:after{{background:#fff}}h1{{font-size:30px}}p{{color:#777771}}.return{{background:#171717;color:#fff;height:56px;border-radius:14px}}small{{color:#9a9a94}}}}@media(prefers-reduced-motion:reduce){{*{{animation:none!important}}}}</style></head><body><main data-tone="{tone}"><div class="mark"><i class="ring"></i></div><p class="eyebrow"><span class="state"></span>{eyebrow}</p><h1>{title}</h1><p>{message}</p>{link_markup}<small>如果 Fabushi 没有自动返回，请点击上方按钮；登录结果仍会通过一次性会话安全领取。</small></main><script>{wake_script}</script></body></html>"#,
        tone = tone,
        eyebrow = eyebrow,
        title = if success {
            "登录成功"
        } else {
            "登录未完成"
        },
        message = html_escape(message),
        link_markup = link_markup,
        wake_script = wake_script,
    ))
}

pub(super) async fn refresh_access_token(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let refresh: RefreshAccessRequest = match request.json().await {
        Ok(refresh) => refresh,
        Err(_) => {
            return error_response(400, "invalid_refresh_request", "refresh token is required");
        }
    };
    if !refresh.refresh_token.starts_with("mrt_") || refresh.refresh_token.len() != 68 {
        return error_response(401, "invalid_refresh_token", "登录会话已失效");
    }
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let token_hash = hash_refresh_token(&refresh.refresh_token);
    let row = worker::query!(
        &database,
        "SELECT rt.token_hash, rt.session_id, rt.generation, rt.state,
                s.user_id, s.device_id, s.expires_at AS session_expires_at, s.revoked_at
         FROM account_refresh_tokens rt
         JOIN account_sessions s ON s.session_id = rt.session_id
         WHERE rt.token_hash = ?1
         LIMIT 1",
        &token_hash
    )?
    .first::<RefreshTokenRow>(None)
    .await?;
    let Some(row) = row else {
        return error_response(401, "invalid_refresh_token", "登录会话已失效");
    };
    let now = now_seconds();
    if row.state != "active" {
        revoke_account_session(&database, &row.session_id, "refresh_token_reuse", now).await?;
        return error_response(401, "refresh_token_reused", "登录会话已撤销，请重新登录");
    }
    if row.revoked_at.is_some() || row.session_expires_at <= now {
        return error_response(401, "refresh_token_expired", "登录会话已过期，请重新登录");
    }
    if let Some(device_id) = refresh.device_id.as_deref()
        && device_id != row.device_id
    {
        return error_response(401, "device_mismatch", "登录设备不匹配，请重新登录");
    }
    let user = lookup_account_user_by_id(&database, &row.user_id).await?;
    let Some(user) = user else {
        revoke_account_session(&database, &row.session_id, "account_missing", now).await?;
        return error_response(401, "account_missing", "账号不存在");
    };

    let next_refresh = new_refresh_token();
    let next_hash = hash_refresh_token(&next_refresh);
    let next_generation = row.generation + 1;
    let (access_token, access_expires_at, access_jti) = issue_account_access_token(
        &context.env,
        &row.user_id,
        &row.device_id,
        &row.session_id,
        now,
    )?;
    let statements = vec![
        worker::query!(
            &database,
            "UPDATE account_refresh_tokens
             SET state = 'used', used_at = ?1, replaced_by_hash = ?2
             WHERE token_hash = ?3 AND state = 'active'",
            now,
            &next_hash,
            &row.token_hash
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_refresh_tokens
             (token_hash, session_id, generation, state, issued_at, expires_at)
             VALUES (?1, ?2, ?3, 'active', ?4, ?5)",
            &next_hash,
            &row.session_id,
            next_generation,
            now,
            row.session_expires_at
        )?,
        worker::query!(
            &database,
            "UPDATE account_sessions
             SET current_refresh_token_hash = ?1, last_used_at = ?2
             WHERE session_id = ?3 AND revoked_at IS NULL",
            &next_hash,
            now,
            &row.session_id
        )?,
        worker::query!(
            &database,
            "INSERT INTO account_auth_events
             (event_id, user_id, session_id, event_type, occurred_at, details_json)
             VALUES (?1, ?2, ?3, 'refresh_rotated', ?4, ?5)",
            Uuid::new_v4().to_string(),
            &row.user_id,
            &row.session_id,
            now,
            json!({"generation": next_generation, "accessJti": access_jti}).to_string()
        )?,
    ];
    if database.batch(statements).await.is_err() {
        return error_response(
            409,
            "refresh_conflict",
            "登录会话正在轮换，请使用最新凭据重试",
        );
    }

    account_session_response(
        &user,
        &access_token,
        &next_refresh,
        access_expires_at,
        row.session_expires_at,
        &row.session_id,
        &row.device_id,
    )
}

pub(super) async fn account_user_info(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "登录已过期，请重新登录"),
    };
    let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let Some(user) = lookup_account_user_by_id(&database, &account.user_id).await? else {
        return error_response(404, "account_missing", "账号不存在");
    };
    Ok(Response::from_json(&serialize_account_user(&user))?.with_headers(auth_headers()))
}

pub(super) async fn account_logout(
    request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let account = match authenticated_account(&request, &context.env) {
        Ok(account) => account,
        Err(_) => return error_response(401, "unauthorized", "登录已过期，请重新登录"),
    };
    if let Some(session_id) = account.session_id {
        let database = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
        revoke_account_session(&database, &session_id, "logout", now_seconds()).await?;
    }
    Ok(
        Response::from_json(&json!({"success": true, "loggedIn": false}))?
            .with_headers(auth_headers()),
    )
}

pub(super) async fn lookup_login_user(
    database: &worker::D1Database,
    identifier: &str,
) -> Result<Option<AccountUserRow>> {
    let select = "SELECT u.id, u.user_no, u.username, u.username_changed_at, u.email,
                         u.nickname, u.avatar, u.phone_number, u.firebase_uid,
                         u.alipay_user_id, u.alipay_nickname, u.alipay_avatar,
                         u.wechat_headimgurl, u.password_hash, u.salt, u.iterations, u.algo,
                         c.password_phc AS upgraded_password_phc,
                         u.main_practice_title, u.main_practice_file_path,
                         u.main_practice_selected_at, u.created_at, u.email_verified,
                         u.membership_type, u.membership_expires_at, u.free_trial_end_date
                  FROM users u
                  LEFT JOIN account_password_credentials c ON c.user_id = CAST(u.id AS TEXT)";
    let (where_clause, normalized) = if identifier.contains('@') {
        ("LOWER(u.email) = ?1", identifier.to_ascii_lowercase())
    } else if looks_like_phone(identifier) {
        ("u.phone_number = ?1", identifier.to_string())
    } else {
        ("u.username = ?1", identifier.to_string())
    };
    let query = format!("{select} WHERE {where_clause} LIMIT 1");
    worker::query!(database, &query, normalized)?
        .first::<AccountUserRow>(None)
        .await
}

pub(super) async fn lookup_account_user_by_id(
    database: &worker::D1Database,
    user_id: &str,
) -> Result<Option<AccountUserRow>> {
    worker::query!(
        database,
        "SELECT u.id, u.user_no, u.username, u.username_changed_at, u.email,
                u.nickname, u.avatar, u.phone_number, u.firebase_uid,
                u.alipay_user_id, u.alipay_nickname, u.alipay_avatar,
                u.wechat_headimgurl, u.password_hash, u.salt, u.iterations, u.algo,
                c.password_phc AS upgraded_password_phc,
                u.main_practice_title, u.main_practice_file_path,
                u.main_practice_selected_at, u.created_at, u.email_verified,
                u.membership_type, u.membership_expires_at, u.free_trial_end_date
         FROM users u
         LEFT JOIN account_password_credentials c ON c.user_id = CAST(u.id AS TEXT)
         WHERE CAST(u.id AS TEXT) = ?1 OR u.username = ?1
         LIMIT 1",
        user_id
    )?
    .first::<AccountUserRow>(None)
    .await
}

fn looks_like_phone(value: &str) -> bool {
    let value = value.strip_prefix('+').unwrap_or(value);
    (6..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn normalize_device_id(device_id: Option<&str>) -> Result<String> {
    let device_id = device_id.map(str::trim).filter(|value| !value.is_empty());
    let Some(device_id) = device_id else {
        return Ok(format!("device:{}", Uuid::new_v4()));
    };
    if device_id.len() > 128
        || !device_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(worker::Error::RustError("invalid device id".into()));
    }
    Ok(device_id.to_string())
}

fn issue_account_access_token(
    env: &Env,
    user_id: &str,
    device_id: &str,
    session_id: &str,
    now: i64,
) -> Result<(String, i64, String)> {
    let expires_at = now + ACCESS_TOKEN_SECONDS;
    let (token, jti) = issue_scoped_account_access_token(
        env,
        user_id,
        device_id,
        session_id,
        now,
        expires_at,
        vec![
            "account.read".to_string(),
            "marketplace.read".to_string(),
            "marketplace.publish".to_string(),
            "wallet.read".to_string(),
            "commerce.purchase".to_string(),
            "model.invoke".to_string(),
        ],
        "access",
    )?;
    Ok((token, expires_at, jti))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn issue_scoped_account_access_token(
    env: &Env,
    user_id: &str,
    device_id: &str,
    session_id: &str,
    now: i64,
    expires_at: i64,
    scope: Vec<String>,
    token_use: &str,
) -> Result<(String, String)> {
    if user_id.trim().is_empty()
        || device_id.trim().is_empty()
        || session_id.trim().is_empty()
        || expires_at <= now
        || scope.is_empty()
        || token_use != "access"
    {
        return Err(worker::Error::RustError(
            "invalid scoped access token request".into(),
        ));
    }
    let jti = Uuid::new_v4().to_string();
    let claims = AccountAccessTokenClaims {
        iss: ACCESS_TOKEN_ISSUER.to_string(),
        sub: user_id.to_string(),
        aud: ACCESS_TOKEN_AUDIENCE.to_string(),
        scope,
        device_id: device_id.to_string(),
        sid: session_id.to_string(),
        jti: jti.clone(),
        iat: usize::try_from(now).unwrap_or_default(),
        exp: usize::try_from(expires_at).unwrap_or(usize::MAX),
        token_use: token_use.to_string(),
    };
    let private_key = env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let key = EncodingKey::from_rsa_pem(private_key.as_bytes()).map_err(jwt_error)?;
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("JWT".to_string());
    header.kid = Some(env.var("ACCESS_TOKEN_KEY_ID")?.to_string());
    let token = encode(&header, &claims, &key).map_err(jwt_error)?;
    Ok((token, jti))
}

pub(super) fn serialize_account_user(user: &AccountUserRow) -> serde_json::Value {
    let super_admin = is_builtin_super_admin_account_id(&user.id.to_string());
    let unlimited_usage = is_builtin_unlimited_account_id(&user.id.to_string())
        || is_builtin_unlimited_account_username(&user.username);
    let avatar = user
        .avatar
        .as_ref()
        .or(user.alipay_avatar.as_ref())
        .or(user.wechat_headimgurl.as_ref());
    let main_practice = user.main_practice_title.as_ref().map(|title| {
        json!({
            "title": title,
            "filePath": user.main_practice_file_path,
            "selectedAt": user.main_practice_selected_at,
        })
    });
    let membership = if unlimited_usage {
        json!({
            "type": "lifetime",
            "expiresAt": null,
            "isActive": true,
            "active": true,
        })
    } else {
        json!({
            "type": user.membership_type.as_deref().unwrap_or("expired"),
            "expiresAt": user.membership_expires_at.as_ref().or(user.free_trial_end_date.as_ref()),
        })
    };
    json!({
        "id": user.id,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "username": user.username,
        "usernameChangedAt": user.username_changed_at,
        "email": user.email.as_deref().unwrap_or_default(),
        "nickname": user.nickname.as_deref().unwrap_or(&user.username),
        "avatar": avatar,
        "phoneNumber": user.phone_number,
        "firebaseUid": user.firebase_uid,
        "alipayProviderSubject": user.alipay_user_id,
        "alipayUserId": user.alipay_user_id,
        "alipayNickname": user.alipay_nickname,
        "alipayAvatar": user.alipay_avatar,
        "hasPassword": user.password_hash.is_some() && user.salt.is_some(),
        "mainPractice": main_practice,
        "createdAt": user.created_at,
        "emailVerified": user.email_verified == Some(1),
        "isAdmin": super_admin,
        "role": if super_admin { "super_admin" } else { "user" },
        "unlimitedUsage": unlimited_usage,
        "membership": membership,
    })
}

#[allow(clippy::too_many_arguments)]
fn account_session_response(
    user: &AccountUserRow,
    access_token: &str,
    refresh_token: &str,
    access_expires_at: i64,
    refresh_expires_at: i64,
    session_id: &str,
    device_id: &str,
) -> Result<Response> {
    Ok(Response::from_json(&json!({
        "accessToken": access_token,
        "refreshToken": refresh_token,
        "tokenType": "Bearer",
        "expiresIn": ACCESS_TOKEN_SECONDS,
        "accessTokenExpiresAt": access_expires_at,
        "refreshTokenExpiresAt": refresh_expires_at,
        "sessionId": session_id,
        "deviceId": device_id,
        "username": user.username,
        "userId": user.id,
        "userNo": user.user_no.unwrap_or(user.id),
        "user": serialize_account_user(user),
    }))?
    .with_headers(auth_headers()))
}

pub(super) async fn record_auth_event(
    database: &worker::D1Database,
    user_id: Option<&str>,
    session_id: Option<&str>,
    event_type: &str,
    now: i64,
) -> Result<()> {
    worker::query!(
        database,
        "INSERT INTO account_auth_events
         (event_id, user_id, session_id, event_type, occurred_at, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5, '{}')",
        Uuid::new_v4().to_string(),
        user_id,
        session_id,
        event_type,
        now
    )?
    .run()
    .await?;
    Ok(())
}

pub(super) async fn account_login_is_rate_limited(
    database: &worker::D1Database,
    user_id: &str,
    now: i64,
) -> Result<bool> {
    let window_start = now - LOGIN_FAILURE_WINDOW_SECONDS;
    let count = worker::query!(
        database,
        "SELECT COUNT(*) AS failure_count
         FROM account_auth_events
         WHERE user_id = ?1 AND event_type = 'login_failed' AND occurred_at >= ?2",
        user_id,
        window_start
    )?
    .first::<LoginFailureCountRow>(None)
    .await?
    .map(|row| row.failure_count)
    .unwrap_or_default();
    Ok(count >= MAX_ACCOUNT_LOGIN_FAILURES)
}

pub(super) async fn revoke_account_session(
    database: &worker::D1Database,
    session_id: &str,
    reason: &str,
    now: i64,
) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    database
        .batch(vec![
            worker::query!(
                database,
                "UPDATE account_sessions
                 SET revoked_at = COALESCE(revoked_at, ?1), revoked_reason = COALESCE(revoked_reason, ?2)
                 WHERE session_id = ?3",
                now,
                reason,
                session_id
            )?,
            worker::query!(
                database,
                "UPDATE account_refresh_tokens SET state = 'revoked'
                 WHERE session_id = ?1 AND state = 'active'",
                session_id
            )?,
            worker::query!(
                database,
                "INSERT INTO account_auth_events
                 (event_id, user_id, session_id, event_type, occurred_at, details_json)
                 SELECT ?1, user_id, session_id, ?2, ?3, '{}'
                 FROM account_sessions WHERE session_id = ?4",
                &event_id,
                reason,
                now,
                session_id
            )?,
        ])
        .await?;
    Ok(())
}

async fn ensure_bound_account_session_active(
    env: &Env,
    user_id: &str,
    session_id: &str,
) -> Result<()> {
    #[derive(Debug, Deserialize)]
    struct BoundSessionRow {
        user_id: String,
        expires_at: i64,
        revoked_at: Option<i64>,
    }

    if user_id.trim().is_empty() || session_id.trim().is_empty() {
        return Err(worker::Error::RustError(
            "invalid session-bound account claims".into(),
        ));
    }
    let database = env.d1(ACCOUNT_DATABASE_BINDING)?;
    let row = worker::query!(
        &database,
        "SELECT user_id, expires_at, revoked_at FROM account_sessions
         WHERE session_id = ?1 LIMIT 1",
        session_id
    )?
    .first::<BoundSessionRow>(None)
    .await?;
    let Some(row) = row else {
        return Err(worker::Error::RustError(
            "account session is not active".into(),
        ));
    };
    if row.user_id != user_id || row.revoked_at.is_some() || row.expires_at <= now_seconds() {
        return Err(worker::Error::RustError(
            "account session is not active".into(),
        ));
    }
    Ok(())
}

pub(super) async fn authenticated_session_account(
    request: &Request,
    env: &Env,
) -> Result<AuthenticatedAccount> {
    let account = authenticated_account(request, env)?;
    let Some(session_id) = account.session_id.as_deref() else {
        return Err(worker::Error::RustError(
            "session-backed Fabushi account required".into(),
        ));
    };
    ensure_bound_account_session_active(env, &account.user_id, session_id).await?;
    Ok(account)
}

pub(super) async fn authenticated_plugin_account(
    request: &Request,
    env: &Env,
    plugin_id: &str,
) -> Result<AuthenticatedAccount> {
    if !is_identifier(plugin_id) {
        return Err(worker::Error::RustError(
            "invalid delegated plugin id".into(),
        ));
    }
    let authorization = request
        .headers()
        .get("Authorization")?
        .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| worker::Error::RustError("invalid Authorization scheme".into()))?;
    let public_key = env.secret("ACCESS_TOKEN_PUBLIC_KEY_PEM")?.to_string();
    let key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(jwt_error)?;
    let audience = format!("plugin:{plugin_id}");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ACCESS_TOKEN_ISSUER]);
    validation.set_audience(&[audience.as_str()]);
    let claims = decode::<PluginAccessTokenClaims>(token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    let expected_scope = format!("miniapp:{plugin_id}");
    if claims.token_use != "plugin"
        || claims.sub.trim().is_empty()
        || claims.sid.trim().is_empty()
        || claims.device_id.trim().is_empty()
        || claims.scope.len() != 1
        || claims.scope.first().map(String::as_str) != Some(expected_scope.as_str())
    {
        return Err(worker::Error::RustError(
            "invalid plugin access token claims".into(),
        ));
    }
    ensure_bound_account_session_active(env, &claims.sub, &claims.sid).await?;
    Ok(AuthenticatedAccount {
        user_id: claims.sub,
        session_id: Some(claims.sid),
        scopes: claims.scope,
        is_test_account: false,
    })
}

pub(super) fn authenticated_user(request: &Request, env: &Env) -> Result<String> {
    Ok(authenticated_account(request, env)?.user_id)
}

pub(super) fn authenticated_account(request: &Request, env: &Env) -> Result<AuthenticatedAccount> {
    let authorization = request
        .headers()
        .get("Authorization")?
        .ok_or_else(|| worker::Error::RustError("missing Authorization header".into()))?;
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| worker::Error::RustError("invalid Authorization scheme".into()))?;
    if let Ok(expected) = env.secret("TEST_ACCOUNT_TOKEN")
        && constant_time_eq(token.as_bytes(), expected.to_string().as_bytes())
    {
        return Ok(AuthenticatedAccount {
            user_id: "user:test_account".to_string(),
            session_id: None,
            scopes: vec![
                "marketplace.read".to_string(),
                "marketplace.publish".to_string(),
                "model.invoke".to_string(),
            ],
            is_test_account: true,
        });
    }
    let public_key = env.secret("ACCESS_TOKEN_PUBLIC_KEY_PEM")?.to_string();
    let key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(jwt_error)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[ACCESS_TOKEN_ISSUER]);
    validation.set_audience(&[ACCESS_TOKEN_AUDIENCE]);
    let claims = decode::<AccountAccessTokenClaims>(token, &key, &validation)
        .map_err(jwt_error)?
        .claims;
    if claims.token_use != "access"
        || claims.sub.trim().is_empty()
        || claims.sid.trim().is_empty()
        || claims.device_id.trim().is_empty()
    {
        return Err(worker::Error::RustError(
            "invalid access token claims".into(),
        ));
    }
    Ok(AuthenticatedAccount {
        user_id: claims.sub,
        session_id: Some(claims.sid),
        scopes: claims.scope,
        is_test_account: false,
    })
}
