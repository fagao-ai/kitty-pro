//! Outbound URL validation, secret redaction and subscription download.
//!
//! Split out of `lib.rs`; the crate root re-exports everything through
//! `pub(crate) use ...::*`, so sibling modules keep using `use super::*`.

use super::*;

/// Redact credentials a dependency may have echoed into an error string.
/// reqwest renders request URLs (including `?token=` query strings and
/// `user:pass@` userinfo) into transport errors, and the UI shows those
/// strings verbatim in toasts and `role=alert` panels — a subscription URL
/// is a credential (README: snapshots contain proxy credentials).
/// Must stay available for every target: the wasm client surfaces the same
/// server errors.
pub(crate) fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for scheme in ["https://", "http://"] {
        let mut rest = out.as_str();
        let mut replacement_start = 0usize;
        while let Some(pos) = rest.find(scheme) {
            let abs = replacement_start + pos;
            let after = &out[abs + scheme.len()..];
            let end = after
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '>')
                .unwrap_or(after.len());
            let url = &after[..end];
            let authority = url.split('/').next().unwrap_or("");
            let mut redacted = (*scheme).to_string();
            if let Some(at) = authority.rfind('@') {
                redacted.push_str("[credential-redacted]@");
                redacted.push_str(&authority[at + 1..]);
            } else {
                redacted.push_str(authority);
            }
            if let Some(path) = url.get(authority.len()..) {
                let (path_part, _query) = match path.split_once('?') {
                    Some((p, _)) => (p, ""),
                    None => (path, ""),
                };
                let _ = _query;
                redacted.push_str(path_part);
                if path.contains('?') {
                    redacted.push_str("?…");
                }
            }
            out.replace_range(abs..abs + scheme.len() + url.len(), &redacted);
            replacement_start = abs + redacted.len();
            rest = &out[replacement_start..];
        }
    }
    out
}

/// S4/S5: centralized outbound URL validator. Strict by default: refuses
/// private / loopback / link-local / multicast / unspecified / documentation
/// IPs and local-looking domains, plus any embedded userinfo. `allow_private`
/// is the documented exception for explicitly configured storage endpoints.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_outbound_url(url: &str) -> Result<(), ApiError> {
    validate_outbound_url_inner(url, false)
}

/// Parameterised core of the outbound validator. `allow_private = true` is used
/// by `sync.rs::validate_storage_outbound` for user-configured LAN NAS / MinIO.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_outbound_url_inner(url: &str, allow_private: bool) -> Result<(), ApiError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| api_err(ApiErrorCode::InvalidInput, "出站地址无效"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(api_err(
            ApiErrorCode::InvalidInput,
            "出站地址只支持 HTTP/HTTPS",
        ));
    }
    // Reject userinfo (`user:pass@`) outright.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(api_err(
            ApiErrorCode::InvalidInput,
            "出站地址不允许携带凭据",
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| api_err(ApiErrorCode::InvalidInput, "出站地址缺少主机"))?;

    // Interpret the host as an IP literal when possible (IPv6 arrives with
    // brackets in `host_str()`), otherwise apply the domain rules.
    let ip = host.parse::<std::net::IpAddr>().or_else(|_| {
        host.trim_matches(|c| c == '[' || c == ']')
            .parse::<std::net::IpAddr>()
    });
    match ip {
        Ok(ip) => {
            if ip_is_forbidden(ip, allow_private) {
                return Err(api_err(
                    ApiErrorCode::InvalidInput,
                    "出站地址指向被禁止的网络地址",
                ));
            }
        }
        Err(_) => {
            let lowered = host.trim_end_matches('.').to_ascii_lowercase();
            if lowered == "localhost"
                || lowered.ends_with(".localhost")
                || lowered.ends_with(".local")
                || lowered.ends_with(".internal")
            {
                return Err(api_err(
                    ApiErrorCode::InvalidInput,
                    "出站地址不允许使用本地域名",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ip_is_forbidden(ip: std::net::IpAddr, allow_private: bool) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            let loopback = o[0] == 127;
            let link_local = o[0] == 169 && o[1] == 254; // 169.254.0.0/16 metadata
            let multicast = (224..=239).contains(&o[0]);
            let unspecified = o == [0, 0, 0, 0];
            let documentation = (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 203 && o[1] == 0 && o[2] == 113);
            let private = o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 100 && (64..=127).contains(&o[1])); // CGNAT
            loopback
                || link_local
                || multicast
                || unspecified
                || documentation
                || (!allow_private && private)
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            let unspecified = s == [0, 0, 0, 0, 0, 0, 0, 0];
            let loopback = s == [0, 0, 0, 0, 0, 0, 0, 1];
            let link_local = (s[0] & 0xffc0) == 0xfe80; // fe80::/10
            let multicast = (s[0] & 0xff00) == 0xff00; // ff00::/8
            let documentation = s[0] == 0x2001 && s[1] == 0x0db8; // 2001:db8::/32
            let unique_local = (s[0] & 0xfe00) == 0xfc00; // fc00::/7 (private)
                                                          // IPv4-mapped / compatible addresses must validate the embedded v4.
            let mapped_v4 = if s[0] == 0
                && s[1] == 0
                && s[2] == 0
                && s[3] == 0
                && s[4] == 0
                && (s[5] == 0 || s[5] == 0xffff)
            {
                Some(std::net::Ipv4Addr::new(
                    (s[6] >> 8) as u8,
                    s[6] as u8,
                    (s[7] >> 8) as u8,
                    s[7] as u8,
                ))
            } else {
                None
            };
            unspecified
                || loopback
                || link_local
                || multicast
                || documentation
                || (!allow_private && unique_local)
                || mapped_v4
                    .is_some_and(|v4| ip_is_forbidden(std::net::IpAddr::V4(v4), allow_private))
        }
    }
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn download_subscription(url: &str) -> Result<String, ApiError> {
    use std::time::Duration;

    // S4: validate the entry URL and every redirect hop before any request.
    validate_outbound_url(url)?;

    let parsed = reqwest::Url::parse(url)
        .map_err(|error| api_err(ApiErrorCode::InvalidInput, format!("订阅地址无效: {error}")))?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("重定向次数过多");
            }
            let allowed = validate_outbound_url(attempt.url().as_str()).is_ok();
            if allowed {
                attempt.follow()
            } else {
                attempt.error("重定向目标不在允许范围")
            }
        }))
        .user_agent("clash.meta kitty-pro/0.1")
        .build()
        .map_err(|error| api_err(ApiErrorCode::Network, error.to_string()))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| api_err(ApiErrorCode::Network, format!("订阅下载失败: {}", error)))?
        .error_for_status()
        .map_err(|error| {
            api_err(
                ApiErrorCode::Network,
                format!("订阅服务返回错误: {}", error),
            )
        })?;

    if response
        .content_length()
        .is_some_and(|size| size > MAX_SUBSCRIPTION_BYTES as u64)
    {
        return Err(api_err(ApiErrorCode::Network, "订阅内容超过 10 MiB 限制"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| api_err(ApiErrorCode::Network, format!("读取订阅失败: {}", error)))?;
    if bytes.len() > MAX_SUBSCRIPTION_BYTES {
        return Err(api_err(ApiErrorCode::Network, "订阅内容超过 10 MiB 限制"));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| api_err(ApiErrorCode::Network, "订阅内容不是有效的 UTF-8 文本"))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) async fn download_subscription(_url: &str) -> Result<String, ApiError> {
    Err(api_err(
        ApiErrorCode::Internal,
        "订阅下载只能由原生服务端执行",
    ))
}
